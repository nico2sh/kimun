use dioxus::prelude::*;

#[derive(Debug)]
pub enum Shortcuts {
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

pub fn key_action<K: AsRef<KeyboardData>>(kd: &K, action: Shortcuts) -> bool {
    let kd = kd.as_ref();
    let code = kd.code();
    match action {
        Shortcuts::OpenSettings => meta_ctrl(kd) && code == Code::Comma,
        Shortcuts::ToggleNoteBrowser => meta_ctrl(kd) && code == Code::Slash,
        Shortcuts::SearchNotes => meta_ctrl(kd) && code == Code::KeyK,
        Shortcuts::OpenNote => meta_ctrl(kd) && code == Code::KeyO,
        Shortcuts::NewJournal => meta_ctrl(kd) && code == Code::KeyJ,
    }
}
