use crossbeam_channel::Sender;
use eframe::egui;
use log::error;

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
    fn view(&mut self, text: &mut String, ui: &mut eframe::egui::Ui) -> anyhow::Result<()> {
        let mut layouter = |ui: &egui::Ui, easymark: &str, wrap_width: f32| {
            let mut layout_job = self.highlighter.highlight(ui.style(), easymark);
            layout_job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(layout_job))
        };

        let output = egui::TextEdit::multiline(text)
            .code_editor()
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace) // for cursor height
            .layouter(&mut layouter)
            .id(ID_VIEWER.into());
        ui.add_sized(ui.available_size(), output);

        // if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
        //     if let Some(mut ccursor_range) = state.cursor.char_range() {
        //         let any_change = shortcuts(ui, code, &mut ccursor_range);
        //         if any_change {
        //             state.cursor.set_char_range(Some(ccursor_range));
        //             state.store(ui.ctx(), response.id);
        //         }
        //     }
        // };
        Ok(())
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
            if let Err(e) = self
                .message_sender
                .send(EditorMessage::SwitchNoteViewer(super::ViewerType::Preview))
            {
                error!("Error sending change view message: {}", e);
            };
        }
    }
}
