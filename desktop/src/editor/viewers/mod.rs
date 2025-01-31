use editor_view::EditorView;
use eframe::egui;
use kimun_core::nfs::VaultPath;
use rendered_view::RenderedView;

use super::EditorMessage;

mod editor_view;
mod highlighter;
mod rendered_view;

pub const ID_VIEWER: &str = "Note Editor";

#[derive(Debug, Clone)]
pub enum ViewerType {
    Nothing,
    Editor(VaultPath),
    Rendered(VaultPath),
}

impl ViewerType {
    pub fn get_view(&self) -> Box<dyn NoteViewer> {
        match self {
            ViewerType::Nothing => Box::new(NoView::new()),
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

pub struct NoView {}

impl NoView {
    pub fn new() -> Self {
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
