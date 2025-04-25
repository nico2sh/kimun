use kimun_core::{nfs::NoteEntryData, note::NoteContentData};
use log::debug;
use rayon::slice::ParallelSliceMut;

use crate::editor::EditorMessage;

use super::{
    filtered_list::{FilteredListFunctionMessage, FilteredListFunctions, StateData},
    vault_browse::{SelectorEntry, SelectorEntryType, SortMode},
};

#[derive(Clone)]
pub struct NoteSelectorFunctions {
    selections: Vec<SelectorEntry>,
    sort_mode: SortMode,
}

impl NoteSelectorFunctions {
    pub fn new(entries: Vec<(NoteEntryData, NoteContentData)>) -> Self {
        let selections = entries
            .iter()
            .map(|(entry, content)| {
                let title = content.title.clone();
                SelectorEntry {
                    path: entry.path.clone(),
                    path_str: entry.path.to_string(),
                    search_str: title.clone(),
                    entry_type: SelectorEntryType::Note { title },
                }
            })
            .collect();
        Self {
            selections,
            sort_mode: SortMode::FileDown,
        }
    }
}

impl FilteredListFunctions<Vec<SelectorEntry>, SelectorEntry> for NoteSelectorFunctions {
    fn init(&self) -> Vec<SelectorEntry> {
        self.selections.clone()
    }

    fn filter<S: AsRef<str>>(
        &self,
        filter_text: S,
        provider: &Vec<SelectorEntry>,
    ) -> Vec<SelectorEntry> {
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut filtered = nucleo::pattern::Pattern::parse(
            filter_text.as_ref(),
            nucleo::pattern::CaseMatching::Ignore,
            nucleo::pattern::Normalization::Smart,
        )
        .match_list(provider, &mut matcher)
        .iter()
        .map(|e| e.0.to_owned())
        .collect::<Vec<SelectorEntry>>();
        filtered.par_sort_by(|a, b| match self.sort_mode {
            SortMode::FileUp => a.path_str.cmp(&b.path_str),
            SortMode::FileDown => b.path_str.cmp(&a.path_str),
            SortMode::TitleUp => a.search_str.cmp(&b.search_str),
            SortMode::TitleDown => b.search_str.cmp(&a.search_str),
        });

        debug!("filtered {} values", filtered.len());
        filtered
    }

    fn on_entry(&self, element: &SelectorEntry) -> Option<FilteredListFunctionMessage<Self>> {
        Some(FilteredListFunctionMessage::ToEditor(
            EditorMessage::OpenNote(element.path.clone()),
        ))
    }

    fn header_element(&self, _state_data: &StateData<SelectorEntry>) -> Option<SelectorEntry> {
        None
    }

    fn button_icon(&self) -> Option<String> {
        Some(self.sort_mode.get_icon())
    }
}
