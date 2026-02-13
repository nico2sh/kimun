use crate::components::{
    note_list::note_browse_entry::{NoteBrowseEntry, SortCriteria},
    search_box::StringSearch,
};
use dioxus::prelude::*;

pub trait SelectorFunctions<S>: Clone
where
    S: StringSearch,
{
    async fn init(&self) -> Vec<NoteBrowseEntry>;
    async fn filter(&self, filter_text: S, initial_items: &[NoteBrowseEntry]) -> Vec<NoteBrowseEntry>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Initializing,
    Ready,
    Filtering,
    Sorting,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StateData<S>
where
    S: StringSearch + 'static,
{
    raw_data: Vec<NoteBrowseEntry>,
    filter_value: S,
    filtered_data: Vec<NoteBrowseEntry>,
    sort_criteria: SortCriteria,
    sort_ascending: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UseNoteList<S>
where
    S: StringSearch + 'static,
{
    inner: Signal<StateData<S>>,
    pub display_data: Signal<Vec<NoteBrowseEntry>>,
    state: Signal<LoadState>,
}

impl<S> UseNoteList<S>
where
    S: StringSearch,
{
    pub fn reset(&mut self) {
        *self.state.write() = LoadState::Initializing;
    }
}

pub fn no_op(e: Vec<NoteBrowseEntry>) -> Vec<NoteBrowseEntry> {
    e
}

pub fn use_note_list<S, F>(
    search_text: Signal<S>,
    sort_criteria: Signal<SortCriteria>,
    sort_ascending: Signal<bool>,
    functions: F,
    on_ready: impl FnOnce(Vec<NoteBrowseEntry>) -> Vec<NoteBrowseEntry> + Clone + 'static,
) -> UseNoteList<S>
where
    F: SelectorFunctions<S> + Clone + Send + 'static,
    S: StringSearch + Clone + 'static,
{
    let mut load_state: Signal<LoadState> = use_signal(|| LoadState::Initializing);

    let mut selected: Signal<Option<usize>> = use_signal(|| None);

    let functions_load = functions.clone();
    let mut state_data = use_signal(|| StateData::default());
    let mut display_data = use_signal(|| vec![]);

    _ = use_resource(move || {
        let current_state = load_state.read().clone();
        let functions = functions_load.clone();
        let on_ready = on_ready.clone();
        async move {
            match current_state {
                LoadState::Initializing => {
                    info!("---=== Initializing");
                    selected.set(None);
                    let result = functions.init().await;
                    state_data.write().raw_data = result;
                    load_state.set(LoadState::Filtering);
                }
                LoadState::Filtering => {
                    debug!("Filtering");
                    selected.set(None);
                    let filter_text = search_text.peek().clone();
                    let raw_data = state_data.peek().raw_data.clone();
                    state_data.write().filter_value = filter_text.clone();
                    let filter_text = filter_text.clone();
                    let rows = functions.filter(filter_text, &raw_data).await;
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

                    let r = on_ready(r);
                    display_data.set(r);
                    load_state.set(LoadState::Ready);
                }
                LoadState::Ready => {
                    debug!("Ready");
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
        display_data,
        state: load_state,
    }
}
