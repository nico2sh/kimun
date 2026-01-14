pub mod note_picker;
pub mod note_search;
pub mod note_select;

use std::sync::Arc;

use dioxus::{logger::tracing::info, prelude::*};
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    app_state::AppState,
    components::{
        focus_manager::FocusComponent,
        icons,
        modal::ModalType,
        note_list::{
            note_browse_entry::{NoteBrowseEntry, NoteEntryType, SortCriteria},
            note_list_loader::{no_op, use_note_list, SelectorFunctions},
            NoteElementActions, NoteList, SelectorHandler,
        },
        preview_pane::PreviewList,
        search_box::{SearchBox, StringSearch},
    },
};

pub struct PreviewData {
    title: String,
    data: String,
    content: String,
}

#[allow(non_snake_case)]
fn SelectorView<F, S>(
    hint: String,
    filter_text: S,
    mut modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    functions: F,
) -> Element
where
    F: SelectorFunctions<S> + Clone + Send + 'static,
    S: StringSearch + Clone + 'static,
{
    let mut app_state: Signal<AppState> = use_context();

    let filter_text_value = use_signal(|| filter_text);
    let sort_criteria_value = use_signal(|| SortCriteria::None);
    let sort_ascending_value = use_signal(|| true);

    let functions_load = functions.clone();

    let note_list_loaded = use_note_list(
        filter_text_value,
        sort_criteria_value,
        sort_ascending_value,
        functions_load,
        no_op,
    );
    let selector_handler = SelectorHandler::build(note_list_loaded.display_data.clone());
    let entries = note_list_loaded.display_data.clone();

    let selector_preview = selector_handler.clone();
    let preview_text = use_resource(move || {
        // We get a copy of the selected one so we don't have borrow issues
        let selected = selector_preview.get_selected();
        let vault = vault.clone();
        async move {
            if let Some(selection) = selected {
                info!("Preview Text for {}", selected.unwrap());
                let r = entries();
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

    let row_number = entries().len();

    let selector_loaded = selector_handler.clone();
    let element_action = ModalNoteListAction { modal_type };
    let action_enter = element_action.clone();

    rsx! {
        div {
            class: "notes-modal",
            autofocus: "true",
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: Event<KeyboardData>| {
                let selector_loaded = selector_loaded.clone();
                let mut element_action = action_enter.clone();
                async move {
                    let key = e.data.code();
                    if key == Code::Escape {
                        modal_type.write().close();
                    }
                    if key == Code::ArrowDown {
                        selector_loaded.select_next();
                    }
                    if key == Code::ArrowUp {
                        selector_loaded.select_prev();
                    }
                    if key == Code::Enter && row_number > 0 {
                        let current_selected = (selector_loaded.get_selected()).unwrap_or(0);
                        if let Some(row) = entries().get(current_selected) {
                            element_action.on_select(row);
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
                    button {
                        class: "send-button",
                        onclick: move |_e| {
                            app_state.write().show_preview_pane(PreviewList::FromQuery("test".to_string()));
                            modal_type.write().close();
                        },
                        icons::FatArrowRight {}
                        span { class: "send-button-text", "To Sidebar" }
                    }
                }
            }
            NoteList {
                entries,
                active_path: VaultPath::root(),
                element_action,
                selector_handler,
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

#[derive(Clone, PartialEq)]
pub struct ModalNoteListAction {
    modal_type: Signal<ModalType>,
}

impl NoteElementActions for ModalNoteListAction {
    fn on_hover(&self, _entry: &NoteBrowseEntry) -> Element {
        rsx! {}
    }

    fn on_select(&mut self, entry: &NoteBrowseEntry) {
        let mut app_state: Signal<AppState> = use_context();
        match &entry.e_type {
            NoteEntryType::Note {
                title: _,
                search_str: _,
            } => {
                app_state.write().set_path(&entry.path, false);
                self.modal_type.write().close();
            }
            NoteEntryType::Journal {
                title: _,
                date_string: _,
                search_str: _,
            } => {
                app_state.write().set_path(&entry.path, false);
                self.modal_type.write().close();
            }
            NoteEntryType::Create { name: _ } => {
                app_state.write().set_path(&entry.path, true);
                self.modal_type.write().close();
            }
            NoteEntryType::Directory { name: _ } => {
                // Do nothing
            }
        }
    }
}
