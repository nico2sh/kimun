mod note_select;
mod vault_browse;
pub mod vault_indexer;

use std::path::PathBuf;

use iced::Task;
use kimun_core::{
    NoteVault,
    nfs::{NoteEntryData, VaultPath},
    note::NoteContentData,
};
use note_select::NoteSelect;
use vault_browse::{VaultBrowseFunctions, VaultNavigator, VaultSearchFunctions};
use vault_indexer::{IndexType, VaultIndexer};

use crate::{ErrorMsg, KimunMessage};

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
        match modal {
            Modals::VaultBrowse(note_vault, vault_path) => {
                // Filtered list
                let (modal, task) = VaultNavigator::new(
                    note_vault.clone(),
                    VaultBrowseFunctions::new(vault_path, note_vault.clone()),
                );
                self.current_modal = Some(Box::new(modal));
                task
            }
            Modals::VaultSearch(note_vault) => {
                // Filtered list
                let (modal, task) = VaultNavigator::new(
                    note_vault.clone(),
                    VaultSearchFunctions::new(note_vault.clone()),
                );
                self.current_modal = Some(Box::new(modal));
                task
            }
            Modals::NoteSelect(items) => {
                let mut modal = NoteSelect::new();
                modal.set_elements(items.into_iter().map(|i| i.into()).collect());
                self.current_modal = Some(Box::new(modal));
                Task::none()
            }
            Modals::VaultIndex(path_buf, index_type) => match NoteVault::new(path_buf) {
                Ok(vault) => {
                    let (modal, task) = VaultIndexer::new(vault, index_type);
                    self.current_modal = Some(Box::new(modal));
                    task
                }
                Err(e) => Task::done(KimunMessage::Error(ErrorMsg::Add(e.to_string()))),
            },
        }
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
    fn should_close_on_click(&self) -> bool;
}

#[derive(Debug, Clone)]
pub enum Modals {
    VaultBrowse(NoteVault, VaultPath),
    VaultSearch(NoteVault),
    NoteSelect(Vec<(NoteEntryData, NoteContentData)>),
    VaultIndex(PathBuf, IndexType),
}
