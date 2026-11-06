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

use std::num::NonZeroU64;
use std::time::Duration;

use gpui_kit::{App, AppContext, Context, SharedString, Task, Window};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::doc_meta::DocMeta;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::session::{
    FlushResult, NoteRead, NoteSession, OpenOutcome, SaveCompleted, SaveOutcome,
};
use tokio::task::JoinError;

use super::Workspace;

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
}

/// Where closing the window stands.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
    /// discards them, as long as the buffer is still at this revision and
    /// the warning's flash is up. A later successful save clears it too.
    Discard(NonZeroU64),
    /// Everything is written: the window may go.
    Closed,
}

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
    Ask,
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
        !self.status.read_only && self.notes.as_ref().is_some_and(|h| h.note.is_some())
    }

    pub(super) fn editor_dirty(&self, cx: &App) -> bool {
        self.editor.read(cx).editor.is_dirty()
    }

    fn editor_revision(&self, cx: &App) -> NonZeroU64 {
        self.editor.read(cx).editor.content_revision()
    }

    /// The user was just told the edits cannot be saved: the next close
    /// discards them, if it comes before any new edit and within
    /// [`DISCARD_WINDOW`].
    fn arm_discard(&mut self, cx: &mut Context<Self>) {
        let revision = self.editor_revision(cx);
        self.closing = Closing::Discard(revision);
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DISCARD_WINDOW).await;
            this.update(cx, |workspace, _| {
                if workspace.closing == Closing::Discard(revision) {
                    workspace.closing = Closing::Open;
                }
            })
            .ok();
        });
        if let Some(host) = self.notes.as_mut() {
            host.discard_expiry = Some(expiry);
        }
    }

    /// Keep the editor from taking input while an open is pending: the keys
    /// would be lost when the new note loads, so they are not taken at all.
    pub(super) fn block_input(&self, blocked: bool, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |view, _| view.input_blocked = blocked);
    }

    /// Why the edits in the editor cannot be written, for a flash.
    pub(super) fn unsaveable(&self) -> String {
        match self.notes.as_ref().and_then(|h| h.note.as_ref()) {
            Some(note) if self.status.read_only => {
                format!("Read-only: your edits to {note} can't be saved.")
            }
            Some(note) => format!("Your edits to {note} can't be saved."),
            None => "Your text is not in a note and can't be saved.".into(),
        }
    }

    // ---- Opening -------------------------------------------------------------

    /// Open the note at `path` in the editor: save the open one, then read
    /// and load this one (the attachment view, if one shows, gives way once
    /// it has loaded). A directory (the root, when the history is empty) or
    /// an attachment is not a note: nothing happens.
    pub(crate) fn open_note(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        self.open_note_with(path, None, cx);
    }

    /// [`Self::open_note`], the note's text then emphasising `emphasis` (the
    /// search needles of the query it was found by) until the first edit.
    pub(crate) fn open_note_with(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if !path.is_note() || matches!(self.closing, Closing::Requested | Closing::Flushing) {
            return;
        }
        // The newest request wins: an attachment still being read is dropped.
        self.forget_attachment_request();
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        host.wanted = Some(path);
        host.wanted_emphasis = emphasis;
        // A file operation or a property write holds the open note (flushed,
        // its tick stopped): the open waits for it to end
        // ([`Self::file_op_ended`]).
        let held = self.note_held();
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        if !host.is_switching() && !held {
            self.switch(cx);
        }
    }

    /// Start the switch to the wanted note: flush the open one first.
    pub(super) fn switch(&mut self, cx: &mut Context<Self>) {
        let can_save = self.can_save();
        let dirty = self.editor_dirty(cx);
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        if host.wanted.is_none() {
            return;
        }
        let flush = if can_save {
            match host
                .session
                .prepare_flush(Some(&self.editor.read(cx).editor))
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
            this.update(cx, |workspace, cx| workspace.switch_flushed(flushed, cx))
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
        result: Result<FlushResult, JoinError>,
        cx: &mut Context<Self>,
    ) -> SaveOutcome {
        let Some(host) = self.notes.as_mut() else {
            return SaveOutcome::Clean;
        };
        let outcome = match result {
            Ok(result) => self.editor.update(cx, |view, cx| {
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
            if self.notes.as_ref().and_then(|h| h.note.as_ref()) == Some(&saved.path) {
                self.refresh_note_properties();
            }
        }
        outcome
    }

    /// The switch's flush ended: read the wanted note, unless the flush
    /// failed (stay, edits kept) or the window is closing.
    fn switch_flushed(
        &mut self,
        flushed: Option<Result<FlushResult, JoinError>>,
        cx: &mut Context<Self>,
    ) {
        let outcome = flushed.map(|result| self.apply_flush(result, cx));
        let dirty = self.editor_dirty(cx);
        let can_save = self.can_save();
        let Some(host) = self.notes.as_mut() else {
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
            self.switch(cx);
            return;
        }
        let Some(path) = host.wanted.take() else {
            return;
        };
        host.reading_emphasis = host.wanted_emphasis.take();
        let read = host.session.begin_open(path);
        host.switching = Some(cx.spawn(async move |this, cx| {
            let read = tokio::spawn(read.run()).await;
            this.update(cx, |workspace, cx| workspace.switch_read(read, cx))
                .ok();
        }));
        self.block_input(true, cx);
    }

    /// The wanted note was read: load it (history, autosave tick), or stay
    /// on the open note and say why not.
    pub(crate) fn switch_read(
        &mut self,
        read: Result<NoteRead, JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(tx) = self.tx.clone() else {
            return;
        };
        let read_only = self.status.read_only;
        let Some(host) = self.notes.as_mut() else {
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
                self.block_input(false, cx);
                self.flash_error(format!("Couldn't open the note: {e}"), cx);
                self.after_switch(cx);
                return;
            }
        };
        self.block_input(false, cx);
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        let previous = host.note.clone();
        let outcome = self.editor.update(cx, |view, cx| {
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
                self.note_gone = None;
                self.status.note = Some(path.clone());
                self.files_note_opened(&path, cx);
                // The note takes the editor Panel back from the Thread or
                // an attachment (the Thread's conversation is kept).
                self.hide_ask(cx);
                self.clear_attachment(cx);
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
        self.after_switch(cx);
    }

    /// A switch ended: hand over to a waiting close, or to the next open.
    fn after_switch(&mut self, cx: &mut Context<Self>) {
        if self.closing == Closing::Requested {
            self.close_flush(cx);
        } else if self.notes.as_ref().is_some_and(|h| h.wanted.is_some()) {
            self.switch(cx);
        }
        // The first open settled (or failed): FILES may pick its folder;
        // LINKS and OUTLINE follow the note while shown.
        self.ensure_files_listing(cx);
        self.refresh_views(cx);
        cx.notify();
    }

    // ---- Leaving the note for another panel -------------------------------

    /// Leave the open note for a panel that is not a note (an attachment,
    /// the Thread), then `then`: flushed first, the tick stopped and the
    /// editor taking no input while it runs, and refused when the note
    /// cannot be let go — a change still being written, a note still
    /// opening, edits that can't be saved, the window closing — or the
    /// flush fails. A refusal flashes why, naming `name` (what was not
    /// opened), and the note stays with its edits, its tick and the
    /// keyboard. `then` runs at once ([`NoteLeft::Now`]) when there is
    /// nothing to write, else when the flush ends, followed by any open
    /// that waited for it. `panel` says which task slot holds the flush.
    /// `then` lets the note go itself ([`Self::release_note`]).
    pub(super) fn leave_note(
        &mut self,
        panel: LeavingFor,
        name: SharedString,
        then: impl FnOnce(&mut Self, NoteLeft, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let can_save = self.can_save();
        if let Some(block) = self.flush_block(cx) {
            self.flash_error(format!("Couldn't open {name}: {}.", block.reason()), cx);
            return;
        }
        if self.notes.as_ref().is_none() {
            return;
        }
        if !can_save {
            then(self, NoteLeft::Now, window, cx);
            return;
        }
        // No tick from here: the flush is the last write of the note.
        let Some(flush) = self.begin_note_flush(cx) else {
            let text = format!("Couldn't open {name}: another save is still running.");
            self.flash_error(text, cx);
            return;
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            let flushed = tokio::spawn(flush.run()).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.note_left(panel, name, then, flushed, window, cx)
            })
            .ok();
        });
        *self.leaving_slot(panel) = Some(task);
        cx.notify();
    }

    /// The flush [`Self::leave_note`] started ended: `then` hears whether
    /// everything is written ([`NoteLeft::Flushed`]) or the note stays (the
    /// flush failed, flashed; or a note was asked for meanwhile, which
    /// opens instead), its tick back.
    fn note_left(
        &mut self,
        panel: LeavingFor,
        name: SharedString,
        then: impl FnOnce(&mut Self, NoteLeft, &mut Window, &mut Context<Self>),
        flushed: Result<FlushResult, JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        *self.leaving_slot(panel) = None;
        let outcome = self.apply_flush(flushed, cx);
        let problem = self.flush_failure(&outcome, cx);
        self.block_input(false, cx);
        let note_wanted = self.notes.as_ref().is_some_and(|h| h.is_open_wanted());
        if let Some(problem) = problem {
            let note = self
                .notes
                .as_ref()
                .and_then(|h| h.note.clone())
                .unwrap_or_else(VaultPath::root);
            self.restart_tick_unless_read_only();
            self.flash_error(
                format!("Couldn't save {note}: {problem}. {name} was not opened."),
                cx,
            );
            then(self, NoteLeft::Stays, window, cx);
        } else if note_wanted {
            // A note was asked for meanwhile: it opens instead.
            self.restart_tick_unless_read_only();
            then(self, NoteLeft::Stays, window, cx);
        } else {
            then(self, NoteLeft::Flushed, window, cx);
        }
        // An open that waited for the flush goes now.
        self.file_op_ended(cx);
    }

    /// The task slot holding the flush of a leave for `panel`.
    fn leaving_slot(&mut self, panel: LeavingFor) -> &mut Option<Task<()>> {
        match panel {
            LeavingFor::Attachment => &mut self.attachment_flushing,
            LeavingFor::Ask => &mut self.ask_flushing,
        }
    }

    /// Let the open note (already flushed) go: no session path, no tick, an
    /// empty clean buffer, so nothing can save while another panel shows;
    /// the status line names no note.
    pub(super) fn release_note(&mut self, cx: &mut Context<Self>) {
        if let Some(host) = self.notes.as_mut() {
            host.session.stop_autosave();
            host.session.abort_autosave();
            host.session.set_path(VaultPath::root());
            host.note = None;
        }
        self.editor.update(cx, |view, cx| {
            view.editor.set_text(String::new());
            view.editor.mark_saved(String::new());
            view.preedit = None;
            view.input_blocked = false;
            cx.notify();
        });
        self.status.note = None;
        self.note_gone = None;
    }

    // ---- Saving --------------------------------------------------------------

    /// The periodic tick: write the open note in the background if dirty.
    pub(crate) fn autosave(&mut self, cx: &mut Context<Self>) {
        // Never while a file operation holds the note: it was flushed, and the
        // path the save would bake in may be renamed or deleted under it.
        // Nor while a property write or its reload does: the buffer's
        // frontmatter is about to be replaced by the disk's.
        if !self.can_save() || self.note_held() {
            return;
        }
        let Some(tx) = self.tx.clone() else {
            return;
        };
        if let Some(host) = self.notes.as_mut() {
            host.session
                .spawn_autosave(Some(&self.editor.read(cx).editor), &tx);
        }
    }

    /// The Save chord: write the open note now (in the background, as the
    /// tick does). Already saved: nothing to do, the Footer says "Saved".
    pub(crate) fn save_note(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.notes.as_ref().and_then(|h| h.note.clone()) else {
            return;
        };
        if self.status.read_only {
            let text = format!(
                "Read-only: {note} can't be saved until the names that differ only by case are fixed."
            );
            self.flash_error(text, cx);
            return;
        }
        if !self.editor_dirty(cx) || self.note_held() {
            return;
        }
        let Some(tx) = self.tx.clone() else {
            return;
        };
        if let Some(host) = self.notes.as_mut()
            && !host
                .session
                .spawn_autosave(Some(&self.editor.read(cx).editor), &tx)
        {
            // A save is in flight (or a switch's flush, which saves anyway):
            // go again when it reports.
            host.save_again = true;
        }
    }

    /// A background save reported: mark the buffer saved (if it still holds
    /// what was written), flash a failure.
    pub(crate) fn autosave_completed(&mut self, done: SaveCompleted, cx: &mut Context<Self>) {
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        let path = done.path.clone();
        let completion = self.editor.update(cx, |view, cx| {
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
            self.refresh_note_properties();
        }
        if completion.wrote && matches!(self.closing, Closing::Discard(_)) {
            self.closing = Closing::Open;
        }
        // OUTLINE follows the headings the open note was saved with.
        if completion.wrote && completion.is_open_note && completion.error.is_none() {
            self.outline_saved(cx);
        }
        if let Some(error) = completion.error {
            self.flash_error(format!("Couldn't save {path}: {error}"), cx);
        } else if again {
            self.save_note(cx);
        }
    }

    // ---- The Footer's counts ----------------------------------------------

    /// Re-read the open note's property count (it was saved).
    pub(super) fn refresh_note_properties(&mut self) {
        if let (Some(host), Some(tx)) = (self.notes.as_mut(), self.tx.as_ref())
            && let Some(note) = host.note.clone()
        {
            host.meta.refresh_properties(&note, tx);
        }
    }

    /// A note, attachment or folder was renamed, moved or deleted: links to
    /// the open note may have been rewritten or gone with it. Re-read both
    /// counts, keeping the ones shown until they land.
    pub(crate) fn refresh_note_counts(&mut self) {
        if let (Some(host), Some(tx)) = (self.notes.as_mut(), self.tx.as_ref())
            && let Some(note) = host.note.clone()
        {
            host.meta.refresh_backlinks(&note, tx);
            host.meta.refresh_properties(&note, tx);
        }
    }

    /// A count fetched in the background: kept when it is for the open note.
    pub(crate) fn note_count_loaded(&mut self, message: AppMessage) {
        if let Some(host) = self.notes.as_mut()
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
            this.update(cx, |workspace, cx| workspace.should_close(cx))
                .unwrap_or(true)
        });
        self._on_quit = Some(cx.on_app_quit(|workspace, cx| {
            if workspace.editor_dirty(cx) {
                let note = workspace
                    .notes
                    .as_ref()
                    .and_then(|h| h.note.as_ref())
                    .map_or_else(|| "the editor".to_string(), ToString::to_string);
                tracing::warn!("quitting with unsaved edits to {note}");
                eprintln!("kimun-gui: quitting with unsaved edits to {note}");
            }
            async {}
        }));
    }

    /// Close the window as its close button does (the Quit chord, the
    /// palette's quit): through [`Self::should_close`], so the open note is
    /// flushed first and the flush closes the window once written.
    pub(crate) fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(cx) {
            window.remove_window();
        }
    }

    /// Whether the window may close now. With edits to write it starts the
    /// close flush and says no; the flush closes the window when done.
    pub(crate) fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        match self.closing {
            Closing::Requested | Closing::Flushing => return false,
            Closing::Closed => return true,
            // Warned, and nothing typed since: the edits go.
            Closing::Discard(revision) if revision == self.editor_revision(cx) => return true,
            // New edits since the warning: start over.
            Closing::Discard(_) => self.closing = Closing::Open,
            Closing::Open => {}
        }
        // No vault: the editor is not even shown.
        if self.notes.is_none() {
            return true;
        }
        let dirty = self.editor_dirty(cx);
        if !self.can_save() {
            if dirty {
                let text = format!("{} Close again to discard them.", self.unsaveable());
                self.arm_discard(cx);
                self.flash_error(text, cx);
                return false;
            }
            return true;
        }
        let Some(host) = self.notes.as_mut() else {
            return true;
        };
        if host.is_switching() {
            host.session.stop_autosave();
            self.closing = Closing::Requested;
            return false;
        }
        if !dirty && !host.session.is_autosave_in_flight() {
            host.session.stop_autosave();
            return true;
        }
        self.close_flush(cx);
        false
    }

    /// A switch ended while a close waited on it, without leaving the open
    /// note clean to close on: give up this close (the flash says why). The
    /// next close tries the flush again, and warns before it discards.
    fn continue_close(&mut self, cx: &mut Context<Self>) {
        if self.closing == Closing::Requested {
            self.closing = Closing::Open;
            self.restart_tick();
            cx.notify();
        }
    }

    pub(super) fn restart_tick(&mut self) {
        if let (Some(host), Some(tx)) = (self.notes.as_mut(), self.tx.as_ref()) {
            host.session.restart_autosave(tx);
        }
    }

    pub(super) fn restart_tick_unless_read_only(&mut self) {
        if !self.status.read_only {
            self.restart_tick();
        }
    }

    /// Flush the open note before the window closes; close it once written.
    fn close_flush(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        host.session.stop_autosave();
        let flush = match host
            .session
            .prepare_flush(Some(&self.editor.read(cx).editor))
        {
            Ok(flush) => flush,
            Err(_) => {
                tracing::warn!("close: a flush is still outstanding");
                self.closing = Closing::Open;
                return;
            }
        };
        self.closing = Closing::Flushing;
        if let Some(note) = self.notes.as_ref().and_then(|h| h.note.clone()) {
            self.flash(format!("Saving {note}…"), cx);
        }
        let window = self.window;
        // Detached: removing the window drops the workspace, which must not
        // cancel the task that removes it.
        cx.spawn(async move |this, cx| {
            let result = tokio::spawn(flush.run()).await;
            let close = this
                .update(cx, |workspace, cx| workspace.close_flushed(result, cx))
                .unwrap_or(false);
            if close && let Some(window) = window {
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
        result: Result<FlushResult, JoinError>,
        cx: &mut Context<Self>,
    ) -> bool {
        let outcome = self.apply_flush(result, cx);
        if let Some(problem) = flush_problem(&outcome) {
            let note = self
                .notes
                .as_ref()
                .and_then(|h| h.note.clone())
                .unwrap_or_else(VaultPath::root);
            self.closing = Closing::Open;
            self.restart_tick();
            self.arm_discard(cx);
            self.flash_error(
                format!("Couldn't save {note}: {problem}. Close again to discard your edits."),
                cx,
            );
            return false;
        }
        if self.editor_dirty(cx) {
            // Typed while the flush ran: write that too. Only when that flush
            // could capture the buffer (never on a pending open, which would
            // spin): otherwise say so and let the next close discard.
            let pending = self
                .notes
                .as_ref()
                .is_some_and(|h| h.session.is_open_pending());
            if rewritable(Some(&outcome)) && !pending {
                self.close_flush(cx);
            } else {
                self.closing = Closing::Open;
                self.restart_tick();
                let text = format!("{} Close again to discard them.", self.unsaveable());
                self.arm_discard(cx);
                self.flash_error(text, cx);
            }
            return false;
        }
        self.closing = Closing::Closed;
        true
    }
}
