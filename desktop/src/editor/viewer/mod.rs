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

pub enum ViewerType {
    Nothing,
    Editor,
    Preview,
}

pub trait NoteViewer: View {
    fn get_type(&self) -> ViewerType;
    fn load_content(&mut self, text: String);
    fn manage_keys(&mut self, ctx: &egui::Context);
    fn update(&mut self, ctx: &egui::Context) -> anyhow::Result<()>;
    fn should_save(&self) -> bool;
    fn get_text(&self) -> String;
}

impl ViewerType {
    pub fn new_view(&self, message_sender: Sender<EditorMessage>) -> Box<dyn NoteViewer> {
        match self {
            ViewerType::Nothing => Box::new(NoView::new(message_sender)),
            ViewerType::Editor => Box::new(EditorView::new(message_sender)),
            ViewerType::Preview => Box::new(RenderedView::new(message_sender)),
        }
    }
}

struct NoView {
    message_sender: Sender<EditorMessage>,
}

impl NoView {
    fn new(message_sender: Sender<EditorMessage>) -> Self {
        Self { message_sender }
    }
}

impl View for NoView {
    fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()> {
        ui.vertical_centered(|ui| {
            ui.label("Open or create a note with cmd + O");
        });
        Ok(())
    }
}

impl NoteViewer for NoView {
    fn get_type(&self) -> ViewerType {
        ViewerType::Nothing
    }

    fn load_content(&mut self, _text: String) {}

    fn manage_keys(&mut self, _ctx: &egui::Context) {}

    fn update(&mut self, _ctx: &egui::Context) -> anyhow::Result<()> {
        Ok(())
    }

    fn should_save(&self) -> bool {
        false
    }

    fn get_text(&self) -> String {
        "".to_string()
    }
}
