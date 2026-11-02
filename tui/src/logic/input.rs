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

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// What a pointer event did: a button press, drag or release, a wheel step, or a bare move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerKind {
    Down(Button),
    Drag(Button),
    Up(Button),
    ScrollUp,
    ScrollDown,
    Moved,
}

/// A pointer event, in cells relative to the area the widget draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PointerInput {
    pub kind: PointerKind,
    /// Cell column / row relative to the top-left of the area the widget draws in.
    pub col: u16,
    pub row: u16,
    pub mods: Mods,
}
