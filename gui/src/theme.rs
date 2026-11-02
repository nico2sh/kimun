//! `ThemeColor` -> egui. `Rgb` maps directly; `Ansi` and `Reset` are
//! terminal-relative and have no terminal here, so they get a fixed meaning:
//! the xterm 16-colour palette, the standard 6x6x6 cube and 24-step grey ramp
//! for 16-255, and, for `Reset`, the colour the *token* implies.

use eframe::egui::{self, Color32};
use kimun_notes::settings::themes::{Theme, ThemeColor};

/// What a `ThemeColor::Reset` resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Background,
    Foreground,
}

const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn ansi(n: u8) -> Color32 {
    match n {
        0..=15 => {
            let (r, g, b) = ANSI16[n as usize];
            Color32::from_rgb(r, g, b)
        }
        16..=231 => {
            let i = n - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            Color32::from_rgb(step(i / 36), step((i / 6) % 6), step(i % 6))
        }
        232..=255 => {
            let v = 8 + 10 * (n - 232);
            Color32::from_rgb(v, v, v)
        }
    }
}

pub fn to_egui(color: &ThemeColor, token: Token, theme: &Theme) -> Color32 {
    match color {
        ThemeColor::Rgb(r, g, b) => Color32::from_rgb(*r, *g, *b),
        ThemeColor::Ansi(n) => ansi(*n),
        // `Reset` means "the terminal's default". The nearest honest equivalent
        // is the theme's own plain background / text colour, never transparent.
        ThemeColor::Reset => match token {
            Token::Background => resolve_non_reset(&theme.bg, Color32::from_rgb(30, 30, 30)),
            Token::Foreground => resolve_non_reset(&theme.fg, Color32::from_rgb(220, 220, 220)),
        },
    }
}

fn resolve_non_reset(c: &ThemeColor, fallback: Color32) -> Color32 {
    match c {
        ThemeColor::Rgb(r, g, b) => Color32::from_rgb(*r, *g, *b),
        ThemeColor::Ansi(n) => ansi(*n),
        ThemeColor::Reset => fallback,
    }
}

pub fn visuals(theme: &Theme) -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let bg = to_egui(&theme.bg, Token::Background, theme);
    let fg = to_egui(&theme.fg, Token::Foreground, theme);
    let hard = to_egui(&theme.bg_hard, Token::Background, theme);
    v.panel_fill = bg;
    v.window_fill = hard;
    v.extreme_bg_color = hard;
    v.override_text_color = Some(fg);
    v.selection.bg_fill = to_egui(&theme.selection_bg, Token::Background, theme);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_passes_through() {
        let t = Theme::default();
        assert_eq!(
            to_egui(&ThemeColor::Rgb(1, 2, 3), Token::Foreground, &t),
            Color32::from_rgb(1, 2, 3)
        );
    }

    #[test]
    fn ansi_cube_and_ramp_match_xterm() {
        assert_eq!(ansi(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(ansi(21), Color32::from_rgb(0, 0, 255));
        assert_eq!(ansi(196), Color32::from_rgb(255, 0, 0));
        assert_eq!(ansi(232), Color32::from_rgb(8, 8, 8));
        assert_eq!(ansi(255), Color32::from_rgb(238, 238, 238));
    }

    #[test]
    fn reset_is_never_transparent() {
        let t = Theme::default();
        for tok in [Token::Background, Token::Foreground] {
            assert_ne!(to_egui(&ThemeColor::Reset, tok, &t).a(), 0);
        }
    }

    #[test]
    fn reset_background_follows_the_theme_not_a_constant() {
        let t = Theme {
            bg: ThemeColor::Rgb(10, 20, 30),
            ..Theme::default()
        };
        assert_eq!(
            to_egui(&ThemeColor::Reset, Token::Background, &t),
            Color32::from_rgb(10, 20, 30)
        );
    }
}
