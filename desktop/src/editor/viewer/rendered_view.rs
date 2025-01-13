use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::{editor::NoteViewer, View};

use super::ViewerType;

pub struct RenderedView {
    cache: CommonMarkCache,
    content: String,
}

impl RenderedView {
    pub fn new() -> Self {
        let cache = CommonMarkCache::default();
        let content = String::new();
        Self { cache, content }
    }
}

impl NoteViewer for RenderedView {
    fn get_type(&self) -> ViewerType {
        ViewerType::Preview
    }

    fn load_content(&mut self, text: String) {
        self.content = text;
    }

    fn manage_keys(&mut self, _ctx: &eframe::egui::Context) {}

    fn update(&mut self, _ctx: &eframe::egui::Context) -> anyhow::Result<()> {
        Ok(())
    }

    fn should_save(&self) -> bool {
        false
    }

    fn get_content(&self) -> String {
        self.content.clone()
    }
}

impl View for RenderedView {
    fn view(&mut self, ui: &mut eframe::egui::Ui) -> anyhow::Result<()> {
        let _common_mark_viewer = CommonMarkViewer::new()
            .show(ui, &mut self.cache, &self.content)
            .response;
        Ok(())
    }
}
