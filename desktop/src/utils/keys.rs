use dioxus::prelude::*;

#[derive(Debug, PartialEq, Eq)]
pub enum Shortcuts {
    None,
    OpenSettings,
    ToggleNoteBrowser,
    SearchNotes,
    OpenNote,
    NewJournal,
    TogglePreview,
    Text(TextAction),
}

#[derive(Debug, PartialEq, Eq)]
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

#[cfg(target_os = "macos")]
pub fn meta_ctrl(ke: &KeyboardData) -> bool {
    ke.modifiers().meta()
}

#[cfg(not(target_os = "macos"))]
pub fn meta_ctrl(ke: &KeyboardData) -> bool {
    ke.modifiers().ctrl()
}

pub fn get_action<K: AsRef<KeyboardData>>(kd: &K) -> Shortcuts {
    let kd = kd.as_ref();
    let code = kd.code();
    if meta_ctrl(kd) {
        match code {
            Code::Comma => Shortcuts::OpenSettings,
            Code::KeyE => Shortcuts::ToggleNoteBrowser,
            Code::KeyK => Shortcuts::SearchNotes,
            Code::KeyO => Shortcuts::OpenNote,
            Code::KeyJ => Shortcuts::NewJournal,
            Code::KeyY => Shortcuts::TogglePreview,
            Code::KeyB => Shortcuts::Text(TextAction::Bold),
            Code::KeyI => Shortcuts::Text(TextAction::Italic),
            Code::KeyU => Shortcuts::Text(TextAction::Underline),
            Code::KeyS => Shortcuts::Text(TextAction::Strikethrough),
            Code::KeyL => Shortcuts::Text(TextAction::Link),
            Code::KeyH => Shortcuts::Text(TextAction::ToggleHeader),
            Code::Digit1 => Shortcuts::Text(TextAction::Header(1)),
            Code::Digit2 => Shortcuts::Text(TextAction::Header(2)),
            Code::Digit3 => Shortcuts::Text(TextAction::Header(3)),
            _ => Shortcuts::None,
        }
    } else {
        Shortcuts::None
    }
}
