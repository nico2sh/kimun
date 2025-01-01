mod data;
mod highlighter;
mod parser;

use std::sync::Arc;

use anyhow::bail;
use data::EditorData;
use eframe::egui;
use highlighter::MemoizedNoteHighlighter;
use notes_core::{
    nfs::{NotePath, VaultEntry},
    NoteVault, SearchResult, VaultBrowseOptionsBuilder,
};

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
    selector: Option<FilteredList<SelectorEntry, SearchResult>>,
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
            bail!("Path not provided")
        }
    }

    fn list_path(
        vault: &Arc<NoteVault>,
        filtered_list: &mut FilteredList<SelectorEntry, SearchResult>,
        search_path: &NotePath,
    ) {
        filtered_list.clear();
        let search_path = if search_path.is_note() {
            search_path.get_parent_path().0
        } else {
            search_path.clone()
        };
        let (browse_options, receiver) = VaultBrowseOptionsBuilder::new(&search_path).build();
        filtered_list.set_channel_rows(receiver);
        let vault = Arc::clone(vault);

        std::thread::spawn(move || {
            vault
                .browse_vault(browse_options)
                .expect("Error getting notes");
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

#[derive(Clone)]
struct SelectorEntry {
    path: NotePath,
    path_str: String,
    entry_type: SelectorEntryType,
}

#[derive(Clone)]
enum SelectorEntryType {
    Note,
    Directory,
    Attachment,
}

impl From<SearchResult> for SelectorEntry {
    fn from(value: SearchResult) -> Self {
        match value {
            SearchResult::Note(note_details) => SelectorEntry {
                path: note_details.path.clone(),
                path_str: note_details.path.to_string(),
                entry_type: SelectorEntryType::Note,
            },
            SearchResult::Directory(directory_details) => SelectorEntry {
                path: directory_details.path.clone(),
                path_str: directory_details.path.to_string(),
                entry_type: SelectorEntryType::Directory,
            },
            SearchResult::Attachment(path) => SelectorEntry {
                path: path.clone(),
                path_str: path.to_string(),
                entry_type: SelectorEntryType::Attachment,
            },
        }
    }
}

impl RowItem for SelectorEntry {
    fn get_label(&self, ui: &mut egui::Ui) -> egui::Response {
        let icon = match &self.entry_type {
            SelectorEntryType::Note => icons::NOTE,
            SelectorEntryType::Directory => icons::DIRECTORY,
            SelectorEntryType::Attachment => icons::ATTACHMENT,
        };
        ui.label(format!("{}   {}", icon, self.path_str))
    }

    fn get_sort_string(&self) -> String {
        match &self.entry_type {
            SelectorEntryType::Note => format!("2{}", self.path_str),
            SelectorEntryType::Directory => format!("1{}", self.path_str),
            SelectorEntryType::Attachment => format!("3{}", self.path_str),
        }
    }

    fn get_message(&self) -> RowMessage {
        match &self.entry_type {
            SelectorEntryType::Note => RowMessage::OpenNote(self.path.clone()),
            SelectorEntryType::Directory => RowMessage::OpenDirectory(self.path.clone()),
            SelectorEntryType::Attachment => RowMessage::Nothing,
        }
    }
}

impl AsRef<str> for SelectorEntry {
    fn as_ref(&self) -> &str {
        &self.path_str
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
                        let content = self.data.note.load_note(&note_path).unwrap();
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
