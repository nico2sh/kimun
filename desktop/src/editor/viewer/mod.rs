use editor_view::EditorView;
use eframe::egui;
use rendered_view::RenderedView;

use crate::View;

pub mod editor_view;
mod highlighter;
pub mod rendered_view;

pub const ID_VIEWER: &str = "Note Editor";

pub enum ViewerType {
    Editor,
    Preview,
}

pub trait NoteViewer: View {
    fn get_type(&self) -> ViewerType;
    fn load_content(&mut self, text: String);
    fn manage_keys(&mut self, ctx: &egui::Context);
    fn update(&mut self, ctx: &egui::Context) -> anyhow::Result<()>;
    fn should_save(&self) -> bool;
    fn get_content(&self) -> String;
}

impl ViewerType {
    pub fn new_view(&self) -> Box<dyn NoteViewer> {
        match self {
            ViewerType::Editor => Box::new(EditorView::new()),
            ViewerType::Preview => Box::new(RenderedView::new()),
        }
    }
}
