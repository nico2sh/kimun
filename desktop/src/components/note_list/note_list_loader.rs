use rayon::prelude::*;
use std::future::Future;

use crate::components::note_list::note_browse_entry::{NoteBrowseEntry, SortCriteria};
use dioxus::prelude::*;

pub trait SelectorFunctions: Clone + Send + 'static {
    fn init(&self) -> impl Future<Output = Vec<NoteBrowseEntry>> + Send;
    fn filter(
        &self,
        filter_text: String,
        initial_items: &[NoteBrowseEntry],
    ) -> impl Future<Output = Vec<NoteBrowseEntry>> + Send;
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Initializing,
    Ready,
    // Forced means that we will trigger the filtering no matter if the filters haven't changed
    Filtering { forced: bool },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SearchStateData {
    pub filter_value: String,
    pub sort_criteria: SortCriteria,
    pub sort_ascending: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UseNoteList {
    inner: Signal<SearchStateData>,
    raw_data: Signal<Vec<NoteBrowseEntry>>,
    pub display_data: Signal<Vec<NoteBrowseEntry>>,
    state: Signal<LoadState>,
}

impl UseNoteList {
    pub fn reset(&mut self) {
        *self.state.write() = LoadState::Initializing;
    }
}

pub fn no_op(e: Vec<NoteBrowseEntry>) -> Vec<NoteBrowseEntry> {
    e
}

const DEBOUNCE_MILLIS: u64 = 200;

pub fn use_note_list<F>(
    search_text: Signal<String>,
    sort_criteria: Signal<SortCriteria>,
    sort_ascending: Signal<bool>,
    functions: F,
    on_ready: impl FnOnce(Vec<NoteBrowseEntry>) -> Vec<NoteBrowseEntry> + Send + Clone + 'static,
) -> UseNoteList
where
    F: SelectorFunctions + Clone + Send + 'static,
{
    let mut load_state: Signal<LoadState> = use_signal(|| LoadState::Initializing);

    let mut selected: Signal<Option<usize>> = use_signal(|| None);

    let functions_load = functions.clone();
    let mut state_data = use_signal(|| SearchStateData::default());
    let mut raw_data = use_signal(|| vec![]);
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
                    raw_data.set(result);
                    load_state.set(LoadState::Filtering { forced: true });
                }
                LoadState::Filtering { forced } => {
                    debug!("Filtering");
                    selected.set(None);
                    // Debounce: wait briefly to batch rapid keystrokes
                    tokio::time::sleep(std::time::Duration::from_millis(DEBOUNCE_MILLIS)).await;
                    let filter_text = search_text.peek().clone();
                    let sort_crit = sort_criteria.peek().clone();
                    let sort_asc = *sort_ascending.peek();
                    // Skip if nothing actually changed after debounce
                    if forced
                        || filter_text != state_data.peek().filter_value
                        || sort_crit != state_data.peek().sort_criteria
                        || sort_asc != state_data.peek().sort_ascending
                    {
                        let raw_data = raw_data.peek().clone();
                        let funcs = functions.clone();
                        let on_ready = on_ready.clone();
                        let can_sort = SortCriteria::None != sort_crit;
                        let result = tokio::spawn(async move {
                            let mut rows = funcs.filter(filter_text.clone(), &raw_data).await;
                            if can_sort {
                                if sort_asc {
                                    rows.par_sort_by_key(|b| b.sort_string_for(&sort_crit));
                                } else {
                                    rows.par_sort_by_key(|b| {
                                        std::cmp::Reverse(b.sort_string_for(&sort_crit))
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
                        debug!("Displaying {} entries", result.len());
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
                        load_state.set(LoadState::Filtering { forced: false });
                    }
                }
            }
        }
    });

    UseNoteList {
        inner: state_data,
        raw_data,
        display_data,
        state: load_state,
    }
}
