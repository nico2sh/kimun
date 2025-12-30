pub mod note_picker;
pub mod note_search;
pub mod note_select;
mod note_select_entry;

use std::rc::Rc;

use dioxus::{logger::tracing::info, prelude::*};

use crate::{
    components::{
        focus_manager::FocusComponent,
        icons,
        modal::{selector::note_select_entry::NoteSelectEntry, ModalType},
        note_select_entry::SortCriteria,
        search_box::SearchBox,
    },
    utils::sparse_vector::SparseVector,
};

trait SelectorFunctions: Clone {
    fn init(&self) -> Vec<NoteSelectEntry>;
    fn filter(&self, filter_text: String, items: &[NoteSelectEntry]) -> Vec<NoteSelectEntry>;
    fn preview(&self, element: &NoteSelectEntry) -> Option<PreviewData>;
    fn on_select(&mut self, element: &NoteSelectEntry) -> bool;
}

pub struct PreviewData {
    title: String,
    data: String,
    content: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Closed,
    Init,
    Loaded(Vec<NoteSelectEntry>),
}

#[derive(Props, Clone, PartialEq)]
struct SelectorViewProps {
    filter_text: Signal<String>,
    load_state: Signal<LoadState>,
    modal_type: Signal<ModalType>,
}

#[allow(non_snake_case)]
fn SelectorView<F>(
    hint: String,
    filter_text: String,
    mut modal_type: Signal<ModalType>,
    functions: F,
) -> Element
where
    F: SelectorFunctions + Clone + Send + 'static,
{
    let filter_text = use_signal(|| filter_text);
    let mut load_state: Signal<LoadState, SyncStorage> = use_signal_sync(|| LoadState::Init);
    // For setting the focus in the text box
    // let mut dialog: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut row_mounts = use_signal(SparseVector::<Rc<MountedData>>::new);
    let mut select_by_mouse = use_signal(|| true);

    let sort_criteria: Signal<Option<SortCriteria>> = use_signal(|| None);
    let sort_ascending = use_signal(|| true);

    let functions_load = functions.clone();

    let filtered_rows = use_resource(move || {
        let filter_text = filter_text.read().clone();
        let current_state = load_state.read().clone();
        let functions = functions_load.clone();
        async move {
            match current_state {
                LoadState::Init => {
                    // row_mounts.write().clear();
                    info!("---=== Initializing");
                    tokio::task::spawn(async move {
                        let items = functions.init();
                        load_state.set(LoadState::Loaded(items.clone()));
                    });

                    // We put the focus on the text
                    // loop {
                    //     if let Some(e) = dialog.with(|f| f.clone()) {
                    //         debug!("Focus input");
                    //         let _ = e.set_focus(true).await;
                    //         break;
                    //     }
                    // }
                    vec![]
                }
                LoadState::Loaded(items) => {
                    selected.set(None);
                    let rows = tokio::spawn(async move { functions.filter(filter_text, &items) })
                        .await
                        .unwrap();
                    info!("We truncate the row mounts with {} values", rows.len());
                    row_mounts.write().truncate(rows.len());
                    rows
                }
                LoadState::Closed => vec![],
            }
        }
    });

    let rows = use_memo(move || match filtered_rows() {
        Some(mut r) => match sort_criteria() {
            Some(sort) => {
                if sort_ascending() {
                    r.sort_by_key(|b| b.sort_string_for(&sort));
                } else {
                    r.sort_by_key(|b| std::cmp::Reverse(b.sort_string_for(&sort)));
                };
                Some(r)
            }
            None => Some(r),
        },
        None => None,
    });

    let functions_preview = functions.clone();
    let preview_text = use_resource(move || {
        let functions_preview = functions_preview.clone();
        // We get a copy of the selected one so we don't have borrow issues
        let selected = selected.read().to_owned();
        async move {
            if let Some(selection) = selected {
                info!("Preview Text for {}", selected.unwrap());
                let r = rows.read().clone();
                let entry = match &r {
                    Some(rows) => rows.get(selection),
                    None => None,
                };
                if let Some(value) = entry {
                    let value_copy = value.to_owned();
                    tokio::spawn(async move { functions_preview.preview(&value_copy) })
                        .await
                        .unwrap()
                } else {
                    None
                }
            } else {
                // Nothing selected
                None
            }
        }
    });

    let row_number = rows.read().clone().unwrap_or_default().len();

    let functions_enter = functions.clone();
    let functions_click = functions.clone();

    rsx! {
        div {
            class: "notes-modal",
            autofocus: "true",
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: Event<KeyboardData>| {
                let mut functions_enter = functions_enter.clone();
                let mounts = row_mounts.read().clone();
                async move {
                    let key = e.data.code();
                    if key == Code::Escape {
                        load_state.set(LoadState::Closed);
                        modal_type.write().close();
                    }
                    if key == Code::ArrowDown {
                        let max_items = row_number;
                        let new_selected = if max_items == 0 {
                            None
                        } else if let Some(ref current_selected) = *selected.read() {
                            let current_selected = current_selected.to_owned();
                            if current_selected < max_items - 1 {
                                Some(current_selected + 1)
                            } else {
                                Some(0)
                            }
                        } else {
                            Some(0)
                        };
                        if let Some(sel) = new_selected {
                            if let Some(mount) = mounts.get(sel) {
                                let _a = mount.scroll_to(ScrollBehavior::Smooth).await;
                                select_by_mouse.set(false);
                            }
                        }
                        selected.set(new_selected);
                    }
                    if key == Code::ArrowUp {
                        let max_items = row_number;
                        let new_selected = if max_items == 0 {
                            None
                        } else if let Some(current_selected) = *selected.read() {
                            if current_selected > 0 {
                                Some(current_selected - 1)
                            } else {
                                Some(max_items - 1)
                            }
                        } else {
                            Some(0)
                        };
                        if let Some(sel) = new_selected {
                            if let Some(mount) = mounts.get(sel) {
                                let _a = mount.scroll_to(ScrollBehavior::Smooth).await;
                                select_by_mouse.set(false);
                            }
                        }
                        selected.set(new_selected);
                    }
                    if key == Code::Enter && row_number > 0 {
                        let current_selected = (*selected.read()).unwrap_or(0);
                        if let Some(rows) = &*rows.read() {
                            if let Some(row) = rows.get(current_selected) {
                                if functions_enter.on_select(&row) {
                                    load_state.set(LoadState::Closed);
                                    modal_type.write().close();
                                } else {
                                    load_state.set(LoadState::Init);
                                }
                            }
                        }
                    }
                }
            },
            div { class: "search-header",
                div { class: "search-title", "Browse Notes" }
                div { class: "header-description", "{hint}" }
                div { class: "search-container",
                    SearchBox {
                        search_text: filter_text,
                        sort_criteria,
                        sort_ascending,
                        input_focus: FocusComponent::ModalInput,
                    }
                    button { class: "send-button",
                        icons::FatArrowRight {}
                        span { class: "send-button-text", "To Sidebar" }
                    }
                }
            }
            div {
                class: "notes-list",
                onmousemove: move |_e| {
                    if !*select_by_mouse.read() {
                        select_by_mouse.set(true);
                    }
                },
                if let Some(rs) = rows.read().clone() {
                    for (index , row) in rs.into_iter().enumerate() {
                        {
                            let mut functions_click = functions_click.clone();
                            rsx! {
                                div {
                                    class: if *selected.read() == Some(index) { "note-item selected" } else { "note-item" },
                                    id: "element-{index}",
                                    onmounted: move |e| {
                                        info!("Adding mount at {} position", index);
                                        row_mounts.write().insert(index, e.data());
                                    },
                                    onmouseenter: move |_e| {
                                        if *select_by_mouse.read() {
                                            selected.set(Some(index));
                                        }
                                    },
                                    onclick: move |e| {
                                        info!("Clicked element");
                                        e.stop_propagation();
                                        if functions_click.on_select(&row) {
                                            load_state.set(LoadState::Closed);
                                            modal_type.write().close();
                                        } else {
                                            load_state.set(LoadState::Init);
                                        }
                                    },
                                    {row.get_view()}
                                }
                            }
                        }
                    }
                } else {
                    div { "Loading..." }
                }
            }
            div { class: "preview-pane",
                match &*preview_text.read_unchecked() {
                    Some(text) => {
                        if let Some(p) = text {
                            rsx! {
                                div { class: "preview-header",
                                    div { class: "preview-title", "{p.title}" }
                                    div { class: "preview-meta", "{p.data}" }
                                }
                                div { class: "preview-content", "{p.content}" }
                            }
                        } else {
                            rsx! {
                                div { class: "no-preview", "" }
                            }
                        }
                    }
                    None => rsx! { "Loading..." },
                }
            }
        }
    }
}
