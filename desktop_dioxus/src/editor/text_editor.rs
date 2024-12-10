use core_notes::{nfs::NotePath, NoteVault};
use std::rc::Rc;

use dioxus::prelude::*;

use crate::AppContext;

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
    let content = use_memo(move || {
        note_path.read().as_ref().map_or_else(String::new, |e| {
            vault.load_note(e).unwrap_or_else(|_e| "".to_string())
        })
    });

    // let class = use_signal(|| String::from("content"));
    rsx! {
        // Markdown {
        //     class: class,
        //     content: "{content}"
        // }
        textarea {
            class: "edittext",
            onmounted: move |e| {
                *editor.write() = Some(e.data());
            },
            spellcheck: false,
            wrap: "hard",
            resize: "none",
            placeholder: "Insert your note text",
            value: "{content}",
        }
    }
}
