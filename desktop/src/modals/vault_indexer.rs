use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use crossbeam_channel::Sender;
use eframe::egui;
use kimun_core::{NoteVault, NotesValidation};
use log::{debug, error};

use super::KimunModal;

pub struct VaultIndexer {
    sender: Sender<IndexStatus>,
    status: Arc<Mutex<IndexStatus>>,
}

impl KimunModal for VaultIndexer {
    fn update(&mut self, ui: &mut egui::Ui) -> bool {
        let status = &*self.status.lock().unwrap();
        let should_close = match status {
            IndexStatus::Closed => true,
            IndexStatus::Error(error) => {
                egui::Modal::new(egui::Id::new("")).show(ui.ctx(), |ui| {
                    ui.set_width(600.0);
                    ui.heading("Indexing Vault");
                    ui.label(format!("Error Indexing: {}", error));
                    if ui.button("Close").clicked() {
                        if let Err(e) = self.sender.send(IndexStatus::Closed) {
                            error!("Error Closing the Indexer Modal: {}", e);
                        }
                    }
                });
                false
            }
            IndexStatus::Indexing(index_type) => {
                egui::Modal::new(egui::Id::new("")).show(ui.ctx(), |ui| {
                    ui.set_width(600.0);
                    ui.heading("Indexing Vault");
                    let message = match index_type {
                        IndexType::Validate => "Validating Vault, please wait.",
                        IndexType::Fast => "Fast checking, please wait.",
                        IndexType::Full => "Fully reindexing, this may take a bit on large Vaults",
                    };
                    ui.label(message);
                });
                false
            }
            IndexStatus::Done => {
                egui::Modal::new(egui::Id::new("")).show(ui.ctx(), |ui| {
                    ui.set_width(600.0);
                    ui.heading("Indexing Vault");
                    ui.label("Finished Indexing.");
                    let close_button = ui.button("Close");
                    if close_button.clicked() {
                        if let Err(e) = self.sender.send(IndexStatus::Closed) {
                            error!("Error Closing the Indexer Modal: {}", e);
                        }
                    }
                });
                false
            }
        };
        should_close
    }
}

impl VaultIndexer {
    pub fn new(ctx: egui::Context) -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let status = Arc::new(Mutex::new(IndexStatus::Closed));
        let status_ref = status.clone();

        let ctx = ctx.clone();
        std::thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                debug!("received index status: {:?}", message);
                *status_ref.lock().unwrap() = message;
                ctx.request_repaint();
            }
        });

        Self { sender, status }
    }

    pub fn start<P: AsRef<Path>>(
        vault_path: P,
        index_type: IndexType,
        ctx: egui::Context,
    ) -> anyhow::Result<Self> {
        let mut indexer = Self::new(ctx);
        indexer.index_vault(vault_path, index_type)?;
        Ok(indexer)
    }

    pub fn index_vault<P: AsRef<Path>>(
        &mut self,
        vault_path: P,
        index_type: IndexType,
    ) -> anyhow::Result<()> {
        let vault = NoteVault::new(vault_path)?;
        let sender = self.sender.clone();
        *self.status.lock().unwrap() = IndexStatus::Indexing(index_type);
        std::thread::spawn(move || {
            match index_type {
                IndexType::Validate => {
                    if let Err(e) = match vault.init_and_validate() {
                        Ok(_) => sender.send(IndexStatus::Closed),
                        Err(error) => sender.send(IndexStatus::Error(format!("{}", error))),
                    } {
                        error!("Error updating status of the indexer: {}", e);
                    }
                }
                IndexType::Fast => {
                    if let Err(e) = match vault.index_notes(NotesValidation::Fast) {
                        Ok(_) => sender.send(IndexStatus::Done),
                        Err(error) => sender.send(IndexStatus::Error(format!("{}", error))),
                    } {
                        error!("Error updating status of the indexer: {}", e);
                    }
                }
                IndexType::Full => {
                    if let Err(e) = match vault.force_rebuild() {
                        Ok(_) => sender.send(IndexStatus::Done),
                        Err(error) => sender.send(IndexStatus::Error(format!("{}", error))),
                    } {
                        error!("Error updating status of the indexer: {}", e);
                    }
                }
            };
        });
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexType {
    Validate,
    Fast,
    Full,
}

#[derive(Debug, PartialEq, Eq)]
enum IndexStatus {
    Closed,
    Error(String),
    Indexing(IndexType),
    Done,
}
