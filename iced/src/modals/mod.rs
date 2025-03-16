mod vault_browse;
mod vault_indexer;

use std::path::PathBuf;

use iced::Task;
use kimun_core::{
    NoteVault,
    nfs::{NoteEntryData, VaultPath},
    note::NoteContentData,
};
use vault_browse::VaultBrowse;
use vault_indexer::IndexType;

use crate::KimunMessage;

pub struct ModalManager {
    pub current_modal: Option<Box<dyn KimunModal>>,
}

impl ModalManager {
    pub fn new() -> Self {
        Self {
            current_modal: None,
        }
    }

    pub fn set_modal(&mut self, modal: Modals) -> Task<KimunMessage> {
        let (modal, task) = match modal {
            Modals::VaultBrowse(note_vault, vault_path) => {
                // Filtered list
                VaultBrowse::new(vault_path, note_vault)
            }
            Modals::VaultSearch(note_vault) => todo!(),
            Modals::NoteSelect(note_vault, items) => todo!(),
            Modals::VaultIndex(path_buf, index_type) => todo!(),
        };
        self.current_modal = Some(Box::new(modal));
        task
    }

    pub fn close_modal(&mut self) -> Task<KimunMessage> {
        self.current_modal = None;
        Task::none()
    }
}

pub trait KimunModal {
    fn view(&self) -> iced::Element<KimunMessage>;
    fn get_width(&self) -> iced::Length;
    fn get_height(&self) -> iced::Length;
    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage>;
    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage>;
}

#[derive(Debug, Clone)]
pub enum Modals {
    VaultBrowse(NoteVault, VaultPath),
    VaultSearch(NoteVault),
    NoteSelect(NoteVault, Vec<(NoteEntryData, NoteContentData)>),
    VaultIndex(PathBuf, IndexType),
}

#[derive(Debug, Clone)]
pub enum ModalMessage {}
