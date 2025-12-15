use dioxus::prelude::*;

#[component]
pub fn PreviewPane() -> Element {
    rsx! {
        div { class: "bar-preview-header",
            button { class: "bar-preview-toggle",
                span { "Preview" }
                span { "▼" }
            }
        }
        div { class: "bar-preview-browser" }
    }
}
