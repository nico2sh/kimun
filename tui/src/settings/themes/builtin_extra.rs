//! Additional built-in themes, defined compactly from a base palette.
//!
//! Each theme lists the 18 base roles below; the remaining UI roles are
//! derived the same way for every theme (see [`Theme::from_palette`]).

use super::{Theme, ThemeColor};

/// Palette order: bg, bg_hard, bg_soft, bg_panel, selection_bg, fg, fg_bright,
/// red, green, yellow, blue, purple, aqua, orange, fg_secondary, gray,
/// border_dim, accent.
type Palette = [&'static str; 18];

fn hex(s: &str) -> ThemeColor {
    ThemeColor::from_string(s).unwrap()
}

/// Blend `top` over `base` at 25% — a tinted background for the replace preview.
fn tint(base: &ThemeColor, top: &ThemeColor) -> ThemeColor {
    match (base, top) {
        (ThemeColor::Rgb(br, bg, bb), ThemeColor::Rgb(tr, tg, tb)) => {
            let mix = |b: u8, t: u8| ((b as u16 * 3 + t as u16) / 4) as u8;
            ThemeColor::Rgb(mix(*br, *tr), mix(*bg, *tg), mix(*bb, *tb))
        }
        _ => base.clone(),
    }
}

impl Theme {
    fn from_palette(name: &str, p: Palette) -> Theme {
        let c: Vec<ThemeColor> = p.iter().map(|s| hex(s)).collect();
        let replace = tint(&c[0], &c[11]);
        Theme {
            name: name.to_string(),
            bg: c[0].clone(),
            bg_hard: c[1].clone(),
            bg_soft: c[2].clone(),
            bg_panel: c[3].clone(),
            selection_bg: c[4].clone(),
            fg: c[5].clone(),
            fg_bright: c[6].clone(),
            cursor: c[5].clone(),
            red: c[7].clone(),
            green: c[8].clone(),
            yellow: c[9].clone(),
            blue: c[10].clone(),
            purple: c[11].clone(),
            aqua: c[12].clone(),
            orange: c[13].clone(),
            fg_secondary: c[14].clone(),
            gray: c[15].clone(),
            selection_fg: c[6].clone(),
            border_dim: c[16].clone(),
            focus_border: c[8].clone(),
            accent: c[17].clone(),
            color_directory: c[10].clone(),
            color_journal_date: c[12].clone(),
            color_search_match: c[8].clone(),
            color_tag: c[13].clone(),
            blockquote_bar: c[17].clone(),
            code_bg: c[3].clone(),
            color_replace_preview: replace,
        }
    }

    /// The themes defined in this module, in presentation order.
    pub(super) fn extra_builtins() -> Vec<Theme> {
        vec![
            Theme::github_dark(),
            Theme::github_dark_dimmed(),
            Theme::github_light(),
            Theme::catppuccin_macchiato(),
            Theme::catppuccin_frappe(),
            Theme::ayu_dark(),
            Theme::ayu_mirage(),
            Theme::ayu_light(),
            Theme::material(),
            Theme::material_palenight(),
            Theme::material_ocean(),
            Theme::night_owl(),
            Theme::night_owl_light(),
            Theme::tokyo_night_moon(),
            Theme::tokyo_night_day(),
            Theme::nightfox(),
            Theme::carbonfox(),
            Theme::dayfox(),
            Theme::rose_pine_moon(),
            Theme::kanagawa_dragon(),
            Theme::zenburn(),
            Theme::cobalt2(),
            Theme::synthwave_84(),
            Theme::everblush(),
            Theme::oxocarbon_dark(),
            Theme::oxocarbon_light(),
            Theme::quiet_light(),
        ]
    }

    pub fn github_dark() -> Self {
        Self::from_palette(
            "GitHub Dark",
            [
                "#0d1117", "#010409", "#161b22", "#12171e", "#243041", "#c9d1d9", "#f0f6fc",
                "#ff7b72", "#7ee787", "#e3b341", "#79c0ff", "#d2a8ff", "#56d4dd", "#ffa657",
                "#8b949e", "#6e7681", "#30363d", "#58a6ff",
            ],
        )
    }

    pub fn github_dark_dimmed() -> Self {
        Self::from_palette(
            "GitHub Dark Dimmed",
            [
                "#22272e", "#1c2128", "#2d333b", "#262c36", "#343c48", "#adbac7", "#cdd9e5",
                "#ff938a", "#8ddb8c", "#daaa3f", "#6cb6ff", "#dcbdfb", "#76e3ea", "#f69d50",
                "#768390", "#636e7b", "#444c56", "#539bf5",
            ],
        )
    }

    pub fn github_light() -> Self {
        Self::from_palette(
            "GitHub Light",
            [
                "#ffffff", "#f6f8fa", "#eaeef2", "#f6f8fa", "#ddf4ff", "#24292f", "#1f2328",
                "#cf222e", "#1a7f37", "#9a6700", "#0969da", "#8250df", "#1b7c83", "#bc4c00",
                "#57606a", "#8c959f", "#d0d7de", "#0969da",
            ],
        )
    }

    pub fn catppuccin_macchiato() -> Self {
        Self::from_palette(
            "Catppuccin Macchiato",
            [
                "#24273a", "#181926", "#363a4f", "#1e2030", "#363a4f", "#cad3f5", "#f4dbd6",
                "#ed8796", "#a6da95", "#eed49f", "#8aadf4", "#c6a0f6", "#8bd5ca", "#f5a97f",
                "#a5adcb", "#6e738d", "#494d64", "#c6a0f6",
            ],
        )
    }

    pub fn catppuccin_frappe() -> Self {
        Self::from_palette(
            "Catppuccin Frappé",
            [
                "#303446", "#232634", "#414559", "#292c3c", "#414559", "#c6d0f5", "#f2d5cf",
                "#e78284", "#a6d189", "#e5c890", "#8caaee", "#ca9ee6", "#81c8be", "#ef9f76",
                "#a5adce", "#737994", "#51576d", "#ca9ee6",
            ],
        )
    }

    pub fn ayu_dark() -> Self {
        Self::from_palette(
            "Ayu Dark",
            [
                "#0b0e14", "#07090d", "#131721", "#0f131a", "#273747", "#bfbdb6", "#e6e1cf",
                "#f07178", "#aad94c", "#e6b450", "#59c2ff", "#d2a6ff", "#95e6cb", "#ff8f40",
                "#8a9199", "#565b66", "#1b1f2b", "#e6b450",
            ],
        )
    }

    pub fn ayu_mirage() -> Self {
        Self::from_palette(
            "Ayu Mirage",
            [
                "#1f2430", "#171b24", "#2a3140", "#232834", "#33415e", "#cccac2", "#f3f4f5",
                "#f28779", "#d5ff80", "#ffcc66", "#73d0ff", "#dfbfff", "#95e6cb", "#ffad66",
                "#9a9a9a", "#707a8c", "#33394a", "#ffcc66",
            ],
        )
    }

    pub fn ayu_light() -> Self {
        Self::from_palette(
            "Ayu Light",
            [
                "#fafafa", "#f3f4f5", "#e7e8e9", "#f0f0f0", "#d3e4f4", "#5c6166", "#2b2f33",
                "#f07171", "#6cbf43", "#c28a00", "#399ee6", "#a37acc", "#4cbf99", "#fa8d3e",
                "#787b80", "#abb0b6", "#d9d8d7", "#ff9940",
            ],
        )
    }

    pub fn material() -> Self {
        Self::from_palette(
            "Material",
            [
                "#263238", "#1c262b", "#2e3c43", "#212c31", "#37474f", "#b0bec5", "#eeffff",
                "#f07178", "#c3e88d", "#ffcb6b", "#82aaff", "#c792ea", "#89ddff", "#f78c6c",
                "#90a4ae", "#546e7a", "#37474f", "#80cbc4",
            ],
        )
    }

    pub fn material_palenight() -> Self {
        Self::from_palette(
            "Material Palenight",
            [
                "#292d3e", "#1b1e2b", "#32374d", "#252837", "#3a3f58", "#a6accd", "#eeffff",
                "#f07178", "#c3e88d", "#ffcb6b", "#82aaff", "#c792ea", "#89ddff", "#f78c6c",
                "#959dcb", "#676e95", "#3a3f58", "#ab47bc",
            ],
        )
    }

    pub fn material_ocean() -> Self {
        Self::from_palette(
            "Material Ocean",
            [
                "#0f111a", "#090b10", "#1a1c25", "#141621", "#1f2233", "#8f93a2", "#eeffff",
                "#f07178", "#c3e88d", "#ffcb6b", "#82aaff", "#c792ea", "#89ddff", "#f78c6c",
                "#717cb4", "#464b5d", "#232637", "#84ffff",
            ],
        )
    }

    pub fn night_owl() -> Self {
        Self::from_palette(
            "Night Owl",
            [
                "#011627", "#010e1a", "#0b2942", "#01121f", "#1d3b53", "#d6deeb", "#ffffff",
                "#ef5350", "#addb67", "#ecc48d", "#82aaff", "#c792ea", "#7fdbca", "#f78c6c",
                "#a2bffc", "#637777", "#122d42", "#7e57c2",
            ],
        )
    }

    pub fn night_owl_light() -> Self {
        Self::from_palette(
            "Night Owl Light",
            [
                "#fbfbfb", "#f0f0f0", "#e0e7ea", "#f6f6f6", "#d3e8f8", "#403f53", "#011627",
                "#de3d3b", "#2aa298", "#c96765", "#4876d6", "#994cc3", "#08916a", "#d6672b",
                "#5f5e75", "#989fb1", "#d9d9d9", "#4876d6",
            ],
        )
    }

    pub fn tokyo_night_moon() -> Self {
        Self::from_palette(
            "Tokyo Night Moon",
            [
                "#222436", "#1b1d2b", "#2f334d", "#1e2030", "#2d3f76", "#c8d3f5", "#e0e6ff",
                "#ff757f", "#c3e88d", "#ffc777", "#82aaff", "#c099ff", "#86e1fc", "#ff966c",
                "#828bb8", "#636da6", "#3b4261", "#82aaff",
            ],
        )
    }

    pub fn tokyo_night_day() -> Self {
        Self::from_palette(
            "Tokyo Night Day",
            [
                "#e1e2e7", "#d0d5e3", "#c4c8da", "#d5d6db", "#b7c1e3", "#3760bf", "#1e2a53",
                "#f52a65", "#587539", "#8c6c3e", "#2e7de9", "#9854f1", "#007197", "#b15c00",
                "#6172b0", "#848cb5", "#a8aecb", "#2e7de9",
            ],
        )
    }

    pub fn nightfox() -> Self {
        Self::from_palette(
            "Nightfox",
            [
                "#192330", "#131a24", "#29394f", "#161f2b", "#2b3b51", "#cdcecf", "#e4e4e5",
                "#c94f6d", "#81b29a", "#dbc074", "#719cd6", "#9d79d6", "#63cdcf", "#f4a261",
                "#aeafb0", "#71839b", "#39506d", "#719cd6",
            ],
        )
    }

    pub fn carbonfox() -> Self {
        Self::from_palette(
            "Carbonfox",
            [
                "#161616", "#0c0c0c", "#252525", "#1b1b1b", "#2a2a2a", "#f2f4f8", "#ffffff",
                "#ee5396", "#25be6a", "#08bdba", "#78a9ff", "#be95ff", "#33b1ff", "#3ddbd9",
                "#b6b8bb", "#7b7c7e", "#353535", "#78a9ff",
            ],
        )
    }

    pub fn dayfox() -> Self {
        Self::from_palette(
            "Dayfox",
            [
                "#f6f2ee", "#e7d2be", "#e4dcd4", "#ebe5df", "#e7d2be", "#3d2b5a", "#302b5d",
                "#a5222f", "#396847", "#ac5402", "#2848a9", "#6e33ce", "#287980", "#955f61",
                "#643f61", "#824d5b", "#d3c7bb", "#2848a9",
            ],
        )
    }

    pub fn rose_pine_moon() -> Self {
        Self::from_palette(
            "Rosé Pine Moon",
            [
                "#232136", "#191724", "#2a273f", "#2a283e", "#393552", "#e0def4", "#f6f5ff",
                "#eb6f92", "#9ccfd8", "#f6c177", "#3e8fb0", "#c4a7e7", "#9ccfd8", "#ea9a97",
                "#908caa", "#6e6a86", "#44415a", "#c4a7e7",
            ],
        )
    }

    pub fn kanagawa_dragon() -> Self {
        Self::from_palette(
            "Kanagawa Dragon",
            [
                "#181616", "#0d0c0c", "#282727", "#12120f", "#2d4f67", "#c5c9c5", "#c8c093",
                "#c4746e", "#8a9a7b", "#c4b28a", "#8ba4b0", "#a292a3", "#8ea4a2", "#b6927b",
                "#a6a69c", "#737c73", "#393836", "#8ba4b0",
            ],
        )
    }

    pub fn zenburn() -> Self {
        Self::from_palette(
            "Zenburn",
            [
                "#3f3f3f", "#2b2b2b", "#4f4f4f", "#383838", "#5f5f5f", "#dcdccc", "#ffffef",
                "#cc9393", "#7f9f7f", "#f0dfaf", "#8cd0d3", "#dc8cc3", "#93e0e3", "#dfaf8f",
                "#9f9f8f", "#7f7f7f", "#5f5f5f", "#f0dfaf",
            ],
        )
    }

    pub fn cobalt2() -> Self {
        Self::from_palette(
            "Cobalt2",
            [
                "#193549", "#122738", "#1f4662", "#15232d", "#0d3a58", "#ffffff", "#ffffff",
                "#ff628c", "#3ad900", "#ffc600", "#0088ff", "#fb94ff", "#80fcff", "#ff9d00",
                "#adb7c9", "#6b8199", "#234d70", "#ffc600",
            ],
        )
    }

    pub fn synthwave_84() -> Self {
        Self::from_palette(
            "Synthwave '84",
            [
                "#262335", "#1a1725", "#34294f", "#241b2f", "#463465", "#ffffff", "#ffffff",
                "#fe4450", "#72f1b8", "#fede5d", "#36f9f6", "#ff7edb", "#03edf9", "#f97e72",
                "#b6b1b1", "#848bbd", "#495495", "#ff7edb",
            ],
        )
    }

    pub fn everblush() -> Self {
        Self::from_palette(
            "Everblush",
            [
                "#141b1e", "#0f1517", "#232a2d", "#181f22", "#2d3437", "#dadada", "#ffffff",
                "#e57474", "#8ccf7e", "#e5c76b", "#67b0e8", "#c47fd5", "#6cbfbf", "#f4a261",
                "#b3b9b8", "#696f70", "#2d3437", "#67b0e8",
            ],
        )
    }

    pub fn oxocarbon_dark() -> Self {
        Self::from_palette(
            "Oxocarbon Dark",
            [
                "#161616", "#0f0f0f", "#262626", "#1c1c1c", "#393939", "#f2f4f8", "#ffffff",
                "#ee5396", "#42be65", "#ffe97b", "#78a9ff", "#be95ff", "#08bdba", "#ff7eb6",
                "#dde1e6", "#525252", "#393939", "#33b1ff",
            ],
        )
    }

    pub fn oxocarbon_light() -> Self {
        Self::from_palette(
            "Oxocarbon Light",
            [
                "#f2f4f8", "#ffffff", "#dde1e6", "#e8ebf0", "#c1c7cd", "#161616", "#000000",
                "#da1e28", "#198038", "#8a6d00", "#0f62fe", "#8a3ffc", "#007d79", "#d12771",
                "#525252", "#8d8d8d", "#c1c7cd", "#0f62fe",
            ],
        )
    }

    pub fn quiet_light() -> Self {
        Self::from_palette(
            "Quiet Light",
            [
                "#f5f5f5", "#ffffff", "#e4e6f1", "#ececec", "#c9d0d9", "#333333", "#000000",
                "#aa3731", "#448c27", "#9a6700", "#4b83cd", "#7a3e9d", "#2d8a82", "#cc6633",
                "#6f6f6f", "#a0a0a0", "#d4d4d4", "#7a3e9d",
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_themes_have_unique_names_across_builtins() {
        let all = Theme::builtins();
        let mut names: Vec<_> = all.iter().map(|t| t.name.clone()).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate built-in theme name");
    }

    #[test]
    fn extra_themes_are_all_truecolor() {
        for t in Theme::extra_builtins() {
            assert!(matches!(t.bg, ThemeColor::Rgb(..)), "{}", t.name);
            assert!(
                matches!(t.color_replace_preview, ThemeColor::Rgb(..)),
                "{}",
                t.name
            );
        }
    }
}
