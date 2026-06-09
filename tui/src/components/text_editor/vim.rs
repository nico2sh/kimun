//! Built-in vim emulation: a modal input interpreter over a `TextArea`.
//! Pure over `&mut TextArea` — no component state, no async (adr/0012).

use super::snapshot::EditorMode;

/// Modal vim state layered over the textarea buffer.
#[derive(Debug)]
pub struct VimEngine {
    mode: EditorMode,
}

impl Default for VimEngine {
    fn default() -> Self {
        // Notes open in Normal mode (vim convention).
        Self { mode: EditorMode::Normal }
    }
}

impl VimEngine {
    #[allow(dead_code)] // used in Plan 1 Task 5
    pub fn mode(&self) -> &EditorMode {
        &self.mode
    }

    /// Footer label for the current mode (e.g. "NORMAL").
    #[allow(dead_code)] // used in Plan 1 Task 5
    pub fn mode_label(&self) -> String {
        self.mode.label().to_string()
    }
}
