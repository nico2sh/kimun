//! The `[gui]` table of the shared `config.toml` (ADR-0049 §6, ADR-0050 §5):
//! the GUI's drawer placement, sizes and key overrides.
//!
//! The TUI never reads it, but every config save goes through
//! [`AppSettings`](super::AppSettings), so the table is kept as the raw TOML it
//! was read as and written back unchanged: keys this version does not know
//! (written by a newer GUI) survive a TUI save. The typed readers below are
//! lenient — a value of the wrong type or out of range reads as its default,
//! never as a parse error, so a typo in `[gui]` cannot send the whole config
//! down the corrupt-file path.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml::{Table, Value};

use crate::logic::layout::Side;

/// A drawer's width when the config does not give one, in the GUI's logical
/// pixels. Clamping to the window is the layout model's job (`WidthLimits`).
pub const DEFAULT_DRAWER_WIDTH: f64 = 260.0;
/// A split drawer's first-slot share when the config does not give one.
pub const DEFAULT_SPLIT_RATIO: f64 = 0.5;
/// The view ids in the left drawer when the config does not say: the file
/// browser, as the TUI opens.
pub const DEFAULT_LEFT_VIEWS: [&str; 1] = ["files"];

const KEYS: &str = "keys";

/// The `[gui]` table. Missing means the defaults, which are today's layout:
/// the file browser in a visible left drawer, the right drawer hidden and
/// empty, no key overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuiSettings {
    table: Table,
}

/// The config key for `side`'s `field` (`drawer_left_width`, ...); the bare
/// view list when `field` is empty.
fn key(side: Side, field: &str) -> String {
    let side = match side {
        Side::Left => "left",
        Side::Right => "right",
    };
    if field.is_empty() {
        format!("drawer_{side}")
    } else {
        format!("drawer_{side}_{field}")
    }
}

/// A TOML number as `f64`, if it is one and finite.
fn finite(value: &Value) -> Option<f64> {
    let n = match value {
        Value::Float(f) => *f,
        Value::Integer(i) => *i as f64,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

impl GuiSettings {
    /// Whether the table holds nothing, so it is left out of the file.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// The view ids in `side`'s drawer, top slot first.
    ///
    /// Missing or not a list: the default (`["files"]` left, nothing right).
    /// Entries that are not non-empty strings are skipped, and a view listed
    /// twice keeps its first place — a view lives in one slot of one drawer,
    /// so the right drawer also drops any id the left one already holds.
    pub fn drawer_views(&self, side: Side) -> Vec<String> {
        let mut views = self.listed_views(side);
        if side == Side::Right {
            let left = self.listed_views(Side::Left);
            views.retain(|v| !left.contains(v));
        }
        views
    }

    fn listed_views(&self, side: Side) -> Vec<String> {
        let Some(Value::Array(items)) = self.table.get(&key(side, "")) else {
            return match side {
                Side::Left => DEFAULT_LEFT_VIEWS.iter().map(|v| v.to_string()).collect(),
                Side::Right => Vec::new(),
            };
        };
        let mut views: Vec<String> = Vec::new();
        for item in items {
            if let Value::String(id) = item {
                let id = id.trim();
                if !id.is_empty() && !views.iter().any(|v| v == id) {
                    views.push(id.to_string());
                }
            }
        }
        views
    }

    /// Replaces `side`'s view list.
    pub fn set_drawer_views(&mut self, side: Side, views: &[String]) {
        let list = views.iter().cloned().map(Value::String).collect();
        self.table.insert(key(side, ""), Value::Array(list));
    }

    /// `side`'s drawer width in logical pixels: a positive finite number, or
    /// [`DEFAULT_DRAWER_WIDTH`].
    pub fn drawer_width(&self, side: Side) -> f64 {
        self.table
            .get(&key(side, "width"))
            .and_then(finite)
            .filter(|w| *w > 0.0)
            .unwrap_or(DEFAULT_DRAWER_WIDTH)
    }

    /// Records `side`'s drawer width. A width that is not positive and
    /// finite is not recorded.
    pub fn set_drawer_width(&mut self, side: Side, width: f64) {
        if width.is_finite() && width > 0.0 {
            self.table
                .insert(key(side, "width"), Value::Float(width.round()));
        }
    }

    /// Whether `side`'s drawer is shown: a boolean, or the default (left
    /// shown, right hidden).
    pub fn drawer_visible(&self, side: Side) -> bool {
        match self.table.get(&key(side, "visible")) {
            Some(Value::Boolean(b)) => *b,
            _ => side == Side::Left,
        }
    }

    /// Records whether `side`'s drawer is shown.
    pub fn set_drawer_visible(&mut self, side: Side, visible: bool) {
        self.table
            .insert(key(side, "visible"), Value::Boolean(visible));
    }

    /// The first slot's share of `side`'s drawer when it is split: a number
    /// strictly between 0 and 1, or [`DEFAULT_SPLIT_RATIO`]. Further
    /// clamping is the layout model's.
    pub fn split_ratio(&self, side: Side) -> f64 {
        self.table
            .get(&key(side, "split"))
            .and_then(finite)
            .filter(|r| *r > 0.0 && *r < 1.0)
            .unwrap_or(DEFAULT_SPLIT_RATIO)
    }

    /// Records `side`'s split ratio. One outside `(0, 1)` is not recorded.
    pub fn set_split_ratio(&mut self, side: Side, ratio: f64) {
        if ratio.is_finite() && ratio > 0.0 && ratio < 1.0 {
            self.table.insert(key(side, "split"), Value::Float(ratio));
        }
    }

    /// The `[gui.keys]` overrides: action name → chords, as written. A single
    /// string counts as a one-chord list, an empty list unbinds; non-string
    /// list items are dropped, and an entry of any other type is skipped (and
    /// logged). Names and chords are validated by the GUI's keymap loader,
    /// which reports what it rejects.
    pub fn key_overrides(&self) -> BTreeMap<String, Vec<String>> {
        let Some(Value::Table(keys)) = self.table.get(KEYS) else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for (action, value) in keys {
            let chords = match value {
                Value::String(chord) => vec![chord.clone()],
                Value::Array(items) => items
                    .iter()
                    .filter_map(|i| i.as_str().map(str::to_string))
                    .collect(),
                other => {
                    tracing::warn!(
                        "[gui.keys] {action}: expected a chord or a list of chords, got {}",
                        other.type_str()
                    );
                    continue;
                }
            };
            out.insert(action.clone(), chords);
        }
        out
    }
}

impl Serialize for GuiSettings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.table.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GuiSettings {
    /// Accepts any value: a table is kept as written, anything else (`gui =
    /// 5`) is logged and read as the defaults rather than failing the config.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Table(table) => Ok(Self { table }),
            other => {
                tracing::warn!(
                    "[gui] is a {}, not a table; using the defaults",
                    other.type_str()
                );
                Ok(Self::default())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    fn gui(toml: &str) -> GuiSettings {
        toml::from_str::<AppSettings>(toml)
            .unwrap_or_else(|e| panic!("config must parse: {e}\n{toml}"))
            .gui
    }

    #[test]
    fn a_missing_table_is_todays_layout() {
        let g = AppSettings::default().gui;
        assert!(g.is_empty());
        assert_eq!(g.drawer_views(Side::Left), ["files"]);
        assert!(g.drawer_views(Side::Right).is_empty());
        assert!(g.drawer_visible(Side::Left));
        assert!(!g.drawer_visible(Side::Right));
        assert_eq!(g.drawer_width(Side::Left), DEFAULT_DRAWER_WIDTH);
        assert_eq!(g.drawer_width(Side::Right), DEFAULT_DRAWER_WIDTH);
        assert_eq!(g.split_ratio(Side::Left), DEFAULT_SPLIT_RATIO);
        assert!(g.key_overrides().is_empty());
        assert_eq!(gui("theme = \"Nord\"\n"), GuiSettings::default());
    }

    #[test]
    fn an_empty_table_is_not_written() {
        let out = toml::to_string(&AppSettings::default()).unwrap();
        assert!(!out.contains("[gui"), "{out}");
    }

    #[test]
    fn typed_values_are_read() {
        let g = gui(r#"
[gui]
drawer_left = ["files", "backlinks"]
drawer_right = ["outline"]
drawer_left_width = 300
drawer_right_width = 222.5
drawer_left_visible = false
drawer_right_visible = true
drawer_left_split = 0.3
[gui.keys]
FindInBuffer = ["primary+shift+f", "alt+f"]
ToggleSidebar = "primary+b"
ReplaceInBuffer = []
"#);
        assert_eq!(g.drawer_views(Side::Left), ["files", "backlinks"]);
        assert_eq!(g.drawer_views(Side::Right), ["outline"]);
        assert_eq!(g.drawer_width(Side::Left), 300.0);
        assert_eq!(g.drawer_width(Side::Right), 222.5);
        assert!(!g.drawer_visible(Side::Left));
        assert!(g.drawer_visible(Side::Right));
        assert_eq!(g.split_ratio(Side::Left), 0.3);
        assert_eq!(g.split_ratio(Side::Right), DEFAULT_SPLIT_RATIO);
        let keys = g.key_overrides();
        assert_eq!(keys["FindInBuffer"], ["primary+shift+f", "alt+f"]);
        assert_eq!(keys["ToggleSidebar"], ["primary+b"]);
        assert!(keys["ReplaceInBuffer"].is_empty());
    }

    #[test]
    fn setters_round_trip_through_the_file_format() {
        let mut settings = AppSettings::default();
        let g = &mut settings.gui;
        g.set_drawer_views(Side::Left, &["files".into()]);
        g.set_drawer_views(Side::Right, &["backlinks".into(), "find".into()]);
        g.set_drawer_width(Side::Left, 287.4);
        g.set_drawer_visible(Side::Left, false);
        g.set_split_ratio(Side::Right, 0.25);
        // Refused: none of these may land in the file.
        g.set_drawer_width(Side::Right, f64::NAN);
        g.set_drawer_width(Side::Right, -1.0);
        g.set_split_ratio(Side::Left, 1.0);

        let written = toml::to_string(&settings).unwrap();
        let back: AppSettings = toml::from_str(&written).unwrap();
        assert_eq!(back.gui, settings.gui);
        let g = &back.gui;
        assert_eq!(g.drawer_views(Side::Right), ["backlinks", "find"]);
        assert_eq!(g.drawer_width(Side::Left), 287.0, "whole pixels");
        assert_eq!(g.drawer_width(Side::Right), DEFAULT_DRAWER_WIDTH);
        assert!(!g.drawer_visible(Side::Left));
        assert_eq!(g.split_ratio(Side::Right), 0.25);
        assert_eq!(g.split_ratio(Side::Left), DEFAULT_SPLIT_RATIO);
    }

    #[test]
    fn garbage_values_fall_back_to_the_defaults() {
        let g = gui(r#"
[gui]
drawer_left = "files"
drawer_right = [1, "", "  ", true, "files", "outline", "outline"]
drawer_left_width = "wide"
drawer_right_width = -40
drawer_left_visible = "yes"
drawer_right_visible = 1
drawer_left_split = 7
drawer_right_split = nan
[gui.keys]
FindInBuffer = 5
ToggleSidebar = ["primary+t", 3]
"#);
        // A non-list is the default; the right list keeps only fresh ids.
        assert_eq!(g.drawer_views(Side::Left), ["files"]);
        assert_eq!(g.drawer_views(Side::Right), ["outline"]);
        assert_eq!(g.drawer_width(Side::Left), DEFAULT_DRAWER_WIDTH);
        assert_eq!(g.drawer_width(Side::Right), DEFAULT_DRAWER_WIDTH);
        assert!(g.drawer_visible(Side::Left));
        assert!(!g.drawer_visible(Side::Right));
        assert_eq!(g.split_ratio(Side::Left), DEFAULT_SPLIT_RATIO);
        assert_eq!(g.split_ratio(Side::Right), DEFAULT_SPLIT_RATIO);
        let keys = g.key_overrides();
        assert!(!keys.contains_key("FindInBuffer"));
        assert_eq!(keys["ToggleSidebar"], ["primary+t"]);
    }

    #[test]
    fn a_gui_value_that_is_not_a_table_does_not_fail_the_config() {
        let s: AppSettings = toml::from_str("theme = \"Nord\"\ngui = 5\n").unwrap();
        assert_eq!(s.theme, "Nord");
        assert_eq!(s.gui, GuiSettings::default());
    }

    /// The TUI owns the config file and rewrites it on every preference
    /// change. A `[gui]` table — including keys this version does not know —
    /// must come back out of a TUI load + save exactly as it went in.
    #[test]
    fn a_tui_load_and_save_keeps_the_gui_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().canonicalize().unwrap().join("config.toml");
        let current = crate::settings::config_migration::CURRENT_CONFIG_VERSION;
        let gui_part = r#"
[gui]
drawer_left = ["files"]
drawer_left_width = 312.0
drawer_right_visible = false
from_a_newer_gui = { anything = [1, 2] }

[gui.keys]
FindInBuffer = ["primary+shift+f"]
NotAnActionYet = "alt+q"
"#;
        std::fs::write(
            &path,
            format!("config_version = {current}\ntheme = \"Nord\"\n{gui_part}"),
        )
        .unwrap();

        let mut settings = AppSettings::load_from_file(path.clone()).unwrap();
        settings.set_theme("Gruvbox Dark".into());
        settings.save_to_disk().unwrap();

        let written: Table = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let expected: Table = toml::from_str(gui_part).unwrap();
        assert_eq!(written.get("gui"), expected.get("gui"));
        assert_eq!(
            written.get("theme").and_then(Value::as_str),
            Some("Gruvbox Dark")
        );
    }
}
