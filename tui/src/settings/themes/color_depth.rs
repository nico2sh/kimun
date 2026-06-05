//! Terminal color-depth detection and theme adaptation.
//!
//! Themes are authored in truecolor (RGB). Terminals that only support 256 or
//! 16 colors get an adapted copy of the theme:
//!
//! - **256 colors** — every RGB role is quantized to the nearest slot of the
//!   xterm-256 palette (6×6×6 color cube + grayscale ramp).
//! - **16 colors** — RGB values cannot be represented faithfully, so the
//!   theme falls back to the built-in ANSI theme's role→slot mapping (the
//!   single source of truth for "which ANSI slot does each role get") and the
//!   user's terminal palette supplies the actual colors.

use super::{Theme, ThemeColor};

/// Color capability of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit RGB.
    TrueColor,
    /// xterm 256-color palette.
    Ansi256,
    /// The 16 standard ANSI colors.
    Ansi16,
}

/// Detect the terminal's color depth from the environment.
///
/// The result is cached for the lifetime of the process — the terminal a TUI
/// runs in cannot change mid-session.
pub fn detect() -> ColorDepth {
    static DEPTH: std::sync::OnceLock<ColorDepth> = std::sync::OnceLock::new();
    *DEPTH.get_or_init(|| {
        from_env(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    })
}

/// Pure detection logic, separated from the environment for testability.
fn from_env(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
    if let Some(ct) = colorterm {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            return ColorDepth::TrueColor;
        }
    }
    if let Some(t) = term {
        let t = t.to_ascii_lowercase();
        // Some terminals advertise truecolor via TERM (e.g. xterm-direct).
        if t.contains("direct") || t.contains("truecolor") {
            return ColorDepth::TrueColor;
        }
        if t.contains("256color") {
            return ColorDepth::Ansi256;
        }
    }
    ColorDepth::Ansi16
}

impl Theme {
    /// Adapt this theme to the terminal the process is running in.
    ///
    /// The one entry point display paths should use — `AppSettings::get_theme()`
    /// and the settings-screen live preview both funnel through it.
    pub fn adapt_to_terminal(self) -> Theme {
        self.adapt(detect())
    }

    /// Return a copy of this theme adapted to the given color depth.
    ///
    /// Truecolor terminals get the theme unchanged.
    pub fn adapt(self, depth: ColorDepth) -> Theme {
        match depth {
            ColorDepth::TrueColor => self,
            ColorDepth::Ansi256 => self.into_quantized_256(),
            ColorDepth::Ansi16 => self.into_ansi16(),
        }
    }

    /// Quantize every RGB role to the nearest xterm-256 palette slot.
    fn into_quantized_256(mut self) -> Theme {
        for color in self.roles_mut() {
            if let ThemeColor::Rgb(r, g, b) = *color {
                *color = ThemeColor::Ansi(nearest_256(r, g, b));
            }
        }
        self
    }

    /// Map every role to its canonical ANSI-16 slot.
    ///
    /// The theme's RGB values are discarded: on a 16-color terminal the user's
    /// palette is the only color source, so role *semantics* (not hues) are
    /// what must survive. The built-in ANSI theme owns the role→slot mapping —
    /// a single source of truth — and only the theme's identity (its name) is
    /// kept.
    fn into_ansi16(self) -> Theme {
        Theme {
            name: self.name,
            ..Theme::ansi()
        }
    }

    /// Mutable iterator over every color role, for whole-theme transforms.
    fn roles_mut(&mut self) -> impl Iterator<Item = &mut ThemeColor> {
        [
            &mut self.bg,
            &mut self.bg_hard,
            &mut self.bg_soft,
            &mut self.bg_panel,
            &mut self.selection_bg,
            &mut self.fg,
            &mut self.fg_bright,
            &mut self.fg_secondary,
            &mut self.gray,
            &mut self.selection_fg,
            &mut self.border_dim,
            &mut self.focus_border,
            &mut self.accent,
            &mut self.cursor,
            &mut self.red,
            &mut self.green,
            &mut self.yellow,
            &mut self.blue,
            &mut self.purple,
            &mut self.aqua,
            &mut self.orange,
            &mut self.color_directory,
            &mut self.color_journal_date,
            &mut self.color_search_match,
            &mut self.color_tag,
            &mut self.blockquote_bar,
            &mut self.code_bg,
        ]
        .into_iter()
    }
}

/// Nearest xterm-256 palette index for an RGB color.
///
/// Considers the 6×6×6 color cube (16–231) and the grayscale ramp (232–255);
/// the 16 base slots are skipped because their colors are user-configurable
/// and unpredictable.
fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    // Cube candidate: snap each channel to the nearest cube level.
    let cube_idx = |c: u8| -> u8 {
        // Levels: 0, 95, 135, 175, 215, 255.
        if c < 48 {
            0
        } else if c < 115 {
            1
        } else {
            ((c as u16 - 35) / 40).min(5) as u8
        }
    };
    let level = |i: u8| -> u8 { if i == 0 { 0 } else { 55 + i * 40 } };
    let (ci, cg, cb) = (cube_idx(r), cube_idx(g), cube_idx(b));
    let cube = (16 + 36 * ci as u16 + 6 * cg as u16 + cb as u16) as u8;
    let cube_rgb = (level(ci), level(cg), level(cb));

    // Gray candidate: ramp 232–255 holds 8 + 10*i for i in 0..24.
    let gray_avg = (r as u16 + g as u16 + b as u16) / 3;
    let gi = if gray_avg < 8 {
        0
    } else {
        (((gray_avg - 8) + 5) / 10).min(23)
    };
    let gray = (232 + gi) as u8;
    let gl = (8 + 10 * gi) as u8;
    let gray_rgb = (gl, gl, gl);

    let dist = |(cr, cg2, cb2): (u8, u8, u8)| -> u32 {
        let dr = r as i32 - cr as i32;
        let dg = g as i32 - cg2 as i32;
        let db = b as i32 - cb2 as i32;
        (dr * dr + dg * dg + db * db) as u32
    };

    if dist(gray_rgb) < dist(cube_rgb) {
        gray
    } else {
        cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_truecolor_from_colorterm() {
        assert_eq!(
            from_env(Some("truecolor"), Some("xterm-256color")),
            ColorDepth::TrueColor
        );
        assert_eq!(from_env(Some("24bit"), None), ColorDepth::TrueColor);
    }

    #[test]
    fn detects_truecolor_from_term_direct() {
        assert_eq!(from_env(None, Some("xterm-direct")), ColorDepth::TrueColor);
    }

    #[test]
    fn detects_256color_from_term() {
        assert_eq!(from_env(None, Some("xterm-256color")), ColorDepth::Ansi256);
        assert_eq!(
            from_env(Some(""), Some("screen-256color")),
            ColorDepth::Ansi256
        );
    }

    #[test]
    fn falls_back_to_ansi16() {
        assert_eq!(from_env(None, Some("xterm")), ColorDepth::Ansi16);
        assert_eq!(from_env(None, None), ColorDepth::Ansi16);
        assert_eq!(from_env(Some("yes"), Some("vt100")), ColorDepth::Ansi16);
    }

    #[test]
    fn nearest_256_known_values() {
        assert_eq!(nearest_256(0, 0, 0), 16); // cube black
        assert_eq!(nearest_256(255, 255, 255), 231); // cube white
        assert_eq!(nearest_256(255, 0, 0), 196); // pure red
        assert_eq!(nearest_256(0, 255, 0), 46); // pure green
        assert_eq!(nearest_256(0, 0, 255), 21); // pure blue
        // Mid gray lands on the grayscale ramp, not the cube.
        let gray = nearest_256(128, 128, 128);
        assert!((232..=255).contains(&gray), "got {}", gray);
    }

    #[test]
    fn truecolor_adapt_is_identity() {
        let theme = Theme::gruvbox_dark();
        assert_eq!(theme.clone().adapt(ColorDepth::TrueColor), theme);
    }

    #[test]
    fn ansi256_adapt_leaves_no_rgb() {
        let theme = Theme::gruvbox_dark().adapt(ColorDepth::Ansi256);
        let mut theme = theme;
        for color in theme.roles_mut() {
            assert!(
                !matches!(color, ThemeColor::Rgb(..)),
                "RGB role survived 256-color adaptation: {}",
                color
            );
        }
    }

    #[test]
    fn ansi16_adapt_delegates_to_builtin_ansi_mapping() {
        let theme = Theme::gruvbox_dark().adapt(ColorDepth::Ansi16);
        // Identity preserved, every role from the built-in ANSI theme — the
        // single source of truth for the role→slot mapping.
        let expected = Theme {
            name: "Gruvbox Dark".to_string(),
            ..Theme::ansi()
        };
        assert_eq!(theme, expected);
    }

    #[test]
    fn ansi16_adapt_has_no_rgb_for_any_builtin() {
        for theme in Theme::builtins() {
            let name = theme.name.clone();
            let mut adapted = theme.adapt(ColorDepth::Ansi16);
            for color in adapted.roles_mut() {
                assert!(
                    !matches!(color, ThemeColor::Rgb(..)),
                    "theme {:?}: RGB role survived 16-color adaptation",
                    name
                );
            }
        }
    }
}
