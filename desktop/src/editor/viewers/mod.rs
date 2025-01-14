use crossbeam_channel::Sender;
use editor_view::EditorView;
use eframe::egui;
use rendered_view::RenderedView;

use crate::View;

use super::EditorMessage;

mod editor_view;
mod highlighter;
mod rendered_view;

pub const ID_VIEWER: &str = "Note Editor";

pub struct Viewer {
    text: String,
    changed: bool,
    message_sender: Sender<EditorMessage>,
    vtype: ViewerType,
    viewer: Box<dyn NoteViewer>,
}

impl Viewer {
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
        self.viewer.view(&mut self.text, ui)
    }
    pub fn get_type(&self) -> ViewerType {
        self.vtype.clone()
    }
    pub fn load_content(&mut self, text: String) {
        self.text = text;
        self.changed = false;
    }
    pub fn manage_keys(&mut self, ctx: &egui::Context) {
        self.viewer.manage_keys(ctx);
    }
    pub fn should_save(&self) -> bool {
        self.changed
    }
    pub fn get_text(&self) -> String {
        self.text.clone()
    }

    pub fn set_view(&mut self, vtype: ViewerType) {
        self.viewer = match vtype {
            ViewerType::Nothing => Box::new(NoView::new()),
            ViewerType::Editor => Box::new(EditorView::new(self.message_sender.clone())),
            ViewerType::Preview => Box::new(RenderedView::new(self.message_sender.clone())),
        };
    }
}

#[derive(Clone)]
pub enum ViewerType {
    Nothing,
    Editor,
    Preview,
}

pub trait NoteViewer {
    fn view(&mut self, text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<()>;
    fn manage_keys(&mut self, ctx: &egui::Context);
}

struct NoView {}

impl NoView {
    fn new() -> Self {
        Self {}
    }
}

impl View for NoView {
    fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()> {
        ui.vertical_centered(|ui| {
            ui.add_space(64.0);
            ui.label("Open or create a note with cmd + O");
        });
        Ok(())
    }
}

impl NoteViewer for NoView {
    fn view(&mut self, _text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<()> {
        ui.vertical_centered(|ui| {
            ui.add_space(64.0);
            ui.label("Open or create a note with cmd + O");
        });
        Ok(())
    }
    fn manage_keys(&mut self, _ctx: &egui::Context) {}
}
