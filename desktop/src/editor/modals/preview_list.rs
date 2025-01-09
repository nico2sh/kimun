use std::sync::mpsc::{Receiver, Sender};

use eframe::egui::ScrollArea;
use log::error;
use notes_core::{nfs::NotePath, NoteDetails, NoteVault};

use super::{
    filtered_list::{FilteredList, FilteredListFunctions, ListElement},
    vault_browse::SelectorEntry,
    EditorModal,
};

enum PreviewState {
    Empty,
    LoadingPreview,
    PreviewLoaded { path: NotePath, text: String },
}

pub trait SelectionPath: PartialEq {
    fn get_path(&self) -> NotePath;
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
    preview_text: String,
    show_preview: bool,
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
        let (state_sender, state_receiver) = std::sync::mpsc::channel();
        Self {
            vault,
            list,
            state: PreviewState::Empty,
            preview_text: String::new(),
            show_preview: true,
            state_sender,
            state_receiver,
        }
    }

    fn update_state(&mut self) {
        while let Ok(state) = self.state_receiver.try_recv() {
            match &state {
                PreviewState::Empty => self.preview_text = "".to_string(),
                PreviewState::LoadingPreview => {}
                PreviewState::PreviewLoaded { path: _, text } => self.preview_text = text.clone(),
            }
            self.state = state;
        }

        let selected_path = self.list.get_selection().and_then(|selection| {
            let selection_path = selection.get_path();
            if selection_path.is_note() {
                Some(selection_path)
            } else {
                self.preview_text = "".to_string();
                None
            }
        });

        match &self.state {
            PreviewState::Empty => {
                if let Some(selected_path) = selected_path {
                    self.load_preview(selected_path);
                }
            }
            PreviewState::LoadingPreview => {}
            PreviewState::PreviewLoaded { path, text: _ } => {
                if let Some(selected_path) = selected_path {
                    if path != &selected_path {
                        self.load_preview(selected_path);
                    }
                }
            }
        }
    }

    fn load_preview(&mut self, path: NotePath) {
        self.state = PreviewState::LoadingPreview;
        if path.is_note() {
            let vault = self.vault.clone();
            let tx = self.state_sender.clone();
            std::thread::spawn(move || {
                let text = vault.load_note(&path).unwrap_or_default();
                if let Err(e) = tx.send(PreviewState::PreviewLoaded { path, text }) {
                    error!("Failed to send a preview load status: {}", e);
                }
            });
        } else if let Err(e) = self.state_sender.send(PreviewState::Empty) {
            error!("Failed to send a preview load status: {}", e);
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
        if self.show_preview {
            self.update_state();
            ui.columns(2, |columns| {
                self.list.update(&mut columns[0]);
                ScrollArea::vertical().show(&mut columns[1], |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(self.preview_text.clone());
                    });
                })
            });
        } else {
            self.list.update(ui);
        }
    }
}

impl SelectionPath for NoteDetails {
    fn get_path(&self) -> NotePath {
        self.path.clone()
    }
}

impl SelectionPath for SelectorEntry {
    fn get_path(&self) -> NotePath {
        self.path.clone()
    }
}

impl PartialEq for SelectorEntry {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
