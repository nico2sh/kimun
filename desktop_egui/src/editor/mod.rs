mod data;
mod highlighter;
mod parser;

use std::sync::Arc;

use core_notes::{
    error::NoteInitError,
    nfs::{EntryData, NoteEntry, NotePath},
    NoteVault, NotesGetterOptions,
};
use data::EditorData;
use eframe::egui;
use highlighter::MemoizedNoteHighlighter;

use super::{
    filtered_list::{
        row::{RowItem, RowMessage},
        FilteredList,
    },
    icons,
    settings::Settings,
    Message, View,
};

pub struct Editor {
    data: EditorData,
    current_directory: NotePath,
    selector: Option<FilteredList<NoteEntry>>,
    highlighter: MemoizedNoteHighlighter,
}

impl Editor {
    pub fn new(settings: &Settings) -> anyhow::Result<Self> {
        if let Some(workspace_dir) = &settings.workspace_dir {
            Ok(Self {
                data: EditorData::new(workspace_dir)?,
                current_directory: settings.last_path.clone(),
                selector: None,
                highlighter: MemoizedNoteHighlighter::default(),
            })
        } else {
            Err(NoteInitError::PathNotProvided)?
        }
    }

    fn list_path(
        note: &Arc<NoteVault>,
        filtered_list: &mut FilteredList<NoteEntry>,
        search_path: &NotePath,
    ) {
        filtered_list.clear();
        let channel = filtered_list.get_channel_rows();
        let note = Arc::clone(note);
        let search_path = if search_path.is_note() {
            search_path.get_parent_path().0
        } else {
            search_path.clone()
        };

        std::thread::spawn(move || {
            let _ = note.get_notes(
                search_path,
                NotesGetterOptions::default().set_sender(channel),
            );
        });
        filtered_list.request_focus();
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

impl RowItem for NoteEntry {
    fn get_label(&self, ui: &mut egui::Ui) -> egui::Response {
        let icon = match &self.data {
            EntryData::Note(_note_data) => icons::NOTE,
            EntryData::Directory(_directory_data) => icons::DIRECTORY,
            EntryData::Attachment => icons::ATTACHMENT,
        };
        ui.label(format!("{}   {}", icon, self.path_string))
    }

    fn get_sort_string(&self) -> String {
        match &self.data {
            EntryData::Note(_note_data) => format!("2{}", self.path_string),
            EntryData::Directory(_directory_data) => {
                format!("1{}", self.path_string)
            }
            EntryData::Attachment => format!("3{}", self.path_string),
        }
    }

    fn get_message(&self) -> RowMessage {
        match &self.data {
            EntryData::Note(note_data) => RowMessage::OpenNote(note_data.path.clone()),
            EntryData::Directory(directory_data) => {
                RowMessage::OpenDirectory(directory_data.path.clone())
            }
            EntryData::Attachment => RowMessage::Nothing,
        }
    }
}

impl View for Editor {
    fn view(&mut self, ui: &mut eframe::egui::Ui) -> Message {
        if ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O))
        {
            let mut filtered_list = FilteredList::new(vec![]);
            Self::list_path(&self.data.note, &mut filtered_list, &self.current_directory);
            self.selector = Some(filtered_list);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.get_editor(ui);
            // let output = egui::TextEdit::multiline(&mut self.data.text)
            //     .desired_width(f32::INFINITY)
            //     .code_editor()
            //     .lock_focus(true);
            // let res = ui.add_sized(ui.available_size(), output);
        });
        if let Some(filtered_list) = self.selector.as_mut() {
            let message = filtered_list.view(ui);

            match message {
                Message::None => {}
                Message::SelectionMessage(row_message) => match row_message {
                    RowMessage::Nothing => {}
                    RowMessage::OpenNote(note_path) => {
                        let content = self.data.note.load_note(note_path.clone()).unwrap();
                        self.data.text = content;
                        self.data.note_path = Some(note_path.clone());
                        self.current_directory = note_path.get_parent_path().0;
                        self.selector = None;
                    }
                    RowMessage::OpenDirectory(directory_path) => {
                        Self::list_path(&self.data.note, filtered_list, &directory_path);
                    }
                },
                Message::CloseWindow => self.selector = None,
            }
        }

        Message::None
    }
}
