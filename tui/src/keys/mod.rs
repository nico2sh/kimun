use std::{collections::HashMap, fmt::Display};

use action_shortcuts::ActionShortcuts;
use itertools::Itertools;
use key_combo::{KeyCombo, KeyModifiers};
use key_strike::KeyStrike;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers as CKeyMods};
use serde::{Deserialize, Serialize, de::Visitor, ser::SerializeMap};

pub mod action_shortcuts;
pub mod key_combo;
pub mod key_strike;
pub mod leader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    bindings: HashMap<KeyCombo, ActionShortcuts>,
}

impl Serialize for KeyBindings {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let kb_map = self.to_hashmap();
        let mut map = serializer.serialize_map(Some(kb_map.len()))?;
        for (k, v) in kb_map
            .iter()
            .sorted_by_key(|(action, _combo)| action.to_owned())
        {
            map.serialize_entry(&k, &v)?;
        }
        map.end()
    }
}

struct DeserializeKeyBindingsVisitor;
impl<'de> Visitor<'de> for DeserializeKeyBindingsVisitor {
    type Value = KeyBindings;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a keybindings map of action names to lists of key combos")
    }
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        use serde::de::{Error, IgnoredAny, IntoDeserializer};

        let mut bindings: HashMap<ActionShortcuts, Vec<KeyCombo>> =
            HashMap::with_capacity(map.size_hint().unwrap_or(0));

        loop {
            // Read the key as a raw String so a bad action name is recoverable.
            let key_str: String = match map.next_key::<String>() {
                Ok(Some(s)) => s,
                Ok(None) => break,
                Err(e) => return Err(e),
            };

            // Parse the action name; on failure, discard the value and continue.
            // The explicit error type pins the generic on `IntoDeserializer`.
            let action = match ActionShortcuts::deserialize(key_str.clone().into_deserializer()) {
                Ok(a) => a,
                Err(e) => {
                    let e: serde::de::value::Error = e;
                    let _ = map.next_value::<IgnoredAny>();
                    tracing::warn!(
                        "Skipping unknown action '{}' in keybindings config: {}",
                        key_str,
                        e
                    );
                    continue;
                }
            };

            match map.next_value::<Vec<KeyCombo>>() {
                Ok(value) => {
                    bindings.insert(action, value);
                }
                Err(e) => {
                    tracing::warn!("Skipping keybindings entry for action '{}': {}", action, e);
                }
            }
        }

        // Essential-action safety net: Quit must always have a binding.
        if !bindings.contains_key(&ActionShortcuts::Quit) {
            let quit_combo = default_quit_combo();

            let conflicting_action = bindings
                .iter()
                .find(|(_, combos)| combos.iter().any(|c| c == &quit_combo))
                .map(|(action, _)| action.clone());

            if let Some(other) = conflicting_action {
                return Err(A::Error::custom(format!(
                    "Quit action has no binding and the default combo Ctrl+Q is already mapped to '{}'. \
                     Add a valid Quit binding to your keybindings config.",
                    other
                )));
            }

            tracing::warn!("Quit action missing from keybindings; restoring default Ctrl+Q");
            bindings.insert(ActionShortcuts::Quit, vec![quit_combo]);
        }

        Ok(KeyBindings::from_hashmap(bindings))
    }
}

impl<'de> Deserialize<'de> for KeyBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(DeserializeKeyBindingsVisitor)
    }
}

impl Display for KeyBindings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut bindings: Vec<(ActionShortcuts, Vec<KeyCombo>)> = vec![];
        for (key, value) in &self.bindings {
            if let Some((_, combos)) = bindings
                .iter_mut()
                .find(|(shortcut, _combos)| shortcut.eq(value))
            {
                combos.push(key.to_owned());
                combos.sort();
            } else {
                bindings.push((value.to_owned(), vec![key.to_owned()]));
            }
        }

        bindings.sort_by_key(|(a, _v)| a.to_owned());
        for (key, value) in &bindings {
            writeln!(
                f,
                "{}: {}",
                key,
                value
                    .iter()
                    .map(|kc| kc.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            )?;
        }

        Ok(())
    }
}

impl KeyBindings {
    pub fn empty() -> Self {
        KeyBindings {
            bindings: HashMap::default(),
        }
    }

    pub fn batch_add(&mut self) -> KeyBindBatch<'_> {
        KeyBindBatch {
            bindings: self,
            modifiers: KeyModifiers::default(),
        }
    }

    pub fn get_action(&self, combo: &KeyCombo) -> Option<ActionShortcuts> {
        self.bindings.get(combo).map(|a| a.to_owned())
    }

    /// Returns the display string of the first combo bound to `action`, or `None`.
    pub fn first_combo_for(&self, action: &ActionShortcuts) -> Option<String> {
        self.bindings
            .iter()
            .find(|(_, a)| *a == action)
            .map(|(combo, _)| combo.to_string())
    }

    pub fn to_hashmap(&self) -> HashMap<ActionShortcuts, Vec<KeyCombo>> {
        let mut bindings: HashMap<ActionShortcuts, Vec<KeyCombo>> = HashMap::new();
        for (combo, action) in &self.bindings {
            let entry = bindings.entry(action.to_owned()).or_default();
            entry.push(combo.to_owned());
            entry.sort();
        }
        bindings
    }

    pub fn from_hashmap(bindings: HashMap<ActionShortcuts, Vec<KeyCombo>>) -> KeyBindings {
        let mut kb = KeyBindings::empty();
        for (action, combos) in &bindings {
            tracing::debug!("from_hashmap: action={} combos={:?}", action, combos);
        }
        for (action, combos) in bindings {
            for combo in combos {
                let valid = combo.is_valid_binding();
                tracing::debug!(
                    "from_hashmap: combo='{}' key={:?} modifiers={:?} valid={}",
                    combo,
                    combo.key,
                    combo.modifiers,
                    valid
                );
                if valid {
                    kb.bindings.insert(combo.to_owned(), action.to_owned());
                } else {
                    tracing::warn!(
                        "Skipping invalid key combo '{}' for action '{}': \
                         only ctrl/alt (with optional shift) + a letter, digit, or \
                         punctuation key, or bare F1–F12 are supported",
                        combo,
                        action
                    );
                }
            }
        }
        kb
    }
}

/// Canonical default combo for [`ActionShortcuts::Quit`]. Sourced once so the
/// deserialize safety net and [`crate::settings::default_keybindings`] can't
/// drift.
pub fn default_quit_combo() -> KeyCombo {
    KeyCombo::new(KeyModifiers::new().and_ctrl(), KeyStrike::KeyQ)
}

pub struct KeyBindBatch<'k> {
    bindings: &'k mut KeyBindings,
    modifiers: KeyModifiers,
}

impl<'k> KeyBindBatch<'k> {
    pub fn with_shift(mut self) -> Self {
        self.modifiers.with_shift();
        self
    }
    pub fn with_ctrl(mut self) -> Self {
        self.modifiers.with_ctrl();
        self
    }
    pub fn with_alt(mut self) -> Self {
        self.modifiers.with_alt();
        self
    }
    /// Same as with_cmd, used for non-macOS
    pub fn with_meta(mut self) -> Self {
        self.modifiers.with_meta_cmd();
        self
    }
    pub fn with_cmd(mut self) -> Self {
        self.modifiers.with_meta_cmd();
        self
    }
    pub fn add(self, key: KeyStrike, action: ActionShortcuts) -> KeyBindBatch<'k> {
        self.bindings
            .bindings
            .insert(KeyCombo::new(self.modifiers, key), action);
        self
    }
}

/// Convert a crossterm [`KeyEvent`] into a [`KeyCombo`] for keybinding lookup.
///
/// Returns `None` for key codes that have no [`KeyStrike`] mapping (e.g. media keys).
/// `BackTab` (Shift+Tab) is normalised to `Tab` with the `shift` modifier set.
pub fn key_event_to_combo(event: &KeyEvent) -> Option<KeyCombo> {
    // Some terminals deliver Ctrl+letter as raw control characters (e.g. Ctrl+Q → '\x11')
    // without setting the CONTROL modifier.  Normalise them here so the rest of the
    // function sees an ordinary letter + an implied ctrl flag.
    let mut implied_ctrl = false;
    let key = match event.code {
        KeyCode::Char(c) => {
            let c = if c as u8 >= 1 && c as u8 <= 26 {
                implied_ctrl = true;
                (c as u8 + b'a' - 1) as char
            } else {
                c
            };
            match c.to_ascii_lowercase() {
                'a' => KeyStrike::KeyA,
                'b' => KeyStrike::KeyB,
                'c' => KeyStrike::KeyC,
                'd' => KeyStrike::KeyD,
                'e' => KeyStrike::KeyE,
                'f' => KeyStrike::KeyF,
                'g' => KeyStrike::KeyG,
                'h' => KeyStrike::KeyH,
                'i' => KeyStrike::KeyI,
                'j' => KeyStrike::KeyJ,
                'k' => KeyStrike::KeyK,
                'l' => KeyStrike::KeyL,
                'm' => KeyStrike::KeyM,
                'n' => KeyStrike::KeyN,
                'o' => KeyStrike::KeyO,
                'p' => KeyStrike::KeyP,
                'q' => KeyStrike::KeyQ,
                'r' => KeyStrike::KeyR,
                's' => KeyStrike::KeyS,
                't' => KeyStrike::KeyT,
                'u' => KeyStrike::KeyU,
                'v' => KeyStrike::KeyV,
                'w' => KeyStrike::KeyW,
                'x' => KeyStrike::KeyX,
                'y' => KeyStrike::KeyY,
                'z' => KeyStrike::KeyZ,
                '0' => KeyStrike::Digit0,
                '1' => KeyStrike::Digit1,
                '2' => KeyStrike::Digit2,
                '3' => KeyStrike::Digit3,
                '4' => KeyStrike::Digit4,
                '5' => KeyStrike::Digit5,
                '6' => KeyStrike::Digit6,
                '7' => KeyStrike::Digit7,
                '8' => KeyStrike::Digit8,
                '9' => KeyStrike::Digit9,
                ',' => KeyStrike::Comma,
                '.' => KeyStrike::Period,
                '/' => KeyStrike::Slash,
                ';' => KeyStrike::Semicolon,
                '\'' => KeyStrike::Quote,
                '[' => KeyStrike::BracketLeft,
                ']' => KeyStrike::BracketRight,
                '\\' => KeyStrike::Backslash,
                '`' => KeyStrike::Backquote,
                '-' => KeyStrike::Minus,
                '=' => KeyStrike::Equal,
                _ => return None,
            }
        }
        KeyCode::Enter => KeyStrike::Enter,
        KeyCode::Backspace => KeyStrike::Backspace,
        KeyCode::Tab | KeyCode::BackTab => KeyStrike::Tab,
        KeyCode::Esc => KeyStrike::Escape,
        KeyCode::Up => KeyStrike::ArrowUp,
        KeyCode::Down => KeyStrike::ArrowDown,
        KeyCode::Left => KeyStrike::ArrowLeft,
        KeyCode::Right => KeyStrike::ArrowRight,
        KeyCode::Home => KeyStrike::Home,
        KeyCode::End => KeyStrike::End,
        KeyCode::PageUp => KeyStrike::PageUp,
        KeyCode::PageDown => KeyStrike::PageDown,
        KeyCode::Delete => KeyStrike::Delete,
        KeyCode::Insert => KeyStrike::Insert,
        KeyCode::F(n) => match n {
            1 => KeyStrike::F1,
            2 => KeyStrike::F2,
            3 => KeyStrike::F3,
            4 => KeyStrike::F4,
            5 => KeyStrike::F5,
            6 => KeyStrike::F6,
            7 => KeyStrike::F7,
            8 => KeyStrike::F8,
            9 => KeyStrike::F9,
            10 => KeyStrike::F10,
            11 => KeyStrike::F11,
            12 => KeyStrike::F12,
            _ => return None,
        },
        _ => return None,
    };

    let mut modifiers = KeyModifiers::default();
    if implied_ctrl || event.modifiers.contains(CKeyMods::CONTROL) {
        modifiers.with_ctrl();
    }
    // BackTab arrives as KeyCode::BackTab (no SHIFT bit set on some terminals).
    if event.modifiers.contains(CKeyMods::SHIFT) || matches!(event.code, KeyCode::BackTab) {
        modifiers.with_shift();
    }
    if event.modifiers.contains(CKeyMods::ALT) {
        modifiers.with_alt();
    }
    if event.modifiers.contains(CKeyMods::SUPER) || event.modifiers.contains(CKeyMods::META) {
        modifiers.with_meta_cmd();
    }

    Some(KeyCombo::new(modifiers, key))
}

#[cfg(test)]
mod tests {
    use super::{
        KeyBindings,
        action_shortcuts::{ActionShortcuts, TextAction},
        key_strike::KeyStrike,
    };

    #[test]
    fn serialize_key_binding() {
        let mut km = KeyBindings::empty();
        km.batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyH, ActionShortcuts::Text(TextAction::Bold))
            .with_alt()
            .add(
                KeyStrike::KeyL,
                ActionShortcuts::Text(TextAction::Header(2)),
            );
        let km_str = toml::to_string(&km).unwrap();

        let expected = r#"NewJournal = ["ctrl&N"]
TextEditor-Bold = ["ctrl&H"]
TextEditor-Header2 = ["ctrl+alt&L"]
"#
        .to_string();
        assert_eq!(expected, km_str);
    }

    #[test]
    fn serialize_key_binding_double_assignment() {
        let mut km = KeyBindings::empty();
        km.batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyH, ActionShortcuts::Text(TextAction::Bold))
            .with_alt()
            .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Bold));
        let km_str = toml::to_string(&km).unwrap();

        let expected = r#"NewJournal = ["ctrl&N"]
TextEditor-Bold = ["ctrl&H", "ctrl+alt&L"]
"#
        .to_string();
        assert_eq!(expected, km_str);
    }

    #[test]
    fn deserialize_key_binding_double_assignment() {
        let mut expected_km = KeyBindings::empty();
        expected_km
            .batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyH, ActionShortcuts::Text(TextAction::Bold))
            .add(KeyStrike::KeyQ, ActionShortcuts::Quit)
            .with_alt()
            .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Bold));

        let km_str = r#"NewJournal = ["ctrl & N"]
TextEditor-Bold = ["ctrl & H", "ctrl+alt & L"]
Quit = ["ctrl & Q"]
"#
        .to_string();

        let km = toml::from_str(&km_str).unwrap();

        assert_eq!(expected_km, km);
    }

    #[test]
    fn deserialize_skips_entry_with_unknown_action() {
        let toml_str = r#"NewJournal = ["ctrl & N"]
NotARealAction = ["ctrl & X"]
Quit = ["ctrl & Q"]
"#;

        let km: KeyBindings = toml::from_str(toml_str).expect("should not error");

        let mut expected = KeyBindings::empty();
        expected
            .batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyQ, ActionShortcuts::Quit);

        assert_eq!(expected, km);
    }

    #[test]
    fn deserialize_skips_entry_with_malformed_combo() {
        let toml_str = r#"NewJournal = ["ctrl & N"]
OpenNote = ["bogus & ZZZZ"]
Quit = ["ctrl & Q"]
"#;

        let km: KeyBindings = toml::from_str(toml_str).expect("should not error");

        let mut expected = KeyBindings::empty();
        expected
            .batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyQ, ActionShortcuts::Quit);

        assert_eq!(expected, km);
    }

    #[test]
    fn deserialize_injects_default_quit_when_missing() {
        let toml_str = r#"NewJournal = ["ctrl & N"]
"#;

        let km: KeyBindings = toml::from_str(toml_str).expect("should not error");

        let mut expected = KeyBindings::empty();
        expected
            .batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyQ, ActionShortcuts::Quit);

        assert_eq!(expected, km);
    }

    #[test]
    fn deserialize_errors_when_quit_missing_and_default_taken() {
        let toml_str = r#"OpenNote = ["ctrl & Q"]
"#;

        let result: Result<KeyBindings, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "expected deserialize to fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Quit") && err_msg.contains("Ctrl+Q"),
            "error message should mention Quit and Ctrl+Q, got: {}",
            err_msg
        );
    }

    #[test]
    fn deserialize_recovers_quit_when_quit_entry_is_malformed() {
        let toml_str = r#"NewJournal = ["ctrl & N"]
Quit = ["bogus & ZZZZ"]
"#;

        let km: KeyBindings = toml::from_str(toml_str).expect("should not error");

        let mut expected = KeyBindings::empty();
        expected
            .batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyQ, ActionShortcuts::Quit);

        assert_eq!(expected, km);
    }
}
