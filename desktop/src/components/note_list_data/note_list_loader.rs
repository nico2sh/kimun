use crate::components::{
    note_list_data::note_select_entry::NoteSelectEntry, note_select_entry::SortCriteria,
};

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Initializing,
    Ready,
    Filtering,
    Sorting,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StateData {
    pub raw_data: Vec<NoteSelectEntry>,
    pub filter_value: String,
    pub filtered_data: Vec<NoteSelectEntry>,
    pub display_data: Vec<NoteSelectEntry>,
    pub sort_criteria: SortCriteria,
    pub sort_ascending: bool,
}
