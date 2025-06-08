use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, NoteVault};
use selector::{note_search::NoteSearch, note_select::NoteSelector};

mod selector;

#[derive(Clone, Debug, PartialEq)]
enum ModalType {
    None,
    NoteSelector,
    NoteSearch,
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
    pub fn set_note_select(&mut self) {
        self.modal_type = ModalType::NoteSelector;
    }
    pub fn set_note_search(&mut self) {
        self.modal_type = ModalType::NoteSearch;
    }
    pub fn get_element(
        modal: Signal<Self>,
        vault: Arc<NoteVault>,
        note_path: SyncSignal<Option<VaultPath>>,
    ) -> Element {
        match &modal.read().modal_type {
            ModalType::None => rsx! {},
            ModalType::NoteSelector => rsx! {
                NoteSelector {
                    modal,
                    vault,
                    note_path,
                    filter_text: "".to_string(),
                }
            },
            ModalType::NoteSearch => rsx! {
                NoteSearch {
                    modal,
                    vault,
                    note_path,
                    filter_text: "".to_string(),
                }
            },
        }
    }
}
