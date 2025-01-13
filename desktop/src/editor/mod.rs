mod modals;
mod viewer;

use std::{any::Any, sync::Arc};

use anyhow::bail;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use log::{debug, error};
use modals::{ModalManager, Modals};
use notes_core::{nfs::VaultPath, NoteVault};
use viewer::{NoteViewer, ViewerType};

use crate::settings::Settings;

use super::View;

const AUTOSAVE_SECS: u64 = 5;

pub struct Editor {
    viewer: Box<dyn NoteViewer>,
    vault: Arc<NoteVault>,
    modal_manager: ModalManager,
    message_sender: Sender<EditorMessage>,
    message_receiver: Receiver<EditorMessage>,
    note_path: Option<VaultPath>,
    request_focus: bool,
}

impl Editor {
    pub fn new(settings: &Settings) -> anyhow::Result<Self> {
        if let Some(workspace_dir) = &settings.workspace_dir {
            let (sender, receiver) = crossbeam_channel::unbounded();
            let vault = NoteVault::new(workspace_dir)?;

            let save_sender = sender.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(AUTOSAVE_SECS));
                if let Err(e) = save_sender.send(EditorMessage::Save) {
                    error!("Error sending a save message: {}", e);
                }
            });

            let note_path = settings.last_paths.last().and_then(|path| {
                if !path.is_note() {
                    None
                } else {
                    Some(path.to_owned())
                }
            });
            let mut editor = Self {
                viewer: ViewerType::Nothing.new_view(sender.clone()),
                vault: Arc::new(vault),
                modal_manager: ModalManager::new(NoteVault::new(workspace_dir)?, sender.clone()),
                message_sender: sender,
                message_receiver: receiver,
                note_path: note_path.clone(),
                request_focus: true,
            };
            editor.load_note_path(&note_path)?;
            Ok(editor)
        } else {
            bail!("Path not provided")
        }
    }

    /// Loads a note from the path
    /// if no path is specified, we put a placeholder view
    /// if the path is a directory, we put a placeholder view
    /// if the path is a note, then we load the note in the current view
    fn load_note_path(&mut self, note_path: &Option<VaultPath>) -> anyhow::Result<()> {
        if let Some(path) = &note_path {
            let content = self.vault.load_note(path)?;
            if !self.note_path.as_ref().is_some_and(|path| path.is_note()) {
                let viewer = ViewerType::Editor.new_view(self.message_sender.clone());
                self.viewer = viewer;
            }
            self.viewer.load_content(content);
        } else {
            self.viewer = ViewerType::Nothing.new_view(self.message_sender.clone());
        };
        self.note_path = note_path.to_owned();
        self.modal_manager.close_modal();

        Ok(())
    }

    fn save_note(&self) -> anyhow::Result<()> {
        debug!("Checking if to save note");
        if let Some(note_path) = &self.note_path {
            if self.viewer.should_save() {
                debug!("Saving note");
                let content = self.viewer.get_text();
                self.vault.save_note(note_path, content)?;
            }
        }
        Ok(())
    }

    fn manage_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            let browse_path = self
                .note_path
                .clone()
                .map(|path| {
                    if path.is_note() {
                        path.get_parent_path().0
                    } else {
                        path
                    }
                })
                .unwrap_or_default();
            self.modal_manager
                .set_modal(Modals::VaultBrowse(browse_path));
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.modal_manager.set_modal(Modals::VaultSearch);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::J)) {
            if let Err(e) = self
                .modal_manager
                .message_sender
                .send(EditorMessage::NewJournal)
            {
                error!("Error opening journal: {}", e);
            }
        }
        self.viewer.manage_keys(ctx);
    }

    fn update(&mut self, ctx: &egui::Context) -> anyhow::Result<()> {
        while let Ok(message) = self.message_receiver.try_recv() {
            match message {
                EditorMessage::OpenNote(note_path) => {
                    self.load_note_path(&Some(note_path))?;
                    self.request_focus = true;
                }
                EditorMessage::NewJournal => {
                    let (data, _content) = self.vault.journal_entry()?;
                    {
                        self.load_note_path(&Some(data.path))?;
                        self.request_focus = true;
                    }
                }
                EditorMessage::NewNote(note_path) => {
                    let mut np = note_path.clone();
                    loop {
                        if self.vault.exists(&np).is_none() {
                            break;
                        } else {
                            np = np.get_name_on_conflict();
                        }
                    }
                    debug!("New note at: {}", np);
                    self.viewer.load_content(String::new());
                    self.note_path = Some(np);
                    self.modal_manager.close_modal();
                    self.request_focus = true;
                }
                EditorMessage::Save => {
                    self.save_note()?;
                }
                EditorMessage::ShowPreview => {
                    self.change_viewer(ViewerType::Preview)?;
                }
                EditorMessage::ShowEditor => {
                    self.change_viewer(ViewerType::Editor)?;
                }
            }
        }
        self.viewer.update(ctx)?;
        Ok(())
    }

    fn change_viewer(&mut self, viewer: ViewerType) -> anyhow::Result<()> {
        self.save_note()?;
        let text = self.viewer.get_text();
        self.viewer = viewer.new_view(self.message_sender.clone());
        self.viewer.load_content(text);
        Ok(())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Err(e) = self.save_note() {
            error!("Error saving note: {}", e);
        }
    }
}

impl View for Editor {
    fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()> {
        self.modal_manager.view(ui)?;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Err(e) = self.viewer.view(ui) {
                error!("Error displaying viewer view: {}", e);
            }
        });

        self.manage_keys(ui.ctx());

        if self.request_focus {
            ui.ctx()
                .memory_mut(|mem| mem.request_focus(viewer::ID_VIEWER.into()));
            self.request_focus = false;
        }

        self.update(ui.ctx())?;

        Ok(())
    }
}

pub(crate) enum EditorMessage {
    OpenNote(VaultPath),
    NewNote(VaultPath),
    ShowPreview,
    ShowEditor,
    NewJournal,
    Save,
}
