use std::rc::Rc;

use crate::pages::editor::{EditorContent, EditorMsg};
use dioxus::{logger::tracing::info, prelude::*};

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
    content: EditorContent,
    // This is to attach the signal to set the focus
    editor_signal: Signal<Option<Rc<MountedData>>>,
    cr: Coroutine<EditorMsg>,
) -> Element {
    info!("Refreshing editor");
    // This manages the editor state
    rsx! {
        div { class: "editor-content",
            {
                match content {
                    EditorContent::Loading => rsx! {
                        div {
                            onmounted: move |e| {
                                *editor_signal.write() = Some(e.data());
                            },
                            "Loading..."
                        }
                    },
                    EditorContent::Enabled { content } => rsx! {
                        textarea {
                            class: "text-editor",
                            id: "textEditor",
                            autofocus: true,
                            onmounted: move |e| {
                                *editor_signal.write() = Some(e.data());
                            },
                            onselect: move |e| {
                                info!("Select event {:?}", e);
                            },
                            oninput: move |e| {
                                cr.send(EditorMsg::Update {
                                    text: e.value(),
                                });
                            },
                            onkeydown: move |e| {
                                match e.key() {
                                    Key::Tab => {
                                        e.prevent_default();
                                    }
                                    _ => {}
                                }
                            },
                            spellcheck: false,
                            wrap: "hard",
                            resize: "none",
                            placeholder: "Start writing something!",
                            value: "{content}",
                        }
                    },
                    EditorContent::Disabled => rsx! {
                        div {
                            onmounted: move |e| {
                                *editor_signal.write() = Some(e.data());
                            },
                            "No note in here...\nCreate or select a note"
                        }
                    },
                }
            }
        }
    }
}
