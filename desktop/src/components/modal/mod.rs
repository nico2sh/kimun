use std::sync::Arc;

use dioxus::prelude::*;
use indexer::Indexer;
use kimun_core::{nfs::VaultPath, NoteVault};
use selector::{note_search::NoteSearch, note_select::NoteSelector};

use crate::components::modal::indexer::IndexType;

pub mod indexer;
mod selector;

#[derive(Clone, Debug, PartialEq)]
enum ModalType {
    None,
    NoteSelector {
        vault: Arc<NoteVault>,
        note_path: SyncSignal<Option<VaultPath>>,
    },
    NoteSearch {
        vault: Arc<NoteVault>,
        note_path: SyncSignal<Option<VaultPath>>,
    },
    Index {
        vault: Arc<NoteVault>,
        index_type: IndexType,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modal {
    modal_type: ModalType,
}

impl Modal {
    pub fn new() -> Self {
        Self {
            modal_type: ModalType::None,
        }
    }
    pub fn is_open(&self) -> bool {
        !matches!(self.modal_type, ModalType::None)
    }
    pub fn close(&mut self) {
        self.modal_type = ModalType::None;
    }
    pub fn set_note_select(
        &mut self,
        vault: Arc<NoteVault>,
        note_path: SyncSignal<Option<VaultPath>>,
    ) {
        self.modal_type = ModalType::NoteSelector { vault, note_path };
    }
    pub fn set_note_search(
        &mut self,
        vault: Arc<NoteVault>,
        note_path: SyncSignal<Option<VaultPath>>,
    ) {
        self.modal_type = ModalType::NoteSearch { vault, note_path };
    }
    pub fn set_indexer(&mut self, vault: Arc<NoteVault>, index_type: IndexType) {
        self.modal_type = ModalType::Index { vault, index_type };
    }
    pub fn get_element(modal: Signal<Self>) -> Element {
        match &modal.read().modal_type {
            ModalType::None => rsx! {},
            ModalType::NoteSelector { vault, note_path } => rsx! {
                div { class: "dialog",
                    NoteSelector {
                        modal,
                        vault: vault.clone(),
                        note_path: note_path.clone(),
                        filter_text: "".to_string(),
                    }
                }
            },
            ModalType::NoteSearch { vault, note_path } => rsx! {
                div { class: "dialog",
                    NoteSearch {
                        modal,
                        vault: vault.clone(),
                        note_path: note_path.clone(),
                        filter_text: "".to_string(),
                    }
                }
            },
            ModalType::Index { vault, index_type } => rsx! {
                div { class: "dialog",
                    Indexer {
                        modal,
                        vault: vault.clone(),
                        index_type: index_type.clone(),
                    }
                }
            },
        }
    }
}
