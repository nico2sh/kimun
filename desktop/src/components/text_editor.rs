use std::rc::Rc;

use crate::pages::editor::EditorMsg;
use dioxus::{
    logger::tracing::{error, info},
    prelude::*,
};

#[component]
pub fn EditorHeader(note_path_display: String, is_dirty: Signal<bool>) -> Element {
    rsx! {
        div { class: "editor-header",
            div { class: "title-section",
                div { class: "title-text", "{note_path_display}" }
                div {
                    class: if !is_dirty() { "status-indicator" } else { "status-indicator unsaved" },
                    id: "saveStatus",
                }
            }
        }
    }
}

#[component]
pub fn TextEditor(
    content: String,
    editor_signal: Signal<Option<Rc<MountedData>>>,
    disabled: bool,
    cr: Coroutine<EditorMsg>,
) -> Element {
    info!("Refreshing editor");
    // This manages the editor state
    rsx! {
        div { class: "editor-content",
            textarea {
                class: "text-editor",
                id: "textEditor",
                autofocus: true,
                onmounted: move |e| {
                    *editor_signal.write() = Some(e.data());
                },
                oninput: move |e| {
                    cr.send(EditorMsg::Update {
                        text: e.value(),
                    });
                },
                onkeydown: move |e| {
                    if disabled {
                        e.prevent_default();
                    } else {
                        match e.key() {
                            Key::Tab => {
                                e.prevent_default();
                            }
                            _ => {}
                        }
                    }
                },
                spellcheck: false,
                wrap: "hard",
                resize: "none",
                placeholder: if disabled { "Create or select a note" } else { "Start writing something!" },
                value: "{content}",
            }
        }
    }
}
