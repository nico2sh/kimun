pub mod vault_indexer;

use std::path::PathBuf;

use crossbeam_channel::Sender;
use eframe::egui;
use kimun_core::{
    nfs::{NoteEntryData, VaultPath},
    note::NoteContentData,
    NoteVault,
};
use log::{debug, error};
use vault_indexer::{IndexType, VaultIndexer};

use crate::editor::components::{
    note_selector::NoteSelectorFunctions,
    preview_list::PreviewList,
    vault_browse::{VaultBrowseFunctions, VaultSearchFunctions},
};

use super::{editor::components::EditorComponent, editor::EditorMessage};

pub trait KimunModal {
    // Returns true if the modal should close after the update
    fn update(&mut self, ui: &mut egui::Ui) -> bool;
}

struct ComponentModal<C>
where
    C: EditorComponent,
{
    component: C,
    message_sender: Sender<EditorMessage>,
}

impl<C> ComponentModal<C>
where
    C: EditorComponent,
{
    fn new(component: C, sender: Sender<EditorMessage>) -> Self {
        Self {
            component,
            message_sender: sender,
        }
    }
}

impl<C> KimunModal for ComponentModal<C>
where
    C: EditorComponent,
{
    fn update(&mut self, ui: &mut egui::Ui) -> bool {
        let modal = egui::Modal::new(egui::Id::new("Modal")).show(ui.ctx(), |ui| {
            ui.set_width(600.0);
            self.component.update(ui)
        });
        let should_close = modal.should_close();
        if let Some(message) = modal.inner {
            if let Err(e) = self.message_sender.send(message) {
                error!("Error sending an update message from modal {}", e);
            }
        }
        should_close
    }
}

pub struct ModalManager {
    ctx: egui::Context,
    current_modal: Option<Box<dyn KimunModal>>,
}

pub enum Modals {
    VaultBrowse(NoteVault, VaultPath, Sender<EditorMessage>),
    VaultSearch(NoteVault, Sender<EditorMessage>),
    NoteSelect(
        NoteVault,
        Vec<(NoteEntryData, NoteContentData)>,
        Sender<EditorMessage>,
    ),
    VaultIndex(PathBuf, IndexType),
}

impl ModalManager {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            ctx,
            current_modal: None,
        }
    }

    pub fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()> {
        if let Some(current_modal) = self.current_modal.as_mut() {
            let should_close = current_modal.update(ui);
            if should_close {
                self.current_modal = None;
            }
        }
        Ok(())
    }

    pub fn set_modal(&mut self, modal: Modals) -> anyhow::Result<()> {
        let modal: Box<dyn KimunModal> = match modal {
            Modals::VaultBrowse(vault, path, sender) => {
                debug!("show browser");
                let content = PreviewList::new(
                    vault.clone(),
                    VaultBrowseFunctions::new(path.clone(), vault.clone()),
                );
                Box::new(ComponentModal::new(content, sender))
            }
            Modals::VaultSearch(vault, sender) => {
                debug!("show searcher");
                let content =
                    PreviewList::new(vault.clone(), VaultSearchFunctions::new(vault.clone()));
                Box::new(ComponentModal::new(content, sender))
            }
            Modals::NoteSelect(vault, data, sender) => {
                debug!("show note select");
                let content = PreviewList::new(vault.clone(), NoteSelectorFunctions::new(data));
                Box::new(ComponentModal::new(content, sender))
            }
            Modals::VaultIndex(vault_path, index_type) => {
                debug!("show indexer");
                let modal = VaultIndexer::start(vault_path, index_type, self.ctx.clone())?;
                Box::new(modal)
            }
        };
        self.current_modal = Some(modal);
        Ok(())
    }

    pub fn close_modal(&mut self) {
        self.current_modal = None;
    }
}
