use std::sync::Arc;

use dioxus::{logger::tracing::debug, prelude::*};
use indexer::Indexer;
use kimun_core::{nfs::VaultPath, NoteVault};
use selector::{note_search::NoteSearch, note_select::NoteSelector};

use crate::components::modal::{
    confirmations::{ConfirmationType, DeleteConfirm, Error, MoveConfirm, RenameConfirm},
    indexer::IndexType,
};

pub mod confirmations;
pub mod indexer;
mod selector;

#[derive(Clone, Debug, PartialEq)]
enum ModalType {
    None,
    Error {
        message: String,
        error: String,
    },
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
    MoveNote {
        vault: Arc<NoteVault>,
        from_path: VaultPath,
    },
    RenameNote {
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
    pub fn set_error(&mut self, message: String, error: String) {
        self.modal_type = ModalType::Error { message, error };
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
        self.modal_type = match confirmation {
            ConfirmationType::Delete(vault_path) => ModalType::DeleteNote {
                vault,
                path: vault_path,
            },
            ConfirmationType::Move(from_path) => ModalType::MoveNote { vault, from_path },
            ConfirmationType::Rename(path) => ModalType::RenameNote { vault, path },
        }
    }

    pub fn get_element(modal: Signal<Self>) -> Element {
        match &modal.read().modal_type {
            ModalType::None => rsx! {},
            ModalType::Error { message, error } => rsx! {
                div { class: "modal-overlay",
                    Error { modal, message, error }
                }
            },
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
            ModalType::MoveNote { vault, from_path } => {
                rsx! {
                    div { class: "modal-overlay",
                        MoveConfirm {
                            modal,
                            vault: vault.clone(),
                            from_path: from_path.clone(),
                        }
                    }
                }
            }
            ModalType::RenameNote { vault, path } => rsx! {
                div { class: "modal-overlay",
                    RenameConfirm { modal, vault: vault.clone(), path: path.clone() }
                }
            },
        }
    }
}
