//! Whether the editor acted on an input event.

/// The editor's answer to an input event, free of any front end's event
/// type: the host maps it onto its own (the TUI's `EventState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// The editor acted on the event; nothing else should.
    Consumed,
    /// The event is not the editor's; the host may route it elsewhere.
    NotConsumed,
}

/// Which way a key went through [`crate::logic::editor::Editor::handle_key`],
/// so a host that keeps state anchored to the text (the TUI's autocomplete
/// popup) can follow it exactly as before the editor was carved out.
///
/// Only the **direct** path ever needed the host's before/after comparison;
/// the vim engine's consumed keys returned early without it, except that a
/// cursor-only vim key refreshed an open popup and a vim clipboard paste
/// re-synced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPath {
    /// The key reached the direct-edit path (plain mode, or vim passed it
    /// through). `text_changed` / `cursor_moved` compare the buffer either
    /// side of *that path only* — not of the vim stage before it. `pasted`:
    /// `Ctrl+V` inserted clipboard text.
    Direct {
        text_changed: bool,
        cursor_moved: bool,
        pasted: bool,
    },
    /// The vim engine moved the cursor (or the Visual selection) only.
    VimCursorOnly,
    /// The vim engine asked for a clipboard paste and clipboard text was
    /// inserted.
    VimPasted,
    /// Anything else the vim engine consumed: an edit, a no-op, a host action.
    VimOther,
}
