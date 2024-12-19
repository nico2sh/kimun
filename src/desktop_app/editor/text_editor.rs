use std::{
    cell::RefCell,
    fmt::{Display, Formatter},
    rc::Rc,
};

use dioxus::prelude::*;
use dioxus_logger::tracing::{debug, info};

use crate::{
    core_notes::{nfs::NotePath, NoteVault},
    desktop_app::AppContext,
};

#[derive(Props, Clone, PartialEq)]
pub struct TextEditorProps {
    note_path: SyncSignal<Option<NotePath>>,
    editor_signal: Signal<Option<Rc<MountedData>>>,
}

#[allow(non_snake_case)]
pub fn TextEditor(props: TextEditorProps) -> Element {
    // to recover the focus
    let mut editor = props.editor_signal;

    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;
    let note_path = props.note_path;

    let content_edit = use_resource(move || {
        let vault = vault.clone();
        async move {
            info!("Loading Content");
            let content_edit = ContentEdit::new(&vault, note_path.read().to_owned());
            content_edit
        }
    });
    let disabled = content_edit
        .read()
        .as_ref()
        .map(|c| !c.is_enabled())
        .unwrap_or_default();

    rsx! {
        textarea {
            class: "edittext",
            onmounted: move |e| {
                *editor.write() = Some(e.data());
            },
            oninput: move |e| {
                if let Some(content) = &*content_edit.read() { content.update_content(e.value()) }
            },
            spellcheck: false,
            wrap: "hard",
            resize: "none",
            placeholder: if disabled
                { "Create or select a note" }
                    else
                { "Start writing something!" },
            disabled: disabled,
            value: match &*content_edit.read() {
                Some(content) => content.get_content(),
                None => "".to_string()
            }
        }
    }
}

#[derive(Debug, PartialEq)]
struct ContentEdit {
    content: RefCell<String>,
    has_changed: bool,
    vault: NoteVault,
    path: Option<NotePath>,
}

impl Display for ContentEdit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content.borrow().to_owned())
    }
}

impl ContentEdit {
    fn new(vault: &NoteVault, path: Option<NotePath>) -> Self {
        let content = match &path {
            Some(path) => vault.load_note(path).unwrap_or_else(|_e| "".to_string()),
            None => "".to_string(),
        };
        Self {
            content: RefCell::new(content),
            has_changed: false,
            vault: vault.clone(),
            path,
        }
    }

    fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    fn save(&mut self) {
        if self.has_changed {
            if let Some(path) = self.path.clone() {
                self.has_changed = false;
                debug!("=================");
                debug!("About to Save");
                let vault = self.vault.clone();
                vault.save_note(path, self.get_content());
                debug!("Content Saved:\n{}", self.get_content());
                debug!("=================");
            }
        }
    }

    fn get_content(&self) -> String {
        self.content.borrow().to_owned()
    }

    // async fn save_async(&mut self) {
    //     if self.has_changed {
    //         if let Some(path) = self.path.clone() {
    //             self.has_changed = false;
    //             debug!("=================");
    //             debug!("About to Save");
    //             let vault = self.vault.clone();
    //             let path = path.clone();
    //             let content = self.content.clone();
    //             vault.save_note(path, content);
    //             debug!("Content Saved:\n{}", self.content);
    //             debug!("=================");
    //         }
    //     }
    // }

    // fn replace_content<S: AsRef<str>>(&mut self, content: S, path: Option<NotePath>) {
    //     // self.save();
    //     self.content = content.as_ref().to_owned();
    //     self.path = path;
    // }

    fn update_content(&self, content: String) {
        // debug!("=================");
        // debug!("Updating content:\n{}", content);
        // debug!("=================");
        *self.content.borrow_mut() = content;
        // self.has_changed = true;
    }
}

impl Drop for ContentEdit {
    fn drop(&mut self) {
        info!("SAVE ME: {}", self.get_content());
    }
}
