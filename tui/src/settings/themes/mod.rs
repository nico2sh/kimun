use ratatui::style::{Color, Style};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::Display;

/// Built-in theme definitions (`Theme::gruvbox_dark()`, `Theme::nord()`, …).
mod builtin;
/// Terminal color-depth detection and 256/16-color theme adaptation.
pub mod color_depth;

#[derive(Debug, Clone, PartialEq)]
pub enum ThemeColor {
    Rgb(u8, u8, u8),
    /// Terminal ANSI color index (0–15 for the standard palette, up to 255 for
    /// 256-color mode). The actual color is determined by the user's terminal.
    Ansi(u8),
    /// The terminal's default foreground or background color.
    Reset,
}

impl Serialize for ThemeColor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ThemeColor::Rgb(r, g, b) => {
                serializer.serialize_str(&format!("#{:02x}{:02x}{:02x}", r, g, b))
            }
            ThemeColor::Ansi(n) => serializer.serialize_str(&format!("ansi:{}", n)),
            ThemeColor::Reset => serializer.serialize_str("reset"),
        }
    }
}

impl<'de> Deserialize<'de> for ThemeColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ThemeColor::from_string(&s).map_err(serde::de::Error::custom)
    }
}

impl ThemeColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        ThemeColor::Rgb(r, g, b)
    }

    /// Convert to the corresponding ratatui `Color`.
    ///
    /// ANSI indices 0–15 map to ratatui's named color variants so they emit
    /// the standard SGR codes (30–37 / 90–97) the terminal's palette is keyed
    /// to, rather than the 256-color `38;5;n` form which some terminals
    /// remap inconsistently for the low 16 slots.
    pub fn to_ratatui(&self) -> Color {
        match self {
            ThemeColor::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
            ThemeColor::Ansi(n) => match n {
                0 => Color::Black,
                1 => Color::Red,
                2 => Color::Green,
                3 => Color::Yellow,
                4 => Color::Blue,
                5 => Color::Magenta,
                6 => Color::Cyan,
                7 => Color::Gray,
                8 => Color::DarkGray,
                9 => Color::LightRed,
                10 => Color::LightGreen,
                11 => Color::LightYellow,
                12 => Color::LightBlue,
                13 => Color::LightMagenta,
                14 => Color::LightCyan,
                15 => Color::White,
                _ => Color::Indexed(*n),
            },
            ThemeColor::Reset => Color::Reset,
        }
    }

    /// Parse a color from a string in various formats:
    /// - RGB: "rgb(255, 128, 0)"
    /// - 3-char hex: "#abc" (expanded to #aabbcc)
    /// - 6-char hex: "#aabbcc"
    /// - ANSI index: "ansi:4" (0–255)
    /// - Terminal default: "reset"
    pub fn from_string(s: &str) -> Result<Self, String> {
        let s = s.trim();

        if s.starts_with('#') {
            Self::from_hex(s)
        } else if s.starts_with("rgb(") && s.ends_with(')') {
            Self::from_rgb_string(s)
        } else if s == "reset" {
            Ok(ThemeColor::Reset)
        } else if let Some(rest) = s.strip_prefix("ansi:") {
            rest.parse::<u8>()
                .map(ThemeColor::Ansi)
                .map_err(|_| format!("Invalid ANSI color index: {}", rest))
        } else {
            Err(format!("Invalid color format: {}", s))
        }
    }

    /// Parse hex color string (#abc or #aabbcc)
    fn from_hex(s: &str) -> Result<Self, String> {
        if !s.starts_with('#') {
            return Err("Hex color must start with #".to_string());
        }

        let hex = &s[1..];

        match hex.len() {
            3 => Self::from_hex_3char(hex),
            6 => Self::from_hex_6char(hex),
            _ => Err(format!(
                "Invalid hex color length: expected 3 or 6 chars, got {}",
                hex.len()
            )),
        }
    }

    /// Parse 3-character hex color (e.g., "abc" -> r=0xaa, g=0xbb, b=0xcc)
    fn from_hex_3char(hex: &str) -> Result<Self, String> {
        if hex.len() != 3 {
            return Err("Expected 3 hex characters".to_string());
        }

        let r = u8::from_str_radix(&hex[0..1].repeat(2), 16)
            .map_err(|_| format!("Invalid hex character in red component: {}", &hex[0..1]))?;
        let g = u8::from_str_radix(&hex[1..2].repeat(2), 16)
            .map_err(|_| format!("Invalid hex character in green component: {}", &hex[1..2]))?;
        let b = u8::from_str_radix(&hex[2..3].repeat(2), 16)
            .map_err(|_| format!("Invalid hex character in blue component: {}", &hex[2..3]))?;

        Ok(ThemeColor::Rgb(r, g, b))
    }

    /// Parse 6-character hex color (e.g., "aabbcc")
    fn from_hex_6char(hex: &str) -> Result<Self, String> {
        if hex.len() != 6 {
            return Err("Expected 6 hex characters".to_string());
        }

        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| format!("Invalid hex characters in red component: {}", &hex[0..2]))?;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| format!("Invalid hex characters in green component: {}", &hex[2..4]))?;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| format!("Invalid hex characters in blue component: {}", &hex[4..6]))?;

        Ok(ThemeColor::Rgb(r, g, b))
    }

    /// Parse RGB string format (e.g., "rgb(255, 128, 0)")
    fn from_rgb_string(s: &str) -> Result<Self, String> {
        if !s.starts_with("rgb(") || !s.ends_with(')') {
            return Err("RGB format must be rgb(r, g, b)".to_string());
        }

        let inner = &s[4..s.len() - 1];
        let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();

        if parts.len() != 3 {
            return Err(format!("RGB format requires 3 values, got {}", parts.len()));
        }

        let r = parts[0]
            .parse::<u8>()
            .map_err(|_| format!("Invalid red value: {}", parts[0]))?;
        let g = parts[1]
            .parse::<u8>()
            .map_err(|_| format!("Invalid green value: {}", parts[1]))?;
        let b = parts[2]
            .parse::<u8>()
            .map_err(|_| format!("Invalid blue value: {}", parts[2]))?;

        Ok(ThemeColor::Rgb(r, g, b))
    }
}

impl Display for ThemeColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeColor::Rgb(r, g, b) => write!(f, "rgb({},{},{})", r, g, b),
            ThemeColor::Ansi(n) => write!(f, "ansi:{}", n),
            ThemeColor::Reset => write!(f, "reset"),
        }
    }
}

/// Theme for the TUI application.
///
/// Fields are named after the UI roles they fill, making it straightforward to
/// map any popular terminal color scheme (Gruvbox, Catppuccin, Tokyo Night, …)
/// to this struct.  Custom themes can be placed as `.toml` files in the themes
/// config directory and will be loaded automatically at startup.
///
/// # Example theme file (`~/.config/kimun/themes/mytheme.toml`)
/// ```toml
/// name = "My Theme"
/// bg               = "#1e1e2e"
/// bg_hard          = "#11111b"
/// bg_soft          = "#313244"
/// bg_panel         = "#181825"
/// selection_bg      = "#313244"
/// fg               = "#cdd6f4"
/// fg_bright        = "#f5e0dc"
/// fg_secondary     = "#a6adc8"
/// gray         = "#6c7086"
/// selection_fg      = "#cdd6f4"
/// border_dim           = "#45475a"
/// focus_border   = "#89b4fa"
/// accent           = "#89b4fa"
/// cursor           = "#f5e0dc"
/// red              = "#f38ba8"
/// green            = "#a6e3a1"
/// yellow           = "#f9e2af"
/// blue             = "#89b4fa"
/// purple           = "#cba6f7"
/// aqua             = "#94e2d5"
/// orange           = "#fab387"
/// color_directory  = "#89dceb"
/// color_journal_date = "#94e2d5"
/// color_search_match = "#a6e3a1"
/// color_tag        = "#fab387"
/// blockquote_bar   = "#cba6f7"
/// code_bg          = "#181825"
/// ```
///
/// Roles introduced after a theme file was written may be omitted — they are
/// derived from the closest sibling role (see [`ThemeToml`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "ThemeToml")]
pub struct Theme {
    pub name: String,

    // ── Backgrounds ─────────────────────────────────────────────────────────
    /// Main/editor background.
    pub bg: ThemeColor,
    /// Hard-contrast background: modals and input fields.
    pub bg_hard: ThemeColor,
    /// Soft background: alternating rows, horizontal rules.
    pub bg_soft: ThemeColor,
    /// Sidebar / panel background (usually slightly offset from `bg`).
    pub bg_panel: ThemeColor,
    /// Background of the currently selected row in lists.
    pub selection_bg: ThemeColor,

    // ── Foreground / text ────────────────────────────────────────────────────
    /// Primary text color.
    pub fg: ThemeColor,
    /// Bright/high-contrast text: titles, headings.
    pub fg_bright: ThemeColor,
    /// Secondary text: filenames, metadata, subdued hints.
    pub fg_secondary: ThemeColor,
    /// Very dim text: placeholders, separators, disabled items.
    pub gray: ThemeColor,
    /// Text color of a selected/highlighted row (often brighter than `fg`).
    pub selection_fg: ThemeColor,

    // ── Borders ──────────────────────────────────────────────────────────────
    /// Default (unfocused) border color.
    pub border_dim: ThemeColor,
    /// Border color when the pane has keyboard focus.
    pub focus_border: ThemeColor,

    // ── Accent ───────────────────────────────────────────────────────────────
    /// Primary accent: title bars, active markers, cursor highlights.
    pub accent: ThemeColor,
    /// Block-cursor color in text fields.
    pub cursor: ThemeColor,

    // ── Accent palette (semantic, theme-bound) ──────────────────────────────
    /// Errors, destructive actions, query negation.
    pub red: ThemeColor,
    /// Success / OK states.
    pub green: ThemeColor,
    /// Warnings, field keys, keycaps.
    pub yellow: ThemeColor,
    /// Wikilink targets.
    pub blue: ThemeColor,
    /// Numbers and date literals.
    pub purple: ThemeColor,
    /// Tags, links, group labels.
    pub aqua: ThemeColor,
    /// Operators and strong accents.
    pub orange: ThemeColor,

    // ── Semantic colors for file-list entries ────────────────────────────────
    /// Color used for directory entries in the file list.
    pub color_directory: ThemeColor,
    /// Color for the journal-date annotation line in journal entries.
    pub color_journal_date: ThemeColor,
    /// Color for highlighted search-match text.
    pub color_search_match: ThemeColor,
    /// Color for #hashtag label spans in the editor.
    pub color_tag: ThemeColor,
    /// Color of the `│` blockquote bar drawn in place of `>` markers.
    pub blockquote_bar: ThemeColor,
    /// Background of fenced and indented code blocks (the "code box").
    /// Inline `code` uses `selection_bg`, not this.
    pub code_bg: ThemeColor,
}

/// Deserialization shadow of [`Theme`].
///
/// Newer roles are optional so theme TOML files written before they existed
/// still parse; missing roles are derived from the closest sibling role (or a
/// chromatic ANSI slot for the accent palette, which adapts to the user's
/// terminal palette) in [`From<ThemeToml>`].
#[derive(Deserialize)]
struct ThemeToml {
    name: String,
    bg: ThemeColor,
    bg_hard: Option<ThemeColor>,
    bg_soft: Option<ThemeColor>,
    bg_panel: ThemeColor,
    selection_bg: ThemeColor,
    fg: ThemeColor,
    fg_bright: Option<ThemeColor>,
    fg_secondary: ThemeColor,
    gray: ThemeColor,
    selection_fg: ThemeColor,
    border_dim: ThemeColor,
    focus_border: ThemeColor,
    accent: ThemeColor,
    cursor: Option<ThemeColor>,
    red: Option<ThemeColor>,
    green: Option<ThemeColor>,
    yellow: Option<ThemeColor>,
    blue: Option<ThemeColor>,
    purple: Option<ThemeColor>,
    aqua: Option<ThemeColor>,
    orange: Option<ThemeColor>,
    color_directory: ThemeColor,
    color_journal_date: ThemeColor,
    color_search_match: ThemeColor,
    color_tag: Option<ThemeColor>,
    blockquote_bar: Option<ThemeColor>,
    code_bg: Option<ThemeColor>,
}

impl From<ThemeToml> for Theme {
    fn from(t: ThemeToml) -> Self {
        let orange = t.orange.unwrap_or(ThemeColor::Ansi(208));
        Theme {
            name: t.name,
            bg_hard: t.bg_hard.unwrap_or_else(|| t.bg_panel.clone()),
            bg_soft: t.bg_soft.unwrap_or_else(|| t.selection_bg.clone()),
            fg_bright: t.fg_bright.unwrap_or_else(|| t.selection_fg.clone()),
            cursor: t.cursor.unwrap_or_else(|| t.fg.clone()),
            red: t.red.unwrap_or(ThemeColor::Ansi(9)),
            green: t.green.unwrap_or(ThemeColor::Ansi(10)),
            yellow: t.yellow.unwrap_or(ThemeColor::Ansi(11)),
            blue: t.blue.unwrap_or(ThemeColor::Ansi(12)),
            purple: t.purple.unwrap_or(ThemeColor::Ansi(13)),
            aqua: t.aqua.unwrap_or(ThemeColor::Ansi(14)),
            color_tag: t.color_tag.unwrap_or_else(|| orange.clone()),
            blockquote_bar: t.blockquote_bar.unwrap_or_else(|| t.accent.clone()),
            code_bg: t.code_bg.unwrap_or_else(|| t.bg_panel.clone()),
            orange,
            bg: t.bg,
            bg_panel: t.bg_panel,
            selection_bg: t.selection_bg,
            fg: t.fg,
            fg_secondary: t.fg_secondary,
            gray: t.gray,
            selection_fg: t.selection_fg,
            border_dim: t.border_dim,
            focus_border: t.focus_border,
            accent: t.accent,
            color_directory: t.color_directory,
            color_journal_date: t.color_journal_date,
            color_search_match: t.color_search_match,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::gruvbox_dark()
    }
}

impl Theme {
    /// Returns the appropriate border style depending on focus state.
    pub fn border_style(&self, focused: bool) -> Style {
        if focused {
            Style::default().fg(self.focus_border.to_ratatui())
        } else {
            Style::default().fg(self.border_dim.to_ratatui())
        }
    }

    /// Base style for most surfaces: theme fg on theme bg.
    pub fn base_style(&self) -> Style {
        Style::default()
            .fg(self.fg.to_ratatui())
            .bg(self.bg.to_ratatui())
    }

    /// Panel style for sidebars and panels: theme fg on bg_panel.
    pub fn panel_style(&self) -> Style {
        Style::default()
            .fg(self.fg.to_ratatui())
            .bg(self.bg_panel.to_ratatui())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    #[test]
    fn test_border_style_focused() {
        let theme = Theme::gruvbox_dark();
        let style = theme.border_style(true);
        assert_eq!(style, Style::default().fg(theme.focus_border.to_ratatui()));
    }

    #[test]
    fn test_border_style_unfocused() {
        let theme = Theme::gruvbox_dark();
        let style = theme.border_style(false);
        assert_eq!(style, Style::default().fg(theme.border_dim.to_ratatui()));
    }

    #[test]
    fn test_from_hex_6char() {
        assert_eq!(
            ThemeColor::from_string("#ff8800").unwrap(),
            ThemeColor::Rgb(255, 136, 0)
        );
    }

    #[test]
    fn test_from_hex_6char_lowercase() {
        assert_eq!(
            ThemeColor::from_string("#abcdef").unwrap(),
            ThemeColor::Rgb(171, 205, 239)
        );
    }

    #[test]
    fn test_from_hex_6char_uppercase() {
        assert_eq!(
            ThemeColor::from_string("#ABCDEF").unwrap(),
            ThemeColor::Rgb(171, 205, 239)
        );
    }

    #[test]
    fn test_from_hex_3char() {
        assert_eq!(
            ThemeColor::from_string("#f80").unwrap(),
            ThemeColor::Rgb(255, 136, 0)
        );
    }

    #[test]
    fn test_from_hex_3char_expansion() {
        assert_eq!(
            ThemeColor::from_string("#abc").unwrap(),
            ThemeColor::Rgb(170, 187, 204)
        );
    }

    #[test]
    fn test_from_hex_3char_black() {
        assert_eq!(
            ThemeColor::from_string("#000").unwrap(),
            ThemeColor::Rgb(0, 0, 0)
        );
    }

    #[test]
    fn test_from_hex_3char_white() {
        assert_eq!(
            ThemeColor::from_string("#fff").unwrap(),
            ThemeColor::Rgb(255, 255, 255)
        );
    }

    #[test]
    fn test_from_rgb_string() {
        assert_eq!(
            ThemeColor::from_string("rgb(255, 128, 0)").unwrap(),
            ThemeColor::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn test_from_rgb_string_no_spaces() {
        assert_eq!(
            ThemeColor::from_string("rgb(255,128,0)").unwrap(),
            ThemeColor::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn test_from_rgb_string_extra_spaces() {
        assert_eq!(
            ThemeColor::from_string("rgb( 255 , 128 , 0 )").unwrap(),
            ThemeColor::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn test_from_rgb_string_min_max() {
        assert_eq!(
            ThemeColor::from_string("rgb(0, 255, 0)").unwrap(),
            ThemeColor::Rgb(0, 255, 0)
        );
    }

    #[test]
    fn test_from_string_with_whitespace() {
        assert_eq!(
            ThemeColor::from_string("  #ff8800  ").unwrap(),
            ThemeColor::Rgb(255, 136, 0)
        );
    }

    #[test]
    fn test_ansi_to_ratatui() {
        // Low 16 ANSI indices map to named ratatui variants
        assert_eq!(ThemeColor::Ansi(0).to_ratatui(), Color::Black);
        assert_eq!(ThemeColor::Ansi(4).to_ratatui(), Color::Blue);
        assert_eq!(ThemeColor::Ansi(7).to_ratatui(), Color::Gray);
        assert_eq!(ThemeColor::Ansi(8).to_ratatui(), Color::DarkGray);
        assert_eq!(ThemeColor::Ansi(15).to_ratatui(), Color::White);
        // Indices >= 16 still use 256-color
        assert_eq!(ThemeColor::Ansi(42).to_ratatui(), Color::Indexed(42));
        assert_eq!(ThemeColor::Reset.to_ratatui(), Color::Reset);
    }

    #[test]
    fn test_invalid_hex_length() {
        let result = ThemeColor::from_string("#ff880");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid hex color length"));
    }

    #[test]
    fn test_invalid_hex_chars() {
        let result = ThemeColor::from_string("#gghhii");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_hash() {
        let result = ThemeColor::from_string("ff8800");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid color format"));
    }

    #[test]
    fn test_invalid_rgb_format() {
        let result = ThemeColor::from_string("rgb(255, 128)");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires 3 values"));
    }

    #[test]
    fn test_rgb_value_out_of_range() {
        let result = ThemeColor::from_string("rgb(256, 128, 0)");
        assert!(result.is_err());
    }

    #[test]
    fn test_rgb_negative_value() {
        let result = ThemeColor::from_string("rgb(-1, 128, 0)");
        assert!(result.is_err());
    }

    #[test]
    fn test_rgb_non_numeric() {
        let result = ThemeColor::from_string("rgb(abc, 128, 0)");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid red value"));
    }

    #[test]
    fn test_invalid_format() {
        let result = ThemeColor::from_string("not a color");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid color format"));
    }

    #[test]
    fn test_empty_string() {
        let result = ThemeColor::from_string("");
        assert!(result.is_err());
    }

    #[test]
    fn test_new_constructor() {
        assert_eq!(ThemeColor::new(255, 128, 0), ThemeColor::Rgb(255, 128, 0));
    }

    #[test]
    fn test_to_ratatui() {
        let color = ThemeColor::new(131, 165, 152);
        assert_eq!(color.to_ratatui(), Color::Rgb(131, 165, 152));
    }

    #[test]
    fn test_theme_color_serialize() {
        #[derive(Serialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let wrapper = Wrapper {
            color: ThemeColor::new(59, 130, 246),
        };
        let serialized = toml::to_string(&wrapper).unwrap();
        assert!(serialized.contains("color = \"#3b82f6\""));
    }

    #[test]
    fn test_theme_color_deserialize() {
        #[derive(Deserialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let toml_str = r###"color = "#3b82f6""###;
        let wrapper: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(wrapper.color, ThemeColor::Rgb(59, 130, 246));
    }

    #[test]
    fn test_theme_color_roundtrip() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let original = Wrapper {
            color: ThemeColor::new(239, 68, 68),
        };
        let serialized = toml::to_string(&original).unwrap();
        let deserialized: Wrapper = toml::from_str(&serialized).unwrap();
        assert_eq!(original.color, deserialized.color);
    }

    #[test]
    fn test_theme_serialize_to_toml() {
        let theme = Theme::gruvbox_dark();
        let toml_string = toml::to_string_pretty(&theme).unwrap();

        assert!(toml_string.contains("name = \"Gruvbox Dark\""));
        assert!(toml_string.contains("bg = \"#282828\""));
        assert!(toml_string.contains("bg_panel = \"#32302f\""));
        assert!(toml_string.contains("focus_border = \"#b8bb26\""));
        assert!(toml_string.contains("color_journal_date = \"#8ec07c\""));
    }

    #[test]
    fn test_theme_deserialize_from_toml() {
        let toml_str = r###"
            name = "Test Theme"
            bg                 = "#282828"
            bg_panel           = "#32302f"
            selection_bg        = "#504945"
            fg                 = "#ebdbb2"
            fg_secondary       = "#a89984"
            gray           = "#7c6f64"
            selection_fg        = "#fbf1c7"
            border_dim             = "#504945"
            focus_border     = "#fabd2f"
            accent             = "#fabd2f"
            color_directory    = "#83a598"
            color_journal_date = "#8ec07c"
            color_search_match = "#b8bb26"
            color_tag          = "#fe8019"
        "###;

        let theme: Theme = toml::from_str(toml_str).unwrap();
        assert_eq!(theme.name, "Test Theme");
        assert_eq!(theme.bg, ThemeColor::new(0x28, 0x28, 0x28));
        assert_eq!(theme.focus_border, ThemeColor::new(0xfa, 0xbd, 0x2f));
        assert_eq!(theme.color_journal_date, ThemeColor::new(0x8e, 0xc0, 0x7c));
    }

    #[test]
    fn test_theme_roundtrip() {
        let original = Theme::tokyo_night();
        let toml_string = toml::to_string_pretty(&original).unwrap();
        let deserialized: Theme = toml::from_str(&toml_string).unwrap();

        assert_eq!(original.name, deserialized.name);
        assert_eq!(original.bg, deserialized.bg);
        assert_eq!(original.fg, deserialized.fg);
        assert_eq!(original.focus_border, deserialized.focus_border);
        assert_eq!(original.color_journal_date, deserialized.color_journal_date);
    }

    #[test]
    fn test_theme_color_serialize_lowercase_hex() {
        #[derive(Serialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let wrapper = Wrapper {
            color: ThemeColor::new(171, 205, 239),
        };
        let serialized = toml::to_string(&wrapper).unwrap();
        assert!(serialized.contains("color = \"#abcdef\""));
    }

    #[test]
    fn test_theme_deserialize_uppercase_hex() {
        #[derive(Deserialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let toml_str = r###"color = "#ABCDEF""###;
        let wrapper: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(wrapper.color, ThemeColor::Rgb(171, 205, 239));
    }

    #[test]
    fn test_theme_deserialize_3char_hex() {
        #[derive(Deserialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let toml_str = r###"color = "#abc""###;
        let wrapper: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(wrapper.color, ThemeColor::Rgb(170, 187, 204));
    }

    #[test]
    fn test_from_ansi_index() {
        assert_eq!(
            ThemeColor::from_string("ansi:4").unwrap(),
            ThemeColor::Ansi(4)
        );
        assert_eq!(
            ThemeColor::from_string("ansi:255").unwrap(),
            ThemeColor::Ansi(255)
        );
    }

    #[test]
    fn test_from_reset() {
        assert_eq!(ThemeColor::from_string("reset").unwrap(), ThemeColor::Reset);
    }

    #[test]
    fn test_all_builtin_themes_serialize() {
        let themes = vec![
            Theme::ansi(),
            Theme::gruvbox_dark(),
            Theme::gruvbox_light(),
            Theme::catppuccin_mocha(),
            Theme::catppuccin_latte(),
            Theme::tokyo_night(),
            Theme::tokyo_night_storm(),
            Theme::solarized_dark(),
            Theme::solarized_light(),
            Theme::nord(),
        ];
        for theme in themes {
            let toml_string = toml::to_string_pretty(&theme).unwrap();
            let roundtrip: Theme = toml::from_str(&toml_string).unwrap();
            assert_eq!(theme.name, roundtrip.name);
            assert_eq!(theme.bg, roundtrip.bg);
        }
    }

    #[test]
    fn test_ansi_theme() {
        let theme = Theme::ansi();
        assert_eq!(theme.name, "ANSI");
        assert_eq!(theme.bg, ThemeColor::Reset);
        assert_eq!(theme.fg, ThemeColor::Reset);
        assert_eq!(theme.selection_bg, ThemeColor::Ansi(4));
        assert_eq!(theme.focus_border, ThemeColor::Ansi(10));
        assert_eq!(theme.color_directory, ThemeColor::Ansi(12));
    }

    #[test]
    fn new_decoration_fields_present_and_deserialize_default() {
        // Built-in theme exposes the fields.
        let t = Theme::gruvbox_dark();
        assert_eq!(
            t.blockquote_bar,
            ThemeColor::from_string("#fabd2f").unwrap()
        );
        assert_eq!(t.code_bg, ThemeColor::from_string("#32302f").unwrap());

        // Old TOML without the fields still deserializes (serde defaults kick in).
        let toml = r##"
            name = "Old"
            bg = "#000000"
            bg_panel = "#111111"
            selection_bg = "#222222"
            fg = "#ffffff"
            fg_secondary = "#cccccc"
            gray = "#888888"
            selection_fg = "#ffffff"
            border_dim = "#333333"
            focus_border = "#444444"
            accent = "#55aaff"
            color_directory = "#66ccee"
            color_journal_date = "#77ddcc"
            color_search_match = "#88eeaa"
        "##;
        let parsed: Theme = toml::from_str(toml).expect("old theme TOML must still parse");
        // Sibling-derived defaults.
        assert_eq!(parsed.blockquote_bar, parsed.accent);
        assert_eq!(parsed.code_bg, parsed.bg_panel);
    }

    #[test]
    fn old_theme_toml_derives_new_roles_from_siblings() {
        // A theme file written before the §1 role expansion must still parse,
        // with the new roles derived from their closest sibling.
        let toml = r##"
            name = "Old"
            bg = "#000000"
            bg_panel = "#111111"
            selection_bg = "#222222"
            fg = "#ffffff"
            fg_secondary = "#cccccc"
            gray = "#888888"
            selection_fg = "#eeeeee"
            border_dim = "#333333"
            focus_border = "#444444"
            accent = "#55aaff"
            color_directory = "#66ccee"
            color_journal_date = "#77ddcc"
            color_search_match = "#88eeaa"
        "##;
        let t: Theme = toml::from_str(toml).expect("old theme TOML must still parse");
        assert_eq!(t.bg_hard, t.bg_panel);
        assert_eq!(t.bg_soft, t.selection_bg);
        assert_eq!(t.fg_bright, t.selection_fg);
        assert_eq!(t.cursor, t.fg);
        // Accent palette falls back to chromatic ANSI slots.
        assert_eq!(t.red, ThemeColor::Ansi(9));
        assert_eq!(t.green, ThemeColor::Ansi(10));
        assert_eq!(t.yellow, ThemeColor::Ansi(11));
        assert_eq!(t.blue, ThemeColor::Ansi(12));
        assert_eq!(t.purple, ThemeColor::Ansi(13));
        assert_eq!(t.aqua, ThemeColor::Ansi(14));
        assert_eq!(t.orange, ThemeColor::Ansi(208));
        // color_tag tracks the orange accent when absent.
        assert_eq!(t.color_tag, t.orange);
    }

    #[test]
    fn new_roles_roundtrip_through_toml() {
        let original = Theme::gruvbox_dark();
        let toml_string = toml::to_string_pretty(&original).unwrap();
        let parsed: Theme = toml::from_str(&toml_string).unwrap();
        assert_eq!(original, parsed);
    }
}
