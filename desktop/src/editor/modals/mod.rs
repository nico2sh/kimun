mod filtered_list;
mod vault_browse;

use std::sync::mpsc::Sender;

use eframe::egui;
use filtered_list::FilteredList;
use log::debug;
use notes_core::{nfs::NotePath, NoteVault};
use vault_browse::VaultBrowseFunctions;

use crate::View;

use super::EditorMessage;

pub struct ModalManager {
    message_sender: Sender<EditorMessage>,
    vault: NoteVault,
    current_modal: Option<Box<dyn EditorModal>>,
}

pub enum Modals {
    VaultBrowse(NotePath),
}

impl View for ModalManager {
    fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()> {
        if let Some(current_modal) = self.current_modal.as_mut() {
            let modal = egui::Modal::new(egui::Id::new("")).show(ui.ctx(), |ui| {
                ui.set_width(400.0);
                // ui.heading("Heading");
                current_modal.update(ui);
            });
            if modal.should_close() {
                self.current_modal = None;
            }
        }
        Ok(())
    }
}

impl ModalManager {
    pub fn new(vault: NoteVault, message_bus: Sender<EditorMessage>) -> Self {
        Self {
            message_sender: message_bus,
            vault,
            current_modal: None,
        }
    }

    pub fn set_modal(&mut self, modal: Modals) {
        match modal {
            Modals::VaultBrowse(path) => {
                debug!("show browser");
                let content = FilteredList::new(
                    VaultBrowseFunctions::new(path.clone(), self.vault.clone()),
                    self.message_sender.clone(),
                );
                self.current_modal = Some(Box::new(content));
            }
        }
    }

    pub fn close_modal(&mut self) {
        self.current_modal = None;
    }
}

pub trait EditorModal {
    fn update(&mut self, ui: &mut egui::Ui);
}
