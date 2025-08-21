use std::fmt::Display;

use dioxus::{events::Key, logger::tracing::error};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub enum KeyStrike {
    #[default]
    None,
    Unknown,
    /// <code class="keycap">`~</code> on a US keyboard. This is the <code class="keycap">半角/全角/漢字</code> (<span class="unicode">hankaku/zenkaku/kanji</span>) key on Japanese keyboards
    Backquote,
    /// Used for both the US <code class="keycap">\|</code> (on the 101-key layout) and also for the key
    /// located between the <code class="keycap">"</code> and <code class="keycap">Enter</code> keys on row C of the 102-,
    /// 104- and 106-key layouts.
    /// Labelled <code class="keycap">#~</code> on a UK (102) keyboard.
    Backslash,
    /// <code class="keycap">[{</code> on a US keyboard.
    BracketLeft,
    /// <code class="keycap">]}</code> on a US keyboard.
    BracketRight,
    /// <code class="keycap">,&lt;</code> on a US keyboard.
    Comma,
    /// <code class="keycap">0)</code> on a US keyboard.
    Digit0,
    /// <code class="keycap">1!</code> on a US keyboard.
    Digit1,
    /// <code class="keycap">2@</code> on a US keyboard.
    Digit2,
    /// <code class="keycap">3#</code> on a US keyboard.
    Digit3,
    /// <code class="keycap">4$</code> on a US keyboard.
    Digit4,
    /// <code class="keycap">5%</code> on a US keyboard.
    Digit5,
    /// <code class="keycap">6^</code> on a US keyboard.
    Digit6,
    /// <code class="keycap">7&amp;</code> on a US keyboard.
    Digit7,
    /// <code class="keycap">8*</code> on a US keyboard.
    Digit8,
    /// <code class="keycap">9(</code> on a US keyboard.
    Digit9,
    /// <code class="keycap">=+</code> on a US keyboard.
    Equal,
    /// Located between the left <code class="keycap">Shift</code> and <code class="keycap">Z</code> keys.
    /// Labelled <code class="keycap">\|</code> on a UK keyboard.
    KeyA,
    /// <code class="keycap">b</code> on a US keyboard.
    KeyB,
    /// <code class="keycap">c</code> on a US keyboard.
    KeyC,
    /// <code class="keycap">d</code> on a US keyboard.
    KeyD,
    /// <code class="keycap">e</code> on a US keyboard.
    KeyE,
    /// <code class="keycap">f</code> on a US keyboard.
    KeyF,
    /// <code class="keycap">g</code> on a US keyboard.
    KeyG,
    /// <code class="keycap">h</code> on a US keyboard.
    KeyH,
    /// <code class="keycap">i</code> on a US keyboard.
    KeyI,
    /// <code class="keycap">j</code> on a US keyboard.
    KeyJ,
    /// <code class="keycap">k</code> on a US keyboard.
    KeyK,
    /// <code class="keycap">l</code> on a US keyboard.
    KeyL,
    /// <code class="keycap">m</code> on a US keyboard.
    KeyM,
    /// <code class="keycap">n</code> on a US keyboard.
    KeyN,
    /// <code class="keycap">o</code> on a US keyboard.
    KeyO,
    /// <code class="keycap">p</code> on a US keyboard.
    KeyP,
    /// <code class="keycap">q</code> on a US keyboard.
    /// Labelled <code class="keycap">a</code> on an AZERTY (e.g., French) keyboard.
    KeyQ,
    /// <code class="keycap">r</code> on a US keyboard.
    KeyR,
    /// <code class="keycap">s</code> on a US keyboard.
    KeyS,
    /// <code class="keycap">t</code> on a US keyboard.
    KeyT,
    /// <code class="keycap">u</code> on a US keyboard.
    KeyU,
    /// <code class="keycap">v</code> on a US keyboard.
    KeyV,
    /// <code class="keycap">w</code> on a US keyboard.
    /// Labelled <code class="keycap">z</code> on an AZERTY (e.g., French) keyboard.
    KeyW,
    /// <code class="keycap">x</code> on a US keyboard.
    KeyX,
    /// <code class="keycap">y</code> on a US keyboard.
    /// Labelled <code class="keycap">z</code> on a QWERTZ (e.g., German) keyboard.
    KeyY,
    /// <code class="keycap">z</code> on a US keyboard.
    /// Labelled <code class="keycap">w</code> on an AZERTY (e.g., French) keyboard, and <code class="keycap">y</code> on a
    /// QWERTZ (e.g., German) keyboard.
    KeyZ,
    /// <code class="keycap">-_</code> on a US keyboard.
    Minus,
    /// <code class="keycap">.></code> on a US keyboard.
    Period,
    /// <code class="keycap">'"</code> on a US keyboard.
    Quote,
    /// <code class="keycap">;:</code> on a US keyboard.
    Semicolon,
    /// <code class="keycap">/?</code> on a US keyboard.
    Slash,
    /// <code class="keycap">Backspace</code> or <code class="keycap">⌫</code>.
    /// Labelled <code class="keycap">Delete</code> on Apple keyboards.
    Backspace,
    /// <code class="keycap">CapsLock</code> or <code class="keycap">⇪</code>
    Enter,
    /// <code class="keycap"> </code> (space)
    Space,
    /// <code class="keycap">Tab</code> or <code class="keycap">⇥</code>
    Tab,
    Delete,
    /// <code class="keycap">End</code> or <code class="keycap">↘</code>
    End,
    /// <code class="keycap">Home</code> or <code class="keycap">↖</code>
    Home,
    /// <code class="keycap">Insert</code> or <code class="keycap">Ins</code>. Not present on Apple keyboards.
    Insert,
    /// <code class="keycap">Page Down</code>, <code class="keycap">PgDn</code> or <code class="keycap">⇟</code>
    PageDown,
    /// <code class="keycap">Page Up</code>, <code class="keycap">PgUp</code> or <code class="keycap">⇞</code>
    PageUp,
    /// <code class="keycap">↓</code>
    ArrowDown,
    /// <code class="keycap">←</code>
    ArrowLeft,
    /// <code class="keycap">→</code>
    ArrowRight,
    /// <code class="keycap">↑</code>
    ArrowUp,
    /// <code class="keycap">Esc</code> or <code class="keycap">⎋</code>
    Escape,
    /// <code class="keycap">PrtScr SysRq</code> or <code class="keycap">Print Screen</code>
    PrintScreen,
    /// <code class="keycap">Scroll Lock</code>
    ScrollLock,
    /// <code class="keycap">Pause Break</code>
    Pause,
    /// Some laptops place this key to the left of the <code class="keycap">↑</code> key.
    /// <code class="keycap">F1</code>
    F1,
    /// <code class="keycap">F2</code>
    F2,
    /// <code class="keycap">F3</code>
    F3,
    /// <code class="keycap">F4</code>
    F4,
    /// <code class="keycap">F5</code>
    F5,
    /// <code class="keycap">F6</code>
    F6,
    /// <code class="keycap">F7</code>
    F7,
    /// <code class="keycap">F8</code>
    F8,
    /// <code class="keycap">F9</code>
    F9,
    /// <code class="keycap">F10</code>
    F10,
    /// <code class="keycap">F11</code>
    F11,
    /// <code class="keycap">F12</code>
    F12,
    /// <code class="keycap">F13</code>
    F13,
    /// <code class="keycap">F14</code>
    F14,
    /// <code class="keycap">F15</code>
    F15,
    /// <code class="keycap">F16</code>
    F16,
    /// <code class="keycap">F17</code>
    F17,
    /// <code class="keycap">F18</code>
    F18,
    /// <code class="keycap">F19</code>
    F19,
    /// <code class="keycap">F20</code>
    F20,
    /// <code class="keycap">F21</code>
    F21,
    /// <code class="keycap">F22</code>
    F22,
    /// <code class="keycap">F23</code>
    F23,
    /// <code class="keycap">F24</code>
    F24,
    /// <code class="keycap">F25</code>
    F25,
    /// <code class="keycap">F26</code>
    F26,
    /// <code class="keycap">F27</code>
    F27,
    /// <code class="keycap">F28</code>
    F28,
    /// <code class="keycap">F29</code>
    F29,
    /// <code class="keycap">F30</code>
    F30,
    /// <code class="keycap">F31</code>
    F31,
    /// <code class="keycap">F32</code>
    F32,
    /// <code class="keycap">F33</code>
    F33,
    /// <code class="keycap">F34</code>
    F34,
    /// <code class="keycap">F35</code>
    F35,
}

impl Display for KeyStrike {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            KeyStrike::None => "<none>",
            KeyStrike::Unknown => "N/A",
            KeyStrike::Backquote => "`",
            KeyStrike::Backslash => "\\",
            KeyStrike::BracketLeft => "[",
            KeyStrike::BracketRight => "]",
            KeyStrike::Comma => ",",
            KeyStrike::Digit0 => "0",
            KeyStrike::Digit1 => "1",
            KeyStrike::Digit2 => "2",
            KeyStrike::Digit3 => "3",
            KeyStrike::Digit4 => "4",
            KeyStrike::Digit5 => "5",
            KeyStrike::Digit6 => "6",
            KeyStrike::Digit7 => "7",
            KeyStrike::Digit8 => "8",
            KeyStrike::Digit9 => "9",
            KeyStrike::Equal => "=",
            KeyStrike::KeyA => "A",
            KeyStrike::KeyB => "B",
            KeyStrike::KeyC => "C",
            KeyStrike::KeyD => "D",
            KeyStrike::KeyE => "E",
            KeyStrike::KeyF => "F",
            KeyStrike::KeyG => "G",
            KeyStrike::KeyH => "H",
            KeyStrike::KeyI => "I",
            KeyStrike::KeyJ => "J",
            KeyStrike::KeyK => "K",
            KeyStrike::KeyL => "L",
            KeyStrike::KeyM => "M",
            KeyStrike::KeyN => "N",
            KeyStrike::KeyO => "O",
            KeyStrike::KeyP => "P",
            KeyStrike::KeyQ => "Q",
            KeyStrike::KeyR => "R",
            KeyStrike::KeyS => "S",
            KeyStrike::KeyT => "T",
            KeyStrike::KeyU => "U",
            KeyStrike::KeyV => "V",
            KeyStrike::KeyW => "W",
            KeyStrike::KeyX => "X",
            KeyStrike::KeyY => "Y",
            KeyStrike::KeyZ => "Z",
            KeyStrike::Minus => "-",
            KeyStrike::Period => ".",
            KeyStrike::Quote => "'",
            KeyStrike::Semicolon => ";",
            KeyStrike::Slash => "/",
            KeyStrike::Backspace => "<Backspace>",
            KeyStrike::Enter => "<Enter>",
            KeyStrike::Space => "<Space>",
            KeyStrike::Tab => "<Tab>",
            KeyStrike::Delete => "<Del>",
            KeyStrike::End => "<End>",
            KeyStrike::Home => "<Home>",
            KeyStrike::Insert => "<Insert>",
            KeyStrike::PageDown => "<PgDn>",
            KeyStrike::PageUp => "<PgUp>",
            KeyStrike::ArrowDown => "↓",
            KeyStrike::ArrowLeft => "←",
            KeyStrike::ArrowRight => "→",
            KeyStrike::ArrowUp => "↑",
            KeyStrike::Escape => "<Esc>",
            KeyStrike::PrintScreen => "<PrintScreen>",
            KeyStrike::ScrollLock => "<ScrlLock>",
            KeyStrike::Pause => "<Pause>",
            KeyStrike::F1 => "<F1>",
            KeyStrike::F2 => "F2",
            KeyStrike::F3 => "F3",
            KeyStrike::F4 => "F4",
            KeyStrike::F5 => "F5",
            KeyStrike::F6 => "F6",
            KeyStrike::F7 => "F7",
            KeyStrike::F8 => "F8",
            KeyStrike::F9 => "F9",
            KeyStrike::F10 => "F10",
            KeyStrike::F11 => "F11",
            KeyStrike::F12 => "F12",
            KeyStrike::F13 => "F13",
            KeyStrike::F14 => "F14",
            KeyStrike::F15 => "F15",
            KeyStrike::F16 => "F16",
            KeyStrike::F17 => "F17",
            KeyStrike::F18 => "F18",
            KeyStrike::F19 => "F19",
            KeyStrike::F20 => "F20",
            KeyStrike::F21 => "F21",
            KeyStrike::F22 => "F22",
            KeyStrike::F23 => "F23",
            KeyStrike::F24 => "F24",
            KeyStrike::F25 => "F25",
            KeyStrike::F26 => "F26",
            KeyStrike::F27 => "F27",
            KeyStrike::F28 => "F28",
            KeyStrike::F29 => "F29",
            KeyStrike::F30 => "F30",
            KeyStrike::F31 => "F31",
            KeyStrike::F32 => "F32",
            KeyStrike::F33 => "F33",
            KeyStrike::F34 => "F34",
            KeyStrike::F35 => "F35",
        };
        write!(f, "{}", text)
    }
}

impl TryFrom<String> for KeyStrike {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = match value.as_str() {
            "a" => KeyStrike::KeyA,
            "b" => KeyStrike::KeyB,
            "c" => KeyStrike::KeyC,
            "d" => KeyStrike::KeyD,
            "e" => KeyStrike::KeyE,
            "f" => KeyStrike::KeyF,
            "g" => KeyStrike::KeyG,
            "h" => KeyStrike::KeyH,
            "i" => KeyStrike::KeyI,
            "j" => KeyStrike::KeyJ,
            "k" => KeyStrike::KeyK,
            "l" => KeyStrike::KeyL,
            "m" => KeyStrike::KeyM,
            "n" => KeyStrike::KeyN,
            "o" => KeyStrike::KeyO,
            "p" => KeyStrike::KeyP,
            "q" => KeyStrike::KeyQ,
            "r" => KeyStrike::KeyR,
            "s" => KeyStrike::KeyS,
            "t" => KeyStrike::KeyT,
            "u" => KeyStrike::KeyU,
            "v" => KeyStrike::KeyV,
            "w" => KeyStrike::KeyW,
            "x" => KeyStrike::KeyX,
            "y" => KeyStrike::KeyY,
            "z" => KeyStrike::KeyZ,
            "0" => KeyStrike::Digit0,
            "1" => KeyStrike::Digit1,
            "2" => KeyStrike::Digit2,
            "3" => KeyStrike::Digit3,
            "4" => KeyStrike::Digit4,
            "5" => KeyStrike::Digit5,
            "6" => KeyStrike::Digit6,
            "7" => KeyStrike::Digit7,
            "8" => KeyStrike::Digit8,
            "9" => KeyStrike::Digit9,
            ";" => KeyStrike::Semicolon,
            "[" => KeyStrike::BracketLeft,
            "]" => KeyStrike::BracketRight,
            "{" => KeyStrike::BracketLeft,
            "}" => KeyStrike::BracketRight,
            "\\" => KeyStrike::Backslash,
            "'" => KeyStrike::Quote,
            "`" => KeyStrike::Backquote,
            "/" => KeyStrike::Slash,
            "-" => KeyStrike::Minus,
            "=" => KeyStrike::Equal,
            "." => KeyStrike::Period,
            "," => KeyStrike::Comma,
            " " => KeyStrike::Space,
            // Additional ones to allow deserialization
            "<Backspace>" => KeyStrike::Backspace,
            "<Enter>" => KeyStrike::Enter,
            "<Space>" => KeyStrike::Space,
            "<Tab>" => KeyStrike::Tab,
            "<Del>" => KeyStrike::Delete,
            "<End>" => KeyStrike::End,
            "<Home>" => KeyStrike::Home,
            "<Insert>" => KeyStrike::Insert,
            "<PgDn>" => KeyStrike::PageDown,
            "<PgUp>" => KeyStrike::PageUp,
            "↓" => KeyStrike::ArrowDown,
            "←" => KeyStrike::ArrowLeft,
            "→" => KeyStrike::ArrowRight,
            "↑" => KeyStrike::ArrowUp,
            "<Esc>" => KeyStrike::Escape,
            "<PrintScreen>" => KeyStrike::PrintScreen,
            "<ScrlLock>" => KeyStrike::ScrollLock,
            "<Pause>" => KeyStrike::Pause,
            "<F1>" => KeyStrike::F1,
            "F2" => KeyStrike::F2,
            "F3" => KeyStrike::F3,
            "F4" => KeyStrike::F4,
            "F5" => KeyStrike::F5,
            "F6" => KeyStrike::F6,
            "F7" => KeyStrike::F7,
            "F8" => KeyStrike::F8,
            "F9" => KeyStrike::F9,
            "F10" => KeyStrike::F10,
            "F11" => KeyStrike::F11,
            "F12" => KeyStrike::F12,
            "F13" => KeyStrike::F13,
            "F14" => KeyStrike::F14,
            "F15" => KeyStrike::F15,
            "F16" => KeyStrike::F16,
            "F17" => KeyStrike::F17,
            "F18" => KeyStrike::F18,
            "F19" => KeyStrike::F19,
            "F20" => KeyStrike::F20,
            "F21" => KeyStrike::F21,
            "F22" => KeyStrike::F22,
            "F23" => KeyStrike::F23,
            "F24" => KeyStrike::F24,
            "F25" => KeyStrike::F25,
            "F26" => KeyStrike::F26,
            "F27" => KeyStrike::F27,
            "F28" => KeyStrike::F28,
            "F29" => KeyStrike::F29,
            "F30" => KeyStrike::F30,
            "F31" => KeyStrike::F31,
            "F32" => KeyStrike::F32,
            "F33" => KeyStrike::F33,
            "F34" => KeyStrike::F34,
            "F35" => KeyStrike::F35,
            // Capital letters because serialization
            "A" => KeyStrike::KeyA,
            "B" => KeyStrike::KeyB,
            "C" => KeyStrike::KeyC,
            "D" => KeyStrike::KeyD,
            "E" => KeyStrike::KeyE,
            "F" => KeyStrike::KeyF,
            "G" => KeyStrike::KeyG,
            "H" => KeyStrike::KeyH,
            "I" => KeyStrike::KeyI,
            "J" => KeyStrike::KeyJ,
            "K" => KeyStrike::KeyK,
            "L" => KeyStrike::KeyL,
            "M" => KeyStrike::KeyM,
            "N" => KeyStrike::KeyN,
            "O" => KeyStrike::KeyO,
            "P" => KeyStrike::KeyP,
            "Q" => KeyStrike::KeyQ,
            "R" => KeyStrike::KeyR,
            "S" => KeyStrike::KeyS,
            "T" => KeyStrike::KeyT,
            "U" => KeyStrike::KeyU,
            "V" => KeyStrike::KeyV,
            "W" => KeyStrike::KeyW,
            "X" => KeyStrike::KeyX,
            "Y" => KeyStrike::KeyY,
            "Z" => KeyStrike::KeyZ,
            _ => KeyStrike::Unknown,
        };
        Ok(value)
    }
}

impl From<KeyStrike> for String {
    fn from(value: KeyStrike) -> Self {
        value.to_string()
    }
}

impl From<Key> for KeyStrike {
    fn from(value: Key) -> Self {
        match value {
            Key::Character(char) => {
                let ks = char.clone().try_into().unwrap_or(KeyStrike::Unknown);
                if ks == KeyStrike::Unknown {
                    error!("Didn't find a key for {}", char);
                }
                ks
            }
            Key::Enter => KeyStrike::Enter,
            Key::Tab => KeyStrike::Tab,
            Key::ArrowDown => KeyStrike::ArrowDown,
            Key::ArrowLeft => KeyStrike::ArrowLeft,
            Key::ArrowRight => KeyStrike::ArrowRight,
            Key::ArrowUp => KeyStrike::ArrowUp,
            Key::End => KeyStrike::End,
            Key::Home => KeyStrike::Home,
            Key::PageDown => KeyStrike::PageDown,
            Key::PageUp => KeyStrike::PageUp,
            Key::Backspace => KeyStrike::Backspace,
            Key::Delete => KeyStrike::Delete,
            Key::Insert => KeyStrike::Insert,
            Key::Escape => KeyStrike::Escape,
            Key::Pause => KeyStrike::Pause,
            Key::PrintScreen => KeyStrike::PrintScreen,
            Key::ScrollLock => KeyStrike::ScrollLock,
            Key::F1 => KeyStrike::F1,
            Key::F2 => KeyStrike::F2,
            Key::F3 => KeyStrike::F3,
            Key::F4 => KeyStrike::F4,
            Key::F5 => KeyStrike::F5,
            Key::F6 => KeyStrike::F6,
            Key::F7 => KeyStrike::F7,
            Key::F8 => KeyStrike::F8,
            Key::F9 => KeyStrike::F9,
            Key::F10 => KeyStrike::F10,
            Key::F11 => KeyStrike::F11,
            Key::F12 => KeyStrike::F12,
            Key::F13 => KeyStrike::F13,
            Key::F14 => KeyStrike::F14,
            Key::F15 => KeyStrike::F15,
            Key::F16 => KeyStrike::F16,
            Key::F17 => KeyStrike::F17,
            Key::F18 => KeyStrike::F18,
            Key::F19 => KeyStrike::F19,
            Key::F20 => KeyStrike::F20,
            Key::F21 => KeyStrike::F21,
            Key::F22 => KeyStrike::F22,
            Key::F23 => KeyStrike::F23,
            Key::F24 => KeyStrike::F24,
            Key::F25 => KeyStrike::F25,
            Key::F26 => KeyStrike::F26,
            Key::F27 => KeyStrike::F27,
            Key::F28 => KeyStrike::F28,
            Key::F29 => KeyStrike::F29,
            Key::F30 => KeyStrike::F30,
            Key::F31 => KeyStrike::F31,
            Key::F32 => KeyStrike::F32,
            Key::F33 => KeyStrike::F33,
            Key::F34 => KeyStrike::F34,
            Key::F35 => KeyStrike::F35,
            _ => KeyStrike::None,
        }
    }
}
