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
                    class: "sidebar-toggle-main",
                    onclick: move |_| {
                        app_state.write().toggle_browser();
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
                if let ContentType::Note { dirty } = editor_state.read().content_type {
                    div {
                        class: if !dirty { "status-indicator" } else { "status-indicator unsaved" },
                        id: "saveStatus",
                    }
                }
            }
        }
    }
}
