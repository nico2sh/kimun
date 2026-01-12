use crate::components::{
    note_list_data::note_select_entry::NoteSelectEntry, note_select_entry::SortCriteria,
};
use dioxus::prelude::*;

pub trait SelectorFunctions: Clone {
    fn init(&self) -> Vec<NoteSelectEntry>;
    fn filter(&self, filter_text: String, items: &[NoteSelectEntry]) -> Vec<NoteSelectEntry>;
    fn on_select(&mut self, element: &NoteSelectEntry) -> bool;
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Initializing,
    Ready,
    Filtering,
    Sorting,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StateData {
    raw_data: Vec<NoteSelectEntry>,
    filter_value: String,
    filtered_data: Vec<NoteSelectEntry>,
    pub display_data: Vec<NoteSelectEntry>,
    sort_criteria: SortCriteria,
    sort_ascending: bool,
}

#[derive(Clone)]
pub struct UseNoteList {
    pub inner: Signal<StateData>,
    state: Signal<LoadState>,
}

impl UseNoteList {
    pub fn reset(&mut self) {
        *self.state.write() = LoadState::Initializing;
    }
}

pub fn use_note_list<F>(
    search_text: Signal<String>,
    sort_criteria: Signal<SortCriteria>,
    sort_ascending: Signal<bool>,
    functions: F,
    on_updated: impl FnOnce(Vec<NoteSelectEntry>) -> () + Clone + 'static,
) -> UseNoteList
where
    F: SelectorFunctions + Clone + Send + 'static,
{
    let mut load_state: Signal<LoadState> = use_signal(|| LoadState::Initializing);

    let mut selected: Signal<Option<usize>> = use_signal(|| None);

    let functions_load = functions.clone();
    let mut state_data = use_signal(|| StateData::default());

    _ = use_resource(move || {
        let current_state = load_state.read().clone();
        let functions = functions_load.clone();
        let on_updated = on_updated.clone();
        async move {
            match current_state {
                LoadState::Initializing => {
                    info!("---=== Initializing");
                    selected.set(None);
                    let result = tokio::task::spawn(async move { functions.init() })
                        .await
                        .unwrap_or_default();
                    state_data.write().raw_data = result;
                    load_state.set(LoadState::Filtering);
                }
                LoadState::Filtering => {
                    debug!("Filtering");
                    selected.set(None);
                    let filter_text = search_text.peek().clone();
                    let raw_data = state_data.peek().raw_data.clone();
                    state_data.write().filter_value = filter_text.clone();
                    let rows =
                        tokio::spawn(async move { functions.filter(filter_text, &raw_data) })
                            .await
                            .unwrap_or_default();
                    info!("We truncate the row mounts with {} values", rows.len());
                    state_data.write().filtered_data = rows;
                    load_state.set(LoadState::Sorting);
                }
                LoadState::Sorting => {
                    debug!("Sorting");
                    let sort_criteria = sort_criteria();
                    let sort_ascending = sort_ascending();
                    state_data.write().sort_criteria = sort_criteria.clone();
                    state_data.write().sort_ascending = sort_ascending;
                    let mut r = state_data.peek().filtered_data.clone();
                    if SortCriteria::None != state_data.peek().sort_criteria {
                        r = tokio::spawn(async move {
                            if sort_ascending {
                                r.sort_by_key(|b| b.sort_string_for(&sort_criteria));
                            } else {
                                r.sort_by_key(|b| {
                                    std::cmp::Reverse(b.sort_string_for(&sort_criteria))
                                });
                            };
                            r
                        })
                        .await
                        .unwrap_or_default();
                    }
                    state_data.write().display_data = r;
                    load_state.set(LoadState::Ready);
                }
                LoadState::Ready => {
                    debug!("Ready");
                    on_updated(state_data().display_data.clone());
                    if search_text() != state_data.peek().filter_value {
                        load_state.set(LoadState::Filtering);
                    } else if sort_criteria() != state_data.peek().sort_criteria
                        || sort_ascending() != state_data.peek().sort_ascending
                    {
                        load_state.set(LoadState::Sorting);
                    }
                }
            }
        }
    });

    UseNoteList {
        inner: state_data,
        state: load_state,
    }
}
