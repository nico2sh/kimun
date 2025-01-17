use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use log::{debug, error};
use notes_core::NoteVault;

use crate::editor::NoteViewer;

use super::{highlighter::MemoizedNoteHighlighter, EditorMessage, ID_VIEWER};

pub struct EditorView {
    message_sender: Sender<EditorMessage>,
    highlighter: MemoizedNoteHighlighter,
    title: Arc<Mutex<String>>,
    title_update: Sender<String>,
}

impl EditorView {
    pub(super) fn new(message_sender: Sender<EditorMessage>) -> Self {
        let highlighter = MemoizedNoteHighlighter::default();
        let title = Arc::new(Mutex::new(String::new()));
        let (title_update, receiver) = crossbeam_channel::unbounded::<String>();
        let editor_view = Self {
            message_sender,
            highlighter,
            title,
            title_update,
        };
        editor_view.title_update_loop(receiver);
        editor_view
    }

    fn title_update_loop(&self, receiver: Receiver<String>) {
        let title_to_update = self.title.clone();
        std::thread::spawn(move || {
            while let Ok(text) = receiver.recv() {
                let title = NoteVault::get_title(text);
                *title_to_update.lock().unwrap() =
                    title.unwrap_or_else(|| "<Untitled>".to_string());
            }
        });
    }
}

impl NoteViewer for EditorView {
    fn view(&mut self, text: &mut String, ui: &mut eframe::egui::Ui) -> anyhow::Result<bool> {
        let mut layouter = |ui: &egui::Ui, easymark: &str, wrap_width: f32| {
            let mut layout_job = self.highlighter.highlight(ui.style(), easymark);
            layout_job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(layout_job))
        };

        let title = self.title.lock().unwrap().clone();
        egui::TopBottomPanel::top("title")
            .resizable(false)
            .min_height(48.0)
            .show_inside(ui, |ui| {
                ui.vertical(|ui| {
                    ui.heading(title);
                })
            });
        let output = egui::TextEdit::multiline(text)
            .font(egui::TextStyle::Monospace) // for cursor height
            .code_editor()
            .lock_focus(true)
            .cursor_at_end(true)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter)
            .id(ID_VIEWER.into());
        let response = ui.add_sized(ui.available_size(), output);

        let text_edit_id = response.id;
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), text_edit_id) {
            if let Some(range) = state.cursor.char_range() {};
        };
        let changed = response.changed();
        if changed {
            debug!("Sending a title update message");
            if let Err(e) = self.title_update.send(text.clone()) {
                error!("Error sending an update to the title: {}", e);
            }
        }
        Ok(changed)
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
                .send(EditorMessage::SwitchNoteViewer(super::ViewerType::Preview))
            {
                error!("Error sending change view message: {}", e);
            };
        }
    }

    fn init(&mut self, text: String) {
        if let Err(e) = self.title_update.send(text) {
            error!("Error sending an init message for setting the title: {}", e);
        }
    }
}
