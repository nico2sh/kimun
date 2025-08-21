use std::{collections::HashMap, fmt::Display};

use action_shortcuts::{ActionShortcuts, TextAction};
use dioxus::logger::tracing::debug;
use itertools::Itertools;
use key_combo::{KeyCombo, KeyModifiers};
use key_strike::KeyStrike;
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

#[cfg(target_os = "macos")]
fn get_kb_buildr_ctrl_meta(key_bindings: &mut KeyBindings) -> KeyBindingBuilder {
    key_bindings.batch_add().with_meta()
}

#[cfg(not(target_os = "macos"))]
fn get_kb_buildr_ctrl_meta(key_bindings: &mut KeyBindings) -> KeyBindingBuilder {
    key_bindings.new_keybinding().with_ctrl()
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut kb = KeyBindings {
            bindings: HashMap::default(),
        };
        // We use meta on macOS, ctrl on Windows
        get_kb_buildr_ctrl_meta(&mut kb)
            .add(KeyStrike::Comma, ActionShortcuts::OpenSettings)
            .add(KeyStrike::Slash, ActionShortcuts::ToggleNoteBrowser)
            .add(KeyStrike::KeyE, ActionShortcuts::SearchNotes)
            .add(KeyStrike::KeyO, ActionShortcuts::OpenNote)
            .add(KeyStrike::KeyJ, ActionShortcuts::NewJournal)
            .add(KeyStrike::KeyY, ActionShortcuts::TogglePreview)
            .add(KeyStrike::KeyB, ActionShortcuts::Text(TextAction::Bold))
            .add(KeyStrike::KeyI, ActionShortcuts::Text(TextAction::Italic))
            .add(
                KeyStrike::KeyU,
                ActionShortcuts::Text(TextAction::Underline),
            )
            .add(
                KeyStrike::KeyS,
                ActionShortcuts::Text(TextAction::Strikethrough),
            )
            .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Link))
            .add(
                KeyStrike::KeyT,
                ActionShortcuts::Text(TextAction::ToggleHeader),
            )
            .add(
                KeyStrike::Digit1,
                ActionShortcuts::Text(TextAction::Header(1)),
            )
            .add(
                KeyStrike::Digit2,
                ActionShortcuts::Text(TextAction::Header(2)),
            )
            .add(
                KeyStrike::Digit3,
                ActionShortcuts::Text(TextAction::Header(3)),
            )
            // =============================
            // We add shift to the modifiers
            // =============================
            .with_shift()
            .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Image));
        kb
    }
}

impl KeyBindings {
    fn empty() -> Self {
        KeyBindings {
            bindings: HashMap::default(),
        }
    }

    pub fn batch_add(&mut self) -> KeyBindingBuilder {
        KeyBindingBuilder {
            bindings: self,
            modifiers: KeyModifiers::default(),
        }
    }

    pub fn get_action(&self, combo: &KeyCombo) -> Option<ActionShortcuts> {
        debug!("Combo: {:?}", combo);
        let bind = self.bindings.get(combo).map(|a| a.to_owned());
        debug!("Binding: {:?}", bind);
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

pub struct KeyBindingBuilder<'k> {
    bindings: &'k mut KeyBindings,
    modifiers: KeyModifiers,
}

impl<'k> KeyBindingBuilder<'k> {
    pub fn with_shift(mut self) -> Self {
        self.modifiers.add_shift();
        self
    }
    pub fn with_ctrl(mut self) -> Self {
        self.modifiers.add_ctrl();
        self
    }
    pub fn with_alt(mut self) -> Self {
        self.modifiers.add_alt();
        self
    }
    /// Same as with_cmd, used for non-macOS
    pub fn with_meta(mut self) -> Self {
        self.modifiers.add_meta_cmd();
        self
    }
    pub fn with_cmd(mut self) -> Self {
        self.modifiers.add_meta_cmd();
        self
    }
    pub fn add(self, key: KeyStrike, action: ActionShortcuts) -> KeyBindingBuilder<'k> {
        self.bindings
            .bindings
            .insert(KeyCombo::new(self.modifiers, key), action);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{key_strike::KeyStrike, ActionShortcuts, KeyBindings, TextAction};

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
