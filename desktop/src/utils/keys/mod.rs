use std::collections::HashMap;

use dioxus::logger::tracing::debug;
use key_combo::{KeyCombo, KeyModifiers, KeyStrike};

pub mod key_combo;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionShortcuts {
    OpenSettings,
    ToggleNoteBrowser,
    SearchNotes,
    OpenNote,
    NewJournal,
    TogglePreview,
    Text(TextAction),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextAction {
    Bold,
    Italic,
    Link,
    Image,
    ToggleHeader,
    Header(u8),
    Underline,
    Strikethrough,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    bindings: HashMap<KeyCombo, ActionShortcuts>,
}

#[cfg(target_os = "macos")]
fn get_kb_buildr_ctrl_meta(key_bindings: &mut KeyBindings) -> KeyBindingBuilder {
    key_bindings.new_keybinding().with_meta()
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
    pub fn new_keybinding(&mut self) -> KeyBindingBuilder {
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
}

pub struct KeyBindingBuilder<'k> {
    bindings: &'k mut KeyBindings,
    modifiers: KeyModifiers,
}

impl<'k> KeyBindingBuilder<'k> {
    pub fn with_shift(mut self) -> Self {
        self.modifiers.and_shift();
        self
    }
    pub fn with_ctrl(mut self) -> Self {
        self.modifiers.and_ctrl();
        self
    }
    pub fn with_alt(mut self) -> Self {
        self.modifiers.and_alt();
        self
    }
    pub fn with_meta(mut self) -> Self {
        self.modifiers.and_meta();
        self
    }
    pub fn add(self, key: KeyStrike, action: ActionShortcuts) -> KeyBindingBuilder<'k> {
        self.bindings
            .bindings
            .insert(KeyCombo::new(self.modifiers, key), action);
        self
    }
}
