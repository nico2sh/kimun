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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    /* Light blue */
    pub accent_blue: ThemeColor,
    /* Standard blue */
    pub accent_blue_dark: ThemeColor,
    /* Light red */
    pub accent_red: ThemeColor,
    /* Standard red */
    pub accent_red_dark: ThemeColor,
    /* Light yellow */
    pub accent_yellow: ThemeColor,
    /* Standard yellow */
    pub accent_yellow_dark: ThemeColor,
    /* Bright green */
    pub accent_green: ThemeColor,
    /* Standard green */
    pub accent_green_dark: ThemeColor,
    /* Light gray */
    pub accent_gray: ThemeColor,
    /* Standard gray */
    pub accent_gray_dark: ThemeColor,

    pub bg_main: ThemeColor,
    pub bg_section: ThemeColor,
    pub bg_hover: ThemeColor,
    pub bg_surface: ThemeColor,
    pub bg_head: ThemeColor,
    pub text_primary: ThemeColor,
    pub text_secondary: ThemeColor,
    pub text_muted: ThemeColor,
    pub text_light: ThemeColor,
    pub text_contrast: ThemeColor,
    pub text_head: ThemeColor,
    pub border_light: ThemeColor,
    // border-focus: var(--primary-color),
    pub border_hover: ThemeColor,
}

impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

impl Theme {
    /// Light theme based on light.css
    pub fn light() -> Self {
        Theme {
            name: "Light".to_string(),
            accent_blue: ThemeColor::from_string("#3b82f6").unwrap(),
            accent_blue_dark: ThemeColor::from_string("#2563eb").unwrap(),
            accent_red: ThemeColor::from_string("#ef4444").unwrap(),
            accent_red_dark: ThemeColor::from_string("#dc2626").unwrap(),
            accent_yellow: ThemeColor::from_string("#eab308").unwrap(),
            accent_yellow_dark: ThemeColor::from_string("#ca8a04").unwrap(),
            accent_green: ThemeColor::from_string("#10b981").unwrap(),
            accent_green_dark: ThemeColor::from_string("#059669").unwrap(),
            accent_gray: ThemeColor::from_string("#6b7280").unwrap(),
            accent_gray_dark: ThemeColor::from_string("#4b5563").unwrap(),
            bg_main: ThemeColor::from_string("#f8fafc").unwrap(),
            bg_section: ThemeColor::from_string("#f1f5f9").unwrap(),
            bg_hover: ThemeColor::from_string("#e2e8f0").unwrap(),
            bg_surface: ThemeColor::from_string("#6b7280").unwrap(),
            bg_head: ThemeColor::from_string("#2563eb").unwrap(), // accent_blue_dark
            text_primary: ThemeColor::from_string("#333333").unwrap(),
            text_secondary: ThemeColor::from_string("#2d3748").unwrap(),
            text_muted: ThemeColor::from_string("#4a5568").unwrap(),
            text_light: ThemeColor::from_string("#6b7280").unwrap(),
            text_contrast: ThemeColor::from_string("#ffffff").unwrap(),
            text_head: ThemeColor::from_string("#ffffff").unwrap(), // text_contrast
            border_light: ThemeColor::from_string("#e2e8f0").unwrap(),
            border_hover: ThemeColor::from_string("#cbd5e0").unwrap(),
        }
    }

    pub fn gruvbox_light() -> Self {
        Theme {
            name: "Gruvbox Light".to_string(),
            accent_blue: ThemeColor::from_string("#458588").unwrap(),
            accent_blue_dark: ThemeColor::from_string("#076678").unwrap(),
            accent_red: ThemeColor::from_string("#cc241d").unwrap(),
            accent_red_dark: ThemeColor::from_string("#9d0006").unwrap(),
            accent_yellow: ThemeColor::from_string("#d79921").unwrap(),
            accent_yellow_dark: ThemeColor::from_string("#b57614").unwrap(),
            accent_green: ThemeColor::from_string("#98971a").unwrap(),
            accent_green_dark: ThemeColor::from_string("#79740e").unwrap(),
            accent_gray: ThemeColor::from_string("#928374").unwrap(),
            accent_gray_dark: ThemeColor::from_string("#7c6f64").unwrap(),
            bg_main: ThemeColor::from_string("#fbf1c7").unwrap(),
            bg_section: ThemeColor::from_string("#f2e5bc").unwrap(),
            bg_hover: ThemeColor::from_string("#ebdbb2").unwrap(),
            bg_surface: ThemeColor::from_string("#a89984").unwrap(),
            bg_head: ThemeColor::from_string("#98971a").unwrap(), // accent_green
            text_primary: ThemeColor::from_string("#3c3836").unwrap(),
            text_secondary: ThemeColor::from_string("#504945").unwrap(),
            text_muted: ThemeColor::from_string("#665c54").unwrap(),
            text_light: ThemeColor::from_string("#7c6f64").unwrap(),
            text_contrast: ThemeColor::from_string("#fbf1c7").unwrap(),
            text_head: ThemeColor::from_string("#fbf1c7").unwrap(), // text_contrast
            border_light: ThemeColor::from_string("#d5c4a1").unwrap(),
            border_hover: ThemeColor::from_string("#bdae93").unwrap(),
        }
    }

    /// Dark theme (inverted from light theme)
    pub fn dark() -> Self {
        Theme {
            name: "Dark".to_string(),
            // Keep accent colors the same as light theme for consistency
            accent_blue: ThemeColor::from_string("#3b82f6").unwrap(),
            accent_blue_dark: ThemeColor::from_string("#2563eb").unwrap(),
            accent_red: ThemeColor::from_string("#ef4444").unwrap(),
            accent_red_dark: ThemeColor::from_string("#dc2626").unwrap(),
            accent_yellow: ThemeColor::from_string("#eab308").unwrap(),
            accent_yellow_dark: ThemeColor::from_string("#ca8a04").unwrap(),
            accent_green: ThemeColor::from_string("#10b981").unwrap(),
            accent_green_dark: ThemeColor::from_string("#059669").unwrap(),
            accent_gray: ThemeColor::from_string("#9ca3af").unwrap(),
            accent_gray_dark: ThemeColor::from_string("#6b7280").unwrap(),
            // Dark background colors
            bg_main: ThemeColor::from_string("#1e1e1e").unwrap(),
            bg_section: ThemeColor::from_string("#2d2d2d").unwrap(),
            bg_hover: ThemeColor::from_string("#3a3a3a").unwrap(),
            bg_surface: ThemeColor::from_string("#4a4a4a").unwrap(),
            bg_head: ThemeColor::from_string("#2563eb").unwrap(), // accent_blue_dark
            // Light text colors for dark background
            text_primary: ThemeColor::from_string("#e4e4e4").unwrap(),
            text_secondary: ThemeColor::from_string("#d1d1d1").unwrap(),
            text_muted: ThemeColor::from_string("#a8a8a8").unwrap(),
            text_light: ThemeColor::from_string("#8a8a8a").unwrap(),
            text_contrast: ThemeColor::from_string("#1e1e1e").unwrap(),
            text_head: ThemeColor::from_string("#1e1e1e").unwrap(), // text_contrast
            // Dark border colors
            border_light: ThemeColor::from_string("#3a3a3a").unwrap(),
            border_hover: ThemeColor::from_string("#4a4a4a").unwrap(),
        }
    }

    pub fn gruvbox_dark() -> Self {
        Theme {
            name: "Gruvbox Dark".to_string(),
            accent_blue: ThemeColor::from_string("#83a598").unwrap(),
            accent_blue_dark: ThemeColor::from_string("#458588").unwrap(),
            accent_red: ThemeColor::from_string("#fb4934").unwrap(),
            accent_red_dark: ThemeColor::from_string("#cc241d").unwrap(),
            accent_yellow: ThemeColor::from_string("#fabd2f").unwrap(),
            accent_yellow_dark: ThemeColor::from_string("#d79921").unwrap(),
            accent_green: ThemeColor::from_string("#b8bb26").unwrap(),
            accent_green_dark: ThemeColor::from_string("#98971a").unwrap(),
            accent_gray: ThemeColor::from_string("#a89984").unwrap(),
            accent_gray_dark: ThemeColor::from_string("#928374").unwrap(),
            bg_main: ThemeColor::from_string("#282828").unwrap(),
            bg_section: ThemeColor::from_string("#32302f").unwrap(),
            bg_hover: ThemeColor::from_string("#504945").unwrap(),
            bg_surface: ThemeColor::from_string("#7c6f64").unwrap(),
            bg_head: ThemeColor::from_string("#d79921").unwrap(), // accent_yellow_dark
            text_primary: ThemeColor::from_string("#ebdbb2").unwrap(),
            text_secondary: ThemeColor::from_string("#d5c4a1").unwrap(),
            text_muted: ThemeColor::from_string("#bdae93").unwrap(),
            text_light: ThemeColor::from_string("#a89984").unwrap(),
            text_contrast: ThemeColor::from_string("#282828").unwrap(),
            text_head: ThemeColor::from_string("#282828").unwrap(), // text_contrast
            border_light: ThemeColor::from_string("#504945").unwrap(),
            border_hover: ThemeColor::from_string("#665c54").unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let theme = Theme::light();
        let toml_string = toml::to_string_pretty(&theme).unwrap();

        // Check that the output contains hex colors
        assert!(toml_string.contains("name = \"Light\""));
        assert!(toml_string.contains("accent_blue = \"#3b82f6\""));
        assert!(toml_string.contains("bg_main = \"#f8fafc\""));
        assert!(toml_string.contains("bg_head = \"#2563eb\""));
        assert!(toml_string.contains("text_contrast = \"#ffffff\""));
        assert!(toml_string.contains("text_head = \"#ffffff\""));
    }

    #[test]
    fn test_theme_deserialize_from_toml() {
        let toml_str = r###"
            name = "Test Theme"
            accent_blue = "#3b82f6"
            accent_blue_dark = "#2563eb"
            accent_red = "#ef4444"
            accent_red_dark = "#dc2626"
            accent_yellow = "#eab308"
            accent_yellow_dark = "#ca8a04"
            accent_green = "#10b981"
            accent_green_dark = "#059669"
            accent_gray = "#6b7280"
            accent_gray_dark = "#4b5563"
            bg_main = "#f8fafc"
            bg_section = "#f1f5f9"
            bg_hover = "#e2e8f0"
            bg_surface = "#6b7280"
            bg_head = "#2563eb"
            text_primary = "#333333"
            text_secondary = "#2d3748"
            text_muted = "#4a5568"
            text_light = "#6b7280"
            text_contrast = "#ffffff"
            text_head = "#ffffff"
            border_light = "#e2e8f0"
            border_hover = "#cbd5e0"
        "###;

        let theme: Theme = toml::from_str(toml_str).unwrap();
        assert_eq!(theme.name, "Test Theme");
        assert_eq!(theme.accent_blue, ThemeColor::new(59, 130, 246));
        assert_eq!(theme.text_contrast, ThemeColor::new(255, 255, 255));
        assert_eq!(theme.bg_head, ThemeColor::new(37, 99, 235));
        assert_eq!(theme.text_head, ThemeColor::new(255, 255, 255));
    }

    #[test]
    fn test_theme_roundtrip() {
        let original = Theme::dark();
        let toml_string = toml::to_string_pretty(&original).unwrap();
        let deserialized: Theme = toml::from_str(&toml_string).unwrap();

        assert_eq!(original.name, deserialized.name);
        assert_eq!(original.accent_blue, deserialized.accent_blue);
        assert_eq!(original.bg_main, deserialized.bg_main);
        assert_eq!(original.text_primary, deserialized.text_primary);
        assert_eq!(original.border_light, deserialized.border_light);
    }

    #[test]
    fn test_all_themes_serialize() {
        let themes = vec![
            Theme::light(),
            Theme::dark(),
            Theme::gruvbox_light(),
            Theme::gruvbox_dark(),
        ];

        for theme in themes {
            let toml_string = toml::to_string_pretty(&theme).unwrap();
            let deserialized: Theme = toml::from_str(&toml_string).unwrap();
            assert_eq!(theme.name, deserialized.name);
            assert_eq!(theme, deserialized);
        }
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
}
