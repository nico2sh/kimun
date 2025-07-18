use std::rc::Rc;

use crate::pages::editor::{EditorContent, EditorMsg};
use dioxus::{logger::tracing::info, prelude::*};

#[component]
pub fn EditorHeader(
    note_path_display: String,
    show_browser: Signal<bool>,
    is_dirty: Signal<bool>,
) -> Element {
    rsx! {
        div { class: "editor-header",
            div { class: "header-left",
                button {
                    class: "sidebar-toggle-main",
                    onclick: move |_| {
                        let showing = *show_browser.read();
                        show_browser.set(!showing);
                    },
                    svg {
                        width: 20,
                        height: 20,
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        line {
                            x1: 3,
                            y1: 6,
                            x2: 21,
                            y2: 6,
                        }
                        line {
                            x1: 3,
                            y1: 12,
                            x2: 21,
                            y2: 12,
                        }
                        line {
                            x1: 3,
                            y1: 18,
                            x2: 21,
                            y2: 18,
                        }
                    }
                }
            }
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
