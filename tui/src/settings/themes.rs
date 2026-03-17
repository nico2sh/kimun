use ratatui::style::{Color, Style};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::Display;

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeColor {
    r: u8,
    g: u8,
    b: u8,
}

impl Serialize for ThemeColor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex = format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b);
        serializer.serialize_str(&hex)
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
        ThemeColor { r, g, b }
    }

    /// Convert to a ratatui `Color::Rgb` value for use in widget styles.
    pub fn to_ratatui(&self) -> Color {
        Color::Rgb(self.r, self.g, self.b)
    }

    /// Parse a color from a string in various formats:
    /// - RGB: "rgb(255, 128, 0)"
    /// - 3-char hex: "#abc" (expanded to #aabbcc)
    /// - 6-char hex: "#aabbcc"
    pub fn from_string(s: &str) -> Result<Self, String> {
        let s = s.trim();

        if s.starts_with('#') {
            Self::from_hex(s)
        } else if s.starts_with("rgb(") && s.ends_with(')') {
            Self::from_rgb_string(s)
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

        Ok(ThemeColor { r, g, b })
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

        Ok(ThemeColor { r, g, b })
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

        Ok(ThemeColor { r, g, b })
    }
}

impl Display for ThemeColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rgb({},{},{})", self.r, self.g, self.b)
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
/// bg_panel         = "#181825"
/// bg_selected      = "#313244"
/// fg               = "#cdd6f4"
/// fg_secondary     = "#a6adc8"
/// fg_muted         = "#6c7086"
/// fg_selected      = "#cdd6f4"
/// border           = "#45475a"
/// border_focused   = "#89b4fa"
/// accent           = "#89b4fa"
/// color_directory  = "#89dceb"
/// color_journal_date = "#94e2d5"
/// color_search_match = "#a6e3a1"
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,

    // ── Backgrounds ─────────────────────────────────────────────────────────
    /// Main/editor background.
    pub bg: ThemeColor,
    /// Sidebar / panel background (usually slightly offset from `bg`).
    pub bg_panel: ThemeColor,
    /// Background of the currently selected row in lists.
    pub bg_selected: ThemeColor,

    // ── Foreground / text ────────────────────────────────────────────────────
    /// Primary text color.
    pub fg: ThemeColor,
    /// Secondary text: filenames, metadata, subdued hints.
    pub fg_secondary: ThemeColor,
    /// Very dim text: placeholders, separators, disabled items.
    pub fg_muted: ThemeColor,
    /// Text color of a selected/highlighted row (often brighter than `fg`).
    pub fg_selected: ThemeColor,

    // ── Borders ──────────────────────────────────────────────────────────────
    /// Default (unfocused) border color.
    pub border: ThemeColor,
    /// Border color when the pane has keyboard focus.
    pub border_focused: ThemeColor,

    // ── Accent ───────────────────────────────────────────────────────────────
    /// Primary accent: title bars, active markers, cursor highlights.
    pub accent: ThemeColor,

    // ── Semantic colors for file-list entries ────────────────────────────────
    /// Color used for directory entries in the file list.
    pub color_directory: ThemeColor,
    /// Color for the journal-date annotation line in journal entries.
    pub color_journal_date: ThemeColor,
    /// Color for highlighted search-match text.
    pub color_search_match: ThemeColor,
}

impl Default for Theme {
    fn default() -> Self {
        Self::gruvbox_dark()
    }
}

impl Theme {
    // ── Built-in themes ──────────────────────────────────────────────────────

    pub fn gruvbox_dark() -> Self {
        Theme {
            name: "Gruvbox Dark".to_string(),
            bg: ThemeColor::from_string("#282828").unwrap(),
            bg_panel: ThemeColor::from_string("#32302f").unwrap(),
            bg_selected: ThemeColor::from_string("#504945").unwrap(),
            fg: ThemeColor::from_string("#ebdbb2").unwrap(),
            fg_secondary: ThemeColor::from_string("#a89984").unwrap(),
            fg_muted: ThemeColor::from_string("#7c6f64").unwrap(),
            fg_selected: ThemeColor::from_string("#fbf1c7").unwrap(),
            border: ThemeColor::from_string("#504945").unwrap(),
            border_focused: ThemeColor::from_string("#fabd2f").unwrap(),
            accent: ThemeColor::from_string("#fabd2f").unwrap(),
            color_directory: ThemeColor::from_string("#83a598").unwrap(),
            color_journal_date: ThemeColor::from_string("#8ec07c").unwrap(),
            color_search_match: ThemeColor::from_string("#b8bb26").unwrap(),
        }
    }

    pub fn gruvbox_light() -> Self {
        Theme {
            name: "Gruvbox Light".to_string(),
            bg: ThemeColor::from_string("#fbf1c7").unwrap(),
            bg_panel: ThemeColor::from_string("#f2e5bc").unwrap(),
            bg_selected: ThemeColor::from_string("#ebdbb2").unwrap(),
            fg: ThemeColor::from_string("#3c3836").unwrap(),
            fg_secondary: ThemeColor::from_string("#7c6f64").unwrap(),
            fg_muted: ThemeColor::from_string("#a89984").unwrap(),
            fg_selected: ThemeColor::from_string("#282828").unwrap(),
            border: ThemeColor::from_string("#d5c4a1").unwrap(),
            border_focused: ThemeColor::from_string("#d79921").unwrap(),
            accent: ThemeColor::from_string("#d79921").unwrap(),
            color_directory: ThemeColor::from_string("#458588").unwrap(),
            color_journal_date: ThemeColor::from_string("#689d6a").unwrap(),
            color_search_match: ThemeColor::from_string("#98971a").unwrap(),
        }
    }

    pub fn catppuccin_mocha() -> Self {
        Theme {
            name: "Catppuccin Mocha".to_string(),
            bg: ThemeColor::from_string("#1e1e2e").unwrap(),
            bg_panel: ThemeColor::from_string("#181825").unwrap(),
            bg_selected: ThemeColor::from_string("#313244").unwrap(),
            fg: ThemeColor::from_string("#cdd6f4").unwrap(),
            fg_secondary: ThemeColor::from_string("#a6adc8").unwrap(),
            fg_muted: ThemeColor::from_string("#6c7086").unwrap(),
            fg_selected: ThemeColor::from_string("#cdd6f4").unwrap(),
            border: ThemeColor::from_string("#45475a").unwrap(),
            border_focused: ThemeColor::from_string("#89b4fa").unwrap(),
            accent: ThemeColor::from_string("#cba6f7").unwrap(),
            color_directory: ThemeColor::from_string("#89dceb").unwrap(),
            color_journal_date: ThemeColor::from_string("#94e2d5").unwrap(),
            color_search_match: ThemeColor::from_string("#a6e3a1").unwrap(),
        }
    }

    pub fn catppuccin_latte() -> Self {
        Theme {
            name: "Catppuccin Latte".to_string(),
            bg: ThemeColor::from_string("#eff1f5").unwrap(),
            bg_panel: ThemeColor::from_string("#e6e9ef").unwrap(),
            bg_selected: ThemeColor::from_string("#ccd0da").unwrap(),
            fg: ThemeColor::from_string("#4c4f69").unwrap(),
            fg_secondary: ThemeColor::from_string("#6c6f85").unwrap(),
            fg_muted: ThemeColor::from_string("#9ca0b0").unwrap(),
            fg_selected: ThemeColor::from_string("#4c4f69").unwrap(),
            border: ThemeColor::from_string("#ccd0da").unwrap(),
            border_focused: ThemeColor::from_string("#1e66f5").unwrap(),
            accent: ThemeColor::from_string("#8839ef").unwrap(),
            color_directory: ThemeColor::from_string("#04a5e5").unwrap(),
            color_journal_date: ThemeColor::from_string("#179299").unwrap(),
            color_search_match: ThemeColor::from_string("#40a02b").unwrap(),
        }
    }

    pub fn tokyo_night() -> Self {
        Theme {
            name: "Tokyo Night".to_string(),
            bg: ThemeColor::from_string("#1a1b26").unwrap(),
            bg_panel: ThemeColor::from_string("#16161e").unwrap(),
            bg_selected: ThemeColor::from_string("#292e42").unwrap(),
            fg: ThemeColor::from_string("#c0caf5").unwrap(),
            fg_secondary: ThemeColor::from_string("#a9b1d6").unwrap(),
            fg_muted: ThemeColor::from_string("#565f89").unwrap(),
            fg_selected: ThemeColor::from_string("#c0caf5").unwrap(),
            border: ThemeColor::from_string("#3b4261").unwrap(),
            border_focused: ThemeColor::from_string("#7aa2f7").unwrap(),
            accent: ThemeColor::from_string("#7aa2f7").unwrap(),
            color_directory: ThemeColor::from_string("#7dcfff").unwrap(),
            color_journal_date: ThemeColor::from_string("#73daca").unwrap(),
            color_search_match: ThemeColor::from_string("#9ece6a").unwrap(),
        }
    }

    pub fn tokyo_night_storm() -> Self {
        Theme {
            name: "Tokyo Night Storm".to_string(),
            bg: ThemeColor::from_string("#24283b").unwrap(),
            bg_panel: ThemeColor::from_string("#1f2335").unwrap(),
            bg_selected: ThemeColor::from_string("#364a82").unwrap(),
            fg: ThemeColor::from_string("#c0caf5").unwrap(),
            fg_secondary: ThemeColor::from_string("#a9b1d6").unwrap(),
            fg_muted: ThemeColor::from_string("#565f89").unwrap(),
            fg_selected: ThemeColor::from_string("#c0caf5").unwrap(),
            border: ThemeColor::from_string("#3b4261").unwrap(),
            border_focused: ThemeColor::from_string("#7aa2f7").unwrap(),
            accent: ThemeColor::from_string("#bb9af7").unwrap(),
            color_directory: ThemeColor::from_string("#7dcfff").unwrap(),
            color_journal_date: ThemeColor::from_string("#73daca").unwrap(),
            color_search_match: ThemeColor::from_string("#9ece6a").unwrap(),
        }
    }

    pub fn solarized_dark() -> Self {
        Theme {
            name: "Solarized Dark".to_string(),
            bg: ThemeColor::from_string("#002b36").unwrap(),
            bg_panel: ThemeColor::from_string("#073642").unwrap(),
            bg_selected: ThemeColor::from_string("#586e75").unwrap(),
            fg: ThemeColor::from_string("#839496").unwrap(),
            fg_secondary: ThemeColor::from_string("#657b83").unwrap(),
            fg_muted: ThemeColor::from_string("#586e75").unwrap(),
            fg_selected: ThemeColor::from_string("#eee8d5").unwrap(),
            border: ThemeColor::from_string("#073642").unwrap(),
            border_focused: ThemeColor::from_string("#268bd2").unwrap(),
            accent: ThemeColor::from_string("#268bd2").unwrap(),
            color_directory: ThemeColor::from_string("#2aa198").unwrap(),
            color_journal_date: ThemeColor::from_string("#859900").unwrap(),
            color_search_match: ThemeColor::from_string("#b58900").unwrap(),
        }
    }

    pub fn solarized_light() -> Self {
        Theme {
            name: "Solarized Light".to_string(),
            bg: ThemeColor::from_string("#fdf6e3").unwrap(),
            bg_panel: ThemeColor::from_string("#eee8d5").unwrap(),
            bg_selected: ThemeColor::from_string("#93a1a1").unwrap(),
            fg: ThemeColor::from_string("#657b83").unwrap(),
            fg_secondary: ThemeColor::from_string("#839496").unwrap(),
            fg_muted: ThemeColor::from_string("#93a1a1").unwrap(),
            fg_selected: ThemeColor::from_string("#073642").unwrap(),
            border: ThemeColor::from_string("#eee8d5").unwrap(),
            border_focused: ThemeColor::from_string("#268bd2").unwrap(),
            accent: ThemeColor::from_string("#268bd2").unwrap(),
            color_directory: ThemeColor::from_string("#2aa198").unwrap(),
            color_journal_date: ThemeColor::from_string("#859900").unwrap(),
            color_search_match: ThemeColor::from_string("#b58900").unwrap(),
        }
    }

    /// Returns the appropriate border style depending on focus state.
    pub fn border_style(&self, focused: bool) -> Style {
        if focused {
            Style::default().fg(self.border_focused.to_ratatui())
        } else {
            Style::default().fg(self.border.to_ratatui())
        }
    }

    pub fn nord() -> Self {
        Theme {
            name: "Nord".to_string(),
            bg: ThemeColor::from_string("#2e3440").unwrap(),
            bg_panel: ThemeColor::from_string("#3b4252").unwrap(),
            bg_selected: ThemeColor::from_string("#434c5e").unwrap(),
            fg: ThemeColor::from_string("#eceff4").unwrap(),
            fg_secondary: ThemeColor::from_string("#d8dee9").unwrap(),
            fg_muted: ThemeColor::from_string("#4c566a").unwrap(),
            fg_selected: ThemeColor::from_string("#eceff4").unwrap(),
            border: ThemeColor::from_string("#434c5e").unwrap(),
            border_focused: ThemeColor::from_string("#81a1c1").unwrap(),
            accent: ThemeColor::from_string("#88c0d0").unwrap(),
            color_directory: ThemeColor::from_string("#81a1c1").unwrap(),
            color_journal_date: ThemeColor::from_string("#8fbcbb").unwrap(),
            color_search_match: ThemeColor::from_string("#a3be8c").unwrap(),
        }
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
        assert_eq!(style, Style::default().fg(theme.border_focused.to_ratatui()));
    }

    #[test]
    fn test_border_style_unfocused() {
        let theme = Theme::gruvbox_dark();
        let style = theme.border_style(false);
        assert_eq!(style, Style::default().fg(theme.border.to_ratatui()));
    }

    #[test]
    fn test_from_hex_6char() {
        let color = ThemeColor::from_string("#ff8800").unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 136);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_from_hex_6char_lowercase() {
        let color = ThemeColor::from_string("#abcdef").unwrap();
        assert_eq!(color.r, 171);
        assert_eq!(color.g, 205);
        assert_eq!(color.b, 239);
    }

    #[test]
    fn test_from_hex_6char_uppercase() {
        let color = ThemeColor::from_string("#ABCDEF").unwrap();
        assert_eq!(color.r, 171);
        assert_eq!(color.g, 205);
        assert_eq!(color.b, 239);
    }

    #[test]
    fn test_from_hex_3char() {
        let color = ThemeColor::from_string("#f80").unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 136);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_from_hex_3char_expansion() {
        let color = ThemeColor::from_string("#abc").unwrap();
        assert_eq!(color.r, 170);
        assert_eq!(color.g, 187);
        assert_eq!(color.b, 204);
    }

    #[test]
    fn test_from_hex_3char_black() {
        let color = ThemeColor::from_string("#000").unwrap();
        assert_eq!(color.r, 0);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_from_hex_3char_white() {
        let color = ThemeColor::from_string("#fff").unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 255);
        assert_eq!(color.b, 255);
    }

    #[test]
    fn test_from_rgb_string() {
        let color = ThemeColor::from_string("rgb(255, 128, 0)").unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 128);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_from_rgb_string_no_spaces() {
        let color = ThemeColor::from_string("rgb(255,128,0)").unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 128);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_from_rgb_string_extra_spaces() {
        let color = ThemeColor::from_string("rgb( 255 , 128 , 0 )").unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 128);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_from_rgb_string_min_max() {
        let color = ThemeColor::from_string("rgb(0, 255, 0)").unwrap();
        assert_eq!(color.r, 0);
        assert_eq!(color.g, 255);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_from_string_with_whitespace() {
        let color = ThemeColor::from_string("  #ff8800  ").unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 136);
        assert_eq!(color.b, 0);
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
        let color = ThemeColor::new(255, 128, 0);
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 128);
        assert_eq!(color.b, 0);
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
        assert_eq!(wrapper.color.r, 59);
        assert_eq!(wrapper.color.g, 130);
        assert_eq!(wrapper.color.b, 246);
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
        assert!(toml_string.contains("border_focused = \"#fabd2f\""));
        assert!(toml_string.contains("color_journal_date = \"#8ec07c\""));
    }

    #[test]
    fn test_theme_deserialize_from_toml() {
        let toml_str = r###"
            name = "Test Theme"
            bg                 = "#282828"
            bg_panel           = "#32302f"
            bg_selected        = "#504945"
            fg                 = "#ebdbb2"
            fg_secondary       = "#a89984"
            fg_muted           = "#7c6f64"
            fg_selected        = "#fbf1c7"
            border             = "#504945"
            border_focused     = "#fabd2f"
            accent             = "#fabd2f"
            color_directory    = "#83a598"
            color_journal_date = "#8ec07c"
            color_search_match = "#b8bb26"
        "###;

        let theme: Theme = toml::from_str(toml_str).unwrap();
        assert_eq!(theme.name, "Test Theme");
        assert_eq!(theme.bg, ThemeColor::new(0x28, 0x28, 0x28));
        assert_eq!(theme.border_focused, ThemeColor::new(0xfa, 0xbd, 0x2f));
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
        assert_eq!(original.border_focused, deserialized.border_focused);
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
        assert_eq!(wrapper.color.r, 171);
        assert_eq!(wrapper.color.g, 205);
        assert_eq!(wrapper.color.b, 239);
    }

    #[test]
    fn test_theme_deserialize_3char_hex() {
        #[derive(Deserialize)]
        struct Wrapper {
            color: ThemeColor,
        }
        let toml_str = r###"color = "#abc""###;
        let wrapper: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(wrapper.color.r, 170);
        assert_eq!(wrapper.color.g, 187);
        assert_eq!(wrapper.color.b, 204);
    }

    #[test]
    fn test_all_builtin_themes_serialize() {
        let themes = vec![
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
}
