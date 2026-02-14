use rayon::prelude::*;
use std::future::Future;

use crate::components::{
    note_list::note_browse_entry::{NoteBrowseEntry, SortCriteria},
    search_box::StringSearch,
};
use dioxus::prelude::*;

pub trait SelectorFunctions<S>: Clone + Send + 'static
where
    S: StringSearch,
{
    fn init(&self) -> impl Future<Output = Vec<NoteBrowseEntry>> + Send;
    fn filter(
        &self,
        filter_text: S,
        initial_items: &[NoteBrowseEntry],
    ) -> impl Future<Output = Vec<NoteBrowseEntry>> + Send;
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Initializing,
    Ready,
    Filtering,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StateData<S>
where
    S: StringSearch + 'static,
{
    raw_data: Vec<NoteBrowseEntry>,
    filter_value: S,
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
    on_ready: impl FnOnce(Vec<NoteBrowseEntry>) -> Vec<NoteBrowseEntry> + Send + Clone + 'static,
) -> UseNoteList<S>
where
    F: SelectorFunctions<S> + Clone + Send + 'static,
    S: StringSearch + Clone + Send + 'static,
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
                    let funcs = functions.clone();
                    let result = tokio::spawn(async move { funcs.init().await })
                        .await
                        .unwrap_or_default();
                    state_data.write().raw_data = result;
                    load_state.set(LoadState::Filtering);
                }
                LoadState::Filtering => {
                    debug!("Filtering");
                    selected.set(None);
                    // Debounce: wait briefly to batch rapid keystrokes
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let filter_text = search_text.peek().clone();
                    let sort_crit = sort_criteria.peek().clone();
                    let sort_asc = *sort_ascending.peek();
                    // Skip if nothing actually changed after debounce
                    if filter_text != state_data.peek().filter_value
                        || sort_crit != state_data.peek().sort_criteria
                        || sort_asc != state_data.peek().sort_ascending
                    {
                        let raw_data = state_data.peek().raw_data.clone();
                        let funcs = functions.clone();
                        let on_ready = on_ready.clone();
                        let can_sort = SortCriteria::None != sort_crit;
                        let result = tokio::spawn(async move {
                            let mut rows =
                                funcs.filter(filter_text.clone(), &raw_data).await;
                            if can_sort {
                                if sort_asc {
                                    rows.par_sort_by_key(|b| {
                                        b.sort_string_for(&sort_crit)
                                    });
                                } else {
                                    rows.par_sort_by_key(|b| {
                                        std::cmp::Reverse(
                                            b.sort_string_for(&sort_crit),
                                        )
                                    });
                                }
                            }
                            on_ready(rows)
                        })
                        .await
                        .unwrap_or_default();
                        {
                            let mut sd = state_data.write();
                            sd.filter_value = search_text.peek().clone();
                            sd.sort_criteria = sort_criteria.peek().clone();
                            sd.sort_ascending = *sort_ascending.peek();
                        }
                        display_data.set(result);
                    }
                    load_state.set(LoadState::Ready);
                }
                LoadState::Ready => {
                    debug!("Ready");
                    if search_text() != state_data.peek().filter_value
                        || sort_criteria() != state_data.peek().sort_criteria
                        || sort_ascending() != state_data.peek().sort_ascending
                    {
                        load_state.set(LoadState::Filtering);
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
