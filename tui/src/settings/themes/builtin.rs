//! Built-in theme definitions.
//!
//! Each constructor maps a popular terminal color scheme's official palette
//! onto the UI roles of [`Theme`]. Custom themes live as `.toml` files in the
//! themes config directory instead — see `AppSettings::theme_list()`.

use super::{Theme, ThemeColor};

impl Theme {
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
            color_tag: ThemeColor::from_string("#fe8019").unwrap(),
            blockquote_bar: ThemeColor::from_string("#fabd2f").unwrap(),
            code_bg: ThemeColor::from_string("#32302f").unwrap(),
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
            color_tag: ThemeColor::from_string("#af3a03").unwrap(),
            blockquote_bar: ThemeColor::from_string("#d79921").unwrap(),
            code_bg: ThemeColor::from_string("#f2e5bc").unwrap(),
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
            color_tag: ThemeColor::from_string("#fab387").unwrap(),
            blockquote_bar: ThemeColor::from_string("#cba6f7").unwrap(),
            code_bg: ThemeColor::from_string("#181825").unwrap(),
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
            color_tag: ThemeColor::from_string("#fe640b").unwrap(),
            blockquote_bar: ThemeColor::from_string("#8839ef").unwrap(),
            code_bg: ThemeColor::from_string("#e6e9ef").unwrap(),
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
            color_tag: ThemeColor::from_string("#ff9e64").unwrap(),
            blockquote_bar: ThemeColor::from_string("#7aa2f7").unwrap(),
            code_bg: ThemeColor::from_string("#16161e").unwrap(),
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
            color_tag: ThemeColor::from_string("#ff9e64").unwrap(),
            blockquote_bar: ThemeColor::from_string("#bb9af7").unwrap(),
            code_bg: ThemeColor::from_string("#1f2335").unwrap(),
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
            color_tag: ThemeColor::from_string("#cb4b16").unwrap(),
            blockquote_bar: ThemeColor::from_string("#268bd2").unwrap(),
            code_bg: ThemeColor::from_string("#073642").unwrap(),
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
            color_tag: ThemeColor::from_string("#cb4b16").unwrap(),
            blockquote_bar: ThemeColor::from_string("#268bd2").unwrap(),
            code_bg: ThemeColor::from_string("#eee8d5").unwrap(),
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
            color_tag: ThemeColor::from_string("#d08770").unwrap(),
            blockquote_bar: ThemeColor::from_string("#88c0d0").unwrap(),
            code_bg: ThemeColor::from_string("#3b4252").unwrap(),
        }
    }

    pub fn dracula() -> Self {
        Theme {
            name: "Dracula".to_string(),
            bg: ThemeColor::from_string("#282a36").unwrap(),
            bg_panel: ThemeColor::from_string("#21222c").unwrap(),
            bg_selected: ThemeColor::from_string("#44475a").unwrap(),
            fg: ThemeColor::from_string("#f8f8f2").unwrap(),
            fg_secondary: ThemeColor::from_string("#bfbfbf").unwrap(),
            fg_muted: ThemeColor::from_string("#6272a4").unwrap(),
            fg_selected: ThemeColor::from_string("#f8f8f2").unwrap(),
            border: ThemeColor::from_string("#44475a").unwrap(),
            border_focused: ThemeColor::from_string("#bd93f9").unwrap(),
            accent: ThemeColor::from_string("#bd93f9").unwrap(),
            color_directory: ThemeColor::from_string("#8be9fd").unwrap(),
            color_journal_date: ThemeColor::from_string("#50fa7b").unwrap(),
            color_search_match: ThemeColor::from_string("#f1fa8c").unwrap(),
            color_tag: ThemeColor::from_string("#ffb86c").unwrap(),
            blockquote_bar: ThemeColor::from_string("#bd93f9").unwrap(),
            code_bg: ThemeColor::from_string("#21222c").unwrap(),
        }
    }

    /// Alucard is Dracula's official light variant.
    pub fn alucard() -> Self {
        Theme {
            name: "Alucard".to_string(),
            bg: ThemeColor::from_string("#fffbeb").unwrap(),
            bg_panel: ThemeColor::from_string("#f2eeda").unwrap(),
            bg_selected: ThemeColor::from_string("#cfcfde").unwrap(),
            fg: ThemeColor::from_string("#1f1f1f").unwrap(),
            fg_secondary: ThemeColor::from_string("#6c664b").unwrap(),
            fg_muted: ThemeColor::from_string("#a8a27f").unwrap(),
            fg_selected: ThemeColor::from_string("#1f1f1f").unwrap(),
            border: ThemeColor::from_string("#e2deca").unwrap(),
            border_focused: ThemeColor::from_string("#644ac9").unwrap(),
            accent: ThemeColor::from_string("#644ac9").unwrap(),
            color_directory: ThemeColor::from_string("#036a96").unwrap(),
            color_journal_date: ThemeColor::from_string("#14710a").unwrap(),
            color_search_match: ThemeColor::from_string("#846e15").unwrap(),
            color_tag: ThemeColor::from_string("#a34d14").unwrap(),
            blockquote_bar: ThemeColor::from_string("#644ac9").unwrap(),
            code_bg: ThemeColor::from_string("#f2eeda").unwrap(),
        }
    }

    pub fn one_dark() -> Self {
        Theme {
            name: "One Dark".to_string(),
            bg: ThemeColor::from_string("#282c34").unwrap(),
            bg_panel: ThemeColor::from_string("#21252b").unwrap(),
            bg_selected: ThemeColor::from_string("#3e4451").unwrap(),
            fg: ThemeColor::from_string("#abb2bf").unwrap(),
            fg_secondary: ThemeColor::from_string("#828997").unwrap(),
            fg_muted: ThemeColor::from_string("#5c6370").unwrap(),
            fg_selected: ThemeColor::from_string("#abb2bf").unwrap(),
            border: ThemeColor::from_string("#3e4451").unwrap(),
            border_focused: ThemeColor::from_string("#61afef").unwrap(),
            accent: ThemeColor::from_string("#61afef").unwrap(),
            color_directory: ThemeColor::from_string("#56b6c2").unwrap(),
            color_journal_date: ThemeColor::from_string("#98c379").unwrap(),
            color_search_match: ThemeColor::from_string("#e5c07b").unwrap(),
            color_tag: ThemeColor::from_string("#d19a66").unwrap(),
            blockquote_bar: ThemeColor::from_string("#61afef").unwrap(),
            code_bg: ThemeColor::from_string("#21252b").unwrap(),
        }
    }

    pub fn one_light() -> Self {
        Theme {
            name: "One Light".to_string(),
            bg: ThemeColor::from_string("#fafafa").unwrap(),
            bg_panel: ThemeColor::from_string("#f0f0f1").unwrap(),
            bg_selected: ThemeColor::from_string("#e5e5e6").unwrap(),
            fg: ThemeColor::from_string("#383a42").unwrap(),
            fg_secondary: ThemeColor::from_string("#696c77").unwrap(),
            fg_muted: ThemeColor::from_string("#a0a1a7").unwrap(),
            fg_selected: ThemeColor::from_string("#383a42").unwrap(),
            border: ThemeColor::from_string("#dbdbdc").unwrap(),
            border_focused: ThemeColor::from_string("#4078f2").unwrap(),
            accent: ThemeColor::from_string("#4078f2").unwrap(),
            color_directory: ThemeColor::from_string("#0184bc").unwrap(),
            color_journal_date: ThemeColor::from_string("#50a14f").unwrap(),
            color_search_match: ThemeColor::from_string("#c18401").unwrap(),
            color_tag: ThemeColor::from_string("#986801").unwrap(),
            blockquote_bar: ThemeColor::from_string("#4078f2").unwrap(),
            code_bg: ThemeColor::from_string("#f0f0f1").unwrap(),
        }
    }

    pub fn monokai() -> Self {
        Theme {
            name: "Monokai".to_string(),
            bg: ThemeColor::from_string("#272822").unwrap(),
            bg_panel: ThemeColor::from_string("#1e1f1c").unwrap(),
            bg_selected: ThemeColor::from_string("#49483e").unwrap(),
            fg: ThemeColor::from_string("#f8f8f2").unwrap(),
            fg_secondary: ThemeColor::from_string("#a59f85").unwrap(),
            fg_muted: ThemeColor::from_string("#75715e").unwrap(),
            fg_selected: ThemeColor::from_string("#f8f8f2").unwrap(),
            border: ThemeColor::from_string("#49483e").unwrap(),
            border_focused: ThemeColor::from_string("#66d9ef").unwrap(),
            accent: ThemeColor::from_string("#f92672").unwrap(),
            color_directory: ThemeColor::from_string("#66d9ef").unwrap(),
            color_journal_date: ThemeColor::from_string("#a6e22e").unwrap(),
            color_search_match: ThemeColor::from_string("#e6db74").unwrap(),
            color_tag: ThemeColor::from_string("#fd971f").unwrap(),
            blockquote_bar: ThemeColor::from_string("#f92672").unwrap(),
            code_bg: ThemeColor::from_string("#1e1f1c").unwrap(),
        }
    }

    pub fn everforest_dark() -> Self {
        Theme {
            name: "Everforest Dark".to_string(),
            bg: ThemeColor::from_string("#2d353b").unwrap(),
            bg_panel: ThemeColor::from_string("#232a2e").unwrap(),
            bg_selected: ThemeColor::from_string("#475258").unwrap(),
            fg: ThemeColor::from_string("#d3c6aa").unwrap(),
            fg_secondary: ThemeColor::from_string("#9da9a0").unwrap(),
            fg_muted: ThemeColor::from_string("#7a8478").unwrap(),
            fg_selected: ThemeColor::from_string("#d3c6aa").unwrap(),
            border: ThemeColor::from_string("#3d484d").unwrap(),
            border_focused: ThemeColor::from_string("#a7c080").unwrap(),
            accent: ThemeColor::from_string("#a7c080").unwrap(),
            color_directory: ThemeColor::from_string("#7fbbb3").unwrap(),
            color_journal_date: ThemeColor::from_string("#83c092").unwrap(),
            color_search_match: ThemeColor::from_string("#dbbc7f").unwrap(),
            color_tag: ThemeColor::from_string("#e69875").unwrap(),
            blockquote_bar: ThemeColor::from_string("#a7c080").unwrap(),
            code_bg: ThemeColor::from_string("#232a2e").unwrap(),
        }
    }

    pub fn everforest_light() -> Self {
        Theme {
            name: "Everforest Light".to_string(),
            bg: ThemeColor::from_string("#fdf6e3").unwrap(),
            bg_panel: ThemeColor::from_string("#f4f0d9").unwrap(),
            bg_selected: ThemeColor::from_string("#e6e2cc").unwrap(),
            fg: ThemeColor::from_string("#5c6a72").unwrap(),
            fg_secondary: ThemeColor::from_string("#829181").unwrap(),
            fg_muted: ThemeColor::from_string("#a6b0a0").unwrap(),
            fg_selected: ThemeColor::from_string("#5c6a72").unwrap(),
            border: ThemeColor::from_string("#e0dcc7").unwrap(),
            border_focused: ThemeColor::from_string("#8da101").unwrap(),
            accent: ThemeColor::from_string("#8da101").unwrap(),
            color_directory: ThemeColor::from_string("#3a94c5").unwrap(),
            color_journal_date: ThemeColor::from_string("#35a77c").unwrap(),
            color_search_match: ThemeColor::from_string("#dfa000").unwrap(),
            color_tag: ThemeColor::from_string("#f57d26").unwrap(),
            blockquote_bar: ThemeColor::from_string("#8da101").unwrap(),
            code_bg: ThemeColor::from_string("#f4f0d9").unwrap(),
        }
    }

    pub fn rose_pine() -> Self {
        Theme {
            name: "Rosé Pine".to_string(),
            bg: ThemeColor::from_string("#191724").unwrap(),
            bg_panel: ThemeColor::from_string("#1f1d2e").unwrap(),
            bg_selected: ThemeColor::from_string("#403d52").unwrap(),
            fg: ThemeColor::from_string("#e0def4").unwrap(),
            fg_secondary: ThemeColor::from_string("#908caa").unwrap(),
            fg_muted: ThemeColor::from_string("#6e6a86").unwrap(),
            fg_selected: ThemeColor::from_string("#e0def4").unwrap(),
            border: ThemeColor::from_string("#26233a").unwrap(),
            border_focused: ThemeColor::from_string("#c4a7e7").unwrap(),
            accent: ThemeColor::from_string("#c4a7e7").unwrap(),
            color_directory: ThemeColor::from_string("#9ccfd8").unwrap(),
            color_journal_date: ThemeColor::from_string("#31748f").unwrap(),
            color_search_match: ThemeColor::from_string("#f6c177").unwrap(),
            color_tag: ThemeColor::from_string("#ebbcba").unwrap(),
            blockquote_bar: ThemeColor::from_string("#c4a7e7").unwrap(),
            code_bg: ThemeColor::from_string("#1f1d2e").unwrap(),
        }
    }

    /// Dawn is Rosé Pine's light variant. Note `bg_panel` (surface) is
    /// lighter than `bg` (base) — that is the official palette layering.
    pub fn rose_pine_dawn() -> Self {
        Theme {
            name: "Rosé Pine Dawn".to_string(),
            bg: ThemeColor::from_string("#faf4ed").unwrap(),
            bg_panel: ThemeColor::from_string("#fffaf3").unwrap(),
            bg_selected: ThemeColor::from_string("#dfdad9").unwrap(),
            fg: ThemeColor::from_string("#575279").unwrap(),
            fg_secondary: ThemeColor::from_string("#797593").unwrap(),
            fg_muted: ThemeColor::from_string("#9893a5").unwrap(),
            fg_selected: ThemeColor::from_string("#575279").unwrap(),
            border: ThemeColor::from_string("#f2e9e1").unwrap(),
            border_focused: ThemeColor::from_string("#907aa9").unwrap(),
            accent: ThemeColor::from_string("#907aa9").unwrap(),
            color_directory: ThemeColor::from_string("#56949f").unwrap(),
            color_journal_date: ThemeColor::from_string("#286983").unwrap(),
            color_search_match: ThemeColor::from_string("#ea9d34").unwrap(),
            color_tag: ThemeColor::from_string("#d7827e").unwrap(),
            blockquote_bar: ThemeColor::from_string("#907aa9").unwrap(),
            code_bg: ThemeColor::from_string("#fffaf3").unwrap(),
        }
    }

    pub fn kanagawa_wave() -> Self {
        Theme {
            name: "Kanagawa Wave".to_string(),
            bg: ThemeColor::from_string("#1f1f28").unwrap(),
            bg_panel: ThemeColor::from_string("#16161d").unwrap(),
            bg_selected: ThemeColor::from_string("#2d4f67").unwrap(),
            fg: ThemeColor::from_string("#dcd7ba").unwrap(),
            fg_secondary: ThemeColor::from_string("#c8c093").unwrap(),
            fg_muted: ThemeColor::from_string("#727169").unwrap(),
            fg_selected: ThemeColor::from_string("#dcd7ba").unwrap(),
            border: ThemeColor::from_string("#54546d").unwrap(),
            border_focused: ThemeColor::from_string("#7e9cd8").unwrap(),
            accent: ThemeColor::from_string("#7e9cd8").unwrap(),
            color_directory: ThemeColor::from_string("#7fb4ca").unwrap(),
            color_journal_date: ThemeColor::from_string("#7aa89f").unwrap(),
            color_search_match: ThemeColor::from_string("#98bb6c").unwrap(),
            color_tag: ThemeColor::from_string("#ffa066").unwrap(),
            blockquote_bar: ThemeColor::from_string("#7e9cd8").unwrap(),
            code_bg: ThemeColor::from_string("#16161d").unwrap(),
        }
    }

    /// Lotus is Kanagawa's light variant.
    pub fn kanagawa_lotus() -> Self {
        Theme {
            name: "Kanagawa Lotus".to_string(),
            bg: ThemeColor::from_string("#f2ecbc").unwrap(),
            bg_panel: ThemeColor::from_string("#e5ddb0").unwrap(),
            bg_selected: ThemeColor::from_string("#c9cbd1").unwrap(),
            fg: ThemeColor::from_string("#545464").unwrap(),
            fg_secondary: ThemeColor::from_string("#716e61").unwrap(),
            fg_muted: ThemeColor::from_string("#8a8980").unwrap(),
            fg_selected: ThemeColor::from_string("#545464").unwrap(),
            border: ThemeColor::from_string("#d5cea3").unwrap(),
            border_focused: ThemeColor::from_string("#4d699b").unwrap(),
            accent: ThemeColor::from_string("#4d699b").unwrap(),
            color_directory: ThemeColor::from_string("#4e8ca2").unwrap(),
            color_journal_date: ThemeColor::from_string("#597b75").unwrap(),
            color_search_match: ThemeColor::from_string("#6f894e").unwrap(),
            color_tag: ThemeColor::from_string("#cc6d00").unwrap(),
            blockquote_bar: ThemeColor::from_string("#4d699b").unwrap(),
            code_bg: ThemeColor::from_string("#e5ddb0").unwrap(),
        }
    }

    /// Uses the terminal's 16 ANSI colors so the theme adapts to whatever
    /// palette the user has configured in their terminal emulator. Works for
    /// both light and dark terminal palettes because backgrounds and primary
    /// foregrounds use `Reset` (the terminal's defaults) and accents are
    /// chromatic ANSI slots whose hue is stable across palettes.
    pub fn ansi() -> Self {
        Theme {
            name: "ANSI".to_string(),
            bg: ThemeColor::Reset,
            bg_panel: ThemeColor::Reset,
            bg_selected: ThemeColor::Ansi(4), // blue
            fg: ThemeColor::Reset,
            fg_secondary: ThemeColor::Ansi(7),        // white
            fg_muted: ThemeColor::Ansi(8),            // bright black
            fg_selected: ThemeColor::Ansi(15),        // bright white
            border: ThemeColor::Ansi(8),              // bright black
            border_focused: ThemeColor::Ansi(6),      // cyan
            accent: ThemeColor::Ansi(6),              // cyan
            color_directory: ThemeColor::Ansi(12),    // bright blue
            color_journal_date: ThemeColor::Ansi(10), // bright green
            color_search_match: ThemeColor::Ansi(11), // bright yellow
            color_tag: ThemeColor::Ansi(3),           // yellow
            blockquote_bar: ThemeColor::Ansi(6),      // cyan (accent)
            // Bright-black (gray) — a subtle code-block box that stays visible
            // on both light and dark terminal palettes. `Reset` would equal the
            // editor background and render no box at all.
            code_bg: ThemeColor::Ansi(8),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_has_a_visible_code_bg() {
        // `Reset` equals the editor background, so a code block would render no
        // box at all (the ANSI-theme regression). Every built-in must use a
        // real color for `code_bg`.
        for theme in [
            Theme::gruvbox_dark(),
            Theme::gruvbox_light(),
            Theme::catppuccin_mocha(),
            Theme::catppuccin_latte(),
            Theme::tokyo_night(),
            Theme::tokyo_night_storm(),
            Theme::solarized_dark(),
            Theme::solarized_light(),
            Theme::nord(),
            Theme::dracula(),
            Theme::alucard(),
            Theme::one_dark(),
            Theme::one_light(),
            Theme::monokai(),
            Theme::everforest_dark(),
            Theme::everforest_light(),
            Theme::rose_pine(),
            Theme::rose_pine_dawn(),
            Theme::kanagawa_wave(),
            Theme::kanagawa_lotus(),
            Theme::ansi(),
        ] {
            assert_ne!(
                theme.code_bg,
                ThemeColor::Reset,
                "theme {:?} has code_bg = Reset → invisible code box",
                theme.name
            );
        }
    }
}
