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
    /// Active visual selection in logical (row, byte-col) coordinates, 0-indexed.
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
            && let Some(cmd) = &self.cmdline {
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
