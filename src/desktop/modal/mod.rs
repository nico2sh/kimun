use dioxus::prelude::*;
use selector::note_search::NoteSelector;

use crate::noters::nfs::NotePath;

mod selector;

#[derive(Clone, Debug, PartialEq)]
enum ModalType {
    None,
    NoteBrowser,
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
    pub fn set_note_search(&mut self) {
        self.modal_type = ModalType::NoteBrowser;
    }
    pub fn get_element(modal: Signal<Self>, note_path: Signal<Option<NotePath>>) -> Element {
        match &modal.read().modal_type {
            ModalType::None => rsx! {},
            ModalType::NoteBrowser => rsx! {
                NoteSelector {
                    modal,
                    note_path,
                    filter_text: "".to_string(),
                }
            },
        }
    }
}
