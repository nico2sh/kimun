use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use kimun_core::nfs::VaultPath;

use crate::editor::NoteViewer;

use super::EditorMessage;

pub struct RenderedView {
    path: VaultPath,
    cache: CommonMarkCache,
    markdown_text: String,
}

impl RenderedView {
    pub fn new(path: &VaultPath) -> Self {
        let cache = CommonMarkCache::default();
        Self {
            path: path.to_owned(),
            cache,
            markdown_text: String::new(),
        }
    }
}

impl NoteViewer for RenderedView {
    fn view(&mut self, _text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool> {
        egui::TopBottomPanel::top("title")
            .resizable(false)
            .min_height(32.0)
            .show_inside(ui, |ui| {
                ui.vertical(|ui| {
                    ui.heading(self.path.to_string());
                })
            });
        let _common_mark_viewer = CommonMarkViewer::new()
            .show(ui, &mut self.cache, &self.markdown_text)
            .response;
        Ok(false)
    }

    fn manage_keys(&mut self, ctx: &egui::Context) -> Option<EditorMessage> {
        if ctx.input_mut(|input| {
            input.consume_key(
                egui::Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
                egui::Key::Space,
            )
        }) {
            Some(EditorMessage::SwitchNoteViewer(super::ViewerType::Editor))
        } else {
            None
        }
    }

    fn init(&mut self, text: String) {
        // TODO: do any processing to render the text properly
        self.markdown_text = text;
    }
}
