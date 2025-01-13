mod modals;
mod viewer;

use std::{any::Any, sync::Arc};

use anyhow::bail;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use log::{debug, error};
use modals::{ModalManager, Modals};
use notes_core::{
    nfs::{load_note, VaultPath},
    NoteVault,
};
use viewer::{NoteViewer, ViewerType};

use super::{settings::Settings, View};

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

            let note_path = settings.last_path.clone().and_then(|path| {
                if !path.is_note() {
                    None
                } else {
                    Some(path)
                }
            });
            let viewer = if let Some(path) = &note_path {
                let content = load_note(workspace_dir, path)?;
                let mut viewer = ViewerType::Editor.new_view(sender.clone());
                viewer.load_content(content);
                viewer
            } else {
                ViewerType::Editor.new_view(sender.clone())
            };
            Ok(Self {
                viewer,
                vault: Arc::new(vault),
                modal_manager: ModalManager::new(NoteVault::new(workspace_dir)?, sender.clone()),
                message_sender: sender,
                message_receiver: receiver,
                note_path,
                request_focus: true,
            })
        } else {
            bail!("Path not provided")
        }
    }

    fn load_note(&mut self, note_path: &VaultPath) -> anyhow::Result<()> {
        if note_path.is_note() {
            let editor_data = self.vault.load_note(note_path)?;
            self.viewer.load_content(editor_data);
            // TODO: Manage the view
            self.note_path = Some(note_path.to_owned());
            self.modal_manager.close_modal();
        } else {
            error!("Path is not a note: {}", note_path);
        }
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
                    self.load_note(&note_path)?;
                    self.request_focus = true;
                }
                EditorMessage::NewJournal => match self.vault.journal_entry() {
                    Ok((data, _content)) => {
                        self.load_note(&data.path)?;
                        self.request_focus = true;
                    }
                    Err(_) => todo!(),
                },
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
