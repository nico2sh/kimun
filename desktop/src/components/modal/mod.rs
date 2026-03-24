use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, info, warn},
    prelude::*,
};
use indexer::Indexer;
use kimun_core::{nfs::VaultPath, NoteVault};
use selector::{note_picker::NotePicker, note_search::NoteSearch, note_select::NoteSelector};

use crate::{
    app_state::AppState,
    components::modal::{
        confirmations::{
            ConfirmationType, CreateDirectory, CreateNote, DeleteConfirm, Error, MoveConfirm,
            RenameConfirm,
        },
        indexer::IndexType,
    },
};

use super::focus_manager::FocusManager;

pub mod confirmations;
pub mod indexer;
mod selector;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum ModalType {
    #[default]
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
    NewNote {
        vault: Arc<NoteVault>,
        path: VaultPath,
    },
    NewDirectory {
        vault: Arc<NoteVault>,
        path: VaultPath,
    },
    NotePicker {
        note_list: Vec<(String, VaultPath)>,
    },
}

impl ModalType {
    pub fn close(&mut self) {
        let focus_manager = use_context::<FocusManager>();
        focus_manager.focus_prev();
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
            ConfirmationType::NewNote(path) => ModalType::NewNote { vault, path },
            ConfirmationType::NewDirectory(path) => ModalType::NewDirectory { vault, path },
        }
    }
    pub fn is_open(&self) -> bool {
        !matches!(self, ModalType::None)
    }
    pub fn should_close_on_click(&self) -> bool {
        match self {
            ModalType::None => true,
            ModalType::Index {
                vault: _,
                index_type: _,
            } => false,
            _ => true,
        }
    }
}

#[component]
pub fn Modal() -> Element {
    let mut app_state: Signal<AppState> = use_context();

    if let ModalType::None = app_state.read().get_modal() {
        return rsx! {};
    }
    rsx! {
        div {
            class: "modal-overlay",
            // We close any modal if we click on the main UI
            onclick: move |e| {
                e.prevent_default();
                if app_state.read().get_modal().is_open()
                    && app_state.read().get_modal().should_close_on_click()
                {
                    app_state.write().close_modal();
                    info!("Close dialog");
                }
            },
            match app_state.read().get_modal() {
                ModalType::None => {
                    warn!("This shouldn't be called");
                    rsx! {}
                }
                ModalType::Error { message, error } => rsx! {
                    Error { message, error }
                },
                ModalType::NoteSelector { vault, from_path } => {
                    rsx! {
                        NoteSelector {
                            vault: vault.clone(),
                            note_path: from_path.clone(),
                            filter_text: "".to_string(),
                        }
                    }
                }
                ModalType::NoteSearch { vault } => rsx! {
                    NoteSearch { vault: vault.clone(), filter_text: "".to_string() }
                },
                ModalType::Index { vault, index_type } => rsx! {
                    Indexer { vault: vault.clone(), index_type: index_type.clone() }
                },
                ModalType::DeleteNote { vault, path } => {
                    rsx! {
                        DeleteConfirm { vault: vault.clone(), path: path.clone() }
                    }
                }
                ModalType::MoveNote { vault, from_path } => {
                    rsx! {
                        MoveConfirm { vault: vault.clone(), from_path: from_path.clone() }
                    }
                }
                ModalType::RenameNote { vault, path } => {
                    rsx! {
                        RenameConfirm { vault: vault.clone(), path: path.clone() }
                    }
                }
                ModalType::NewNote { vault, path } => rsx! {
                    CreateNote { vault: vault.clone(), from_path: path.clone() }
                },
                ModalType::NewDirectory { vault, path } => rsx! {
                    CreateDirectory { vault: vault.clone(), from_path: path.clone() }
                },
                ModalType::NotePicker { note_list } => rsx! {
                    NotePicker { note_list: note_list.to_owned() }
                },
            }
        }
    }
}
