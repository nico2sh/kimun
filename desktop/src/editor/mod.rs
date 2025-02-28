pub mod components;
mod save_manager;
mod viewers;

use std::sync::{Arc, Mutex};

use anyhow::bail;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use kimun_core::{nfs::VaultPath, NoteVault};
use log::{debug, error};
use save_manager::SaveManager;
use viewers::{editor_view::EditorView, rendered_view::RenderedView, NoteViewer, ViewerType};

use crate::{
    modals::{vault_indexer::IndexType, ModalManager, Modals},
    settings::Settings,
    WindowAction,
};

use super::MainView;

pub struct Editor {
    settings: Settings,
    viewer: Box<dyn NoteViewer>,
    raw_text: String,
    note_path: VaultPath,
    vault: NoteVault,
    save_manager: SaveManager,
    modal_manager: ModalManager,
    message_sender: Sender<EditorMessage>,
    next_action: Arc<Mutex<Option<EditorMessage>>>,
    request_focus: bool,
    request_windows_switch: Option<WindowAction>,
}

impl Editor {
    pub fn new(
        vault: &NoteVault,
        note_path: &VaultPath,
        ctx: egui::Context,
    ) -> anyhow::Result<Self> {
        let settings = Settings::load_from_disk()?;
        let (sender, receiver) = crossbeam_channel::unbounded();
        let next_action = Arc::new(Mutex::new(None));
        start_event_loop(receiver, next_action.clone(), ctx.clone());

        let vault = vault.to_owned();
        let note_path = note_path.to_owned();
        let mut modal_manager = ModalManager::new(ctx);
        modal_manager.set_modal(Modals::VaultIndex(
            vault.workspace_path.clone(),
            IndexType::Validate,
        ))?;
        let save_manager = SaveManager::new(String::new(), &note_path, &vault);
        let mut editor = Self {
            settings: settings.clone(),
            viewer: Box::new(EditorView::new(&note_path)),
            raw_text: String::new(),
            modal_manager,
            save_manager,
            vault,
            note_path: note_path.clone(),
            message_sender: sender,
            next_action,
            request_focus: true,
            request_windows_switch: None,
        };
        editor.load_note_path(&note_path)?;
        editor.save_manager.init_loop();

        Ok(editor)
    }

    /// Loads a note from the path
    /// if the path is a note, then we load the note in the current view
    /// if not, we return an error
    fn load_note_path(&mut self, note_path: &VaultPath) -> anyhow::Result<()> {
        if note_path.is_note() && self.vault.exists(note_path).is_some() {
            let text = self.vault.get_note_text(note_path)?;
            self.settings.add_path_history(note_path);
            self.settings.save_to_disk()?;
            self.set_content(note_path, text);
        } else {
            bail!("Note path is not a note or vault path doesn't exist")
        };

        Ok(())
    }

    fn set_content(&mut self, path: &VaultPath, text: String) {
        self.raw_text = text.clone();
        self.save_manager.load(&text, path);

        self.viewer.init(text);
    }

    fn save_note(&mut self) -> anyhow::Result<()> {
        self.save_manager.save()
    }

    fn manage_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            let path = self.save_manager.get_path();
            let browse_path = if path.is_note() {
                path.get_parent_path().0
            } else {
                path
            };
            let _e = self.modal_manager.set_modal(Modals::VaultBrowse(
                self.vault.clone(),
                browse_path,
                self.message_sender.clone(),
            ));
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            let _e = self.modal_manager.set_modal(Modals::VaultSearch(
                self.vault.clone(),
                self.message_sender.clone(),
            ));
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
        if let Some(message) = self.viewer.manage_keys(ctx) {
            if let Err(e) = self.message_sender.send(message) {
                error!("Error sending view message: {}", e);
            };
        }
    }

    fn update_messages(&mut self, _ctx: &egui::Context) -> anyhow::Result<()> {
        let next_action = {
            let mut lock = self.next_action.lock().unwrap();
            let next_action = lock.clone();
            *lock = None;
            next_action
        };

        if let Some(message) = next_action {
            debug!("Action: {:?}", message);
            match message {
                EditorMessage::OpenNote(note_path) => {
                    self.load_note_path(&note_path)?;
                    self.modal_manager.close_modal();
                    self.request_focus = true;
                }
                EditorMessage::OpenCreateOrSearchNote(path) => {
                    let result = self.vault.open_or_search(&path)?;
                    match result.len() {
                        0 => {
                            let path = VaultPath::new(path);
                            if let Err(e) = self.message_sender.send(EditorMessage::NewNote(path)) {
                                error!("Error sending an editor message: {}", e);
                            }
                        }
                        1 => {
                            let path = result.first().unwrap().0.path.clone();
                            if let Err(e) = self.message_sender.send(EditorMessage::OpenNote(path))
                            {
                                error!("Error sending an editor message: {}", e);
                            }
                        }
                        _ => {
                            // TODO: Many results, we show a selector
                        }
                    }
                }
                EditorMessage::NewJournal => {
                    let (data, _content) = self.vault.journal_entry()?;
                    {
                        self.load_note_path(&data.path)?;
                        self.modal_manager.close_modal();
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
                    self.set_content(&np, String::new());
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
                    self.request_windows_switch = Some(WindowAction::Settings)
                }
            }
        }
        Ok(())
    }

    fn change_viewer(&mut self, viewer: ViewerType) -> anyhow::Result<()> {
        self.save_note()?;
        self.viewer = match viewer {
            ViewerType::Editor => Box::new(EditorView::new(&self.note_path)),
            ViewerType::Rendered => Box::new(RenderedView::new(
                &self.note_path,
                self.message_sender.clone(),
            )),
        };
        self.viewer.init(self.raw_text.clone());
        Ok(())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Err(e) = self.save_manager.save() {
            error!("Error saving note: {}", e);
        }
    }
}

impl MainView for Editor {
    fn update(&mut self, ui: &mut egui::Ui) -> anyhow::Result<Option<WindowAction>> {
        self.modal_manager.view(ui)?;
        egui::ScrollArea::vertical()
            .show(ui, |ui| match self.viewer.view(&mut self.raw_text, ui) {
                Ok(changed) => {
                    if changed {
                        self.save_manager.update_text(&self.raw_text);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            })
            .inner?;

        self.manage_keys(ui.ctx());

        if self.request_focus {
            ui.ctx()
                .memory_mut(|mem| mem.request_focus(viewers::ID_VIEWER.into()));
            self.request_focus = false;
        }

        self.update_messages(ui.ctx())?;

        Ok(self.request_windows_switch.clone())
    }
}

fn start_event_loop(
    receiver: Receiver<EditorMessage>,
    message_container: Arc<Mutex<Option<EditorMessage>>>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        while let Ok(message) = receiver.recv() {
            *message_container.lock().unwrap() = Some(message);
            ctx.request_repaint();
        }
    });
}

#[derive(Clone, Debug)]
pub enum EditorMessage {
    OpenNote(VaultPath),
    OpenCreateOrSearchNote(String),
    NewNote(VaultPath),
    SwitchNoteViewer(ViewerType),
    NewJournal,
    Save,
    OpenSettings,
}
