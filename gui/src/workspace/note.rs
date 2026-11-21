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
//!
//! **Child modules** (in `note/`). Each owns one part of the steps above.
//! `open` chooses where an open goes: a group, a new tab, a split or the
//! queue. `switch` runs one switch in a group: flush, read and load.
//! `leave` lets the open note go for another panel and empties a group.
//! `save` owns the autosave tick, the Save chord and the write report.
//! `footer` fetches the counts and the git state that the Footer shows.
//! `close` owns the flush and the warning when the window closes or the app
//! quits.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::time::Duration;

use gpui_kit::{App, Context, Entity, Task};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::doc_meta::DocMeta;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::session::{NoteSession, SaveOutcome};

use super::Workspace;
use crate::editor_view::EditorView;

pub(super) use self::leave::{LeavingFor, NoteLeft};

mod close;
mod footer;
mod leave;
mod open;
mod save;
mod switch;

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
    /// The source line (0-based) the caret goes to once the wanted note
    /// has loaded ([`Workspace::open_note_at`]); replaced by every open.
    wanted_line: Option<usize>,
    /// The line of the note being read, applied once it has loaded.
    reading_line: Option<usize>,
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
    /// Test-only: how often an autosave tick looked at this group, and how
    /// many saves a tick began for it.
    #[cfg(test)]
    pub(crate) tick_checks: usize,
    #[cfg(test)]
    pub(crate) tick_writes: usize,
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
            wanted_line: None,
            reading_line: None,
            switching: None,
            save_again: false,
            discard_expiry: None,
            #[cfg(test)]
            tick_checks: 0,
            #[cfg(test)]
            tick_writes: 0,
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
        self.target_ref().cloned()
    }

    /// Forget the wanted note, with its search needles and its line.
    fn drop_wanted(&mut self) {
        self.wanted = None;
        self.wanted_emphasis = None;
        self.wanted_line = None;
    }

    /// [`Self::target`], borrowed.
    pub(crate) fn target_ref(&self) -> Option<&VaultPath> {
        self.wanted
            .as_ref()
            .or(self.session.pending_open())
            .or(self.note.as_ref())
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

impl Workspace {
    /// Whether the open note may be written now: a note is open and the
    /// vault is not read-only.
    pub(crate) fn can_save(&self) -> bool {
        self.can_save_in(self.layout.active_group())
    }

    /// [`Self::can_save`] for the note `group` holds.
    pub(super) fn can_save_in(&self, group: GroupId) -> bool {
        !self.status.read_only && self.host_in(group).is_some_and(|h| h.note.is_some())
    }

    pub(super) fn editor_dirty(&self, cx: &App) -> bool {
        self.active_state().editor.read(cx).editor.is_dirty()
    }

    /// [`Self::editor_dirty`] for the editor of `group`.
    pub(super) fn editor_dirty_in(&self, group: GroupId, cx: &App) -> bool {
        self.state(group)
            .is_some_and(|s| s.editor.read(cx).editor.is_dirty())
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

    /// Why the edits in the editor of `group` cannot be written, for a flash.
    pub(super) fn unsaveable_in(&self, group: GroupId) -> String {
        match self.host_in(group).and_then(|h| h.note.as_ref()) {
            Some(note) if self.status.read_only => {
                format!("Read-only: your edits to {note} can't be saved.")
            }
            Some(note) => format!("Your edits to {note} can't be saved."),
            None => "Your text is not in a note and can't be saved.".into(),
        }
    }

    /// The note session of `group`, if it has one.
    pub(super) fn host_in(&self, group: GroupId) -> Option<&NoteHost> {
        self.state(group).and_then(|s| s.notes.as_ref())
    }

    /// The note `group` has open, as a failure message names it: the vault
    /// root when it has none.
    pub(super) fn note_name_in(&self, group: GroupId) -> VaultPath {
        self.host_in(group)
            .and_then(|h| h.note.clone())
            .unwrap_or_else(VaultPath::root)
    }

    /// The note session of `group`, mutable, if it has one.
    pub(super) fn host_in_mut(&mut self, group: GroupId) -> Option<&mut NoteHost> {
        self.state_mut(group).and_then(|s| s.notes.as_mut())
    }

    /// (Re)start the autosave tick of `group`'s session.
    pub(super) fn restart_tick_in(&mut self, group: GroupId) {
        if let Some((host, tx)) = self.host_and_tx_in(group) {
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

    /// The note session of `group`, mutable, and the app channel. `None`
    /// when either is missing.
    pub(super) fn host_and_tx_in(&mut self, group: GroupId) -> Option<(&mut NoteHost, &AppTx)> {
        let host = self.groups.get_mut(&group).and_then(|s| s.notes.as_mut())?;
        Some((host, self.tx.as_ref()?))
    }

    /// [`Self::host_and_tx_in`] for the active group.
    pub(super) fn active_host_and_tx(&mut self) -> Option<(&mut NoteHost, &AppTx)> {
        let host = Self::active_in(&mut self.groups, &self.layout)
            .notes
            .as_mut()?;
        Some((host, self.tx.as_ref()?))
    }

    /// The editor of `group` and its note session, mutable. `None` when
    /// the group or its session is missing.
    pub(super) fn editor_and_host_in(
        &mut self,
        group: GroupId,
    ) -> Option<(&Entity<EditorView>, &mut NoteHost)> {
        let state = self.groups.get_mut(&group)?;
        Some((&state.editor, state.notes.as_mut()?))
    }
}
