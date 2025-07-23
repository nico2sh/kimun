use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::components::modal::ModalManager;

pub enum ConfirmationType {
    Delete(VaultPath),
    Move(VaultPath, VaultPath),
    Rename(VaultPath, String),
}

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
                    "Delete"
                }
            }
        }
    }
}

#[component]
pub fn MoveConfirm(
    modal: Signal<ModalManager>,
    vault: Arc<NoteVault>,
    from_path: VaultPath,
    to_path: VaultPath,
) -> Element {
    let dest_path = use_signal(|| to_path);
    rsx! {
        div { class: "modal",
            div { class: "modal-header",
                div { class: "modal-title", "Move Note" }
                div { class: "modal-subtitle", "Moving: \"{from_path}\"?" }
            }
            div { class: "modal-body", "<List of paths>" }
            div { class: "modal-actions",
                button {
                    class: "modal-btn secondary",
                    onclick: move |_| {
                        modal.write().close();
                    },
                    "Cancel"
                }
                button {
                    class: "modal-btn primary",
                    onclick: move |_| {
                        modal.write().close();
                    },
                    "Move"
                }
            }
        }
    }
}

#[component]
pub fn RenameConfirm(
    modal: Signal<ModalManager>,
    vault: Arc<NoteVault>,
    path: VaultPath,
    new_name: String,
) -> Element {
    let new_name = use_signal(|| new_name);
    let current_name = path.get_name();
    rsx! {
        div { class: "modal",
            div { class: "modal-header",
                div { class: "modal-title", "Move Note" }
                div { class: "modal-subtitle", "Current name: \"{current_name}\"?" }
            }
            div { class: "modal-body", "<List of paths>" }
            div { class: "modal-actions",
                button {
                    class: "modal-btn secondary",
                    onclick: move |_| {
                        modal.write().close();
                    },
                    "Cancel"
                }
                button {
                    class: "modal-btn primary",
                    onclick: move |_| {
                        modal.write().close();
                    },
                    "Move"
                }
            }
        }
    }
}
