use crossbeam_channel::Sender;
use editor_view::EditorView;
use eframe::egui;
use kimun_core::nfs::VaultPath;
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
    vtype: ViewerType,
    viewer: Box<dyn NoteViewer>,
}

impl NoteViewerManager {
    pub fn new(message_sender: Sender<EditorMessage>) -> Self {
        Self {
            text: String::new(),
            changed: false,
            message_sender: message_sender.clone(),
            vtype: ViewerType::Nothing,
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
    pub fn load_content(&mut self, path: &VaultPath, text: String) {
        self.text = text.clone();
        self.changed = false;
        let new_type = match &self.vtype {
            ViewerType::Nothing => ViewerType::Editor(path.clone()),
            ViewerType::Editor(_vault_path) => ViewerType::Editor(path.to_owned()),
            ViewerType::Preview(_vault_path) => ViewerType::Preview(path.to_owned()),
        };
        self.set_view(new_type);
    }
    pub fn manage_keys(&mut self, ctx: &egui::Context) {
        self.viewer.manage_keys(ctx);
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
    pub fn set_view(&mut self, vtype: ViewerType) {
        self.viewer = match &vtype {
            ViewerType::Nothing => Box::new(NoView::new()),
            ViewerType::Editor(path) => {
                Box::new(EditorView::new(self.message_sender.clone(), path))
            }
            ViewerType::Preview(path) => {
                Box::new(RenderedView::new(self.message_sender.clone(), path))
            }
        };
        self.vtype = vtype;
        self.viewer.init(self.text.clone());
    }
}

#[derive(Debug, Clone)]
pub enum ViewerType {
    Nothing,
    Editor(VaultPath),
    Preview(VaultPath),
}

pub trait NoteViewer {
    fn view(&mut self, text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool>;
    fn init(&mut self, text: String);
    fn manage_keys(&mut self, ctx: &egui::Context);
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
    fn manage_keys(&mut self, _ctx: &egui::Context) {}

    fn init(&mut self, _text: String) {}
}
