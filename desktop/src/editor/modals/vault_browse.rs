use eframe::egui;
use log::debug;
use notes_core::{nfs::NotePath, NoteVault, SearchResult, VaultBrowseOptionsBuilder};
use rayon::slice::ParallelSliceMut;

use crate::icons;

use super::{
    filtered_list::{FilteredListFunctionMessage, FilteredListFunctions, ListElement},
    EditorMessage,
};

#[derive(Clone)]
pub struct VaultBrowseFunctions {
    path: NotePath, // add code here
    vault: NoteVault,
}

impl VaultBrowseFunctions {
    pub fn new(path: NotePath, vault: NoteVault) -> Self {
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
            results.push(entry.into());
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
        filtered.par_sort_by(|a, b| a.get_sort_string().cmp(&b.get_sort_string()));

        debug!("filtered {} values", filtered.len());
        filtered
    }

    fn on_entry(&mut self, element: &SelectorEntry) -> Option<FilteredListFunctionMessage> {
        match element.entry_type {
            SelectorEntryType::Note { title: _ } => Some(FilteredListFunctionMessage::ToEditor(
                EditorMessage::OpenNote(element.path.clone()),
            )),
            SelectorEntryType::Directory => {
                let directory = element.path.clone();
                debug!("new path: {}", directory);
                self.path = directory;
                Some(FilteredListFunctionMessage::ResetState)
            }
            SelectorEntryType::Attachment => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SelectorEntry {
    pub path: NotePath,
    pub path_str: String,
    pub entry_type: SelectorEntryType,
}

#[derive(Clone, Debug)]
pub enum SelectorEntryType {
    Note { title: String },
    Directory,
    Attachment,
}

impl From<SearchResult> for SelectorEntry {
    fn from(value: SearchResult) -> Self {
        match value {
            SearchResult::Note(note_details) => SelectorEntry {
                path: note_details.path.clone(),
                path_str: note_details.path.get_parent_path().1,
                entry_type: SelectorEntryType::Note {
                    title: note_details.get_title(),
                },
            },
            SearchResult::Directory(directory_details) => SelectorEntry {
                path: directory_details.path.clone(),
                path_str: directory_details.path.get_parent_path().1,
                entry_type: SelectorEntryType::Directory,
            },
            SearchResult::Attachment(path) => SelectorEntry {
                path: path.clone(),
                path_str: path.get_parent_path().1,
                entry_type: SelectorEntryType::Attachment,
            },
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
        }
    }
}
impl SelectorEntry {
    pub fn get_sort_string(&self) -> String {
        match &self.entry_type {
            SelectorEntryType::Note { title: _ } => format!("2{}", self.path),
            SelectorEntryType::Directory => format!("1{}", self.path),
            SelectorEntryType::Attachment => format!("3{}", self.path),
        }
    }
}

impl AsRef<str> for SelectorEntry {
    fn as_ref(&self) -> &str {
        &self.path_str
    }
}
