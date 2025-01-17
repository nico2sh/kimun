use crossbeam_channel::Sender;
use eframe::egui;
use log::{debug, error};

use crate::editor::NoteViewer;

use super::{highlighter::MemoizedNoteHighlighter, EditorMessage, ID_VIEWER};

pub struct EditorView {
    message_sender: Sender<EditorMessage>,
    highlighter: MemoizedNoteHighlighter,
}

impl EditorView {
    pub(super) fn new(message_sender: Sender<EditorMessage>) -> Self {
        let highlighter = MemoizedNoteHighlighter::default();
        Self {
            message_sender,
            highlighter,
        }
    }
}

impl NoteViewer for EditorView {
    fn view(&mut self, text: &mut String, ui: &mut eframe::egui::Ui) -> anyhow::Result<bool> {
        let mut layouter = |ui: &egui::Ui, easymark: &str, wrap_width: f32| {
            let mut layout_job = self.highlighter.highlight(ui.style(), easymark);
            layout_job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(layout_job))
        };

        // egui::TopBottomPanel::top("title")
        //     .resizable(false)
        //     .min_height(48.0)
        //     .show_inside(ui, |ui| {
        //         ui.vertical(|ui| {
        //             ui.heading("");
        //         })
        //     });
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
        Ok(response.changed())
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
}
