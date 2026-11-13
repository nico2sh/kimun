//! The **flush before the open note is let go**: what leaving the note for
//! another panel ([`Workspace::leave_note`]), a file operation, the
//! properties dialog and a search open each did by hand. Each site still owns
//! its wording and what follows the flush; the shared steps are here:
//!
//! 1. [`Workspace::flush_block`]: may the note be let go at all?
//! 2. [`Workspace::begin_note_flush`]: stop the tick, take the buffer,
//!    block typing.
//! 3. [`Workspace::flush_failure`]: after the flush landed, is everything
//!    written?

use gpui_kit::{App, Context};
use kimun_notes::logic::session::{FlushRequest, SaveOutcome};

use super::Workspace;
use super::group::active_mut;
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
        let host = self.active_state().notes.as_ref()?;
        if self.closing != Closing::Open {
            Some(FlushBlock::Closing)
        } else if self.note_held() {
            Some(FlushBlock::Held)
        } else if host.is_switching() || host.is_open_wanted() {
            Some(FlushBlock::Opening)
        } else if host.note.is_some() && self.editor_dirty(cx) && !self.can_save() {
            Some(FlushBlock::Unsaveable)
        } else {
            None
        }
    }

    /// Start the one write of the open note before it is let go: no tick
    /// from here, the buffer taken, typing blocked. `None` when another
    /// flush still runs (the tick is back, unless the vault is read-only;
    /// nothing else changed). The
    /// caller runs the request, applies it ([`Self::apply_flush`]) and
    /// unblocks typing.
    pub(super) fn begin_note_flush(&mut self, cx: &mut Context<Self>) -> Option<FlushRequest> {
        let state = active_mut!(self);
        let host = state.notes.as_mut()?;
        host.session.stop_autosave();
        match host
            .session
            .prepare_flush(Some(&state.editor.read(cx).editor))
        {
            Ok(flush) => {
                self.block_input(true, cx);
                Some(flush)
            }
            Err(_) => {
                // Never in a read-only vault, where the tick does not run.
                self.restart_tick_unless_read_only();
                None
            }
        }
    }

    /// What is still unwritten after a flush landed: the flush's own
    /// problem, else text typed since (nothing types while it runs, but a
    /// buffer still dirty must not be run over).
    pub(super) fn flush_failure(&self, outcome: &SaveOutcome, cx: &App) -> Option<String> {
        flush_problem(outcome).or_else(|| {
            self.editor_dirty(cx)
                .then(|| "the editor still has unsaved edits".to_string())
        })
    }
}
