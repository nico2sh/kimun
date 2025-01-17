use crossbeam_channel::Sender;
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use log::error;

use crate::editor::NoteViewer;

use super::EditorMessage;

pub struct RenderedView {
    message_sender: Sender<EditorMessage>,
    cache: CommonMarkCache,
}

impl RenderedView {
    pub(super) fn new(message_sender: Sender<EditorMessage>) -> Self {
        let cache = CommonMarkCache::default();
        Self {
            message_sender,
            cache,
        }
    }
}

impl NoteViewer for RenderedView {
    fn view(&mut self, text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool> {
        let _common_mark_viewer = CommonMarkViewer::new()
            .show(ui, &mut self.cache, text)
            .response;
        Ok(false)
    }

    fn manage_keys(&mut self, ctx: &egui::Context) {
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
            if let Err(e) = self
                .message_sender
                .send(EditorMessage::SwitchNoteViewer(super::ViewerType::Editor))
            {
                error!("Error sending change view message: {}", e);
            };
        }
    }

    fn init(&mut self, _text: String) {}
}
