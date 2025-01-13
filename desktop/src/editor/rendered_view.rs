use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::View;

pub struct RenderedView {
    cache: CommonMarkCache,
}

impl RenderedView {
    pub fn new() -> Self {
        let cache = CommonMarkCache::default();
        Self { cache }
    }
}

impl View for RenderedView {
    fn view(&mut self, ui: &mut eframe::egui::Ui) -> anyhow::Result<()> {
        let common_mark_viewer = CommonMarkViewer::new()
            .show(ui, &mut self.cache, "")
            .response;
        Ok(())
    }
}
