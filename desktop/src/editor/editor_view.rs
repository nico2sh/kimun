use std::path::Path;

use eframe::egui;

use crate::View;

use super::{data::EditorData, highlighter::MemoizedNoteHighlighter, NoteViewer, ID_VIEWER};

pub struct EditorView {
    text: String,
    changed: bool,
    highlighter: MemoizedNoteHighlighter,
}

impl EditorView {
    pub fn new() -> Self {
        let highlighter = MemoizedNoteHighlighter::default();
        Self {
            text: String::new(),
            changed: false,
            highlighter,
        }
    }
}

impl NoteViewer for EditorView {
    fn manage_keys(&mut self, _ctx: &egui::Context) {
        // TODO: Editor specific hot keys
    }

    fn update(&mut self, _ctx: &egui::Context) -> anyhow::Result<()> {
        // What happens when we update
        Ok(())
    }

    fn load_content(&mut self, text: String) {
        self.text = text;
        self.changed = false;
    }

    fn should_save(&self) -> bool {
        self.changed
    }

    fn get_content(&self) -> String {
        self.text.clone()
    }
}

impl View for EditorView {
    fn view(&mut self, ui: &mut eframe::egui::Ui) -> anyhow::Result<()> {
        let mut layouter = |ui: &egui::Ui, easymark: &str, wrap_width: f32| {
            let mut layout_job = self.highlighter.highlight(ui.style(), easymark);
            layout_job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(layout_job))
        };

        let output = egui::TextEdit::multiline(&mut self.text)
            .code_editor()
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace) // for cursor height
            .layouter(&mut layouter)
            .id(ID_VIEWER.into());
        let response = ui.add_sized(ui.available_size(), output);

        if response.changed() {
            self.changed = true;
        }

        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
            if let Some(mut ccursor_range) = state.cursor.char_range() {
                // let any_change = shortcuts(ui, code, &mut ccursor_range);
                // if any_change {
                //     state.cursor.set_char_range(Some(ccursor_range));
                //     state.store(ui.ctx(), response.id);
                // }
            }
        };
        Ok(())
    }
}
