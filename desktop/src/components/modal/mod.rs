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
pub enum ModalType {
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

impl ModalType {
    pub fn close(&mut self) {
        *self = ModalType::None;
    }
    pub fn set_error(&mut self, message: String, error: String) {
        *self = ModalType::Error { message, error };
    }
    pub fn set_note_select(&mut self, vault: Arc<NoteVault>, note_path: VaultPath) {
        *self = ModalType::NoteSelector {
            vault,
            from_path: note_path,
        };
    }
    pub fn set_note_search(&mut self, vault: Arc<NoteVault>) {
        *self = ModalType::NoteSearch { vault };
    }
    pub fn set_indexer(&mut self, vault: Arc<NoteVault>, index_type: IndexType) {
        debug!("[Modal] Set Modal Indexer");
        *self = ModalType::Index { vault, index_type };
    }
    pub fn set_confirm(&mut self, vault: Arc<NoteVault>, confirmation: ConfirmationType) {
        *self = match confirmation {
            ConfirmationType::Delete(vault_path) => ModalType::DeleteNote {
                vault,
                path: vault_path,
            },
            ConfirmationType::Move(from_path) => ModalType::MoveNote { vault, from_path },
            ConfirmationType::Rename(path) => ModalType::RenameNote { vault, path },
        }
    }
    pub fn is_open(&self) -> bool {
        match self {
            ModalType::None => false,
            _ => true,
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ModalProps {
    modal_type: Signal<ModalType>,
}

#[component]
pub fn Modal(props: ModalProps) -> Element {
    let modal_type = props.modal_type;
    let mt = &*modal_type.read();

    if let ModalType::None = mt {
        return rsx! {};
    }
    rsx! {
        div { class: "modal-overlay",
            match &*modal_type.read() {
                ModalType::None => rsx! {},
                ModalType::Error { message, error } => rsx! {
                    Error { modal_type, message, error }
                },
                ModalType::NoteSelector { vault, from_path } => {
                    rsx! {
                        div { class: "modal-overlay",
                            NoteSelector {
                                modal_type,
                                vault: vault.clone(),
                                note_path: from_path.clone(),
                                filter_text: "".to_string(),
                            }
                        }
                    }
                }
                ModalType::NoteSearch { vault } => rsx! {
                    div { class: "modal-overlay",
                        NoteSearch { modal_type, vault: vault.clone(), filter_text: "".to_string() }
                    }
                },
                ModalType::Index { vault, index_type } => rsx! {
                    div { class: "modal-overlay",
                        Indexer {
                            modal_type,
                            vault: vault.clone(),
                            index_type: index_type.clone(),
                        }
                    }
                },
                ModalType::DeleteNote { vault, path } => {
                    rsx! {
                        div { class: "modal-overlay",
                            DeleteConfirm { modal_type, vault: vault.clone(), path: path.clone() }
                        }
                    }
                }
                ModalType::MoveNote { vault, from_path } => {
                    rsx! {
                        div { class: "modal-overlay",
                            MoveConfirm { modal_type, vault: vault.clone(), from_path: from_path.clone() }
                        }
                    }
                }
                ModalType::RenameNote { vault, path } => rsx! {
                    div { class: "modal-overlay",
                        RenameConfirm { modal_type, vault: vault.clone(), path: path.clone() }
                    }
                },
            }
        }
    }
}
