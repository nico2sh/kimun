mod modals;
mod viewers;

use std::sync::{atomic::AtomicBool, Arc};

use anyhow::bail;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use log::{debug, error};
use modals::{ModalManager, Modals};
use notes_core::{nfs::VaultPath, NoteVault};
use viewers::{NoteViewer, NoteViewerManager, ViewerType};

use crate::{settings::Settings, WindowSwitch};

use super::MainView;

const AUTOSAVE_SECS: u64 = 5;

pub struct Editor {
    settings: Settings,
    viewer: NoteViewerManager,
    vault: Arc<NoteVault>,
    modal_manager: ModalManager,
    message_sender: Sender<EditorMessage>,
    message_receiver: Receiver<EditorMessage>,
    note_path: Option<VaultPath>,
    request_focus: bool,
    request_windows_switch: Option<WindowSwitch>,
    save_loop: Arc<AtomicBool>,
}

impl Editor {
    pub fn new(settings: &Settings, recreate_index: bool) -> anyhow::Result<Self> {
        if let Some(workspace_dir) = &settings.workspace_dir {
            let (sender, receiver) = crossbeam_channel::unbounded();
            let vault = NoteVault::new(workspace_dir)?;
            if recreate_index {
                vault.init_and_validate()?;
            }

            let save_sender = sender.clone();

            let note_path = settings.last_paths.last().and_then(|path| {
                if !path.is_note() {
                    None
                } else {
                    Some(path.to_owned())
                }
            });
            let save_loop = Arc::new(AtomicBool::from(true));
            let should_save = save_loop.clone();
            std::thread::spawn(move || {
                while should_save.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_secs(AUTOSAVE_SECS));
                    if let Err(e) = save_sender.send(EditorMessage::Save) {
                        error!("Error sending a save message: {}", e);
                    };
                }
            });
            let mut editor = Self {
                settings: settings.clone(),
                viewer: NoteViewerManager::new(sender.clone()),
                modal_manager: ModalManager::new(vault.clone(), sender.clone()),
                vault: Arc::new(vault),
                message_sender: sender,
                message_receiver: receiver,
                note_path: note_path.clone(),
                request_focus: true,
                request_windows_switch: None,
                save_loop,
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
            if path.is_note() && self.vault.exists(path).is_some() {
                let content = self.vault.load_note(path)?;
                // If the current NotePath used to be a non-note we assume the view wasn't the editor
                if matches!(self.viewer.get_type(), ViewerType::Nothing) {
                    self.viewer.set_view(ViewerType::Editor);
                }
                self.settings.add_path_history(path);
                self.settings.save_to_disk()?;
                self.viewer.load_content(content);
            } else {
                self.viewer.set_view(ViewerType::Nothing);
            }
        } else {
            self.viewer.set_view(ViewerType::Nothing);
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
            if let Err(e) = self.message_sender.send(EditorMessage::NewJournal) {
                error!("Error opening journal: {}", e);
            }
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            if let Err(e) = self.message_sender.send(EditorMessage::OpenSettings) {
                error!("Error opening journal: {}", e);
            }
        }
        self.viewer.manage_keys(ctx);
    }

    fn update_messages(&mut self, _ctx: &egui::Context) -> anyhow::Result<()> {
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
                EditorMessage::SwitchNoteViewer(viewer_type) => {
                    self.change_viewer(viewer_type)?;
                }
                EditorMessage::OpenSettings => {
                    self.request_windows_switch = Some(WindowSwitch::Settings)
                }
            }
        }
        Ok(())
    }

    fn change_viewer(&mut self, viewer: ViewerType) -> anyhow::Result<()> {
        self.save_note()?;
        let text = self.viewer.get_text();
        self.viewer.set_view(viewer);
        self.viewer.load_content(text);
        Ok(())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.save_loop
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if let Err(e) = self.save_note() {
            error!("Error saving note: {}", e);
        }
    }
}

impl MainView for Editor {
    fn update(&mut self, ui: &mut egui::Ui) -> anyhow::Result<Option<WindowSwitch>> {
        self.modal_manager.view(ui)?;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Err(e) = self.viewer.view(ui) {
                error!("Error displaying viewer view: {}", e);
            }
        });

        self.manage_keys(ui.ctx());

        if self.request_focus {
            ui.ctx()
                .memory_mut(|mem| mem.request_focus(viewers::ID_VIEWER.into()));
            self.request_focus = false;
        }

        self.update_messages(ui.ctx())?;

        Ok(self.request_windows_switch)
    }
}

pub(crate) enum EditorMessage {
    OpenNote(VaultPath),
    NewNote(VaultPath),
    SwitchNoteViewer(ViewerType),
    NewJournal,
    Save,
    OpenSettings,
}
