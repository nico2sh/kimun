use eframe::egui;
use kimun_core::note::NoteDetails;

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
    fn view(&mut self, note: &mut NoteDetails, ui: &mut egui::Ui) -> anyhow::Result<bool>;
    fn init(&mut self, details: &NoteDetails);
    fn manage_keys(&mut self, ctx: &egui::Context) -> Option<EditorMessage>;
}
