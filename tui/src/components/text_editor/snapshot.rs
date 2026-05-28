use std::borrow::Cow;
use std::num::NonZeroU64;

/// Atomic view of the editor's `(lines, cursor, content_revision)`
/// tuple at a single point in time. Producers (today
/// `TextEditorComponent::view_snapshot`) own the construction-time
/// invariant: the cursor's row is in-bounds for `lines`. Consumers
/// (`view.rs`, `click_to_logical_u16`, the autocomplete host, etc.)
/// take a `&EditorSnapshot<'_>` and skip the per-leaf `.get()`
/// guards that previously defended against drift between cursor and
/// lines.
///
/// The `Cow` lets the Textarea backend borrow its lines directly
/// (zero clone) while the Nvim backend clones out from behind its
/// `Mutex` (the lines must outlive the `MutexGuard`, which is
/// dropped before the snapshot is returned).
pub struct EditorSnapshot<'a> {
    pub lines: Cow<'a, [String]>,
    /// `(row, col)` with `row < lines.len()` (clamped at
    /// construction when the producer's source was stale) UNLESS
    /// `lines.is_empty()`, in which case the snapshot represents an
    /// empty buffer and `cursor` is `(0, 0)`.
    pub cursor: (usize, usize),
    /// Content identity at construction. Stable across cursor moves;
    /// bumps on real text changes only (see
    /// [[decouple-text-revision]]).
    pub content_revision: NonZeroU64,
}

impl<'a> EditorSnapshot<'a> {
    /// Borrow-mode constructor for the Textarea backend and tests.
    pub fn borrowed(
        lines: &'a [String],
        cursor: (usize, usize),
        content_revision: NonZeroU64,
    ) -> Self {
        Self {
            lines: Cow::Borrowed(lines),
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
    ) -> EditorSnapshot<'static> {
        EditorSnapshot {
            lines: Cow::Owned(lines),
            cursor,
            content_revision,
        }
    }

    /// `true` when the cursor row is a valid index into `lines`.
    /// `false` only when `lines` is empty (in which case both row 0
    /// and any other row are out of bounds).
    pub fn cursor_in_bounds(&self) -> bool {
        self.cursor.0 < self.lines.len()
    }

    /// Cursor row guaranteed in-bounds for `lines`. Returns `0` on
    /// an empty buffer (the only case where the producer cannot
    /// clamp to a valid index).
    pub fn cursor_row_clamped(&self) -> usize {
        if self.lines.is_empty() {
            0
        } else {
            self.cursor.0.min(self.lines.len() - 1)
        }
    }

    /// Cursor row's logical line. Returns the empty slice when
    /// `lines` is empty.
    pub fn cursor_line(&self) -> &str {
        self.lines
            .get(self.cursor_row_clamped())
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Global byte offset of the cursor across `lines.join("\n")`.
    /// Mirrors `autocomplete_glue::row_char_col_to_byte` but consumes
    /// the snapshot directly so callers (e.g. the autocomplete
    /// controller) don't need to depend on the editor's glue
    /// module. Clamps the char column to the row's char count, then
    /// returns the byte position of the char-column within the
    /// joined buffer.
    pub fn cursor_byte_offset(&self) -> usize {
        let row = self.cursor.0;
        let mut byte = 0;
        for line in self.lines.iter().take(row) {
            byte += line.len() + 1; // +1 for the implicit `\n` separator
        }
        let Some(line) = self.lines.get(row) else {
            return byte;
        };
        byte
            + line
                .char_indices()
                .nth(self.cursor.1)
                .map(|(b, _)| b)
                .unwrap_or(line.len())
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
    pub mode: NvimMode,
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
            mode: NvimMode::Normal,
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
        if self.mode == NvimMode::Command
            && let Some(cmd) = &self.cmdline
        {
            return format!("{}\u{2590}", cmd); // ▐ block cursor
        }
        self.mode.label().to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NvimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,
    Other(String),
}

impl NvimMode {
    pub fn label(&self) -> &str {
        match self {
            NvimMode::Normal => "NORMAL",
            NvimMode::Insert => "INSERT",
            NvimMode::Visual => "VISUAL",
            NvimMode::VisualLine => "V-LINE",
            NvimMode::Command => "COMMAND",
            NvimMode::Other(_) => "OTHER",
        }
    }

    /// Parse the one- or two-character mode string returned by `nvim_get_mode`.
    pub fn from_nvim_str(s: &str) -> Self {
        match s {
            "n" | "no" | "nov" | "noV" | "no\x16" => NvimMode::Normal,
            "i" => NvimMode::Insert,
            "v" => NvimMode::Visual,
            "V" => NvimMode::VisualLine,
            "c" => NvimMode::Command,
            other => NvimMode::Other(other.to_string()),
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
        let snap: EditorSnapshot<'_> = EditorSnapshot::owned(Vec::new(), (0, 0), rev(1));
        assert!(!snap.cursor_in_bounds());
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
        assert_eq!(NvimMode::Normal.label(), "NORMAL");
    }

    #[test]
    fn mode_label_insert() {
        assert_eq!(NvimMode::Insert.label(), "INSERT");
    }

    #[test]
    fn mode_label_visual() {
        assert_eq!(NvimMode::Visual.label(), "VISUAL");
    }

    #[test]
    fn mode_label_visual_line() {
        assert_eq!(NvimMode::VisualLine.label(), "V-LINE");
    }

    #[test]
    fn mode_label_command() {
        assert_eq!(NvimMode::Command.label(), "COMMAND");
    }

    #[test]
    fn mode_from_str_normal() {
        assert!(matches!(NvimMode::from_nvim_str("n"), NvimMode::Normal));
    }

    #[test]
    fn mode_from_str_insert() {
        assert!(matches!(NvimMode::from_nvim_str("i"), NvimMode::Insert));
    }

    #[test]
    fn mode_from_str_visual() {
        assert!(matches!(NvimMode::from_nvim_str("v"), NvimMode::Visual));
    }

    #[test]
    fn mode_from_str_visual_line() {
        assert!(matches!(NvimMode::from_nvim_str("V"), NvimMode::VisualLine));
    }

    #[test]
    fn mode_from_str_command() {
        assert!(matches!(NvimMode::from_nvim_str("c"), NvimMode::Command));
    }

    #[test]
    fn mode_from_str_unknown() {
        let m = NvimMode::from_nvim_str("R");
        assert!(matches!(m, NvimMode::Other(_)));
        if let NvimMode::Other(s) = m {
            assert_eq!(s, "R");
        }
    }

    #[test]
    fn footer_label_normal_mode() {
        let snap = NvimSnapshot {
            mode: NvimMode::Normal,
            cmdline: None,
            ..Default::default()
        };
        assert_eq!(snap.footer_label(), "NORMAL");
    }

    #[test]
    fn footer_label_command_mode_with_cmdline() {
        let snap = NvimSnapshot {
            mode: NvimMode::Command,
            cmdline: Some(":set nu".to_string()),
            ..Default::default()
        };
        assert_eq!(snap.footer_label(), ":set nu\u{2590}");
    }

    #[test]
    fn footer_label_command_mode_no_cmdline() {
        let snap = NvimSnapshot {
            mode: NvimMode::Command,
            cmdline: None,
            ..Default::default()
        };
        assert_eq!(snap.footer_label(), "COMMAND");
    }
}
