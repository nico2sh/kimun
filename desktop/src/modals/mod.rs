mod vault_browser;

use crate::View;
use eframe::egui;
use log::{debug, info};
use notes_core::{nfs::NotePath, NoteVault, SearchResult};
use vault_browser::{SelectorEntry, VaultBrowser};

pub struct ModalManager {
    vault: NoteVault,
    current_modal: Option<Box<dyn EditorModal>>,
}

pub enum Modals {
    None,
    VaultBrowser(NotePath),
}

impl View for ModalManager {
    fn view(&mut self, ui: &mut egui::Ui) -> crate::Message {
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
        crate::Message::None
    }
}

impl ModalManager {
    pub fn new(vault: NoteVault) -> Self {
        Self {
            vault,
            current_modal: None,
        }
    }

    pub fn set_modal(&mut self, modal: Modals) {
        match modal {
            Modals::None => {}
            Modals::VaultBrowser(path) => {
                debug!("show browser");
                let mut content = VaultBrowser::new(self.vault.clone());
                content.browse_path(&path);
                self.current_modal = Some(Box::new(content));
            }
        }
    }
}

pub trait EditorModal {
    fn update(&mut self, ui: &mut egui::Ui);
}
