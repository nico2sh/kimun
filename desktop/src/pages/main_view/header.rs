use dioxus::prelude::*;
// use dioxus_radio::hooks::use_radio;
use kimun_core::nfs::VaultPath;

use crate::{
    app_state::AppState,
    editor_state::{ContentType, EditorState},
};

#[derive(Clone, PartialEq, Props)]
pub struct EditorHeaderProps {
    path: ReadSignal<VaultPath>,
}

#[component]
pub fn EditorHeader(props: EditorHeaderProps) -> Element {
    let mut app_state: Signal<AppState> = use_context();

    let note_path_display = props.path.read().to_string();
    // let app_state = use_radio::<AppState, KimunChannel>(KimunChannel::Header);
    let editor_state: Signal<EditorState> = use_context();
    rsx! {
        div { class: "editor-header",
            div { class: "header-left",
                button {
                    class: "header-button",
                    onclick: move |_| {
                        app_state.write().toggle_browser();
                    },

                    svg {
                        class: "icon-header",
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
                if let ContentType::Note { dirty } = editor_state.read().content_type {
                    div {
                        class: if !dirty { "status-indicator" } else { "status-indicator unsaved" },
                        id: "saveStatus",
                    }
                }
                button { class: if app_state.read().show_preview_pane.is_none() { "header-button" } else { "header-button active" },
                    svg {
                        class: "icon-header",
                        onclick: move |_| {
                            if app_state.read().show_preview_pane.is_some() {
                                app_state.write().hide_preview_pane();
                            } else {
                                app_state.write().show_preview_pane(None);
                            }
                        },
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.747 0 3.332.477 4.5 1.253v13C19.832 18.477 18.247 18 16.5 18c-1.746 0-3.332.477-4.5 1.253",
                        }
                    }
                }
            }
        }
    }
}
