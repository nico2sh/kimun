//! The `[gui]` table of the shared `config.toml` (ADR-0049 §6, ADR-0050 §5):
//! the GUI's drawer placements, sizes, key overrides, window decorations and
//! reduced motion.
//!
//! Each placed view has its own entry, `[gui.views.<view-id>]`, with its
//! `side` (`"left"` or `"right"`), its drawer `slot` (an index, 0 the top)
//! and whether it is `shown`; a view never placed has none. Each drawer's
//! geometry stays in flat keys: `drawer_<side>_width`, `drawer_<side>_visible`
//! and `drawer_<side>_split` (one ratio per drawer).
//!
//! Earlier builds wrote per-drawer lists instead — `drawer_<side>` (every
//! view placed there, top position first), `drawer_<side>_bottom` and
//! `drawer_<side>_shown` — and before that `drawer_<side>_hidden` and
//! `drawer_right_home`. They are still read, as legacy input, when the file
//! has no `[gui.views]`, and dropped on the next GUI save.
//!
//! The TUI never reads it, but every config save goes through
//! [`AppSettings`](super::AppSettings), so the table is kept as the raw TOML it
//! was read as and written back unchanged: keys this version does not know
//! (written by a newer GUI) survive a TUI save. The typed readers below are
//! lenient — a value of the wrong type or out of range reads as its default,
//! never as a parse error, so a typo in `[gui]` cannot send the whole config
//! down the corrupt-file path.
//!
//! `reduce_motion = true` keeps the desktop app's decorative motion still (the
//! Welcome wordmark's bounce, loading spinners). It is the desktop app's
//! Preferences > Display > Reduce motion, applied as soon as it is saved; the
//! TUI never shows it. Missing means `false`: the system's own reduced-motion
//! setting is not read (GPUI offers no query for it). There is no desktop-app
//! page in the user docs yet, so this is where the key is described.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml::{Table, Value};

use crate::logic::layout::{MAX_DRAWER_SLOTS, Side};

/// A drawer's width when the config does not give one, in the GUI's logical
/// pixels. Clamping to the window is the layout model's job (`WidthLimits`).
pub const DEFAULT_DRAWER_WIDTH: f64 = 260.0;
/// A split drawer's first-slot share when the config does not give one.
pub const DEFAULT_SPLIT_RATIO: f64 = 0.5;
/// The view ids in the left drawer when the config does not say: the file
/// browser, as the TUI opens.
pub const DEFAULT_LEFT_VIEWS: [&str; 1] = ["files"];

const KEYS: &str = "keys";
const VIEWS: &str = "views";
const WINDOW_DECORATIONS: &str = "window_decorations";
const REDUCE_MOTION: &str = "reduce_motion";

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

/// One view's placement as `[gui.views.<view-id>]` records it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewEntry {
    /// The drawer it is placed in.
    pub side: Side,
    /// Its drawer slot, an index from the top (0).
    pub slot: usize,
    /// Whether it is shown in that slot.
    pub shown: bool,
}

/// The `side` value of a view entry.
fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

/// A view entry read leniently: `None` unless it is a table with a `side` of
/// `"left"` or `"right"` and a `slot` that is a non-negative integer. A slot
/// past the last is the last; `shown` is true only when it is `true`.
fn view_entry(value: &Value) -> Option<ViewEntry> {
    let Value::Table(entry) = value else {
        return None;
    };
    let side = match entry.get("side").and_then(Value::as_str).map(str::trim) {
        Some("left") => Side::Left,
        Some("right") => Side::Right,
        _ => return None,
    };
    let slot = match entry.get("slot") {
        Some(Value::Integer(i)) if *i >= 0 => {
            usize::try_from(*i).map_or(MAX_DRAWER_SLOTS - 1, |i| i.min(MAX_DRAWER_SLOTS - 1))
        }
        _ => return None,
    };
    let shown = matches!(entry.get("shown"), Some(Value::Boolean(true)));
    Some(ViewEntry { side, slot, shown })
}

/// A view hidden from a split drawer, as builds before placements recorded
/// it in `drawer_<side>_hidden`: `{ view = "files", with = "find", slot = 0,
/// split = 0.3 }`. Read only, to load such a file into placements.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyHiddenSlot {
    /// The hidden view's id.
    pub view: String,
    /// The id of the view that stayed in the drawer.
    pub with: String,
    /// The hidden view's slot: 0 (above `with`) or 1 (below).
    pub slot: usize,
    /// The drawer's first-slot share when the view was hidden.
    pub split: f64,
}

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

/// The non-empty string ids of a TOML list, trimmed, first occurrence
/// kept; `None` when `value` is missing or not a list.
fn id_list(value: Option<&Value>) -> Option<Vec<String>> {
    let Some(Value::Array(items)) = value else {
        return None;
    };
    let mut ids: Vec<String> = Vec::new();
    for id in items.iter().filter_map(Value::as_str).map(str::trim) {
        if !id.is_empty() && !ids.iter().any(|v| v == id) {
            ids.push(id.to_string());
        }
    }
    Some(ids)
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

    /// `[gui.views]`: each view id with its placement, in id order. `None`
    /// when the table is missing or not a table — a file from before view
    /// entries, whose placements are in the legacy lists. Ids are trimmed;
    /// an empty id, a repeated one (after trimming) and an entry
    /// [`ViewEntry`] cannot read are skipped. Deciding which ids are views,
    /// and settling two shown at one slot, is the GUI's.
    pub fn view_entries(&self) -> Option<Vec<(String, ViewEntry)>> {
        let Some(Value::Table(views)) = self.table.get(VIEWS) else {
            return None;
        };
        let mut out: Vec<(String, ViewEntry)> = Vec::new();
        for (id, value) in views {
            let id = id.trim();
            if id.is_empty() || out.iter().any(|(seen, _)| seen == id) {
                continue;
            }
            if let Some(entry) = view_entry(value) {
                out.push((id.to_string(), entry));
            }
        }
        Some(out)
    }

    /// Rewrite `[gui.views]` as `entries`, plus the entries already there
    /// whose id `keep` accepts and `entries` does not name, kept exactly as
    /// written (a newer build's views survive). Every other entry goes.
    pub fn set_view_entries(
        &mut self,
        entries: &[(String, ViewEntry)],
        keep: impl Fn(&str) -> bool,
    ) {
        let mut views = Table::new();
        if let Some(Value::Table(old)) = self.table.get(VIEWS) {
            for (id, value) in old {
                if keep(id) && !entries.iter().any(|(name, _)| name == id) {
                    views.insert(id.clone(), value.clone());
                }
            }
        }
        for (id, entry) in entries {
            let mut table = Table::new();
            table.insert("side".into(), Value::String(side_name(entry.side).into()));
            table.insert("slot".into(), Value::Integer(entry.slot as i64));
            table.insert("shown".into(), Value::Boolean(entry.shown));
            views.insert(id.clone(), Value::Table(table));
        }
        self.table.insert(VIEWS.into(), Value::Table(views));
    }

    /// Legacy: the ids of the views placed in `side`'s drawer, shown or
    /// hidden, top position first, from the `drawer_<side>` list.
    ///
    /// Missing or not a list: the default (`["files"]` left, nothing right).
    /// Entries that are not non-empty strings are skipped, and a view listed
    /// twice keeps its first place — a view is placed in one drawer, so the
    /// right drawer also drops any id the left one already holds.
    pub fn drawer_views(&self, side: Side) -> Vec<String> {
        let mut views = self.listed_views(side);
        if side == Side::Right {
            let left = self.listed_views(Side::Left);
            views.retain(|v| !left.contains(v));
        }
        views
    }

    fn listed_views(&self, side: Side) -> Vec<String> {
        id_list(self.table.get(&key(side, ""))).unwrap_or_else(|| match side {
            Side::Left => DEFAULT_LEFT_VIEWS.iter().map(|v| v.to_string()).collect(),
            Side::Right => Vec::new(),
        })
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

    /// Legacy `drawer_<side>_bottom`: the ids placed at the bottom slot
    /// position, as written. `None` when missing or not a list (every view
    /// but the first then reads as the bottom).
    pub fn drawer_bottom(&self, side: Side) -> Option<Vec<String>> {
        id_list(self.table.get(&key(side, "bottom")))
    }

    /// Legacy `drawer_<side>_shown`: the ids of the shown views, as written.
    /// `None` when missing or not a list (the first view listed at each
    /// slot position then reads as shown).
    pub fn drawer_shown(&self, side: Side) -> Option<Vec<String>> {
        id_list(self.table.get(&key(side, "shown")))
    }

    /// The view an earlier build recorded hiding from `side`'s split drawer
    /// (`drawer_<side>_hidden`), if well formed: two non-empty distinct ids,
    /// a slot of 0 or 1 and a split strictly between 0 and 1
    /// ([`DEFAULT_SPLIT_RATIO`] when missing). Anything else reads as
    /// `None`.
    pub fn legacy_hidden_slot(&self, side: Side) -> Option<LegacyHiddenSlot> {
        let Some(Value::Table(entry)) = self.table.get(&key(side, "hidden")) else {
            return None;
        };
        let id = |field: &str| {
            entry
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        };
        let (view, with) = (id("view")?, id("with")?);
        let slot = match entry.get("slot") {
            Some(Value::Integer(i @ 0..=1)) => *i as usize,
            _ => return None,
        };
        let split = match entry.get("split") {
            None => DEFAULT_SPLIT_RATIO,
            Some(value) => finite(value).filter(|r| *r > 0.0 && *r < 1.0)?,
        };
        (view != with).then_some(LegacyHiddenSlot {
            view,
            with,
            slot,
            split,
        })
    }

    /// `drawer_right_home`, as an earlier build wrote it: the ids of views
    /// in no slot that opened on the right. Missing or not a list: none.
    pub fn legacy_right_home_views(&self) -> Vec<String> {
        id_list(self.table.get(&key(Side::Right, "home"))).unwrap_or_default()
    }

    /// Drop the keys of earlier builds that `[gui.views]` replaces: the
    /// lists `drawer_<side>`, `drawer_<side>_bottom` and `drawer_<side>_shown`,
    /// and `drawer_<side>_hidden` and `drawer_right_home`.
    pub fn remove_legacy_keys(&mut self) {
        for side in [Side::Left, Side::Right] {
            for field in ["", "bottom", "shown", "hidden"] {
                self.table.remove(&key(side, field));
            }
        }
        self.table.remove(&key(Side::Right, "home"));
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
        let entry = |side, slot, shown| ViewEntry { side, slot, shown };
        g.set_view_entries(
            &[
                ("files".into(), entry(Side::Left, 0, true)),
                ("backlinks".into(), entry(Side::Right, 0, false)),
                ("find".into(), entry(Side::Right, 1, true)),
            ],
            |_| false,
        );
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
        assert_eq!(
            g.view_entries().unwrap(),
            [
                ("backlinks".to_string(), entry(Side::Right, 0, false)),
                ("files".to_string(), entry(Side::Left, 0, true)),
                ("find".to_string(), entry(Side::Right, 1, true)),
            ]
        );
        assert!(written.contains("[gui.views.find]"), "{written}");
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
    fn legacy_placement_lists_are_read_and_garbage_reads_as_none() {
        let g = gui(r#"
[gui]
drawer_left_bottom = ["find"]
drawer_left_shown = ["files", "find"]
drawer_right_shown = []
"#);
        assert_eq!(g.drawer_bottom(Side::Left).unwrap(), ["find"]);
        assert_eq!(g.drawer_shown(Side::Left).unwrap(), ["files", "find"]);
        assert_eq!(g.drawer_shown(Side::Right).unwrap(), [] as [&str; 0]);
        assert_eq!(g.drawer_bottom(Side::Right), None);
        assert_eq!(g.view_entries(), None, "no [gui.views]: legacy input");

        let g = gui(r#"
[gui]
drawer_left_bottom = "find"
drawer_left_shown = ["tags", 3, "", "tags", " files "]
"#);
        assert_eq!(g.drawer_bottom(Side::Left), None);
        assert_eq!(g.drawer_shown(Side::Left).unwrap(), ["tags", "files"]);
    }

    #[test]
    fn view_entries_are_read_leniently() {
        let entry = |side, slot, shown| ViewEntry { side, slot, shown };
        let g = gui(r#"
[gui.views]
files = { side = "left", slot = 0, shown = true }
find = { side = "right", slot = 1 }
tags = { side = "left", slot = 9, shown = true }
links = { side = "up", slot = 0, shown = true }
outline = { side = "left", slot = -1, shown = true }
config = { side = "left", slot = "top", shown = true }
cfg = "left"
" find " = { side = "left", slot = 0, shown = true }
"" = { side = "left", slot = 0, shown = true }
newer = { side = "right", slot = 0, shown = "yes", extra = 1 }
"#);
        assert_eq!(
            g.view_entries().unwrap(),
            [
                // " find " sorts first, trimmed; the later "find" repeats it.
                ("find".to_string(), entry(Side::Left, 0, true)),
                ("files".to_string(), entry(Side::Left, 0, true)),
                ("newer".to_string(), entry(Side::Right, 0, false)),
                // Past the last slot: the last.
                (
                    "tags".to_string(),
                    entry(Side::Left, MAX_DRAWER_SLOTS - 1, true)
                ),
            ]
        );
        // `views` not a table: no entries (the legacy lists are read).
        assert_eq!(gui("[gui]\nviews = 3\n").view_entries(), None);
        // An empty table: none placed, legacy lists ignored.
        assert_eq!(
            gui("[gui]\ndrawer_left = [\"find\"]\n[gui.views]\n").view_entries(),
            Some(Vec::new())
        );
    }

    #[test]
    fn set_view_entries_keeps_only_what_keep_accepts() {
        let mut g = gui(r#"
[gui.views]
files = { side = "left", slot = 0, shown = true }
tags = { side = "right", slot = 0, shown = true }
newer = { side = "right", slot = 1, shown = true, extra = [1, 2] }
"#);
        g.set_view_entries(
            &[(
                "files".into(),
                ViewEntry {
                    side: Side::Right,
                    slot: 1,
                    shown: false,
                },
            )],
            |id| id == "newer" || id == "files",
        );
        let written: Table = toml::from_str(&toml::to_string(&g).unwrap()).unwrap();
        let expected: Table = toml::from_str(
            r#"
[views]
files = { side = "right", slot = 1, shown = false }
newer = { side = "right", slot = 1, shown = true, extra = [1, 2] }
"#,
        )
        .unwrap();
        assert_eq!(written, expected);
    }

    #[test]
    fn legacy_keys_are_read_and_removed() {
        let mut g = gui(r#"
[gui]
drawer_left = ["find"]
drawer_left_bottom = []
drawer_left_shown = ["find"]
drawer_right = ["outline"]
drawer_right_bottom = []
drawer_right_shown = []
drawer_left_width = 300
drawer_left_visible = true
drawer_left_split = 0.4
drawer_left_hidden = { view = "files", with = "find", slot = 1 }
drawer_right_hidden = { view = "tags", with = "outline", slot = 0, split = 0.3 }
drawer_right_home = ["tags", 3, "", "tags", "links"]
kept = 1
"#);
        let left = g.legacy_hidden_slot(Side::Left).unwrap();
        assert_eq!((left.view.as_str(), left.with.as_str()), ("files", "find"));
        assert_eq!((left.slot, left.split), (1, DEFAULT_SPLIT_RATIO));
        assert_eq!(g.legacy_hidden_slot(Side::Right).unwrap().split, 0.3);
        assert_eq!(g.legacy_right_home_views(), ["tags", "links"]);
        g.remove_legacy_keys();
        assert_eq!(g.legacy_hidden_slot(Side::Left), None);
        assert!(g.legacy_right_home_views().is_empty());
        // The lists go too; the drawers' geometry stays.
        let written: Table = toml::from_str(&toml::to_string(&g).unwrap()).unwrap();
        let expected: Table = toml::from_str(
            "drawer_left_width = 300\ndrawer_left_visible = true\n\
             drawer_left_split = 0.4\nkept = 1\n",
        )
        .unwrap();
        assert_eq!(written, expected);

        for bad in [
            "drawer_left_hidden = \"files\"",
            "drawer_left_hidden = { view = \"files\", with = \"files\", slot = 0 }",
            "drawer_left_hidden = { view = \"files\", slot = 0 }",
            "drawer_left_hidden = { view = \"files\", with = \"find\", slot = 2 }",
            "drawer_left_hidden = { view = \"files\", with = \"find\", slot = \"top\" }",
            "drawer_left_hidden = { view = \"files\", with = \"find\", slot = 0, split = 7 }",
            "drawer_left_hidden = { view = 1, with = \"find\", slot = 0 }",
        ] {
            let g = gui(&format!("[gui]\n{bad}\ndrawer_right_home = \"tags\"\n"));
            assert_eq!(g.legacy_hidden_slot(Side::Left), None, "{bad}");
            assert!(g.legacy_right_home_views().is_empty());
        }
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
        let mut target = gui("[gui]\ndrawer_left_width = 300.0\nwindow_decorations = \"system\"\n");
        target.apply_changes(&before, &after);
        assert!(target.reduce_motion());
        assert_eq!(
            target.window_decorations(),
            WindowDecorations::Client,
            "removed"
        );
        assert_eq!(target.drawer_width(Side::Left), 300.0, "kept");
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
drawer_left = ["files", "tags", "find"]
drawer_left_bottom = ["find"]
drawer_left_shown = ["files"]
drawer_left_width = 312.0
drawer_right_visible = false
drawer_right_shown = []
drawer_left_hidden = { view = "find", with = "files", slot = 1, split = 0.3 }
drawer_right_home = ["tags"]
from_a_newer_gui = { anything = [1, 2] }

[gui.views.files]
side = "left"
slot = 0
shown = true

[gui.views.find]
side = "right"
slot = 1
shown = false

[gui.views.from_a_newer_gui]
side = "right"
slot = 2
shown = true
extra = "kept"

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
