use dioxus::prelude::*;
use dioxus_radio::hooks::use_radio;
use kimun_core::nfs::VaultPath;

use crate::state::{AppState, ContentType, KimunChannel};

#[derive(Clone, PartialEq, Props)]
pub struct EditorHeaderProps {
    path: ReadOnlySignal<VaultPath>,
    show_browser: Signal<bool>,
}

#[component]
pub fn EditorHeader(props: EditorHeaderProps) -> Element {
    let note_path_display = props.path.read().to_string();
    let mut show_browser = props.show_browser;
    let app_state = use_radio::<AppState, KimunChannel>(KimunChannel::Header);
    rsx! {
        div { class: "editor-header",
            div { class: "header-left",
                button {
                    class: "sidebar-toggle-main",
                    onclick: move |_| {
                        let showing = *props.show_browser.read();
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
                if let ContentType::Note { dirty } = app_state.read().content_type {
                    div {
                        class: if !dirty { "status-indicator" } else { "status-indicator unsaved" },
                        id: "saveStatus",
                    }
                }
            }
        }
    }
}
