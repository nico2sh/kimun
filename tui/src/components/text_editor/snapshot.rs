use std::num::NonZeroU64;

/// Atomic view of the editor's `(lines, cursor, content_revision)`
/// tuple at a single point in time. Producers (today
/// `TextEditorComponent::view_snapshot`) own the construction-time
/// invariant: the cursor's row is in-bounds for `lines`. Consumers
/// (`view.rs`, `click_to_logical_u16`, the autocomplete host, etc.)
/// take a `&EditorSnapshot` and skip the per-leaf `.get()`
/// guards that previously defended against drift between cursor and
/// lines.
///
/// The `Cow` lets the Textarea backend borrow its lines directly
/// (zero clone) while the Nvim backend clones out from behind its
/// `Mutex` (the lines must outlive the `MutexGuard`, which is
/// dropped before the snapshot is returned).
pub struct EditorSnapshot {
    /// The text itself. Cloning one is O(1) — the rope shares its structure — so
    /// a snapshot *is* the buffer's text rather than a copy of it, and the cursor
    /// below cannot drift away from what it was read with.
    pub text: ropetext::Text,
    /// `(row, col)`, clamped at construction when the producer's source was
    /// stale. A text always has at least one row, so there is no empty-buffer
    /// case to special-case.
    pub cursor: (usize, usize),
    /// Content identity at construction. Stable across cursor moves;
    /// bumps on real text changes only (see
    /// [[decouple-text-revision]]).
    pub content_revision: NonZeroU64,
}

impl EditorSnapshot {
    /// Build one from rows, for the nvim backend and for tests.
    pub fn borrowed(
        lines: &[String],
        cursor: (usize, usize),
        content_revision: NonZeroU64,
    ) -> Self {
        Self {
            text: ropetext::Text::from(lines.join("\n").as_str()),
            cursor,
            content_revision,
        }
    }

    /// The hot path: the buffer already holds the text, so nothing is rebuilt.
    pub fn of_buffer(
        text: ropetext::Text,
        cursor: (usize, usize),
        content_revision: NonZeroU64,
    ) -> Self {
        Self {
            text,
            cursor,
            content_revision,
        }
    }

    /// Owned-mode constructor for the Nvim backend (lines cloned out
    /// from behind the `Mutex`) and for tests that don't have a
    /// long-lived borrow.
    pub fn owned(
        lines: Vec<String>,
        cursor: (usize, usize),
        content_revision: NonZeroU64,
    ) -> EditorSnapshot {
        EditorSnapshot {
            text: ropetext::Text::from(lines.join("\n").as_str()),
            cursor,
            content_revision,
        }
    }

    /// `true` when the cursor's row exists. A text always has at least one row,
    /// so this is false only for a cursor past the end.
    pub fn cursor_in_bounds(&self) -> bool {
        self.cursor.0 < self.text.line_count()
    }

    /// Cursor row, guaranteed to exist.
    pub fn cursor_row_clamped(&self) -> usize {
        self.cursor.0.min(self.text.line_count().saturating_sub(1))
    }

    /// The cursor row's text.
    pub fn cursor_line(&self) -> std::borrow::Cow<'_, str> {
        self.text
            .line(self.cursor_row_clamped())
            .unwrap_or_default()
    }

    /// The cursor's byte offset into the whole buffer.
    ///
    /// Was a row walk summing lengths; the text addresses it directly. The row
    /// is clamped and an unrepresentable column falls back to the end of the
    /// buffer — the return type leaves no way to refuse, and the callers (the
    /// autocomplete controller) treat the offset as a trigger point rather than
    /// an edit site.
    pub fn cursor_byte_offset(&self) -> usize {
        self.text
            .position(
                self.cursor_row_clamped(),
                ropetext::Column::new(self.cursor.1),
            )
            .map(|at| at.byte())
            .unwrap_or_else(|| self.text.len_bytes())
    }
}

/// Cached state from a running `nvim --embed` process.
///
/// Written by async refresh tasks; read synchronously by the render path.
#[derive(Debug, Clone)]
pub struct NvimSnapshot {
    /// Buffer lines (0-indexed).
    pub lines: Vec<String>,
    /// Cursor position (row, col), 0-indexed.
    pub cursor: (usize, usize),
    pub mode: EditorMode,
    /// Set when mode is `Command` — the full command line including the type prefix
    /// (e.g., `":set nu"` or `"/pattern"`). `None` in all other modes.
    pub cmdline: Option<String>,
    /// `true` after every keystroke, cleared by `mark_saved()`.
    pub dirty: bool,
    /// Monotonically increasing; incremented every time `lines` actually changes.
    /// Used by `view.update()` so the parse cache is rebuilt from fresh content,
    /// not from whatever lines happened to be in the snapshot when the key was pressed.
    pub content_gen: u64,
    /// Active visual selection in logical (row, char-col) coordinates, 0-indexed.
    /// `None` when not in a visual mode. For `VisualLine` the end col is `usize::MAX`.
    pub visual_selection: Option<((usize, usize), (usize, usize))>,
}

impl Default for NvimSnapshot {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
            mode: EditorMode::Normal,
            cmdline: None,
            dirty: false,
            content_gen: 0,
            visual_selection: None,
        }
    }
}

impl NvimSnapshot {
    /// The string to display in the footer mode indicator.
    ///
    /// In command mode, shows the live command line with a block cursor appended.
    /// In all other modes, shows the mode label (e.g., `"NORMAL"`).
    pub fn footer_label(&self) -> String {
        if self.mode == EditorMode::Command
            && let Some(cmd) = &self.cmdline
        {
            return format!("{}\u{2590}", cmd); // ▐ block cursor
        }
        self.mode.label().to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditorMode {
    Normal,
    Insert,
    Replace,
    Visual,
    VisualLine,
    Command,
    Other(String),
}

impl EditorMode {
    pub fn label(&self) -> &str {
        match self {
            EditorMode::Normal => "NORMAL",
            EditorMode::Insert => "INSERT",
            EditorMode::Replace => "REPLACE",
            EditorMode::Visual => "VISUAL",
            EditorMode::VisualLine => "V-LINE",
            EditorMode::Command => "COMMAND",
            EditorMode::Other(_) => "OTHER",
        }
    }

    /// Parse the one- or two-character mode string returned by `nvim_get_mode`.
    /// Nvim-only: the vim engine sets its mode directly, never through this.
    pub fn from_nvim_str(s: &str) -> Self {
        match s {
            "n" | "no" | "nov" | "noV" | "no\x16" => EditorMode::Normal,
            "i" => EditorMode::Insert,
            "R" => EditorMode::Replace,
            "v" => EditorMode::Visual,
            "V" => EditorMode::VisualLine,
            "c" => EditorMode::Command,
            other => EditorMode::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    #[test]
    fn snapshot_borrowed_passes_cursor_through() {
        let lines = vec!["a".to_string(), "b".to_string()];
        let snap = EditorSnapshot::borrowed(&lines, (1, 0), rev(5));
        assert_eq!(snap.cursor, (1, 0));
        assert!(snap.cursor_in_bounds());
        assert_eq!(snap.cursor_line(), "b");
    }

    #[test]
    fn snapshot_helpers_on_empty_buffer() {
        let snap: EditorSnapshot = EditorSnapshot::owned(Vec::new(), (0, 0), rev(1));
        // An empty buffer still has one empty row, so (0, 0) is a real place.
        assert!(snap.cursor_in_bounds());
        assert_eq!(snap.cursor_row_clamped(), 0);
        assert_eq!(snap.cursor_line(), "");
    }

    #[test]
    fn snapshot_cursor_byte_offset_across_rows() {
        let lines = vec!["hello".to_string(), "wørld".to_string()];
        // Row 1, col 2 (after 'w', 'ø') — bytes: 'hello\n' = 6 + 'wø' = 3 = 9.
        let snap = EditorSnapshot::borrowed(&lines, (1, 2), rev(1));
        assert_eq!(snap.cursor_byte_offset(), 9);
    }

    #[test]
    fn snapshot_clamps_stale_cursor_row() {
        // Tests cursor_row_clamped behavior — the field itself is
        // populated by the producer, not by these helpers.
        let lines = vec!["only".to_string()];
        let snap = EditorSnapshot::borrowed(&lines, (5, 2), rev(1));
        assert_eq!(snap.cursor_row_clamped(), 0);
        assert_eq!(snap.cursor_line(), "only");
    }

    #[test]
    fn default_snapshot_is_not_dirty() {
        let snap = NvimSnapshot::default();
        assert!(!snap.dirty);
    }

    #[test]
    fn mode_label_normal() {
        assert_eq!(EditorMode::Normal.label(), "NORMAL");
    }

    #[test]
    fn mode_label_insert() {
        assert_eq!(EditorMode::Insert.label(), "INSERT");
    }

    #[test]
    fn mode_label_visual() {
        assert_eq!(EditorMode::Visual.label(), "VISUAL");
    }

    #[test]
    fn mode_label_visual_line() {
        assert_eq!(EditorMode::VisualLine.label(), "V-LINE");
    }

    #[test]
    fn mode_label_command() {
        assert_eq!(EditorMode::Command.label(), "COMMAND");
    }

    #[test]
    fn mode_from_str_normal() {
        assert!(matches!(EditorMode::from_nvim_str("n"), EditorMode::Normal));
    }

    #[test]
    fn mode_from_str_insert() {
        assert!(matches!(EditorMode::from_nvim_str("i"), EditorMode::Insert));
    }

    #[test]
    fn mode_from_str_visual() {
        assert!(matches!(EditorMode::from_nvim_str("v"), EditorMode::Visual));
    }

    #[test]
    fn mode_from_str_visual_line() {
        assert!(matches!(
            EditorMode::from_nvim_str("V"),
            EditorMode::VisualLine
        ));
    }

    #[test]
    fn mode_from_str_command() {
        assert!(matches!(
            EditorMode::from_nvim_str("c"),
            EditorMode::Command
        ));
    }

    #[test]
    fn mode_from_str_replace() {
        assert!(matches!(
            EditorMode::from_nvim_str("R"),
            EditorMode::Replace
        ));
    }

    #[test]
    fn mode_from_str_unknown() {
        let m = EditorMode::from_nvim_str("t"); // terminal mode — unmapped
        assert!(matches!(m, EditorMode::Other(_)));
        if let EditorMode::Other(s) = m {
            assert_eq!(s, "t");
        }
    }

    #[test]
    fn footer_label_normal_mode() {
        let snap = NvimSnapshot {
            mode: EditorMode::Normal,
            cmdline: None,
            ..Default::default()
        };
        assert_eq!(snap.footer_label(), "NORMAL");
    }

    #[test]
    fn footer_label_command_mode_with_cmdline() {
        let snap = NvimSnapshot {
            mode: EditorMode::Command,
            cmdline: Some(":set nu".to_string()),
            ..Default::default()
        };
        assert_eq!(snap.footer_label(), ":set nu\u{2590}");
    }

    #[test]
    fn footer_label_command_mode_no_cmdline() {
        let snap = NvimSnapshot {
            mode: EditorMode::Command,
            cmdline: None,
            ..Default::default()
        };
        assert_eq!(snap.footer_label(), "COMMAND");
    }
}
