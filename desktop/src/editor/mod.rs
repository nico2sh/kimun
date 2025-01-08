mod data;
mod highlighter;
mod modals;
mod parser;

use std::sync::mpsc::Receiver;

use anyhow::bail;
use data::EditorData;
use eframe::egui;
use highlighter::MemoizedNoteHighlighter;
use modals::{ModalManager, Modals};
use notes_core::{nfs::NotePath, NoteVault};

use super::{settings::Settings, View};

pub struct Editor {
    data: EditorData,
    modal_manager: ModalManager,
    message_receiver: Receiver<EditorMessage>,
    current_directory: NotePath,
    // selector: Option<FilteredList<SelectorEntry, SearchResult>>,
    highlighter: MemoizedNoteHighlighter,
}

impl Editor {
    pub fn new(settings: &Settings) -> anyhow::Result<Self> {
        if let Some(workspace_dir) = &settings.workspace_dir {
            let (sender, receiver) = std::sync::mpsc::channel();
            Ok(Self {
                data: EditorData::new(workspace_dir)?,
                modal_manager: ModalManager::new(NoteVault::new(workspace_dir)?, sender),
                message_receiver: receiver,
                current_directory: settings.last_path.clone(),
                highlighter: MemoizedNoteHighlighter::default(),
            })
        } else {
            bail!("Path not provided")
        }
    }

    fn get_editor(&mut self, ui: &mut egui::Ui) {
        let mut layouter = |ui: &egui::Ui, easymark: &str, wrap_width: f32| {
            let mut layout_job = self.highlighter.highlight(ui.style(), easymark);
            layout_job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(layout_job))
        };

        let output = egui::TextEdit::multiline(&mut self.data.text)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace) // for cursor height
            .layouter(&mut layouter);
        let response = ui.add_sized(ui.available_size(), output);

        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
            if let Some(mut ccursor_range) = state.cursor.char_range() {
                // let any_change = shortcuts(ui, code, &mut ccursor_range);
                // if any_change {
                //     state.cursor.set_char_range(Some(ccursor_range));
                //     state.store(ui.ctx(), response.id);
                // }
            }
        };
    }
}

impl View for Editor {
    fn view(&mut self, ui: &mut eframe::egui::Ui) -> anyhow::Result<()> {
        if ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O))
        {
            self.modal_manager
                .set_modal(Modals::VaultBrowse(NotePath::root()));
        }
        if ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        {
            self.modal_manager.set_modal(Modals::VaultSearch);
        }

        self.modal_manager.view(ui)?;

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.get_editor(ui);
        });

        while let Ok(message) = self.message_receiver.try_recv() {
            match message {
                EditorMessage::OpenNote(note_path) => {
                    let content = self.data.note.load_note(&note_path).unwrap();
                    self.data.text = content;
                    self.data.note_path = Some(note_path.clone());
                    self.current_directory = note_path.get_parent_path().0;
                    self.modal_manager.close_modal();
                    ui.ctx().request_repaint();
                }
            }
        }

        Ok(())
    }
}

pub(crate) enum EditorMessage {
    OpenNote(NotePath),
}
