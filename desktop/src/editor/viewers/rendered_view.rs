use crossbeam_channel::Sender;
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use kimun_core::note::{Link, LinkType, NoteDetails};
use log::{debug, error};

use crate::editor::NoteViewer;

use super::EditorMessage;

pub struct RenderedView {
    markdown_text: String,
    message_sender: Sender<EditorMessage>,
    cache: CommonMarkCache,
    link_hooks: Vec<String>,
}

impl RenderedView {
    pub fn new(message_sender: Sender<EditorMessage>) -> Self {
        let cache = CommonMarkCache::default();
        Self {
            message_sender,
            cache,
            link_hooks: vec![],
            markdown_text: String::new(),
        }
    }
}

fn get_link_hooks(links: Vec<Link>) -> Vec<String> {
    let mut hooks = vec![];
    for link in &links {
        if let LinkType::Note(name) = &link.ltype {
            let path_string = link.link.clone();
            // cache.add_link_hook(&path_string);
            hooks.push(path_string);
        }
    }
    hooks
}

impl NoteViewer for RenderedView {
    fn view(&mut self, note_details: &mut NoteDetails, ui: &mut egui::Ui) -> anyhow::Result<bool> {
        for path_label in &self.link_hooks {
            if Some(true) == self.cache.get_link_hook(path_label) {
                debug!("Clicked on {}", path_label);
                if let Err(e) = self
                    .message_sender
                    .send(EditorMessage::OpenCreateOrSearchNote(path_label.to_owned()))
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
                    ui.heading(note_details.path.to_string());
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

    fn init(&mut self, note_details: &NoteDetails) {
        self.link_hooks.clear();
        self.cache.link_hooks_clear();
        let ml = note_details.get_markdown_and_links();
        self.markdown_text = ml.text;
        // let link_hooks = get_link_hooks(ml.links);

        for link in ml.links {
            if let LinkType::Note(_path) = link.ltype {
                let path_string = link.link.clone();
                // cache.add_link_hook(&path_string);
                self.link_hooks.push(path_string.clone());
                self.cache.add_link_hook(path_string);
            }
        }
    }
}
