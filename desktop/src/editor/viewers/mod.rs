use editor_view::EditorView;
use eframe::egui;
use kimun_core::nfs::VaultPath;
use rendered_view::RenderedView;

use super::EditorMessage;

pub mod editor_view;
mod highlighter;
mod rendered_view;

pub const ID_VIEWER: &str = "Note Editor";

#[derive(Debug, Clone)]
pub enum ViewerType {
    Editor(VaultPath),
    Rendered(VaultPath),
}

impl ViewerType {
    pub fn get_view(&self) -> Box<dyn NoteViewer> {
        match self {
            ViewerType::Editor(vault_path) => Box::new(EditorView::new(vault_path)),
            ViewerType::Rendered(vault_path) => Box::new(RenderedView::new(vault_path)),
        }
    }
}

pub trait NoteViewer {
    fn view(&mut self, text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool>;
    fn init(&mut self, text: String);
    fn manage_keys(&mut self, ctx: &egui::Context) -> Option<EditorMessage>;
    fn view_change_on_content(&self, vault_path: &VaultPath) -> Box<dyn NoteViewer>;
}
