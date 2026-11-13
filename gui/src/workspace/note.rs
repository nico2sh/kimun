//! The **note session**'s GPUI host (M5b-2 G2): open → edit → save.
//!
//! The logic [`NoteSession`] owns the rules (one writer at a time, saves
//! declining while an open is pending, stale completions); this file drives
//! its prepare / run / apply steps from GPUI. Each `run` future is spawned on
//! tokio (the main thread has the runtime entered) and its `JoinHandle` is
//! awaited in a `cx.spawn`, so no borrow of the session or the buffer lives
//! across an await, and every wake-up comes from the thread driving tokio.
//!
//! **Opening.** One switch runs at a time: save the open note (a flush), then
//! read the new one and load it. An open asked for meanwhile replaces the
//! wanted note and runs when the current switch ends. A flush that fails
//! keeps the old note open (its edits stay in the buffer) and says why. A
//! note that is not there is offered for creation (the create dialog); one
//! that would not read is flashed. Either way the editor stays on the note
//! it had. An open from a search (the search browser) carries the query's
//! needles: once the note has loaded the editor emphasises them, until the
//! first edit (the logic editor's rule).
//!
//! **Saving.** The periodic tick and the Save chord write in the background
//! and report on the app channel; a failed write is flashed with core's
//! error. Nothing is written while the vault is read-only (a case conflict)
//! or before a note was opened.
//!
//! **Closing.** Closing the window with something to save flushes first
//! (bounded by the session's timeouts) and closes once it is written. A
//! failed flush, or edits that cannot be saved (read-only, no note), keep
//! the window open with a flash saying so; closing again discards them.
//! What the close ends is [`Workspace::closes_app`]: on macOS only a quit
//! (Cmd+Q, the palette, the leader) ends the app, and the close button leaves
//! it in the dock to open again; elsewhere any close ends it.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;

use gpui_kit::{App, AppContext, Context, SharedString, Task, Window};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::doc_meta::DocMeta;
use kimun_notes::logic::layout_tree::{Axis, GroupId};
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::session::{
    FlushResult, NoteRead, NoteSession, OpenOutcome, SaveCompleted, SaveOutcome,
};
use kimun_notes::util::git_status::GitSummary;
use tokio::task::JoinError;

use super::Workspace;
use super::group::active_mut;
use super::session::QueuedOpen;

/// How long the "close again to discard" warning holds: a close within it
/// (and before any new edit) discards the unsaved edits. Its own clock, not
/// the toast's: the kit's toast can be paused by hovering, closed early or
/// never shown (no toast layer), and none of that may move this window. The
/// same five seconds as the kit's toast autohide, so the two usually end
/// together.
pub(crate) const DISCARD_WINDOW: Duration = Duration::from_secs(5);

/// The note session and the switch in progress. `Some` on the workspace only
/// when a vault is open.
pub(crate) struct NoteHost {
    pub(crate) session: NoteSession,
    /// The note the editor holds (the last open that succeeded), `None`
    /// before the first one. Saves need it.
    pub(crate) note: Option<VaultPath>,
    /// The note asked for, not yet being read.
    wanted: Option<VaultPath>,
    /// The search needles the wanted note opens with (an open from a
    /// search); replaced by every open.
    wanted_emphasis: Option<Vec<String>>,
    /// The needles of the note being read, applied once it has loaded.
    reading_emphasis: Option<Vec<String>>,
    /// The switch under way (its flush, then its read). Dropped with the
    /// workspace; the tokio halves finish on their own.
    switching: Option<Task<()>>,
    /// A Save chord found a background save in flight: save again once it
    /// reports, so the edits made since are written too.
    save_again: bool,
    /// Lifts [`Closing::Discard`] when its warning's time is up.
    discard_expiry: Option<Task<()>>,
    /// The open note's backlink and property counts for the Footer,
    /// fetched in the background on open, save and file operations.
    pub(crate) meta: DocMeta,
}

impl NoteHost {
    pub(crate) fn new(session: NoteSession) -> Self {
        let meta = DocMeta::new(session.vault().clone());
        Self {
            meta,
            session,
            note: None,
            wanted: None,
            wanted_emphasis: None,
            reading_emphasis: None,
            switching: None,
            save_again: false,
            discard_expiry: None,
        }
    }

    /// Whether a switch (flush or read) is under way.
    pub(crate) fn is_switching(&self) -> bool {
        self.switching.is_some()
    }

    /// Whether an open was asked for and waits for the switch under way.
    pub(crate) fn is_open_wanted(&self) -> bool {
        self.wanted.is_some()
    }

    /// Whether this group holds `path`: the note it shows, or the one it
    /// was asked to open or is opening (so a note still on its way counts).
    pub(crate) fn holds(&self, path: &VaultPath) -> bool {
        [
            self.note.as_ref(),
            self.wanted.as_ref(),
            self.session.pending_open(),
        ]
        .into_iter()
        .flatten()
        .any(|held| held.is_like(path))
    }

    /// The note this group is on as the session record keeps it: the one
    /// asked for or being read (a note still on its way counts as where the
    /// group goes), else the one it shows. `None` for an empty group, and
    /// for one showing an attachment (it holds no note).
    pub(crate) fn target(&self) -> Option<VaultPath> {
        self.wanted
            .as_ref()
            .or(self.session.pending_open())
            .or(self.note.as_ref())
            .cloned()
    }
}

/// Where closing the window stands.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) enum Closing {
    /// Not closing.
    #[default]
    Open,
    /// Asked while a switch ran: the switch hands over to the close flush
    /// when it ends.
    Requested,
    /// The close flush is running; the window closes when it has written.
    Flushing,
    /// The user was told the edits cannot be saved: the next close
    /// discards them, as long as every group is still the one it was, at
    /// the revision it was at ([`EditorRevisions`]), and the warning's
    /// flash is up. A later successful save clears it too.
    Discard(EditorRevisions),
    /// Everything is written: the window may go.
    Closed,
}

/// Where the text of every group stands: each group with its editor's
/// content revision. Any edit in any group, and any group added or taken
/// away, makes it differ; where the groups sit does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorRevisions(HashMap<GroupId, NonZeroU64>);

/// Why a flush wrote nothing, for a flash; `None` when it did its job.
pub(super) fn flush_problem(outcome: &SaveOutcome) -> Option<String> {
    match outcome {
        SaveOutcome::Saved(_) | SaveOutcome::Clean => None,
        SaveOutcome::Failed(e) => Some(e.clone()),
        SaveOutcome::InFlightTimedOut => Some("an earlier save did not finish in time".into()),
        SaveOutcome::FlushPending => Some("another save is still running".into()),
    }
}

/// Whether a buffer still dirty after this flush may be flushed again: the
/// flush wrote (`Saved`) or had nothing to capture yet (`Clean`).
fn rewritable(outcome: Option<&SaveOutcome>) -> bool {
    matches!(outcome, Some(SaveOutcome::Saved(_) | SaveOutcome::Clean))
}

/// The panel the open note is left for ([`Workspace::leave_note`]): each
/// holds its flush in its own slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeavingFor {
    Attachment,
}

/// How leaving the open note went ([`Workspace::leave_note`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoteLeft {
    /// Nothing had to be written: the panel may show now.
    Now,
    /// The flush wrote everything: the panel may show.
    Flushed,
    /// The note stays (the flush failed, or a note was asked for
    /// meanwhile); its tick is back.
    Stays,
}

impl Workspace {
    /// Whether the open note may be written now: a note is open and the
    /// vault is not read-only.
    pub(crate) fn can_save(&self) -> bool {
        self.can_save_in(self.layout.active_group())
    }

    /// [`Self::can_save`] for the note `group` holds.
    pub(super) fn can_save_in(&self, group: GroupId) -> bool {
        !self.status.read_only
            && self
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .is_some_and(|h| h.note.is_some())
    }

    pub(super) fn editor_dirty(&self, cx: &App) -> bool {
        self.active_state().editor.read(cx).editor.is_dirty()
    }

    /// [`Self::editor_dirty`] for the editor of `group`.
    pub(super) fn editor_dirty_in(&self, group: GroupId, cx: &App) -> bool {
        self.state(group)
            .is_some_and(|s| s.editor.read(cx).editor.is_dirty())
    }

    /// Where the text of every group stands ([`EditorRevisions`]).
    fn editors_revision(&self, cx: &App) -> EditorRevisions {
        EditorRevisions(
            self.group_ids()
                .into_iter()
                .filter_map(|group| {
                    self.state(group)
                        .map(|s| (group, s.editor.read(cx).editor.content_revision()))
                })
                .collect(),
        )
    }

    /// The user was just told the edits cannot be saved: the next close
    /// discards them, if it comes before any new edit (in any group) and
    /// within [`DISCARD_WINDOW`].
    fn arm_discard(&mut self, cx: &mut Context<Self>) {
        let revision = self.editors_revision(cx);
        self.closing = Closing::Discard(revision.clone());
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DISCARD_WINDOW).await;
            this.update(cx, |workspace, _| {
                if workspace.closing == Closing::Discard(revision) {
                    workspace.closing = Closing::Open;
                }
            })
            .ok();
        });
        if let Some(host) = self.active_state_mut().notes.as_mut() {
            host.discard_expiry = Some(expiry);
        }
    }

    /// Keep `group`'s editor from taking input while an open (or a flush) is
    /// pending: the keys would be lost when the new note loads, so they are
    /// not taken at all.
    pub(super) fn block_input_in(&self, group: GroupId, blocked: bool, cx: &mut Context<Self>) {
        if let Some(state) = self.state(group) {
            state
                .editor
                .update(cx, |view, _| view.input_blocked = blocked);
        }
    }

    /// Why the edits in the editor cannot be written, for a flash.
    pub(super) fn unsaveable(&self) -> String {
        self.unsaveable_in(self.layout.active_group())
    }

    /// [`Self::unsaveable`] for the editor of `group`.
    pub(super) fn unsaveable_in(&self, group: GroupId) -> String {
        match self
            .state(group)
            .and_then(|s| s.notes.as_ref())
            .and_then(|h| h.note.as_ref())
        {
            Some(note) if self.status.read_only => {
                format!("Read-only: your edits to {note} can't be saved.")
            }
            Some(note) => format!("Your edits to {note} can't be saved."),
            None => "Your text is not in a note and can't be saved.".into(),
        }
    }

    // ---- Opening -------------------------------------------------------------

    /// The group holding `path` ([`NoteHost::holds`]), in any tab. A note
    /// is open in at most one group (ADR-0058).
    pub(crate) fn locate_note(&self, path: &VaultPath) -> Option<GroupId> {
        self.group_where(|host| host.holds(path))
    }

    /// Open the note at `path` in the active group's editor: save the open
    /// one, then read and load this one (the attachment view, if one shows,
    /// gives way once it has loaded). A note another group holds is not
    /// opened again: that group's tab shows and its editor takes the
    /// keyboard. A directory (the root, when the history is empty) or an
    /// attachment is not a note: nothing happens.
    pub(crate) fn open_note(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        self.open_note_with(path, None, cx);
    }

    /// Open the note at `path` in a new tab, after the last one: its one
    /// group starts empty, becomes the active group with the keyboard, and
    /// the note opens into it. A note already open somewhere is focused
    /// there instead ([`Self::focus_group`]); no tab is made.
    #[allow(
        dead_code,
        reason = "the tab and split chords (a later task) call it; tests use it now"
    )]
    pub(crate) fn open_note_in_new_tab(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        // A stored record still restoring would replace the new tab: the
        // open waits for it ([`super::session`]).
        if self.session_restoring() && self.may_open(&path) {
            self.queue_open(path, None, QueuedOpen::NewTab);
            return;
        }
        if let Some(group) = self.locate_note(&path) {
            self.focus_group(group, cx);
            return;
        }
        if !self.may_open(&path) {
            return;
        }
        let Some(state) = self.new_group_state(cx) else {
            return;
        };
        let (_, group) = self.layout.region_mut().new_tab();
        // Another tab shows: a zoom ends with the one it showed.
        self.layout.zoom(None);
        self.insert_group(group, state, cx);
        self.focus_group(group, cx);
        self.open_note_in(group, path, cx);
    }

    /// Open the note at `path` in a new group split off the active one
    /// along `axis`, right after it: the group starts empty, becomes the
    /// active group with the keyboard, and the note opens into it. A note
    /// already open somewhere is focused there instead; no group is made.
    /// Refused with a flash while the shown tab's close writes its notes.
    #[allow(
        dead_code,
        reason = "the tab and split chords (a later task) call it; tests use it now"
    )]
    pub(crate) fn open_note_in_split(
        &mut self,
        path: VaultPath,
        axis: Axis,
        cx: &mut Context<Self>,
    ) {
        // As for a new tab: the split waits for a restore on its way.
        if self.session_restoring() && self.may_open(&path) {
            self.queue_open(path, None, QueuedOpen::Split(axis));
            return;
        }
        if let Some(group) = self.locate_note(&path) {
            self.focus_group(group, cx);
            return;
        }
        if !self.may_open(&path) {
            return;
        }
        if self.tab_mid_close(self.layout.region().shown()) {
            self.flash_error(super::tab_strip::TAB_CLOSING, cx);
            return;
        }
        let Some(state) = self.new_group_state(cx) else {
            return;
        };
        let active = self.layout.active_group();
        let Some(group) = self.layout.region_mut().split_group(active, axis) else {
            return;
        };
        self.insert_group(group, state, cx);
        self.focus_group(group, cx);
        self.open_note_in(group, path, cx);
    }

    /// Whether an open of `path` may start: it is a note and the window is
    /// not closing.
    fn may_open(&self, path: &VaultPath) -> bool {
        path.is_note() && !matches!(self.closing, Closing::Requested | Closing::Flushing)
    }

    /// [`Self::open_note`] in the editor of `group`, active or not, with no
    /// one-copy check: the caller made sure no other group holds `path`.
    pub(crate) fn open_note_in(&mut self, group: GroupId, path: VaultPath, cx: &mut Context<Self>) {
        self.open_note_in_with(group, path, None, cx);
    }

    /// [`Self::open_note`], the note's text then emphasising `emphasis` (the
    /// search needles of the query it was found by) until the first edit.
    pub(crate) fn open_note_with(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        let group = self.layout.active_group();
        // One copy: a note open in another group is shown there. The
        // active group's own note opens again, as it always did.
        if let Some(holder) = self.locate_note(&path)
            && holder != group
        {
            self.focus_group(holder, cx);
            return;
        }
        self.open_note_in_with(group, path, emphasis, cx);
    }

    fn open_note_in_with(
        &mut self,
        group: GroupId,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if !self.may_open(&path) {
            return;
        }
        // The session record is still being restored: it wins, and this
        // open goes into its active group once it landed (the newest
        // request kept, [`super::session`]).
        if self.session_restoring() {
            self.queue_open(path, emphasis, QueuedOpen::Plain);
            return;
        }
        // The newest request wins: an attachment still being read is dropped.
        self.forget_attachment_request(group);
        let Some(host) = self.state_mut(group).and_then(|s| s.notes.as_mut()) else {
            return;
        };
        host.wanted = Some(path);
        host.wanted_emphasis = emphasis;
        // A file operation or a property write holds the open note (flushed,
        // its tick stopped): the open waits for it to end
        // ([`Self::file_op_ended`]).
        let held = self.note_held();
        let Some(host) = self.state_mut(group).and_then(|s| s.notes.as_mut()) else {
            return;
        };
        if !host.is_switching() && !held {
            self.switch(group, cx);
        }
    }

    /// Start the switch to the wanted note: flush the open one first.
    pub(super) fn switch(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let can_save = self.can_save_in(group);
        let dirty = self.editor_dirty_in(group, cx);
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(host) = state.notes.as_mut() else {
            return;
        };
        if host.wanted.is_none() {
            return;
        }
        let flush = if can_save {
            match host
                .session
                .prepare_flush(Some(&state.editor.read(cx).editor))
            {
                Ok(flush) => Some(flush),
                // One switch or close at a time holds the only flush.
                Err(_) => {
                    tracing::warn!("note switch: a flush is still outstanding");
                    return;
                }
            }
        } else {
            None
        };
        host.switching = Some(cx.spawn(async move |this, cx| {
            let flushed = match flush {
                Some(flush) => Some(tokio::spawn(flush.run()).await),
                None => None,
            };
            this.update(cx, |workspace, cx| {
                workspace.switch_flushed(group, flushed, cx)
            })
            .ok();
        }));
        if !can_save && dirty {
            let text = format!("{} They were not kept.", self.unsaveable());
            self.flash_error(text, cx);
        }
    }

    /// Apply a finished flush to the session and the buffer.
    pub(super) fn apply_flush(
        &mut self,
        group: GroupId,
        result: Result<FlushResult, JoinError>,
        cx: &mut Context<Self>,
    ) -> SaveOutcome {
        let Some(state) = self.groups.get_mut(&group) else {
            return SaveOutcome::Clean;
        };
        let Some(host) = state.notes.as_mut() else {
            return SaveOutcome::Clean;
        };
        let outcome = match result {
            Ok(result) => state.editor.update(cx, |view, cx| {
                let outcome = host.session.apply_flush(result, Some(&mut view.editor));
                cx.notify();
                outcome
            }),
            Err(e) => SaveOutcome::Failed(e.to_string()),
        };
        if matches!(outcome, SaveOutcome::Saved(_)) && matches!(self.closing, Closing::Discard(_)) {
            self.closing = Closing::Open;
        }
        if let SaveOutcome::Saved(saved) = &outcome {
            self.files_note_saved(saved, cx);
            if self
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.note.as_ref())
                == Some(&saved.path)
            {
                self.refresh_note_properties_in(group);
            }
        }
        outcome
    }

    /// The switch's flush ended: read the wanted note, unless the flush
    /// failed (stay, edits kept) or the window is closing.
    fn switch_flushed(
        &mut self,
        group: GroupId,
        flushed: Option<Result<FlushResult, JoinError>>,
        cx: &mut Context<Self>,
    ) {
        let outcome = flushed.map(|result| self.apply_flush(group, result, cx));
        let dirty = self.editor_dirty_in(group, cx);
        let can_save = self.can_save_in(group);
        let Some(host) = self.groups.get_mut(&group).and_then(|s| s.notes.as_mut()) else {
            return;
        };
        host.switching = None;
        if let Some(problem) = outcome.as_ref().and_then(flush_problem) {
            host.wanted = None;
            host.wanted_emphasis = None;
            let note = host.note.clone().unwrap_or_else(VaultPath::root);
            self.flash_error(
                format!("Couldn't save {note}: {problem}. It stays open."),
                cx,
            );
            self.continue_close(cx);
            return;
        }
        if self.closing == Closing::Requested {
            host.wanted = None;
            host.wanted_emphasis = None;
            self.close_flush(cx);
            return;
        }
        // Typed while the flush ran: write that too before leaving. Only
        // when that flush could capture the buffer: never on a pending open
        // (it captures nothing, and the loop would never end).
        if can_save && dirty && rewritable(outcome.as_ref()) && !host.session.is_open_pending() {
            self.switch(group, cx);
            return;
        }
        let Some(path) = host.wanted.take() else {
            return;
        };
        host.reading_emphasis = host.wanted_emphasis.take();
        let read = host.session.begin_open(path);
        host.switching = Some(cx.spawn(async move |this, cx| {
            let read = tokio::spawn(read.run()).await;
            this.update(cx, |workspace, cx| workspace.switch_read(group, read, cx))
                .ok();
        }));
        self.block_input_in(group, true, cx);
    }

    /// The wanted note was read: load it (history, autosave tick), or stay
    /// on the open note and say why not.
    pub(crate) fn switch_read(
        &mut self,
        group: GroupId,
        read: Result<NoteRead, JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(tx) = self.tx.clone() else {
            return;
        };
        let read_only = self.status.read_only;
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(host) = state.notes.as_mut() else {
            return;
        };
        host.switching = None;
        let emphasis = host.reading_emphasis.take();
        let read = match read {
            Ok(read) => read,
            Err(e) => {
                // The read never came back: drop the pending open, so the
                // note the buffer holds saves again.
                tracing::error!("note read failed to finish: {e}");
                host.session.cancel_open();
                self.block_input_in(group, false, cx);
                self.flash_error(format!("Couldn't open the note: {e}"), cx);
                self.after_switch(group, cx);
                return;
            }
        };
        self.block_input_in(group, false, cx);
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(host) = state.notes.as_mut() else {
            return;
        };
        let previous = host.note.clone();
        let outcome = state.editor.update(cx, |view, cx| {
            let outcome = host.session.finish_open(read, Some(&mut view.editor), &tx);
            if matches!(outcome, OpenOutcome::Opened(_)) {
                // A composition in progress belonged to the old text.
                view.preedit = None;
                // After the load, so the new text's revision owns them.
                if let Some(needles) = emphasis {
                    view.editor.set_search_needles(needles);
                }
            }
            cx.notify();
            outcome
        });
        match outcome {
            OpenOutcome::Opened(path) => {
                if read_only {
                    host.session.stop_autosave();
                }
                host.note = Some(path.clone());
                host.meta.load_note_counts(&path, &tx);
                self.forget_note_gone(group);
                // A menu on the group names the note it showed before.
                if previous.as_ref() != Some(&path) {
                    self.forget_note_menu_of(group, cx);
                }
                if group == self.layout.active_group() {
                    self.status.note = Some(path.clone());
                    self.files_note_opened(&path, cx);
                    // The note takes the editor Panel back from an attachment.
                    self.clear_attachment(cx);
                } else if let Some(state) = self.state_mut(group) {
                    // Not on display: its attachment goes quietly.
                    state._attachment_events = None;
                    state.attachment = None;
                }
            }
            // The editor still holds the note it had: point the session back
            // at it, so nothing is saved under the missing path.
            // The create dialog offers to make it (the TUI's rule).
            OpenOutcome::NotFound(path) => {
                host.session
                    .set_path(previous.unwrap_or_else(VaultPath::root));
                self.offer_create(path, cx);
            }
            OpenOutcome::ReadFailed { path, .. } => {
                host.session
                    .set_path(previous.unwrap_or_else(VaultPath::root));
                self.flash_error(format!("Couldn't open {path} (see the log)"), cx);
            }
            OpenOutcome::Superseded => {}
        }
        self.after_switch(group, cx);
    }

    /// A switch ended: hand over to a waiting close, or to the next open.
    fn after_switch(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self.closing == Closing::Requested {
            self.close_flush(cx);
        } else if self
            .state(group)
            .and_then(|s| s.notes.as_ref())
            .is_some_and(|h| h.wanted.is_some())
        {
            self.switch(group, cx);
        }
        // The first open settled (or failed): FILES may pick its folder;
        // LINKS and OUTLINE follow the note while shown.
        self.ensure_files_listing(cx);
        self.refresh_views(cx);
        cx.notify();
    }

    // ---- Leaving the note for another panel -------------------------------

    /// Leave the note `group` holds for a panel that is not a note (an
    /// attachment), then `then`: flushed first, the tick stopped and the
    /// editor taking no input while it runs, and refused when the note
    /// cannot be let go — a change still being written, a note still
    /// opening, edits that can't be saved, the window closing — or the
    /// flush fails. A refusal flashes why, naming `name` (what was not
    /// opened), and the note stays with its edits, its tick and the
    /// keyboard. `then` runs at once ([`NoteLeft::Now`]) when there is
    /// nothing to write, else when the flush ends, followed by any open
    /// that waited for it. `panel` says which task slot holds the flush.
    /// `then` lets the note go itself ([`Self::release_note_in`]).
    pub(super) fn leave_note(
        &mut self,
        group: GroupId,
        panel: LeavingFor,
        name: SharedString,
        then: impl FnOnce(&mut Self, NoteLeft, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let can_save = self.can_save_in(group);
        if let Some(block) = self.flush_block_in(group, cx) {
            self.flash_error(format!("Couldn't open {name}: {}.", block.reason()), cx);
            return;
        }
        if self.state(group).and_then(|s| s.notes.as_ref()).is_none() {
            return;
        }
        if !can_save {
            then(self, NoteLeft::Now, window, cx);
            return;
        }
        // No tick from here: the flush is the last write of the note.
        let Some(flush) = self.begin_note_flush_in(group, cx) else {
            let text = format!("Couldn't open {name}: another save is still running.");
            self.flash_error(text, cx);
            return;
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            let flushed = tokio::spawn(flush.run()).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.note_left(group, panel, name, then, flushed, window, cx)
            })
            .ok();
        });
        if let Some(slot) = self.leaving_slot(group, panel) {
            *slot = Some(task);
        }
        cx.notify();
    }

    /// The flush [`Self::leave_note`] started ended: `then` hears whether
    /// everything is written ([`NoteLeft::Flushed`]) or the note stays (the
    /// flush failed, flashed; or a note was asked for meanwhile, which
    /// opens instead), its tick back.
    #[allow(clippy::too_many_arguments)]
    fn note_left(
        &mut self,
        group: GroupId,
        panel: LeavingFor,
        name: SharedString,
        then: impl FnOnce(&mut Self, NoteLeft, &mut Window, &mut Context<Self>),
        flushed: Result<FlushResult, JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(slot) = self.leaving_slot(group, panel) {
            *slot = None;
        }
        let outcome = self.apply_flush(group, flushed, cx);
        let problem = self.flush_failure_in(group, &outcome, cx);
        self.block_input_in(group, false, cx);
        let host = self.state(group).and_then(|s| s.notes.as_ref());
        let note_wanted = host.is_some_and(|h| h.is_open_wanted());
        if let Some(problem) = problem {
            let note = host
                .and_then(|h| h.note.clone())
                .unwrap_or_else(VaultPath::root);
            self.restart_tick_unless_read_only_in(group);
            self.flash_error(
                format!("Couldn't save {note}: {problem}. {name} was not opened."),
                cx,
            );
            then(self, NoteLeft::Stays, window, cx);
        } else if note_wanted {
            // A note was asked for meanwhile: it opens instead.
            self.restart_tick_unless_read_only_in(group);
            then(self, NoteLeft::Stays, window, cx);
        } else {
            then(self, NoteLeft::Flushed, window, cx);
        }
        // An open that waited for the flush goes now.
        self.file_op_ended(cx);
    }

    /// The task slot holding the flush of a leave for `panel` in `group`.
    fn leaving_slot(&mut self, group: GroupId, panel: LeavingFor) -> Option<&mut Option<Task<()>>> {
        let state = self.state_mut(group)?;
        Some(match panel {
            LeavingFor::Attachment => &mut state.attachment_flushing,
        })
    }

    /// Let the note `group` holds (already flushed) go: no session path, no
    /// tick, an empty clean buffer, so nothing can save while another
    /// panel shows; for the active group the status line names no note.
    pub(super) fn release_note_in(&mut self, group: GroupId, cx: &mut Context<Self>) {
        self.empty_group(group, cx);
        self.forget_note_gone(group);
    }

    /// `group` no longer says its note went away (a note opened there, it
    /// was let go on purpose, or it closed).
    pub(super) fn forget_note_gone(&mut self, group: GroupId) {
        if self.note_gone.as_ref().is_some_and(|(g, _)| *g == group) {
            self.note_gone = None;
        }
    }

    /// Make `group` an empty group where it is: every writer of its note
    /// stopped, no session path, no note, an empty clean buffer that takes
    /// input. Nothing is saved: the caller flushed it, or its note is gone.
    /// The Footer names no note when `group` is the active one.
    pub(super) fn empty_group(&mut self, group: GroupId, cx: &mut Context<Self>) {
        // A menu on the group names the note it no longer holds.
        self.forget_note_menu_of(group, cx);
        let Some(state) = self.state_mut(group) else {
            return;
        };
        if let Some(host) = state.notes.as_mut() {
            host.session.stop_autosave();
            host.session.abort_autosave();
            host.session.set_path(VaultPath::root());
            host.note = None;
        }
        state.editor.update(cx, |view, cx| {
            view.editor.set_text(String::new());
            view.editor.mark_saved(String::new());
            view.preedit = None;
            view.input_blocked = false;
            cx.notify();
        });
        if group == self.layout.active_group() {
            self.status.note = None;
        }
    }

    // ---- Saving --------------------------------------------------------------

    /// The periodic tick: write the note of every group in the background
    /// if dirty (each group's session ticks, and each tick writes them all:
    /// a group with nothing new to write declines).
    pub(crate) fn autosave(&mut self, cx: &mut Context<Self>) {
        // Never while a file operation holds the note: it was flushed, and the
        // path the save would bake in may be renamed or deleted under it.
        // Nor while a property write or its reload does: the buffer's
        // frontmatter is about to be replaced by the disk's.
        if self.note_held() {
            return;
        }
        let Some(tx) = self.tx.clone() else {
            return;
        };
        for group in self.group_ids() {
            if !self.can_save_in(group) {
                continue;
            }
            if let Some(state) = self.groups.get_mut(&group)
                && let Some(host) = state.notes.as_mut()
            {
                host.session
                    .spawn_autosave(Some(&state.editor.read(cx).editor), &tx);
            }
        }
    }

    /// The Save chord: write the open note now (in the background, as the
    /// tick does). Already saved: nothing to do, the Footer says "Saved".
    pub(crate) fn save_note(&mut self, cx: &mut Context<Self>) {
        self.save_note_in(self.layout.active_group(), cx);
    }

    /// [`Self::save_note`] for the note `group` holds.
    fn save_note_in(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let Some(note) = self
            .state(group)
            .and_then(|s| s.notes.as_ref())
            .and_then(|h| h.note.clone())
        else {
            return;
        };
        if self.status.read_only {
            let text = format!(
                "Read-only: {note} can't be saved until the names that differ only by case are fixed."
            );
            self.flash_error(text, cx);
            return;
        }
        if !self.editor_dirty_in(group, cx) || self.note_held() {
            return;
        }
        let Some(tx) = self.tx.clone() else {
            return;
        };
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        if let Some(host) = state.notes.as_mut()
            && !host
                .session
                .spawn_autosave(Some(&state.editor.read(cx).editor), &tx)
        {
            // A save is in flight (or a switch's flush, which saves anyway):
            // go again when it reports.
            host.save_again = true;
        }
    }

    /// The group whose note session a save of `path` was made for: the one
    /// whose session names it (a note is open in one group), else the
    /// active one (a stale report, which marks nothing).
    fn group_saving(&self, path: &VaultPath) -> GroupId {
        self.group_where(|h| h.session.path() == path)
            .unwrap_or_else(|| self.layout.active_group())
    }

    /// A background save reported: mark the buffer of the group that saved
    /// it saved (if it still holds what was written), flash a failure.
    pub(crate) fn autosave_completed(&mut self, done: SaveCompleted, cx: &mut Context<Self>) {
        let group = self.group_saving(&done.path);
        let active = group == self.layout.active_group();
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(host) = state.notes.as_mut() else {
            return;
        };
        let path = done.path.clone();
        let completion = state.editor.update(cx, |view, cx| {
            let completion = host
                .session
                .on_autosave_completed(done, Some(&mut view.editor));
            cx.notify();
            completion
        });
        let again = std::mem::take(&mut host.save_again);
        if let Some(saved) = &completion.saved {
            self.files_note_saved(saved, cx);
        }
        // The Footer's property count follows what was written (the TUI's
        // rule: the open note's completion, written or not).
        if completion.is_open_note {
            self.refresh_note_properties_in(group);
        }
        // A write changed the working tree.
        if completion.wrote {
            self.refresh_git();
        }
        if completion.wrote && matches!(self.closing, Closing::Discard(_)) {
            self.closing = Closing::Open;
        }
        // OUTLINE follows the headings the open note was saved with.
        if active && completion.wrote && completion.is_open_note && completion.error.is_none() {
            self.outline_saved(cx);
        }
        if let Some(error) = completion.error {
            self.flash_error(format!("Couldn't save {path}: {error}"), cx);
        } else if again {
            self.save_note_in(group, cx);
        }
    }

    // ---- The Footer's counts ----------------------------------------------

    /// Re-read the property count of the note `group` holds (it was saved).
    pub(super) fn refresh_note_properties_in(&mut self, group: GroupId) {
        if let (Some(host), Some(tx)) = (
            self.groups.get_mut(&group).and_then(|s| s.notes.as_mut()),
            self.tx.as_ref(),
        ) && let Some(note) = host.note.clone()
        {
            host.meta.refresh_properties(&note, tx);
        }
    }

    /// A note, attachment or folder was renamed, moved or deleted: links to
    /// the open note may have been rewritten or gone with it. Re-read both
    /// counts, keeping the ones shown until they land.
    pub(crate) fn refresh_note_counts(&mut self) {
        if let (Some(host), Some(tx)) = (active_mut!(self).notes.as_mut(), self.tx.as_ref())
            && let Some(note) = host.note.clone()
        {
            host.meta.refresh_backlinks(&note, tx);
            host.meta.refresh_properties(&note, tx);
        }
    }

    /// Re-read the workspace's git summary (throttled; a no-op with no
    /// repository to look at).
    pub(crate) fn refresh_git(&mut self) {
        if let (Some(host), Some(tx)) = (active_mut!(self).notes.as_mut(), self.tx.as_ref()) {
            host.meta.refresh_git(tx);
        }
    }

    /// A git summary fetch landed: the Footer shows what the note meta keeps
    /// (a failed fetch keeps the last known state).
    pub(crate) fn git_loaded(
        &mut self,
        root: &Path,
        status: Option<GitSummary>,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.active_state_mut().notes.as_mut() else {
            return;
        };
        host.meta.git_loaded(root, status);
        let shown = host.meta.git().copied();
        if self.status.git != shown {
            self.status.git = shown;
            cx.notify();
        }
    }

    /// Leader `g s`: look again at the footer's summary, and toast the full
    /// state: the branch, how far it is from its upstream, and the changed
    /// paths by kind. Read off the UI thread; the toast arrives on the app
    /// channel.
    pub(crate) fn show_git_status(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(tx)) = (active_mut!(self).notes.as_mut(), self.tx.clone()) else {
            return;
        };
        // The one `git status` run answers both: the toast and the footer.
        host.meta.begin_git_probe();
        let root = host
            .session
            .vault()
            .workspace_path()
            .as_path()
            .to_path_buf();
        tokio::spawn(async move {
            let detail = kimun_notes::util::git_status::detail(root.clone()).await;
            let text = match &detail {
                Some(detail) => detail.describe(),
                None => "Not a git repository".to_string(),
            };
            tx.send(AppMessage::GitStatusLoaded {
                root,
                status: detail.as_ref().map(GitSummary::from),
            })
            .ok();
            tx.send(AppMessage::FlashMessage(text)).ok();
        });
        cx.notify();
    }

    /// A count fetched in the background: kept when it is for the open note.
    /// It goes to the group holding the note it counts, active or not.
    pub(crate) fn note_count_loaded(&mut self, message: AppMessage) {
        let counted = match &message {
            AppMessage::BacklinkCountLoaded { path, .. }
            | AppMessage::PropertyCountLoaded { path, .. } => Some(path),
            _ => None,
        };
        let group = counted
            .and_then(|path| self.group_where(|h| h.note.as_ref() == Some(path)))
            .unwrap_or_else(|| self.layout.active_group());
        if let Some(host) = self.state_mut(group).and_then(|s| s.notes.as_mut())
            && let Some(note) = host.note.as_ref()
        {
            // Only the counts are routed here: nothing comes back.
            host.meta.handle(message, note);
        }
    }

    // ---- Closing -------------------------------------------------------------

    /// Ask the window to check with the workspace before it closes (see the
    /// module docs), and warn in the log when the app quits another way with
    /// unsaved edits.
    pub(crate) fn guard_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window = Some(window.window_handle());
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            // The operating system's close button (or Cmd+W): the window, not the app.
            this.update(cx, |workspace, cx| {
                workspace.set_close_intent(false);
                let close = workspace.should_close(cx);
                // Off macOS the app ends with its window. The window goes by itself
                // now that the guard said yes: only the app has to be asked to end.
                if close && workspace.closes_app() {
                    workspace.end_app(cx);
                } else if close {
                    workspace.flush_layout();
                }
                close
            })
            .unwrap_or(true)
        });
        self._on_quit = Some(cx.on_app_quit(|workspace, cx| {
            for group in workspace.group_ids_active_first() {
                if !workspace.editor_dirty_in(group, cx) {
                    continue;
                }
                let note = workspace
                    .state(group)
                    .and_then(|s| s.notes.as_ref())
                    .and_then(|h| h.note.as_ref())
                    .map_or_else(|| "the editor".to_string(), ToString::to_string);
                tracing::warn!("quitting with unsaved edits to {note}");
                eprintln!("kimun-gui: quitting with unsaved edits to {note}");
            }
            // The session's and the layout's last writes land before the
            // app goes.
            workspace.write_session_now();
            let layout_writer = workspace.layout_writer.clone();
            async move { layout_writer.flush() }
        }));
    }

    /// Close the window as its close button does: through [`Self::should_close`],
    /// so the open note is flushed first and the flush closes the window once
    /// written. Whether the app ends with it is [`Self::closes_app`].
    pub(crate) fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_close_intent(false);
        self.begin_close(window, cx);
    }

    /// Record what the close about to start is for: a window close (`false`) or a
    /// quit (`true`). Every way in states its own intent, so a quit that was refused
    /// cannot turn a later window close into a quit. While a close is already under
    /// way (waiting on a switch, or on its flush) only a quit can change it, to a
    /// quit: a window close arriving then must not downgrade a quit, or the flush
    /// would end with only the window gone and the app left in the dock.
    fn set_close_intent(&mut self, quit: bool) {
        let under_way = matches!(self.closing, Closing::Requested | Closing::Flushing);
        self.quit_on_close = if under_way {
            self.quit_on_close || quit
        } else {
            quit
        };
    }

    fn begin_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(cx) {
            self.finish_close(window, cx);
        }
    }

    /// Quit the app (the Quit chord, the palette's quit, the leader's quit): the
    /// same close, and the app ends with the window on every platform.
    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_close_intent(true);
        self.begin_close(window, cx);
    }

    /// Whether the close in progress ends the app: a quit always does, a window
    /// close only off macOS (where the app stays in the dock to open again).
    pub(crate) fn closes_app(&self) -> bool {
        self.quit_on_close || !self.stays_in_dock
    }

    /// The close is allowed and nothing is left to write: end the app, or just
    /// the window. Quitting rather than removing the window keeps this workspace
    /// alive for the app's quit hook (the layout's last write).
    fn finish_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closes_app() {
            self.end_app(cx);
        } else {
            self.flush_layout();
            window.remove_window();
        }
    }

    /// Wait for the layout's last write before this window goes. Removing the window
    /// drops the workspace, and with it the app-quit hook that flushes the writer: the
    /// window the app opens next from the dock reads the config file, and it must find
    /// the layout this one left.
    fn flush_layout(&mut self) {
        #[cfg(test)]
        {
            self.layout_flushes += 1;
        }
        // The tabs and groups as the window leaves them, at once.
        self.write_session_now();
        self.layout_writer.flush();
    }

    /// End the app. The one place that asks the platform to quit, so the tests
    /// can count the asks (the test platform's quit does nothing).
    fn end_app(&mut self, cx: &mut App) {
        #[cfg(test)]
        {
            self.quits += 1;
        }
        // The tabs and groups as the app leaves them, at once (the quit
        // hook writes them again, for a quit that does not come by here).
        self.write_session_now();
        // An update replaced this app: start it again as this one ends. A
        // failure only costs the user the manual start.
        if std::mem::take(&mut self.updates.restart_pending)
            && let Err(e) = self.updates.relauncher.relaunch_after_exit()
        {
            tracing::warn!("could not restart after the update: {e}");
        }
        cx.quit();
    }

    /// Whether the window may close now. With edits to write, in any group,
    /// it starts the close flush and says no; the flush closes the window
    /// when done.
    pub(crate) fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        // An update installing (or the note's flush before it): refused and
        // said, not queued, as Preferences refuse it while they index.
        if self.update_busy() {
            self.flash_error(kimun_notes::logic::update::CLOSE_REFUSED_TEXT, cx);
            return false;
        }
        // Unsaved preferences ask first, as their Cancel does.
        if !self.preferences_allow_close(cx) {
            return false;
        }
        match &self.closing {
            Closing::Requested | Closing::Flushing => return false,
            Closing::Closed => return true,
            // Warned, and nothing typed since: the edits go.
            Closing::Discard(revision) if *revision == self.editors_revision(cx) => return true,
            // New edits since the warning: start over.
            Closing::Discard(_) => self.closing = Closing::Open,
            Closing::Open => {}
        }
        // No vault: the editor is not even shown.
        if self.active_state().notes.is_none() {
            return true;
        }
        let groups = self.group_ids_active_first();
        // A switch under way in a group that saves: the close waits for it.
        if groups
            .iter()
            .any(|g| self.can_save_in(*g) && self.host_in(*g).is_some_and(NoteHost::is_switching))
        {
            self.stop_ticks();
            self.closing = Closing::Requested;
            return false;
        }
        let to_write = groups.iter().any(|g| {
            self.can_save_in(*g)
                && (self.editor_dirty_in(*g, cx)
                    || self
                        .host_in(*g)
                        .is_some_and(|h| h.session.is_autosave_in_flight()))
        });
        if to_write {
            self.close_flush(cx);
            return false;
        }
        // Nothing can be written: edits that cannot be saved warn first.
        if let Some(group) = groups
            .iter()
            .copied()
            .find(|g| !self.can_save_in(*g) && self.editor_dirty_in(*g, cx))
        {
            let text = format!("{} Close again to discard them.", self.unsaveable_in(group));
            self.arm_discard(cx);
            self.flash_error(text, cx);
            return false;
        }
        for group in groups {
            if self.can_save_in(group)
                && let Some(host) = self.state_mut(group).and_then(|s| s.notes.as_mut())
            {
                host.session.stop_autosave();
            }
        }
        true
    }

    /// The note session of `group`, if it has one.
    pub(super) fn host_in(&self, group: GroupId) -> Option<&NoteHost> {
        self.state(group).and_then(|s| s.notes.as_ref())
    }

    /// Stop the autosave tick of every group.
    fn stop_ticks(&mut self) {
        for state in self.groups.values_mut() {
            if let Some(host) = state.notes.as_mut() {
                host.session.stop_autosave();
            }
        }
    }

    /// Start the autosave tick of every group again.
    fn restart_ticks(&mut self) {
        for group in self.group_ids() {
            self.restart_tick_in(group);
        }
    }

    /// A switch ended while a close waited on it, without leaving the open
    /// note clean to close on: give up this close (the flash says why). The
    /// next close tries the flush again, and warns before it discards.
    fn continue_close(&mut self, cx: &mut Context<Self>) {
        if self.closing == Closing::Requested {
            self.closing = Closing::Open;
            self.restart_ticks();
            cx.notify();
        }
    }

    /// (Re)start the autosave tick of `group`'s session.
    pub(super) fn restart_tick_in(&mut self, group: GroupId) {
        if let (Some(host), Some(tx)) = (
            self.groups.get_mut(&group).and_then(|s| s.notes.as_mut()),
            self.tx.as_ref(),
        ) {
            host.session.restart_autosave(tx);
        }
    }

    /// [`Self::restart_tick_in`], unless the vault is read-only (no tick
    /// runs there).
    pub(super) fn restart_tick_unless_read_only_in(&mut self, group: GroupId) {
        if !self.status.read_only {
            self.restart_tick_in(group);
        }
    }

    /// Flush the note of every group that saves before the window closes
    /// (all at once); close once everything is written. A switch still
    /// under way in one of them is waited for first.
    fn close_flush(&mut self, cx: &mut Context<Self>) {
        let groups: Vec<GroupId> = self
            .group_ids_active_first()
            .into_iter()
            .filter(|g| self.can_save_in(*g))
            .collect();
        if groups
            .iter()
            .any(|g| self.host_in(*g).is_some_and(NoteHost::is_switching))
        {
            self.closing = Closing::Requested;
            return;
        }
        self.stop_ticks();
        let mut flushes = Vec::new();
        for group in groups {
            let Some(state) = self.groups.get_mut(&group) else {
                continue;
            };
            let Some(host) = state.notes.as_mut() else {
                continue;
            };
            match host
                .session
                .prepare_flush(Some(&state.editor.read(cx).editor))
            {
                Ok(flush) => flushes.push((group, flush)),
                Err(_) => {
                    // The flushes prepared before it go unrun, given back
                    // so their background saves finish; every group gets
                    // its tick back, and the flash says why the window
                    // stays.
                    tracing::warn!("close: a flush is still outstanding");
                    for (begun, flush) in flushes {
                        self.give_back_flush_in(begun, flush);
                    }
                    self.closing = Closing::Open;
                    self.restart_ticks();
                    self.flash_error(
                        "Another save is still running. The window stays open.".to_string(),
                        cx,
                    );
                    return;
                }
            }
        }
        self.closing = Closing::Flushing;
        if let Some(note) = flushes
            .first()
            .and_then(|(group, _)| self.host_in(*group))
            .and_then(|h| h.note.clone())
        {
            self.flash(format!("Saving {note}…"), cx);
        }
        let window = self.window;
        // Detached: removing the window drops the workspace, which must not
        // cancel the task that removes it.
        cx.spawn(async move |this, cx| {
            let results = super::file_ops::run_all(flushes).await;
            // `Some(true)`: the app was asked to end; `Some(false)`: only the window
            // goes; `None`: the close did not go through.
            let ended = this
                .update(cx, |workspace, cx| {
                    if !workspace.close_flushed(results, cx) {
                        return None;
                    }
                    let quit = workspace.closes_app();
                    if quit {
                        workspace.end_app(cx);
                    } else {
                        workspace.flush_layout();
                    }
                    Some(quit)
                })
                .unwrap_or(None);
            if ended == Some(false)
                && let Some(window) = window
            {
                cx.update_window(window, |_, window, _| window.remove_window())
                    .ok();
            }
        })
        .detach();
    }

    /// The close flush ended: close when everything is written; otherwise
    /// stay open and say why (closing again discards).
    fn close_flushed(
        &mut self,
        results: Vec<(GroupId, Result<FlushResult, JoinError>)>,
        cx: &mut Context<Self>,
    ) -> bool {
        let outcomes: Vec<(GroupId, SaveOutcome)> = results
            .into_iter()
            .map(|(group, result)| (group, self.apply_flush(group, result, cx)))
            .collect();
        if let Some((group, problem)) = outcomes
            .iter()
            .find_map(|(group, outcome)| flush_problem(outcome).map(|p| (*group, p)))
        {
            let note = self
                .host_in(group)
                .and_then(|h| h.note.clone())
                .unwrap_or_else(VaultPath::root);
            self.closing = Closing::Open;
            self.restart_ticks();
            self.arm_discard(cx);
            self.flash_error(
                format!("Couldn't save {note}: {problem}. Close again to discard your edits."),
                cx,
            );
            return false;
        }
        let dirty: Vec<GroupId> = self
            .group_ids_active_first()
            .into_iter()
            .filter(|g| self.editor_dirty_in(*g, cx))
            .collect();
        if !dirty.is_empty() {
            // Typed while the flush ran: write that too. Only when that flush
            // could capture the buffer (never on a pending open, which would
            // spin): otherwise say so and let the next close discard.
            let stuck = dirty.iter().copied().find(|group| {
                let flushed = outcomes
                    .iter()
                    .find(|(g, _)| g == group)
                    .map(|(_, outcome)| outcome);
                let pending = self
                    .host_in(*group)
                    .is_some_and(|h| h.session.is_open_pending());
                !rewritable(flushed) || pending
            });
            match stuck {
                None => self.close_flush(cx),
                Some(group) => {
                    self.closing = Closing::Open;
                    self.restart_ticks();
                    let text =
                        format!("{} Close again to discard them.", self.unsaveable_in(group));
                    self.arm_discard(cx);
                    self.flash_error(text, cx);
                }
            }
            return false;
        }
        self.closing = Closing::Closed;
        true
    }
}
