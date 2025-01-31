use crossbeam_channel::Sender;
use editor_view::EditorView;
use eframe::egui;
use kimun_core::nfs::VaultPath;
use log::error;
use rendered_view::RenderedView;

use super::EditorMessage;

mod editor_view;
mod highlighter;
mod rendered_view;

pub const ID_VIEWER: &str = "Note Editor";

pub struct NoteViewerManager {
    text: String,
    changed: bool,
    message_sender: Sender<EditorMessage>,
    viewer: Box<dyn NoteViewer>,
}

impl NoteViewerManager {
    pub fn new(message_sender: Sender<EditorMessage>) -> Self {
        Self {
            text: String::new(),
            changed: false,
            message_sender: message_sender.clone(),
            viewer: Box::new(NoView::new()),
        }
    }
    pub fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()> {
        match self.viewer.view(&mut self.text, ui) {
            Ok(changed) => {
                if changed {
                    self.changed = true;
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
    pub fn manage_keys(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.viewer.manage_keys(ctx) {
            if let Err(e) = self.message_sender.send(message) {
                error!("Error sending view message: {}", e);
            };
        }
    }
    pub fn should_save(&self) -> bool {
        self.changed
    }
    pub fn report_saved(&mut self) {
        self.changed = false;
    }
    pub fn get_text(&self) -> String {
        self.text.clone()
    }
    pub fn load_content(&mut self, path: &VaultPath, text: String) {
        self.text = text.clone();
        self.changed = false;

        self.viewer = self.viewer.view_change_on_content(path);
        self.viewer.init(text);
    }
    pub fn set_view(&mut self, vtype: ViewerType) {
        self.viewer = vtype.get_view();
        self.viewer.init(self.text.clone());
    }
}

#[derive(Debug, Clone)]
pub enum ViewerType {
    Nothing,
    Editor(VaultPath),
    Preview(VaultPath),
}

impl ViewerType {
    fn get_view(&self) -> Box<dyn NoteViewer> {
        match self {
            ViewerType::Nothing => Box::new(NoView::new()),
            ViewerType::Editor(vault_path) => Box::new(EditorView::new(vault_path)),
            ViewerType::Preview(vault_path) => Box::new(RenderedView::new(vault_path)),
        }
    }
}

pub trait NoteViewer {
    fn view(&mut self, text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool>;
    fn init(&mut self, text: String);
    fn manage_keys(&mut self, ctx: &egui::Context) -> Option<EditorMessage>;
    fn view_change_on_content(&self, vault_path: &VaultPath) -> Box<dyn NoteViewer>;
}

struct NoView {}

impl NoView {
    fn new() -> Self {
        Self {}
    }
}

impl NoteViewer for NoView {
    fn view(&mut self, _text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool> {
        ui.vertical_centered(|ui| {
            ui.add_space(64.0);
            ui.label("Open or create a note with cmd + O");
        });
        Ok(false)
    }
    fn manage_keys(&mut self, _ctx: &egui::Context) -> Option<EditorMessage> {
        None
    }

    fn init(&mut self, _text: String) {}

    fn view_change_on_content(&self, vault_path: &VaultPath) -> Box<dyn NoteViewer> {
        Box::new(EditorView::new(vault_path))
    }
}
