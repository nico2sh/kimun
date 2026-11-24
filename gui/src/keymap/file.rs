//! The standalone keymap file (ADR-0064): a top-level `[keys]` table of
//! overrides only, action name to a list of chords as written. An empty list
//! means "no shortcut". Pure text in and out, headless-testable.
//!
//! **Import never steals.** A hand-edited or imported file keeps the
//! loader's rule: the first claim wins. What the loader drops is reported in
//! [`Imported::dropped`](crate::keymap::file::Imported::dropped); the rest is kept. Only text that is not TOML, or
//! has no `[keys]` table, is a [`FileError`](crate::keymap::file::FileError).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use kimun_notes::logic::actions::ActionShortcuts;
use toml::{Table, Value};

use super::{Keymap, KeymapError, Platform};

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
    /// the same). A chord dropped for a clash stays: the loader drops it
    /// again on every load, and the row then shows what it got.
    pub(crate) overrides: BTreeMap<String, Vec<String>>,
    /// What the loader rejected: unknown actions, bad chords, clashes.
    pub(crate) dropped: Vec<KeymapError>,
}

/// `overrides` as the text of a keymap file: a `[keys]` table sorted by
/// action name, a cleared action as `Action = []`. No overrides give an
/// empty table.
pub(crate) fn export(overrides: &BTreeMap<String, Vec<String>>) -> String {
    let keys: Table = overrides
        .iter()
        .map(|(action, chords)| {
            let list = chords.iter().cloned().map(Value::String).collect();
            (action.clone(), Value::Array(list))
        })
        .collect();
    let mut file = Table::new();
    file.insert(KEYS.into(), Value::Table(keys));
    // A table of arrays of strings always serializes.
    toml::to_string(&file).unwrap_or_else(|_| format!("[{KEYS}]\n"))
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
    let (_, dropped) = Keymap::load(platform, &overrides);
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
    Ok(Imported { overrides, dropped })
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
    kimun_core::system::replace_atomically(path, export(overrides).as_bytes())
        .map_err(|e| e.to_string())
}

/// Read the keymap file at `path`, as [`import`] does.
pub(crate) fn read_file(path: &Path, platform: Platform) -> Result<Imported, FileError> {
    let text = std::fs::read_to_string(path).map_err(|e| FileError::Unreadable(e.to_string()))?;
    import(&text, platform)
}
