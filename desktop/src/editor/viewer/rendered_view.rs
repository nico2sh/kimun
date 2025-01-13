use crossbeam_channel::Sender;
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use log::error;

use crate::{editor::NoteViewer, View};

use super::{EditorMessage, ViewerType};

pub struct RenderedView {
    message_sender: Sender<EditorMessage>,
    cache: CommonMarkCache,
    content: String,
}

impl RenderedView {
    pub(super) fn new(message_sender: Sender<EditorMessage>) -> Self {
        let cache = CommonMarkCache::default();
        let content = String::new();
        Self {
            message_sender,
            cache,
            content,
        }
    }
}

impl NoteViewer for RenderedView {
    fn get_type(&self) -> ViewerType {
        ViewerType::Preview
    }

    fn load_content(&mut self, text: String) {
        self.content = text;
    }

    fn manage_keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| {
            input.consume_key(
                egui::Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
                egui::Key::P,
            )
        }) {
            if let Err(e) = self.message_sender.send(EditorMessage::ShowEditor) {
                error!("Error sending change view message: {}", e);
            };
        }
    }

    fn update(&mut self, _ctx: &eframe::egui::Context) -> anyhow::Result<()> {
        Ok(())
    }

    fn should_save(&self) -> bool {
        false
    }

    fn get_text(&self) -> String {
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
