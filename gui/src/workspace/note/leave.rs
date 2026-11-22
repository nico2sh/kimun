//! Letting the open note go for another panel, such as an attachment or a
//! facet view, and emptying a group.
//! It does not show that panel.

use gpui_kit::{Context, SharedString, Task, Window};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::session::FlushResult;
use tokio::task::JoinError;

use crate::workspace::Workspace;

/// The panel the open note is left for ([`Workspace::leave_note`]): each
/// holds its flush in its own slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum LeavingFor {
    Attachment,
    Facet,
}

/// How leaving the open note went ([`Workspace::leave_note`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum NoteLeft {
    /// Nothing had to be written: the panel may show now.
    Now,
    /// The flush wrote everything: the panel may show.
    Flushed,
    /// The note stays because the flush failed (flashed); its tick is back.
    Stays,
    /// The note stays because a note was asked for meanwhile, which opens
    /// instead; its tick is back. Nothing failed.
    Redirected,
}

/// Whether [`Workspace::leave_note`] started the leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum LeaveStart {
    /// `then` ran, or runs when the flush ends.
    Started,
    /// The note cannot be let go now (flashed when it has a reason): `then`
    /// never runs.
    Refused,
}

impl Workspace {
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
    /// `then` lets the note go itself ([`Self::release_note_in`]). Says
    /// whether the leave started, so a caller can undo what it did for it.
    pub(in crate::workspace) fn leave_note(
        &mut self,
        group: GroupId,
        panel: LeavingFor,
        name: SharedString,
        then: impl FnOnce(&mut Self, NoteLeft, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LeaveStart {
        let can_save = self.can_save_in(group);
        if let Some(block) = self.flush_block_in(group, cx) {
            self.flash_error(format!("Couldn't open {name}: {}.", block.reason()), cx);
            return LeaveStart::Refused;
        }
        if self.host_in(group).is_none() {
            return LeaveStart::Refused;
        }
        if !can_save {
            then(self, NoteLeft::Now, window, cx);
            return LeaveStart::Started;
        }
        // No tick from here: the flush is the last write of the note.
        let Some(flush) = self.begin_note_flush_in(group, cx) else {
            let text = format!("Couldn't open {name}: another save is still running.");
            self.flash_error(text, cx);
            return LeaveStart::Refused;
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
        LeaveStart::Started
    }

    /// The flush [`Self::leave_note`] started ended: `then` hears whether
    /// everything is written ([`NoteLeft::Flushed`]) or the note stays (the
    /// flush failed, flashed: [`NoteLeft::Stays`]; or a note was asked for
    /// meanwhile, which opens instead: [`NoteLeft::Redirected`]), its tick
    /// back.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn note_left(
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
        let host = self.host_in(group);
        let note_wanted = host.is_some_and(|h| h.is_open_wanted());
        if let Some(problem) = problem {
            let note = self.note_name_in(group);
            self.restart_tick_unless_read_only_in(group);
            self.flash_error(
                format!("Couldn't save {note}: {problem}. {name} was not opened."),
                cx,
            );
            then(self, NoteLeft::Stays, window, cx);
        } else if note_wanted {
            // A note was asked for meanwhile: it opens instead.
            self.restart_tick_unless_read_only_in(group);
            then(self, NoteLeft::Redirected, window, cx);
        } else {
            then(self, NoteLeft::Flushed, window, cx);
        }
        // An open that waited for the flush goes now.
        self.file_op_ended(cx);
    }

    /// The task slot holding the flush of a leave for `panel` in `group`.
    pub(super) fn leaving_slot(
        &mut self,
        group: GroupId,
        panel: LeavingFor,
    ) -> Option<&mut Option<Task<()>>> {
        let state = self.state_mut(group)?;
        Some(match panel {
            LeavingFor::Attachment => &mut state.attachment_flushing,
            LeavingFor::Facet => &mut state.facet_flushing,
        })
    }

    /// Let the note `group` holds (already flushed) go: no session path, no
    /// tick, an empty clean buffer, so nothing can save while another
    /// panel shows; for the active group the status line names no note.
    pub(in crate::workspace) fn release_note_in(&mut self, group: GroupId, cx: &mut Context<Self>) {
        self.empty_group(group, cx);
        self.forget_note_gone(group);
    }

    /// `group` no longer says its note went away (a note opened there, it
    /// was let go on purpose, or it closed).
    pub(in crate::workspace) fn forget_note_gone(&mut self, group: GroupId) {
        if self.note_gone.as_ref().is_some_and(|(g, _)| *g == group) {
            self.note_gone = None;
        }
    }

    /// Make `group` an empty group where it is: every writer of its note
    /// stopped, no session path, no note, an empty clean buffer that takes
    /// input, no facet view. Nothing is saved: the caller flushed it, or its
    /// note is gone. The Footer names no note when `group` is the active one.
    pub(in crate::workspace) fn empty_group(&mut self, group: GroupId, cx: &mut Context<Self>) {
        // A menu on the group names the note it no longer holds.
        self.forget_note_menu_of(group, cx);
        self.drop_facet_in(group);
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
}
