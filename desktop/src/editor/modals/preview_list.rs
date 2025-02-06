use crossbeam_channel::{Receiver, Sender};
use eframe::egui::ScrollArea;
use kimun_core::{nfs::VaultPath, NoteDetails, NoteVault};
use log::error;

use super::{
    filtered_list::{FilteredList, FilteredListFunctions, ListElement},
    vault_browse::SelectorEntry,
    EditorModal,
};

enum PreviewState {
    Empty,
    LoadingPreview,
    PreviewNote { path: VaultPath, text: String },
    PreviewDirectory { path: VaultPath },
}

pub trait SelectionPath: PartialEq {
    fn get_path(&self) -> VaultPath;
}

pub struct PreviewList<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static + SelectionPath,
{
    vault: NoteVault,
    list: FilteredList<F, P, D>,
    state: PreviewState,
    state_sender: Sender<PreviewState>,
    state_receiver: Receiver<PreviewState>,
}

impl<F, P, D> PreviewList<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static + SelectionPath,
{
    pub fn new(vault: NoteVault, list: FilteredList<F, P, D>) -> Self {
        let (state_sender, state_receiver) = crossbeam_channel::unbounded();
        Self {
            vault,
            list,
            state: PreviewState::Empty,
            state_sender,
            state_receiver,
        }
    }

    fn update_state(&mut self) {
        while let Ok(state) = self.state_receiver.try_recv() {
            self.state = state;
        }

        let selected_path = self.list.get_selection().and_then(|selection| {
            let selection_path = selection.get_path();
            if selection_path.is_note() {
                Some(selection_path)
            } else {
                None
            }
        });

        // We changed the path
        if let Some(selected_path) = selected_path {
            match &self.state {
                PreviewState::Empty => {
                    self.load_preview(selected_path);
                }
                PreviewState::LoadingPreview => {}
                PreviewState::PreviewNote { path, text: _ } => {
                    if path != &selected_path {
                        self.load_preview(selected_path);
                    }
                }
                PreviewState::PreviewDirectory { path } => {
                    if path != &selected_path {
                        self.load_preview(selected_path);
                    }
                }
            }
        } else {
            self.state = PreviewState::Empty;
        }
    }

    fn load_preview(&mut self, path: VaultPath) {
        self.state = PreviewState::LoadingPreview;
        if path.is_note() {
            let vault = self.vault.clone();
            let tx = self.state_sender.clone();
            std::thread::spawn(move || {
                let text = vault.get_note_text(&path).unwrap_or_default();
                if let Err(e) = tx.send(PreviewState::PreviewNote { path, text }) {
                    error!("Failed to send a preview load status: {}", e);
                }
            });
        } else if let Err(e) = self
            .state_sender
            .send(PreviewState::PreviewDirectory { path })
        {
            error!("Failed to send a preview load status: {}", e);
        }
    }

    fn show_preview_area(&self, ui: &mut eframe::egui::Ui) {
        match &self.state {
            PreviewState::Empty => {
                ui.label("");
            }
            PreviewState::LoadingPreview => {
                ui.label("");
            }
            PreviewState::PreviewNote { path: _, text } => {
                ui.label(text);
            }
            PreviewState::PreviewDirectory { path: _ } => {
                ui.label("");
            }
        }
    }
}

impl<F, P, D> EditorModal for PreviewList<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static + SelectionPath,
{
    fn update(&mut self, ui: &mut eframe::egui::Ui) {
        self.update_state();
        ui.columns(2, |columns| {
            self.list.update(&mut columns[0]);
            ScrollArea::vertical().show(&mut columns[1], |ui| {
                ui.horizontal_wrapped(|ui| {
                    self.show_preview_area(ui);
                });
            })
        });
    }
}

impl SelectionPath for NoteDetails {
    fn get_path(&self) -> VaultPath {
        self.path.clone()
    }
}

impl SelectionPath for SelectorEntry {
    fn get_path(&self) -> VaultPath {
        self.path.clone()
    }
}

impl PartialEq for SelectorEntry {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
