//! The standalone keymap file (ADR-0064): a top-level `[keys]` table, action
//! name to a list of chords as written. An export writes every bindable action,
//! an imported file may hold only some. An empty list means "no shortcut". Pure
//! text in and out, headless-testable.
//!
//! **Import takes the chord.** A line of an imported file binds its chord as an
//! interactive edit does: the action that holds it, by default or in an earlier
//! line, loses it (`KeyDraft::replace`, which reports each take). This file
//! module only reads: [`Imported::dropped`](crate::keymap::file::Imported::dropped)
//! lists what the loader cannot read (unknown actions, bad chords). Text that is
//! not TOML, or has no `[keys]` table, is a [`FileError`](crate::keymap::file::FileError).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use kimun_notes::logic::actions::ActionShortcuts;
use toml::{Table, Value};

use super::{Keymap, KeymapError, Platform, parse_chord};

/// The table that holds the overrides.
const KEYS: &str = "keys";

/// A keymap file that cannot be read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileError {
    /// The file could not be read.
    Unreadable(String),
    /// The text is not valid TOML.
    NotToml(String),
    /// The text is TOML, but has no `[keys]` table.
    NoKeysTable,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::Unreadable(why) => write!(f, "Could not read the file: {why}"),
            FileError::NotToml(why) => write!(f, "This is not a keymap file: {why}"),
            FileError::NoKeysTable => write!(f, "This is not a keymap file: no [keys] table."),
        }
    }
}

/// What an imported file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Imported {
    /// The overrides as written, less what the loader could not read: an
    /// unknown action goes, and so does a chord that does not parse (an
    /// action left with none keeps an empty list, which the loader reads
    /// the same). A chord that clashes stays: the draft gives it to the
    /// line that names it.
    pub(crate) overrides: BTreeMap<String, Vec<String>>,
    /// What the loader rejected: unknown actions and bad chords.
    pub(crate) dropped: Vec<KeymapError>,
    /// Lines left out because the file comes from the other family of
    /// systems and they equal that system's defaults: the count and the
    /// system's name. `None` when nothing was left out.
    pub(crate) other_defaults: Option<(usize, &'static str)>,
}

/// The table that names the system that wrote the file.
const PLATFORM: &str = "platform";

/// The system this build runs on, as a keymap file names it.
pub(crate) fn os_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// The default table of the system `label` names: Linux and Windows share
/// one. `None` for a label this build does not know.
fn family(label: &str) -> Option<Platform> {
    match label {
        "macos" => Some(Platform::MacOs),
        "linux" | "windows" => Some(Platform::LinuxWindows),
        _ => None,
    }
}

/// A label as a person reads it.
fn display(label: &str) -> &'static str {
    match label {
        "macos" => "macOS",
        "windows" => "Windows",
        _ => "Linux",
    }
}

/// `overrides` as the text of a keymap file written on the system `label`
/// names: a top-level `platform` line, then a `[keys]` table sorted by
/// action name, a cleared action as `Action = []`. No overrides give an
/// empty table.
pub(crate) fn export(overrides: &BTreeMap<String, Vec<String>>, label: &str) -> String {
    let keys: Table = overrides
        .iter()
        .map(|(action, chords)| {
            let list = chords.iter().cloned().map(Value::String).collect();
            (action.clone(), Value::Array(list))
        })
        .collect();
    let mut file = Table::new();
    file.insert(PLATFORM.into(), Value::String(label.into()));
    file.insert(KEYS.into(), Value::Table(keys));
    // A table of arrays of strings always serializes.
    toml::to_string(&file).unwrap_or_else(|_| format!("{PLATFORM} = \"{label}\"\n[{KEYS}]\n"))
}

/// The overrides in `text`, and what the loader drops from them on
/// `platform`.
pub(crate) fn import(text: &str, platform: Platform) -> Result<Imported, FileError> {
    let file: Table = text
        .parse()
        .map_err(|e: toml::de::Error| FileError::NotToml(e.message().to_string()))?;
    let Some(Value::Table(keys)) = file.get(KEYS) else {
        return Err(FileError::NoKeysTable);
    };
    let mut overrides = read_keys(keys);
    let other_defaults = match file.get(PLATFORM).and_then(Value::as_str) {
        Some(label) => match family(label) {
            Some(source) if source != platform => {
                let skipped = drop_source_defaults(&mut overrides, source);
                (skipped > 0).then(|| (skipped, display(label)))
            }
            _ => None,
        },
        None => None,
    };
    let (_, mut dropped) = Keymap::load(platform, &overrides);
    for error in &dropped {
        match error {
            KeymapError::UnknownAction { name } => {
                overrides.remove(name);
            }
            KeymapError::InvalidChord { action, chord, .. } => {
                // By what the name reads as: an alias (`OpenSettings`) is
                // the same action.
                for (name, chords) in overrides.iter_mut() {
                    if ActionShortcuts::try_from(name.clone()).as_ref() == Ok(action) {
                        chords.retain(|c| c != chord);
                    }
                }
            }
            KeymapError::Collision { .. } => {}
        }
    }
    // A clash is no loss: the draft gives the chord to the line that
    // names it (`KeyDraft::replace`) and reports whom it took it from.
    dropped.retain(|e| !matches!(e, KeymapError::Collision { .. }));
    Ok(Imported {
        overrides,
        dropped,
        other_defaults,
    })
}

/// Remove each line of `overrides` that gives its action the chords the
/// `source` system has by default, read as that system reads them. The
/// number removed.
fn drop_source_defaults(overrides: &mut BTreeMap<String, Vec<String>>, source: Platform) -> usize {
    let (defaults, _) = Keymap::load(source, &BTreeMap::new());
    let before = overrides.len();
    overrides.retain(|name, chords| {
        let Ok(action) = ActionShortcuts::try_from(name.clone()) else {
            return true;
        };
        let parsed: Result<Vec<_>, _> = chords.iter().map(|c| parse_chord(c, source)).collect();
        let Ok(parsed) = parsed else {
            return true;
        };
        let default = defaults.chords_for(&action);
        let same = parsed.len() == default.len() && parsed.iter().all(|c| default.contains(c));
        !same
    });
    before - overrides.len()
}

/// Action name to chords, as `GuiSettings::key_overrides` reads `[gui.keys]`:
/// a string is a one-chord list, an empty list unbinds, items that are not
/// strings are skipped, and an entry of any other type is skipped.
fn read_keys(keys: &Table) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for (action, value) in keys {
        let chords = match value {
            Value::String(chord) => vec![chord.clone()],
            Value::Array(items) => items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect(),
            _ => continue,
        };
        out.insert(action.clone(), chords);
    }
    out
}

/// Write the keymap file for `overrides` to `path`, atomically.
pub(crate) fn write_file(
    path: &Path,
    overrides: &BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    kimun_core::system::replace_atomically(path, export(overrides, os_label()).as_bytes())
        .map_err(|e| e.to_string())
}

/// Read the keymap file at `path`, as [`import`] does.
pub(crate) fn read_file(path: &Path, platform: Platform) -> Result<Imported, FileError> {
    let text = std::fs::read_to_string(path).map_err(|e| FileError::Unreadable(e.to_string()))?;
    import(&text, platform)
}
