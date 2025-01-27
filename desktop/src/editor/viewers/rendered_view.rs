use crossbeam_channel::Sender;
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use kimun_core::nfs::VaultPath;
use log::error;

use crate::editor::NoteViewer;

use super::EditorMessage;

pub struct RenderedView {
    path: VaultPath,
    message_sender: Sender<EditorMessage>,
    cache: CommonMarkCache,
}

impl RenderedView {
    pub(super) fn new(message_sender: Sender<EditorMessage>, path: &VaultPath) -> Self {
        let cache = CommonMarkCache::default();
        Self {
            path: path.to_owned(),
            message_sender,
            cache,
        }
    }
}

impl NoteViewer for RenderedView {
    fn view(&mut self, text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool> {
        egui::TopBottomPanel::top("title")
            .resizable(false)
            .min_height(32.0)
            .show_inside(ui, |ui| {
                ui.vertical(|ui| {
                    ui.heading(self.path.to_string());
                })
            });
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
            if let Err(e) = self.message_sender.send(EditorMessage::SwitchNoteViewer(
                super::ViewerType::Editor(self.path.clone()),
            )) {
                error!("Error sending change view message: {}", e);
            };
        }
    }

    fn init(&mut self, _text: String) {}
}
