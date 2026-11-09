//! [`NoteBuffer`]: the slice of an editor buffer the note session drives.

use std::num::NonZeroU64;

use kimun_core::nfs::VaultPath;

use crate::logic::editor::Editor;

/// What [`super::NoteSession`] needs from the buffer showing the open note:
/// its text, its dirty clock, and the lifecycle calls that load, reload and
/// mark it saved. Implemented by the Logic [`Editor`] (the GUI's buffer) and by
/// the TUI's `TextEditorComponent`, which fronts both the textarea `Editor` and
/// the nvim backend, so one session drives every engine.
pub trait NoteBuffer {
    /// Whether the buffer holds edits newer than the last save.
    fn is_dirty(&self) -> bool;
    /// The whole buffer as one string.
    fn get_text(&self) -> String;
    /// The current content revision (bumped on every edit).
    fn content_revision(&self) -> NonZeroU64;
    /// Show `note` with `text` (a reload when `note` is already on screen).
    fn open_note(&mut self, note: VaultPath, text: String);
    /// Reload the note on screen with `text`.
    fn set_text(&mut self, text: String);
    /// The note on screen was renamed to `note`.
    fn renamed_to(&mut self, note: VaultPath);
    /// Synchronous mark-saved: clean iff `text` is still the buffer's text.
    fn mark_saved(&mut self, text: String);
    /// Mark clean iff `rev` is still the current revision (a stale
    /// completion marks nothing).
    fn mark_saved_at_revision(&mut self, rev: NonZeroU64);
}

impl NoteBuffer for Editor {
    fn is_dirty(&self) -> bool {
        Editor::is_dirty(self)
    }

    fn get_text(&self) -> String {
        Editor::get_text(self)
    }

    fn content_revision(&self) -> NonZeroU64 {
        Editor::content_revision(self)
    }

    fn open_note(&mut self, note: VaultPath, text: String) {
        Editor::open_note(self, note, text);
    }

    fn set_text(&mut self, text: String) {
        Editor::set_text(self, text);
    }

    fn renamed_to(&mut self, note: VaultPath) {
        Editor::renamed_to(self, note);
    }

    fn mark_saved(&mut self, text: String) {
        Editor::mark_saved(self, text);
    }

    fn mark_saved_at_revision(&mut self, rev: NonZeroU64) {
        Editor::mark_saved_at_revision(self, rev);
    }
}
