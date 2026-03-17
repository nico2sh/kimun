use std::{collections::HashMap, fmt::Display};

use action_shortcuts::ActionShortcuts;
use itertools::Itertools;
use key_combo::{KeyCombo, KeyModifiers};
use key_strike::KeyStrike;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers as CKeyMods};
use serde::{de::Visitor, ser::SerializeMap, Deserialize, Serialize};

pub mod action_shortcuts;
pub mod key_combo;
pub mod key_strike;

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
        formatter.write_str("A valid path with `/` separators, no need of starting `/`")
    }
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut bindings: HashMap<ActionShortcuts, Vec<KeyCombo>> =
            HashMap::with_capacity(map.size_hint().unwrap_or(0));
        // TODO: If an entry fails, ignore
        while let Some((key, value)) = map.next_entry()? {
            bindings.insert(key, value);
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
        let bind = self.bindings.get(combo).map(|a| a.to_owned());
        bind
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
        for (action, combos) in bindings {
            for combo in combos {
                kb.bindings.insert(combo.to_owned(), action.to_owned());
            }
        }
        kb
    }
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
    let key = match event.code {
        KeyCode::Char(c) => match c.to_ascii_lowercase() {
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
        },
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
    if event.modifiers.contains(CKeyMods::CONTROL) {
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
        action_shortcuts::{ActionShortcuts, TextAction},
        key_strike::KeyStrike,
        KeyBindings,
    };

    #[test]
    fn serialize_key_binding() {
        let mut km = KeyBindings::empty();
        km.batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::TogglePreview)
            .add(KeyStrike::KeyH, ActionShortcuts::Text(TextAction::Bold))
            .with_alt()
            .add(
                KeyStrike::KeyL,
                ActionShortcuts::Text(TextAction::Header(2)),
            );
        let km_str = toml::to_string(&km).unwrap();

        let expected = r#"TogglePreview = ["ctrl & N"]
TextEditor-Bold = ["ctrl & H"]
TextEditor-Header2 = ["ctrl+alt & L"]
"#
        .to_string();
        assert_eq!(expected, km_str);
    }

    #[test]
    fn serialize_key_binding_double_assignment() {
        let mut km = KeyBindings::empty();
        km.batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyN, ActionShortcuts::TogglePreview)
            .add(KeyStrike::KeyH, ActionShortcuts::Text(TextAction::Bold))
            .with_alt()
            .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Bold));
        let km_str = toml::to_string(&km).unwrap();

        let expected = r#"TogglePreview = ["ctrl & N"]
TextEditor-Bold = ["ctrl & H", "ctrl+alt & L"]
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
            .add(KeyStrike::KeyN, ActionShortcuts::TogglePreview)
            .add(KeyStrike::KeyH, ActionShortcuts::Text(TextAction::Bold))
            .with_alt()
            .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Bold));

        let km_str = r#"TogglePreview = ["ctrl & N"]
TextEditor-Bold = ["ctrl & H", "ctrl+alt & L"]
"#
        .to_string();

        let km = toml::from_str(&km_str).unwrap();

        assert_eq!(expected_km, km);
    }
}
