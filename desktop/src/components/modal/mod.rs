use std::sync::Arc;

use dioxus::{html::div, logger::tracing::debug, prelude::*};
use indexer::Indexer;
use kimun_core::{nfs::VaultPath, NoteVault};
use selector::{note_search::NoteSearch, note_select::NoteSelector};

use crate::components::modal::{
    confirmations::{ConfirmationType, DeleteConfirm},
    indexer::IndexType,
};

pub mod confirmations;
pub mod indexer;
mod selector;

#[derive(Clone, Debug, PartialEq)]
enum ModalType {
    None,
    NoteSelector {
        vault: Arc<NoteVault>,
        from_path: VaultPath,
    },
    NoteSearch {
        vault: Arc<NoteVault>,
    },
    Index {
        vault: Arc<NoteVault>,
        index_type: IndexType,
    },
    DeleteNote {
        vault: Arc<NoteVault>,
        path: VaultPath,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModalManager {
    modal_type: ModalType,
}

impl ModalManager {
    pub fn new() -> Self {
        Self {
            modal_type: ModalType::None,
        }
    }
    pub fn is_open(&self) -> bool {
        !matches!(self.modal_type, ModalType::None)
    }
    pub fn close(&mut self) {
        debug!("[Modal] Closing Modal");
        self.modal_type = ModalType::None;
    }
    pub fn set_note_select(&mut self, vault: Arc<NoteVault>, note_path: VaultPath) {
        self.modal_type = ModalType::NoteSelector {
            vault,
            from_path: note_path,
        };
    }
    pub fn set_note_search(&mut self, vault: Arc<NoteVault>) {
        self.modal_type = ModalType::NoteSearch { vault };
    }
    pub fn set_indexer(&mut self, vault: Arc<NoteVault>, index_type: IndexType) {
        debug!("[Modal] Set Modal Indexer");
        self.modal_type = ModalType::Index { vault, index_type };
    }
    pub fn set_confirm(&mut self, vault: Arc<NoteVault>, confirmation: ConfirmationType) {
        match confirmation {
            ConfirmationType::Delete(vault_path) => {
                self.modal_type = ModalType::DeleteNote {
                    vault,
                    path: vault_path,
                }
            }
            ConfirmationType::Move(vault_path, vault_path1) => todo!(),
            ConfirmationType::Rename(vault_path, _) => todo!(),
        }
    }

    pub fn get_element(modal: Signal<Self>) -> Element {
        match &modal.read().modal_type {
            ModalType::None => rsx! {},
            ModalType::NoteSelector { vault, from_path } => rsx! {
                div { class: "modal-overlay",
                    NoteSelector {
                        modal,
                        vault: vault.clone(),
                        note_path: from_path.clone(),
                        filter_text: "".to_string(),
                    }
                }
            },
            ModalType::NoteSearch { vault } => rsx! {
                div { class: "modal-overlay",
                    NoteSearch {
                        modal,
                        vault: vault.clone(),
                        filter_text: "".to_string(),
                    }
                }
            },
            ModalType::Index { vault, index_type } => rsx! {
                div { class: "modal-overlay",
                    Indexer {
                        modal,
                        vault: vault.clone(),
                        index_type: index_type.clone(),
                    }
                }
            },
            ModalType::DeleteNote { vault, path } => {
                rsx! {
                    div { class: "modal-overlay",
                        DeleteConfirm {
                            modal,
                            vault: vault.clone(),
                            path: path.clone(),
                        }
                    }
                }
            }
        }
    }
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
fn ModalButton(text: String, button_type: ButtonType, onclick: fn(Event<MouseData>)) -> Element {
    rsx! {
        button { class: "{button_type.get_class()}", onclick, "{text}" }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum ButtonType {
    Primary,
    Secondary,
    Danger,
}

impl ButtonType {
    fn get_class(&self) -> String {
        match self {
            ButtonType::Primary => "modal-btn-primary",
            ButtonType::Secondary => "modal-btn-secondary",
            ButtonType::Danger => "modal-btn-danger",
        }
        .to_string()
    }
}
