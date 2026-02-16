pub mod note_picker;
pub mod note_search;
pub mod note_select;

use std::sync::Arc;

use dioxus::{logger::tracing::info, prelude::*};
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    app_state::{AppState, PreviewListState},
    components::{
        focus_manager::FocusComponent,
        icons,
        note_list::{
            note_browse_entry::{NoteBrowseEntry, NoteEntryType, SortCriteria},
            note_list_loader::{no_op, use_note_list, SelectorFunctions},
            NoteElementActions, NoteList, SelectorHandler,
        },
        search_box::SearchBox,
    },
    settings::AppSettings,
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
    vault: Arc<NoteVault>,
    functions: F,
    send_to_preview: bool,
) -> Element
where
    F: SelectorFunctions + Clone + Send + 'static,
{
    debug!("== Modal file loaded");

    let mut app_state: Signal<AppState> = use_context();
    let settings: Signal<AppSettings> = use_context();

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
            match selected {
                Some(selection) => {
                    info!("Preview Text for {}", selection);
                    let r = entries();
                    match r.get(selection) {
                        Some(value) => match &value.e_type {
                            NoteEntryType::Note { .. } => {
                                let value_copy = value.to_owned();
                                tokio::spawn(async move {
                                    let preview = vault
                                        .load_note(&value_copy.get_path())
                                        .await
                                        .map_or_else(
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
                                .unwrap_or_default()
                            }
                            NoteEntryType::Create { name } => Some(PreviewData {
                                title: "Create New Note".to_string(),
                                data: value.get_path().to_string(),
                                content: format!("New note will be created: {}", name),
                            }),
                            _ => None,
                        },
                        None => None,
                    }
                }
                None => None,
            }
        }
    });

    // Memoize theme to avoid re-reading on every render
    let theme_memo = use_memo(move || settings().get_theme());
    let theme = theme_memo();

    let selector_loaded = selector_handler.clone();
    let element_action = ModalNoteListAction {};
    let action_enter = element_action.clone();
    let mut send_hover = use_signal(|| false);

    rsx! {
        div {
            class: "notes-modal",
            background_color: "{theme.bg_section}",
            border_color: "{theme.border_light}",
            autofocus: "true",
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: Event<KeyboardData>| {
                let selector_loaded = selector_loaded.clone();
                let mut element_action = action_enter.clone();
                async move {
                    let key = e.data.code();
                    if key == Code::Escape {
                        app_state.write().close_modal();
                    }
                    if key == Code::ArrowDown {
                        selector_loaded.select_next();
                    }
                    if key == Code::ArrowUp {
                        selector_loaded.select_prev();
                    }
                    if key == Code::Enter {
                        let row_number = entries.peek().len();
                        if row_number > 0 {
                            let current_selected = (selector_loaded.get_selected()).unwrap_or(0);
                            if let Some(row) = entries.peek().get(current_selected) {
                                element_action.on_select(row);
                            }
                        }
                    }
                }
            },
            div {
                class: "search-header",
                background_color: "{theme.bg_head}",
                color: "{theme.text_head}",
                border_bottom_color: "{theme.border_light}",
                div { class: "search-title", "Browse Notes" }
                div { class: "header-description", color: "{theme.text_head}", "{hint}" }
                div { class: "search-container",
                    SearchBox {
                        search_text: filter_text_value,
                        sort_criteria: sort_criteria_value,
                        sort_ascending: sort_ascending_value,
                        input_focus: FocusComponent::ModalInput,
                    }
                    if send_to_preview {
                        button {
                            class: "send-button",
                            border_color: if send_hover() { "{theme.accent_blue}" } else { "{theme.border_light}" },
                            color: if send_hover() { "{theme.accent_blue}" } else { "{theme.text_primary}" },
                            background_color: if send_hover() { "{theme.bg_hover}" } else { "{theme.bg_main}" },
                            onmouseenter: move |_e| send_hover.set(true),
                            onmouseleave: move |_e| send_hover.set(false),
                            onclick: move |_e| {
                                app_state
                                    .write()
                                    .show_preview_pane(
                                        Some(
                                            PreviewListState::new(
                                                filter_text_value.read().to_string(),
                                                sort_criteria_value(),
                                                sort_ascending_value(),
                                            ),
                                        ),
                                    );
                                app_state.write().close_modal();
                            },
                            icons::FatArrowRight {}
                            span { class: "send-button-text", "To Sidebar" }
                        }
                    }
                }
            }
            NoteList {
                entries,
                active_path: VaultPath::root(),
                element_action,
                selector_handler,
                load_state: note_list_loaded.state,
            }
            div { class: "preview-pane", background_color: "{theme.bg_main}",
                match &*preview_text.read_unchecked() {
                    Some(text) => {
                        if let Some(p) = text {
                            rsx! {
                                div { class: "preview-header", border_bottom_color: "{theme.border_light}",
                                    div { class: "preview-title", color: "{theme.text_primary}", "{p.title}" }
                                    div { class: "preview-meta", color: "{theme.text_light}", "{p.data}" }
                                }
                                div { class: "preview-content", color: "{theme.text_secondary}", "{p.content}" }
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
pub struct ModalNoteListAction {}

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
                app_state.write().close_modal();
            }
            NoteEntryType::Journal {
                title: _,
                date_string: _,
                search_str: _,
            } => {
                app_state.write().set_path(&entry.path, false);
                app_state.write().close_modal();
            }
            NoteEntryType::Create { name: _ } => {
                app_state.write().set_path(&entry.path, true);
                app_state.write().close_modal();
            }
            NoteEntryType::Directory { name: _ } => {
                // Do nothing
            }
        }
    }
}
