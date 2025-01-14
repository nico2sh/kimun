mod filtered_list;
mod preview_list;
mod vault_browse;

use crossbeam_channel::Sender;
use eframe::egui;
use filtered_list::FilteredList;
use log::debug;
use notes_core::{nfs::VaultPath, NoteVault};
use preview_list::PreviewList;
use vault_browse::{VaultBrowseFunctions, VaultSearchFunctions};

use super::EditorMessage;

pub struct ModalManager {
    message_sender: Sender<EditorMessage>,
    vault: NoteVault,
    current_modal: Option<Box<dyn EditorModal>>,
}

pub enum Modals {
    VaultBrowse(VaultPath),
    VaultSearch,
}

impl ModalManager {
    pub fn new(vault: NoteVault, message_bus: Sender<EditorMessage>) -> Self {
        Self {
            message_sender: message_bus,
            vault,
            current_modal: None,
        }
    }

    pub fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()> {
        if let Some(current_modal) = self.current_modal.as_mut() {
            let modal = egui::Modal::new(egui::Id::new("")).show(ui.ctx(), |ui| {
                ui.set_width(600.0);
                // ui.heading("Heading");
                current_modal.update(ui);
            });
            if modal.should_close() {
                self.current_modal = None;
            }
        }
        Ok(())
    }

    pub fn set_modal(&mut self, modal: Modals) {
        match modal {
            Modals::VaultBrowse(path) => {
                debug!("show browser");
                let content = PreviewList::new(
                    self.vault.clone(),
                    FilteredList::new(
                        VaultBrowseFunctions::new(path.clone(), self.vault.clone()),
                        self.message_sender.clone(),
                    ),
                );
                self.current_modal = Some(Box::new(content));
            }
            Modals::VaultSearch => {
                debug!("show searcher");
                let content = PreviewList::new(
                    self.vault.clone(),
                    FilteredList::new(
                        VaultSearchFunctions::new(self.vault.clone()),
                        self.message_sender.clone(),
                    ),
                );
                self.current_modal = Some(Box::new(content));
            }
        };
    }

    pub fn close_modal(&mut self) {
        self.current_modal = None;
    }
}

pub trait EditorModal {
    fn update(&mut self, ui: &mut egui::Ui);
}
