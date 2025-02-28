use crossbeam_channel::Sender;
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use kimun_core::{
    nfs::VaultPath,
    note::{Link, LinkType, NoteDetails},
};
use log::{debug, error};

use crate::editor::NoteViewer;

use super::EditorMessage;

pub struct RenderedView {
    path: VaultPath,
    message_sender: Sender<EditorMessage>,
    cache: CommonMarkCache,
    link_hooks: Vec<String>,
    markdown_text: String,
}

impl RenderedView {
    pub fn new(path: &VaultPath, message_sender: Sender<EditorMessage>) -> Self {
        let cache = CommonMarkCache::default();
        Self {
            path: path.to_owned(),
            message_sender,
            cache,
            link_hooks: vec![],
            markdown_text: String::new(),
        }
    }

    fn add_link_hooks(&mut self, links: Vec<Link>) {
        for link in &links {
            if let LinkType::Note(name) = &link.ltype {
                let path_string = name.to_string();
                self.cache.add_link_hook(&path_string);
                self.link_hooks.push(path_string);
            }
        }
    }
}

impl NoteViewer for RenderedView {
    fn view(&mut self, _text: &mut String, ui: &mut egui::Ui) -> anyhow::Result<bool> {
        for link in &self.link_hooks {
            if Some(true) == self.cache.get_link_hook(link) {
                debug!("Clicked on {}", link);
                if let Err(e) = self
                    .message_sender
                    .send(EditorMessage::OpenCreateOrSearchNote(link.to_owned()))
                {
                    error!("Error sending a message to open a note: {}", e);
                }
            }
        }
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
        let details = NoteDetails::new(&self.path, text);
        let ml = details.get_markdown_and_links();
        self.markdown_text = ml.text;
        self.add_link_hooks(ml.links);
    }
}
