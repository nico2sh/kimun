use std::{thread::sleep, time::Duration};

use log::debug;
use notes_core::{nfs::NotePath, NoteVault, VaultBrowseOptionsBuilder};
use rayon::slice::ParallelSliceMut;

use super::{
    filtered_list::{
        FilteredListFunctionMessage, FilteredListFunctions, SelectorEntry, SelectorEntryType,
    },
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

impl FilteredListFunctions<Vec<SelectorEntry>, Vec<SelectorEntry>> for VaultBrowseFunctions {
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

    fn get_elements(&self, data: &Vec<SelectorEntry>) -> Vec<SelectorEntry> {
        data.to_owned()
    }
}
