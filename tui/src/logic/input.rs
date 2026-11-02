//! Terminal-neutral key input for the editing engines.
//!
//! A key here is what the user *typed* — `Key::Char('$')`, `Key::Char('é')` —
//! not which physical key produced it. That is the difference from
//! [`crate::keys::key_combo::KeyCombo`], which names keys for shortcut lookup and
//! cannot represent most punctuation or any non-ASCII text.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub sup: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        sup: false,
    };
    pub const CTRL: Mods = Mods {
        ctrl: true,
        ..Mods::NONE
    };
    pub const ALT: Mods = Mods {
        alt: true,
        ..Mods::NONE
    };
    pub const SHIFT: Mods = Mods {
        shift: true,
        ..Mods::NONE
    };
    pub const SUPER: Mods = Mods {
        sup: true,
        ..Mods::NONE
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyInput {
    pub key: Key,
    pub mods: Mods,
}

impl KeyInput {
    pub const fn new(key: Key, mods: Mods) -> Self {
        Self { key, mods }
    }
    pub const fn plain(key: Key) -> Self {
        Self::new(key, Mods::NONE)
    }
    pub const fn ctrl(c: char) -> Self {
        Self::new(Key::Char(c), Mods::CTRL)
    }
}
