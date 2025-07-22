use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::components::modal::ModalManager;

// General Modal
#[component]
fn BasicModal(title: String, subtitle: String, body: Element, actions: Element) -> Element {
    rsx! {
        div { class: "modal",
            div { class: "modal-header",
                div { class: "modal-title", {title} }
                div { class: "modal-subtitle", {subtitle} }
            }
            div { class: "modal-body", {body} }
            div { class: "modal-actions", {actions} }
        }
    }
}

#[component]
pub fn DeleteConfirm(
    modal: Signal<ModalManager>,
    vault: Arc<NoteVault>,
    path: VaultPath,
) -> Element {
    rsx! {
        div { class: "modal",
            div { class: "modal-header",
                div { class: "modal-title", "Delete Note" }
                div { class: "modal-subtitle", "Are you sure you want to delete \"{path}\"?" }
            }
            div { class: "modal-body", "This action cannot be undone." }
            div { class: "modal-actions",
                button {
                    class: "modal-btn secondary",
                    onclick: move |_| {
                        modal.write().close();
                    },
                    "Cancel"
                }
                button {
                    class: "modal-btn danger",
                    onclick: move |_| {
                        modal.write().close();
                    },
                    "Cancel"
                }
            }
        }
    }
}
