use dioxus::prelude::*;

#[derive(Debug, PartialEq, Eq)]
pub enum Shortcuts {
    None,
    OpenSettings,
    ToggleNoteBrowser,
    SearchNotes,
    OpenNote,
    NewJournal,
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
            Code::Slash => Shortcuts::ToggleNoteBrowser,
            Code::KeyK => Shortcuts::SearchNotes,
            Code::KeyO => Shortcuts::OpenNote,
            Code::KeyJ => Shortcuts::NewJournal,
            _ => Shortcuts::None,
        }
    } else {
        Shortcuts::None
    }
}
