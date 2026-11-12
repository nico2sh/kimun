//! The `[gui]` table of the shared `config.toml` (ADR-0049 §6, ADR-0050 §5,
//! ADR-0052 §5): the GUI's live layout, active preset, key overrides, window
//! decorations and reduced motion.
//!
//! `[gui.layout]` is the live layout tree: a nested table (a split with
//! weighted children, down to slots and editor groups) whose shape belongs to
//! the layout model (`logic::layout_tree`). This module only holds it as raw
//! TOML. `layout_preset` names the preset (from the separate presets file)
//! the layout started from, when the user picked one. Collapse and zoom are
//! session-only and never written here.
//!
//! The TUI never reads it, but every config save goes through
//! [`AppSettings`](super::AppSettings), so the table is kept as the raw TOML it
//! was read as and written back unchanged: keys this version does not know
//! (written by a newer GUI) survive a TUI save. The typed readers below are
//! lenient — a value of the wrong type reads as missing, never as a parse
//! error, so a typo in `[gui]` cannot send the whole config down the
//! corrupt-file path.
//!
//! `rail` (`left`, `right`, `top`, `bottom`) and `rail_hidden` place and hide
//! the desktop app's Activity rail (ADR-0055); both default to the old
//! behavior and the TUI ignores them.
//!
//! `reduce_motion = true` keeps the desktop app's decorative motion still (the
//! Welcome wordmark's bounce, loading spinners). It is the desktop app's
//! Preferences > Display > Reduce motion, applied as soon as it is saved; the
//! TUI never shows it. Missing means `false`: the system's own reduced-motion
//! setting is not read (GPUI offers no query for it). The user docs describe
//! the key on the desktop-app page (`docs/content/using-kimun/desktop-app.md`).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml::{Table, Value};

use crate::logic::layout_tree::RailPosition;

const KEYS: &str = "keys";
const LAYOUT: &str = "layout";
const LAYOUT_PRESET: &str = "layout_preset";
const WINDOW_DECORATIONS: &str = "window_decorations";
const REDUCE_MOTION: &str = "reduce_motion";
const RAIL: &str = "rail";
const RAIL_HIDDEN: &str = "rail_hidden";

/// `window_decorations`: who draws the GUI window's title bar and frame.
/// Read once, when the window opens (changing it needs a restart). macOS
/// ignores it: the native traffic lights always stay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowDecorations {
    /// `"client"`: the GUI draws its own title bar in the theme's colours
    /// (Linux, Windows).
    #[default]
    Client,
    /// `"system"`: the operating system's title bar and frame, the escape
    /// hatch for a desktop where client decorations misbehave.
    System,
}

impl WindowDecorations {
    /// The value as the config file writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            WindowDecorations::Client => "client",
            WindowDecorations::System => "system",
        }
    }
}

/// The `[gui]` table. Missing means the defaults: the default layout, no
/// preset, no key overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuiSettings {
    table: Table,
}

impl GuiSettings {
    /// Whether the table holds nothing, so it is left out of the file.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// `[gui.layout]`: the live layout tree as raw TOML, untouched. `None`
    /// when missing or not a table. Reading it into a tree, and falling back
    /// to the default layout when it does not build, is the layout model's.
    pub fn layout(&self) -> Option<Value> {
        match self.table.get(LAYOUT) {
            Some(table @ Value::Table(_)) => Some(table.clone()),
            _ => None,
        }
    }

    /// Records `[gui.layout]`, replacing the table there.
    pub fn set_layout(&mut self, layout: Value) {
        self.table.insert(LAYOUT.into(), layout);
    }

    /// `layout_preset`: the name of the active preset, trimmed. `None` when
    /// missing, blank or not a string.
    pub fn layout_preset(&self) -> Option<String> {
        self.table
            .get(LAYOUT_PRESET)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    }

    /// Records the active preset's name; `None` removes the key.
    pub fn set_layout_preset(&mut self, name: Option<&str>) {
        match name {
            Some(name) => {
                self.table
                    .insert(LAYOUT_PRESET.into(), Value::String(name.into()));
            }
            None => {
                self.table.remove(LAYOUT_PRESET);
            }
        }
    }

    /// `window_decorations`: `"client"` or `"system"` (trimmed, any case).
    /// Missing reads as [`WindowDecorations::Client`]; any other value is
    /// logged and reads as the default too.
    pub fn window_decorations(&self) -> WindowDecorations {
        match self.table.get(WINDOW_DECORATIONS) {
            None => WindowDecorations::default(),
            Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("client") => {
                WindowDecorations::Client
            }
            Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("system") => {
                WindowDecorations::System
            }
            Some(other) => {
                tracing::warn!(
                    "[gui] window_decorations: expected \"client\" or \"system\", got {other}; \
                     using \"client\""
                );
                WindowDecorations::default()
            }
        }
    }

    /// Records `window_decorations`.
    pub fn set_window_decorations(&mut self, decorations: WindowDecorations) {
        self.table.insert(
            WINDOW_DECORATIONS.into(),
            Value::String(decorations.as_str().into()),
        );
    }

    /// `reduce_motion`: `true` keeps the GUI's decorative motion still (the
    /// Welcome wordmark's bounce, the kit's spinners). Read when the app
    /// starts and after every Preferences save. Missing reads as `false`;
    /// any other value than a boolean is logged and reads as `false` too.
    pub fn reduce_motion(&self) -> bool {
        match self.table.get(REDUCE_MOTION) {
            None => false,
            Some(Value::Boolean(reduce)) => *reduce,
            Some(other) => {
                tracing::warn!(
                    "[gui] reduce_motion: expected true or false, got {other}; using false"
                );
                false
            }
        }
    }

    /// Records `reduce_motion`.
    pub fn set_reduce_motion(&mut self, reduce: bool) {
        self.table
            .insert(REDUCE_MOTION.into(), Value::Boolean(reduce));
    }

    /// `rail`: the window edge the desktop app's Activity rail sits on
    /// (ADR-0055): `"left"` (the default), `"right"`, `"top"` or `"bottom"`,
    /// trimmed, any case. Missing reads as the default; any other value is
    /// logged and reads as the default too.
    pub fn rail(&self) -> RailPosition {
        let Some(value) = self.table.get(RAIL) else {
            return RailPosition::default();
        };
        let found = value.as_str().map(str::trim).and_then(|name| {
            RailPosition::ALL
                .into_iter()
                .find(|rail| name.eq_ignore_ascii_case(rail.name()))
        });
        found.unwrap_or_else(|| {
            tracing::warn!(
                "[gui] rail: expected \"left\", \"right\", \"top\" or \"bottom\", got {value}; \
                 using \"left\""
            );
            RailPosition::default()
        })
    }

    /// Records `rail`; the default position removes the key.
    pub fn set_rail(&mut self, rail: RailPosition) {
        if rail == RailPosition::default() {
            self.table.remove(RAIL);
        } else {
            self.table
                .insert(RAIL.into(), Value::String(rail.name().into()));
        }
    }

    /// `rail_hidden`: `true` hides the rail (ADR-0055). Missing reads as
    /// `false`; any other value than a boolean is logged and reads as
    /// `false` too.
    pub fn rail_hidden(&self) -> bool {
        match self.table.get(RAIL_HIDDEN) {
            None => false,
            Some(Value::Boolean(hidden)) => *hidden,
            Some(other) => {
                tracing::warn!(
                    "[gui] rail_hidden: expected true or false, got {other}; using false"
                );
                false
            }
        }
    }

    /// Records `rail_hidden`; `false` removes the key.
    pub fn set_rail_hidden(&mut self, hidden: bool) {
        if hidden {
            self.table.insert(RAIL_HIDDEN.into(), Value::Boolean(true));
        } else {
            self.table.remove(RAIL_HIDDEN);
        }
    }

    /// Apply to this table the top-level keys `after` changed from `before`
    /// (set where changed or added, removed where removed); every other key
    /// is kept as it is here. For a write that edits a few `[gui]` keys of a
    /// copy over the table as it is on disk now, so the edit lands and
    /// nothing else the copy holds (an older layout) reverts the file.
    pub fn apply_changes(&mut self, before: &GuiSettings, after: &GuiSettings) {
        for (key, value) in &after.table {
            if before.table.get(key) != Some(value) {
                self.table.insert(key.clone(), value.clone());
            }
        }
        for key in before.table.keys() {
            if !after.table.contains_key(key) {
                self.table.remove(key);
            }
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
    fn a_missing_table_is_the_defaults() {
        let g = AppSettings::default().gui;
        assert!(g.is_empty());
        assert_eq!(g.layout(), None);
        assert_eq!(g.layout_preset(), None);
        assert!(g.key_overrides().is_empty());
        assert_eq!(gui("theme = \"Nord\"\n"), GuiSettings::default());
    }

    #[test]
    fn an_empty_table_is_not_written() {
        let out = toml::to_string(&AppSettings::default()).unwrap();
        assert!(!out.contains("[gui"), "{out}");
    }

    #[test]
    fn key_overrides_are_read() {
        let g = gui(r#"
[gui.keys]
FindInBuffer = ["primary+shift+f", "alt+f"]
ToggleSidebar = "primary+b"
ReplaceInBuffer = []
"#);
        let keys = g.key_overrides();
        assert_eq!(keys["FindInBuffer"], ["primary+shift+f", "alt+f"]);
        assert_eq!(keys["ToggleSidebar"], ["primary+b"]);
        assert!(keys["ReplaceInBuffer"].is_empty());
    }

    #[test]
    fn garbage_key_overrides_are_skipped() {
        let g = gui(r#"
[gui.keys]
FindInBuffer = 5
ToggleSidebar = ["primary+t", 3]
"#);
        let keys = g.key_overrides();
        assert!(!keys.contains_key("FindInBuffer"));
        assert_eq!(keys["ToggleSidebar"], ["primary+t"]);
    }

    #[test]
    fn window_decorations_are_read_leniently_and_round_trip() {
        assert_eq!(
            AppSettings::default().gui.window_decorations(),
            WindowDecorations::Client
        );
        let read = |value: &str| gui(&format!("[gui]\nwindow_decorations = {value}\n"));
        assert_eq!(
            read("\"system\"").window_decorations(),
            WindowDecorations::System
        );
        assert_eq!(
            read("\" System \"").window_decorations(),
            WindowDecorations::System
        );
        assert_eq!(
            read("\"client\"").window_decorations(),
            WindowDecorations::Client
        );
        // Unknown values and wrong types fall back, and the config still loads.
        for garbage in ["\"native\"", "\"\"", "1", "true", "[\"system\"]"] {
            assert_eq!(
                read(garbage).window_decorations(),
                WindowDecorations::Client,
                "{garbage}"
            );
        }

        let mut settings = AppSettings::default();
        settings
            .gui
            .set_window_decorations(WindowDecorations::System);
        let written = toml::to_string(&settings).unwrap();
        assert!(
            written.contains("window_decorations = \"system\""),
            "{written}"
        );
        let back: AppSettings = toml::from_str(&written).unwrap();
        assert_eq!(back.gui.window_decorations(), WindowDecorations::System);
        assert_eq!(back.gui, settings.gui);
    }

    #[test]
    fn reduce_motion_is_read_leniently_and_round_trips() {
        assert!(!AppSettings::default().gui.reduce_motion());
        let read = |value: &str| gui(&format!("[gui]\nreduce_motion = {value}\n"));
        assert!(read("true").reduce_motion());
        assert!(!read("false").reduce_motion());
        // Wrong types fall back to motion, and the config still loads.
        for garbage in ["\"yes\"", "1", "\"true\"", "[true]", "{ on = true }"] {
            assert!(!read(garbage).reduce_motion(), "{garbage}");
        }

        let mut settings = AppSettings::default();
        settings.gui.set_reduce_motion(true);
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("reduce_motion = true"), "{written}");
        let back: AppSettings = toml::from_str(&written).unwrap();
        assert!(back.gui.reduce_motion());
        assert_eq!(back.gui, settings.gui);
        // Next to the other `[gui]` keys, which it leaves alone.
        let mut both = read("true");
        both.set_window_decorations(WindowDecorations::System);
        assert!(both.reduce_motion());
        assert_eq!(both.window_decorations(), WindowDecorations::System);
    }

    /// Only the keys the edit changed (set or removed) move onto the target;
    /// the target's other keys stay.
    #[test]
    fn apply_changes_moves_only_the_changed_keys() {
        let before = gui("[gui]\nreduce_motion = false\nwindow_decorations = \"system\"\n");
        let mut after = before.clone();
        after.set_reduce_motion(true);
        after.table.remove(WINDOW_DECORATIONS);
        let mut target = gui("[gui]\nkept = 300.0\nwindow_decorations = \"system\"\n");
        target.apply_changes(&before, &after);
        assert!(target.reduce_motion());
        assert_eq!(
            target.window_decorations(),
            WindowDecorations::Client,
            "removed"
        );
        assert_eq!(target.table.get("kept"), Some(&Value::Float(300.0)), "kept");
        let mut untouched = target.clone();
        untouched.apply_changes(&after, &after);
        assert_eq!(untouched, target, "no change, nothing moves");
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
reduce_motion = true
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

    #[test]
    fn layout_round_trips_as_a_nested_table() {
        let layout: Value = toml::from_str(
            r#"
kind = "split"
direction = "horizontal"
[[children]]
weight = 1.0
[children.node]
kind = "slot"
views = ["files"]
"#,
        )
        .unwrap();
        let mut settings = AppSettings::default();
        assert_eq!(settings.gui.layout(), None);
        settings.gui.set_layout(layout.clone());
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("[gui.layout]"), "{written}");
        let back: AppSettings = toml::from_str(&written).unwrap();
        assert_eq!(back.gui.layout(), Some(layout));
        assert_eq!(back.gui, settings.gui);
    }

    #[test]
    fn rail_reads_default_when_absent_blank_or_invalid() {
        assert_eq!(GuiSettings::default().rail(), RailPosition::Left);
        for bad in [
            "rail = \"diagonal\"",
            "rail = 3",
            "rail = \"\"",
            "rail = true",
        ] {
            assert_eq!(
                gui(&format!("[gui]\n{bad}\n")).rail(),
                RailPosition::Left,
                "{bad}"
            );
        }
        assert_eq!(gui("[gui]\nrail = \" Top \"\n").rail(), RailPosition::Top);
    }

    #[test]
    fn rail_round_trips_and_default_removes_the_key() {
        let mut g = GuiSettings::default();
        for rail in [RailPosition::Right, RailPosition::Top, RailPosition::Bottom] {
            g.set_rail(rail);
            assert_eq!(g.rail(), rail);
        }
        g.set_rail(RailPosition::Left);
        assert_eq!(g.rail(), RailPosition::Left);
        assert!(g.is_empty(), "the default removes the key");
    }

    #[test]
    fn rail_hidden_reads_default_false_and_round_trips() {
        assert!(!GuiSettings::default().rail_hidden());
        assert!(!gui("[gui]\nrail_hidden = \"yes\"\n").rail_hidden());
        let mut g = GuiSettings::default();
        g.set_rail_hidden(true);
        assert!(g.rail_hidden());
        g.set_rail_hidden(false);
        assert!(!g.rail_hidden());
        assert!(g.is_empty(), "the default removes the key");
    }

    #[test]
    fn apply_changes_carries_rail_and_rail_hidden() {
        let before = GuiSettings::default();
        let mut after = before.clone();
        after.set_rail(RailPosition::Bottom);
        after.set_rail_hidden(true);
        let mut disk = GuiSettings::default();
        disk.set_layout_preset(Some("kept"));
        disk.apply_changes(&before, &after);
        assert_eq!(disk.rail(), RailPosition::Bottom);
        assert!(disk.rail_hidden());
        assert_eq!(disk.layout_preset().as_deref(), Some("kept"));
    }

    #[test]
    fn other_gui_keys_are_left_alone_by_the_rail_keys() {
        let mut g = gui("[gui]\nlayout_preset = \"a\"\nreduce_motion = true\n");
        g.set_rail(RailPosition::Right);
        g.set_rail_hidden(true);
        assert_eq!(g.layout_preset().as_deref(), Some("a"));
        assert!(g.reduce_motion());
    }

    #[test]
    fn layout_preset_reads_and_clears() {
        let mut g = GuiSettings::default();
        assert_eq!(g.layout_preset(), None);
        g.set_layout_preset(Some("  writing "));
        assert_eq!(g.layout_preset().as_deref(), Some("writing"));
        g.set_layout_preset(Some("   "));
        assert_eq!(g.layout_preset(), None, "blank reads as none");
        g.set_layout_preset(Some("focus"));
        g.set_layout_preset(None);
        assert_eq!(g.layout_preset(), None);
        assert!(g.is_empty(), "the key is removed, not blanked");
        assert_eq!(gui("[gui]\nlayout_preset = 3\n").layout_preset(), None);
    }

    #[test]
    fn malformed_layout_reads_as_none_not_error() {
        for bad in ["layout = 5", "layout = \"x\"", "layout = [1, 2]"] {
            let g = gui(&format!("[gui]\n{bad}\n"));
            assert_eq!(g.layout(), None, "{bad}");
        }
    }

    /// A TUI load + save keeps `[gui.layout]` and keys it does not know.
    #[test]
    fn tui_save_keeps_gui_layout_and_unknown_keys_byte_for_byte() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().canonicalize().unwrap().join("config.toml");
        let current = crate::settings::config_migration::CURRENT_CONFIG_VERSION;
        let gui_part = r#"
[gui]
layout_preset = "writing"

[gui.layout]
kind = "split"
direction = "horizontal"
future_field = { anything = [1, 2] }

[[gui.layout.children]]
weight = 1.0

[gui.layout.children.node]
kind = "slot"
views = ["files"]

[gui.future]
x = 1
y = [true, "z"]
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
        assert!(written["gui"]["layout"].is_table());
        assert!(written["gui"]["future"].is_table());
    }
}
