//! What the editor shows, as data: one [`FrameRow`] of semantic [`Run`]s per
//! visible visual line, and where the cursor sits. A painter (the TUI's
//! ratatui view today) turns this into cells; nothing here knows about a
//! terminal, a theme or a style.

use crate::logic::editor::markdown::runs::Run;

/// One visible visual line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRow {
    /// The logical (buffer) row this visual line draws from.
    pub logical_row: usize,
    /// Whether this is the first visual line of its logical row.
    pub first: bool,
    pub runs: Vec<Run>,
}

/// The editor's visible state for one pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorFrame {
    pub rows: Vec<FrameRow>,
    /// Pane-relative cursor cell (col, row), `None` when unfocused, empty, or off-screen.
    pub cursor: Option<(u16, u16)>,
}
