use eframe::egui;

use super::EditorMessage;

pub mod editor_view;
mod highlighter;
pub mod rendered_view;

pub const ID_VIEWER: &str = "Note Editor";

#[derive(Debug, Clone)]
pub enum ViewerType {
    Editor,
    Rendered,
}

pub trait NoteViewer {
    fn view(&mut self, text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool>;
    fn init(&mut self, text: String);
    fn manage_keys(&mut self, ctx: &egui::Context) -> Option<EditorMessage>;
}
