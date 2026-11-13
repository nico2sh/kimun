//! The **flush before the open note is let go**: what leaving the note for
//! another panel ([`Workspace::leave_note`]), a file operation, the
//! properties dialog and a search open each did by hand. Each site still owns
//! its wording and what follows the flush; the shared steps are here:
//!
//! 1. [`Workspace::flush_block`]: may the note be let go at all?
//! 2. [`Workspace::begin_note_flush_in`]: stop the tick, take the buffer,
//!    block typing.
//! 3. [`Workspace::flush_failure_in`]: after the flush landed, is everything
//!    written?

use gpui_kit::{App, Context};
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::session::{FlushRequest, SaveOutcome};

use super::Workspace;
use super::note::{Closing, flush_problem};

/// Why the open note cannot be let go now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FlushBlock {
    /// The window is closing.
    Closing,
    /// Another operation holds the note: a file operation, a property
    /// write or its reload, another flush, a vault change.
    Held,
    /// A note is still opening (or one is wanted).
    Opening,
    /// The note has edits and nothing can write them.
    Unsaveable,
}

impl FlushBlock {
    /// The reason in a sentence's middle ("Couldn't open X: {reason}.").
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::Closing => "the window is closing",
            Self::Held => "another operation on the note is still running",
            Self::Opening => "a note is still opening",
            Self::Unsaveable => "the note's edits can't be saved",
        }
    }
}

impl Workspace {
    /// The first reason the open note cannot be flushed and let go, if any.
    pub(super) fn flush_block(&self, cx: &App) -> Option<FlushBlock> {
        self.flush_block_in(self.layout.active_group(), cx)
    }

    /// [`Self::flush_block`] for the note `group` holds.
    pub(super) fn flush_block_in(&self, group: GroupId, cx: &App) -> Option<FlushBlock> {
        let host = self.state(group)?.notes.as_ref()?;
        if self.closing != Closing::Open {
            Some(FlushBlock::Closing)
        } else if self.note_held() {
            Some(FlushBlock::Held)
        } else if host.is_switching() || host.is_open_wanted() {
            Some(FlushBlock::Opening)
        } else if host.note.is_some() && self.editor_dirty_in(group, cx) && !self.can_save_in(group)
        {
            Some(FlushBlock::Unsaveable)
        } else {
            None
        }
    }

    /// Start the one write of the note `group` holds before it is let go:
    /// no tick from here, the buffer taken, typing blocked. `None` when
    /// another flush still runs (the tick is back, unless the vault is
    /// read-only; nothing else changed). The caller keeps `group`, runs the
    /// request, applies it to that group ([`Self::apply_flush`]) and
    /// unblocks its typing, whichever group is active by then.
    pub(super) fn begin_note_flush_in(
        &mut self,
        group: GroupId,
        cx: &mut Context<Self>,
    ) -> Option<FlushRequest> {
        let state = self.groups.get_mut(&group)?;
        let host = state.notes.as_mut()?;
        host.session.stop_autosave();
        match host
            .session
            .prepare_flush(Some(&state.editor.read(cx).editor))
        {
            Ok(flush) => {
                self.block_input_in(group, true, cx);
                Some(flush)
            }
            Err(_) => {
                // Never in a read-only vault, where the tick does not run.
                self.restart_tick_unless_read_only_in(group);
                None
            }
        }
    }

    /// Let go of a flush begun in `group` ([`Self::begin_note_flush_in`])
    /// that will not run (a later group refused its own): the background
    /// save it took over goes back to the group's session and finishes, as
    /// dropping the request would abort it. The caller gives the tick and
    /// the keyboard back.
    pub(super) fn give_back_flush_in(&mut self, group: GroupId, flush: FlushRequest) {
        if let Some(host) = self.groups.get_mut(&group).and_then(|s| s.notes.as_mut()) {
            host.session.give_back_flush(flush);
        }
    }

    /// What is still unwritten in `group` after its flush landed: the
    /// flush's own problem, else text typed since (nothing types while it
    /// runs, but a buffer still dirty must not be run over).
    pub(super) fn flush_failure_in(
        &self,
        group: GroupId,
        outcome: &SaveOutcome,
        cx: &App,
    ) -> Option<String> {
        flush_problem(outcome).or_else(|| {
            self.editor_dirty_in(group, cx)
                .then(|| "the editor still has unsaved edits".to_string())
        })
    }
}
