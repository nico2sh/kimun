use eframe::egui;
use log::{debug, error};
use notes_core::{nfs::VaultPath, NoteDetails, NoteVault, SearchResult, VaultBrowseOptionsBuilder};
use rayon::slice::ParallelSliceMut;

use crate::icons;

use super::{
    filtered_list::{FilteredListFunctionMessage, FilteredListFunctions, ListElement, StateData},
    EditorMessage,
};

#[derive(Clone)]
pub struct VaultBrowseFunctions {
    path: VaultPath, // add code here
    vault: NoteVault,
}

impl VaultBrowseFunctions {
    pub fn new(path: VaultPath, vault: NoteVault) -> Self {
        Self { path, vault }
    }
}

impl FilteredListFunctions<Vec<SelectorEntry>, SelectorEntry> for VaultBrowseFunctions {
    fn init(&self) -> Vec<SelectorEntry> {
        let search_path = if self.path.is_note() {
            self.path.get_parent_path().0
        } else {
            self.path.to_owned()
        };
        let (browse_options, receiver) = VaultBrowseOptionsBuilder::new(&search_path).build();

        debug!("Retreiving notes for dialog");
        self.vault
            .browse_vault(browse_options)
            .expect("Error getting notes");

        let mut results = vec![];
        while let Ok(entry) = receiver.recv() {
            match &entry {
                SearchResult::Note(_note_details) => results.push(entry.into()),
                SearchResult::Directory(directory_details) => {
                    if directory_details.path != self.path {
                        results.push(entry.into());
                    }
                }
                SearchResult::Attachment(_note_path) => {}
            }
        }
        debug!("Retrieved {} elements", results.len());
        results
    }

    fn filter<S: AsRef<str>>(
        &self,
        filter_text: S,
        data: &Vec<SelectorEntry>,
    ) -> Vec<SelectorEntry> {
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut filtered = nucleo::pattern::Pattern::parse(
            filter_text.as_ref(),
            nucleo::pattern::CaseMatching::Ignore,
            nucleo::pattern::Normalization::Smart,
        )
        .match_list(data, &mut matcher)
        .iter()
        .map(|e| e.0.to_owned())
        .collect::<Vec<SelectorEntry>>();
        if self.path != VaultPath::root() {
            filtered.push(SelectorEntry::up_dir(&self.path));
        }
        filtered.par_sort_by(|a, b| a.get_sort_string().cmp(&b.get_sort_string()));

        debug!("filtered {} values", filtered.len());
        filtered
    }

    fn on_entry(&self, element: &SelectorEntry) -> Option<FilteredListFunctionMessage<Self>> {
        match element.entry_type {
            SelectorEntryType::Note { title: _ } => Some(FilteredListFunctionMessage::ToEditor(
                EditorMessage::OpenNote(element.path.clone()),
            )),
            SelectorEntryType::Directory => {
                let directory = element.path.clone();
                debug!("new path: {}", directory);
                let new_one = Self {
                    path: directory,
                    vault: self.vault.clone(),
                };
                // self.path = directory;
                Some(FilteredListFunctionMessage::ResetState(new_one))
            }
            SelectorEntryType::Attachment => None,
            SelectorEntryType::NewNote => Some(FilteredListFunctionMessage::ToEditor(
                EditorMessage::NewNote(element.path.clone()),
            )),
        }
    }

    fn header_element(&self, state_data: &StateData<SelectorEntry>) -> Option<SelectorEntry> {
        if !state_data.filter_text.is_empty() {
            Some(SelectorEntry::new_note(&self.path, &state_data.filter_text))
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct VaultSearchFunctions {
    vault: NoteVault,
}

impl VaultSearchFunctions {
    pub fn new(vault: NoteVault) -> Self {
        Self { vault }
    }
}

impl FilteredListFunctions<(), NoteDetails> for VaultSearchFunctions {
    fn init(&self) {}

    fn filter<S: AsRef<str>>(&self, filter_text: S, _provider: &()) -> Vec<NoteDetails> {
        if filter_text.as_ref().is_empty() {
            return vec![];
        }

        match self.vault.search_notes(filter_text, true) {
            Ok(result) => result,
            Err(e) => {
                error!("Error searching notes: {}", e);
                vec![]
            }
        }
    }

    fn on_entry(&self, element: &NoteDetails) -> Option<FilteredListFunctionMessage<Self>> {
        Some(FilteredListFunctionMessage::ToEditor(
            EditorMessage::OpenNote(element.path.clone()),
        ))
    }

    fn header_element(&self, _state_data: &StateData<NoteDetails>) -> Option<NoteDetails> {
        None
    }
}

impl ListElement for NoteDetails {
    fn draw_element(&self, ui: &mut egui::Ui) -> egui::Response {
        let icon = icons::NOTE;
        let path = self.path.to_owned();
        ui.label(format!("{}  {}\n{}", icon, self.get_title(), path))
    }
}

#[derive(Clone, Debug)]
pub struct SelectorEntry {
    pub path: VaultPath,
    pub path_str: String,
    pub search_str: String,
    pub entry_type: SelectorEntryType,
}

#[derive(Clone, Debug)]
pub enum SelectorEntryType {
    Note { title: String },
    Directory,
    Attachment,
    NewNote,
}

impl From<SearchResult> for SelectorEntry {
    fn from(value: SearchResult) -> Self {
        match value {
            SearchResult::Note(note_details) => {
                let title = note_details.get_title();
                let path = note_details.path;
                let file_name = path.get_parent_path().1;
                let file_name_no_ext = file_name.strip_suffix(".md").unwrap_or(file_name.as_str());
                let search_str = if title.contains(file_name_no_ext) {
                    title.clone()
                } else {
                    format!("{} {}", title, file_name_no_ext)
                };
                SelectorEntry {
                    path: path.clone(),
                    path_str: path.get_parent_path().1,
                    search_str,
                    entry_type: SelectorEntryType::Note { title },
                }
            }
            SearchResult::Directory(directory_details) => {
                let name = directory_details.path.get_parent_path().1;
                SelectorEntry {
                    path: directory_details.path.clone(),
                    path_str: name.clone(),
                    search_str: name,
                    entry_type: SelectorEntryType::Directory,
                }
            }
            SearchResult::Attachment(path) => {
                let name = path.get_parent_path().1;
                SelectorEntry {
                    path: path.clone(),
                    path_str: name.clone(),
                    search_str: name,
                    entry_type: SelectorEntryType::Attachment,
                }
            }
        }
    }
}

impl ListElement for SelectorEntry {
    fn draw_element(&self, ui: &mut egui::Ui) -> egui::Response {
        match &self.entry_type {
            SelectorEntryType::Note { title } => {
                let icon = icons::NOTE;
                let path = self.path_str.to_owned();
                ui.label(format!("{}  {}\n{}", icon, title, path))
                // let mut job = egui::text::LayoutJob::default();
                // job.append(
                //     format!("{}   {}\n", icon, title).as_str(),
                //     0.0,
                //     egui::TextFormat::default(),
                // );
                // job.append(
                //     path.as_str(),
                //     0.0,
                //     egui::TextFormat {
                //         italics: true,
                //         ..Default::default()
                //     },
                // );
                // ui.label(job)
            }
            SelectorEntryType::Directory => {
                let icon = icons::DIRECTORY;
                let path = self.path_str.to_owned();
                ui.label(format!("{}  {}", icon, path))
                // let mut job = egui::text::LayoutJob::default();
                // job.append(
                //     format!("{}   {}", icon, self.path_str).as_str(),
                //     0.0,
                //     egui::TextFormat::default(),
                // );
                // ui.label(job)
            }
            SelectorEntryType::Attachment => {
                let icon = icons::ATTACHMENT;
                let path = self.path_str.to_owned();
                ui.label(format!("{}  {}", icon, path))
                // let mut job = egui::text::LayoutJob::default();
                // job.append(
                //     format!("{}   {}", icon, self.path_str).as_str(),
                //     0.0,
                //     egui::TextFormat::default(),
                // );
                // ui.label(job)
            }
            SelectorEntryType::NewNote => {
                let icon = icons::NOTE;
                let path = self.path_str.to_owned();
                let response = ui.label(format!("{}  Create new note as `{}`", icon, path));
                ui.style().interact(&response);
                response
            }
        }
    }
}

impl SelectorEntry {
    fn up_dir(from_path: &VaultPath) -> Self {
        let parent = from_path.get_parent_path().0;
        Self {
            path: parent,
            path_str: "..".to_string(),
            search_str: ".. up".to_string(),
            entry_type: SelectorEntryType::Directory,
        }
    }

    fn new_note(base_path: &VaultPath, note_text: &str) -> Self {
        let file_name = VaultPath::file_from(note_text);
        let path = base_path.append(&file_name);

        Self {
            path_str: path.to_string(),
            path,
            search_str: "New Note".to_string(),
            entry_type: SelectorEntryType::NewNote,
        }
    }

    fn get_sort_string(&self) -> String {
        match &self.entry_type {
            SelectorEntryType::Note { title: _ } => format!("2{}", self.path),
            SelectorEntryType::Directory => format!("1{}", self.path),
            SelectorEntryType::Attachment => format!("3{}", self.path),
            SelectorEntryType::NewNote => "0".to_string(),
        }
    }
}

impl AsRef<str> for SelectorEntry {
    fn as_ref(&self) -> &str {
        &self.search_str
    }
}
