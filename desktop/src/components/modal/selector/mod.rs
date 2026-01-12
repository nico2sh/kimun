pub mod note_picker;
pub mod note_search;
pub mod note_select;

use std::{rc::Rc, sync::Arc};

use dioxus::{logger::tracing::info, prelude::*};
use kimun_core::NoteVault;

use crate::{
    components::{
        focus_manager::FocusComponent,
        icons,
        modal::ModalType,
        note_list_data::note_list_loader::{use_note_list, SelectorFunctions},
        note_select_entry::SortCriteria,
        search_box::SearchBox,
    },
    utils::sparse_vector::SparseVector,
};

pub struct PreviewData {
    title: String,
    data: String,
    content: String,
}

#[allow(non_snake_case)]
fn SelectorView<F>(
    hint: String,
    filter_text: String,
    mut modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    functions: F,
) -> Element
where
    F: SelectorFunctions + Clone + Send + 'static,
{
    let filter_text_value = use_signal(|| filter_text);
    let sort_criteria_value = use_signal(|| SortCriteria::None);
    let sort_ascending_value = use_signal(|| true);

    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut row_mounts = use_signal(SparseVector::<Rc<MountedData>>::new);
    let mut select_by_mouse = use_signal(|| true);

    let functions_load = functions.clone();

    let note_list_loaded = use_note_list(
        filter_text_value,
        sort_criteria_value,
        sort_ascending_value,
        functions_load,
        move |r| {
            debug!("Truncating at {} rows", r.len());
            row_mounts.write().truncate(r.len());
        },
    );
    let state_data = note_list_loaded.inner.clone();

    let preview_text = use_resource(move || {
        // We get a copy of the selected one so we don't have borrow issues
        let selected = selected.read().to_owned();
        let vault = vault.clone();
        async move {
            if let Some(selection) = selected {
                info!("Preview Text for {}", selected.unwrap());
                let r = state_data().display_data;
                let entry = r.get(selection);
                if let Some(value) = entry {
                    let value_copy = value.to_owned();
                    tokio::spawn(async move {
                        let preview = vault.load_note(&value_copy.get_path()).map_or_else(
                            |e| PreviewData {
                                title: "Error loading preview...".to_string(),
                                data: e.to_string(),
                                content: "".to_string(),
                            },
                            |d| PreviewData {
                                title: d.get_title(),
                                data: d.path.to_string(),
                                content: d.raw_text,
                            },
                        );
                        Some(preview)
                    })
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

    let row_number = state_data().display_data.len();

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
                let mut note_list_loaded = note_list_loaded.clone();
                async move {
                    let key = e.data.code();
                    if key == Code::Escape {
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
                        if let Some(row) = state_data().display_data.get(current_selected) {
                            if functions_enter.on_select(&row) {
                                modal_type.write().close();
                            } else {
                                note_list_loaded.reset();
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
                        search_text: filter_text_value,
                        sort_criteria: sort_criteria_value,
                        sort_ascending: sort_ascending_value,
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
                for (index , row) in state_data().display_data.into_iter().enumerate() {
                    {
                        let mut note_list_loaded = note_list_loaded.clone();
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
                                        modal_type.write().close();
                                    } else {
                                        note_list_loaded.reset();
                                    }
                                },
                                {row.get_view()}
                            }
                        }
                    }
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
