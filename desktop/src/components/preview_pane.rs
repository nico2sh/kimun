use std::{fmt::Display, sync::Arc};

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, note::MarkdownNote, NoteVault};

use crate::components::{
    focus_manager::FocusComponent,
    modal::ModalType,
    note_list::note_browse_entry::{NoteBrowseEntry, SortCriteria},
    note_list::{
        note_list_loader::{no_op, use_note_list, SelectorFunctions},
        NoteElementActions, NoteList, SelectorHandler,
    },
    preview::Markdown,
    search_box::{SearchBox, StringSearch},
};

#[derive(Clone, PartialEq, Debug)]
pub enum PreviewList {
    FromQuery(String),
    FromList(String, Vec<NoteBrowseEntry>),
}

impl Default for PreviewList {
    fn default() -> Self {
        Self::FromQuery("".to_string())
    }
}

impl StringSearch for PreviewList {
    fn change_value(&mut self, value: String) {
        *self = PreviewList::FromQuery(value);
    }
}

impl Display for PreviewList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                PreviewList::FromList(query, _items) => query,
                PreviewList::FromQuery(query) => query,
            }
        )
    }
}

enum PreviewContent {
    None,
    Note(MarkdownNote),
    Err(String),
}

#[derive(Clone, PartialEq, Props)]
pub struct PreviewPaneProps {
    vault: Arc<NoteVault>,
    #[props(default = PreviewList::FromQuery(String::new()))]
    source: PreviewList,
    modal_type: Signal<ModalType>,
    #[props(default = SortCriteria::None)]
    sort_criteria: SortCriteria,
    #[props(default = true)]
    sort_ascending: bool,
}

#[derive(Clone)]
pub struct PreviewListFunctions {
    pub vault: Arc<NoteVault>,
}

impl SelectorFunctions<PreviewList> for PreviewListFunctions {
    fn init(&self) -> Vec<NoteBrowseEntry> {
        vec![]
    }

    fn filter(
        &self,
        filter_text: PreviewList,
        _initial_items: &[NoteBrowseEntry],
    ) -> Vec<NoteBrowseEntry> {
        match &filter_text {
            PreviewList::FromQuery(_query) => {
                let filter_text = filter_text.to_owned();
                match self.vault.search_notes(filter_text.to_string()) {
                    Ok(res) => res
                        .into_iter()
                        .map(|(entry, content)| {
                            NoteBrowseEntry::from_note_details(entry.path, content)
                        })
                        .collect::<Vec<NoteBrowseEntry>>(),
                    Err(e) => {
                        error!("Error searching notes: {}", e);
                        vec![]
                    }
                }
            }
            PreviewList::FromList(_query, items) => items.to_owned(),
        }
    }
}

#[component]
pub fn PreviewPane(props: PreviewPaneProps) -> Element {
    let vault = props.vault;
    let active_path = use_signal(|| VaultPath::root());
    let modal_type = props.modal_type;
    let mut show_search = use_signal(|| false);

    let source = use_signal(|| props.source);

    let sort_criteria = use_signal(|| props.sort_criteria);
    let sort_ascending = use_signal(|| props.sort_ascending);

    let functions = PreviewListFunctions {
        vault: vault.clone(),
    };

    let loaded_note_list = use_note_list(source, sort_criteria, sort_ascending, functions, no_op);
    let selector_handler = SelectorHandler::build(loaded_note_list.display_data.clone());
    let entries = loaded_note_list.display_data;

    let preview_vault = vault.clone();
    let preview_content = use_resource(move || {
        let vault_content = preview_vault.clone();
        async move {
            if active_path.read().is_root_or_empty() {
                PreviewContent::None
            } else {
                match vault_content.load_note(&active_path.read()) {
                    Ok(note) => PreviewContent::Note(note.get_markdown_and_links()),
                    Err(e) => PreviewContent::Err(format!("Error loading Note: {}", e)),
                }
            }
        }
    });

    rsx! {
        div { class: "bar-preview-header",
            div { class: "bar-preview-header-top",
                button { class: "bar-preview-title-btn",
                    span { class: "bar-preview-title", "Quick Browser" }
                    svg {
                        class: "icon",
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M19 9l-7 7-7-7",
                        }
                    }
                }
                button {
                    class: "bar-preview-search-btn",
                    onclick: move |_e| {
                        show_search.set(!show_search());
                    },
                    svg {
                        class: "icon",
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z",
                        }
                    }
                }
            }
        }
        div { class: "bar-preview-browser",
            {
                rsx! {
                    NoteList {
                        entries,
                        active_path: active_path.read().to_owned(),
                        element_action: NoHoverAction { active_path },
                        selector_handler,
                        compact: true,
                    }
                }
            }
        
        }
        if show_search() {
            {
                rsx! {
                    div {
                        class: "bar-preview-search-popup-overlay",
                        onclick: move |_e| show_search.set(false),
                        div { class: "bar-preview-search-popup", onclick: |e| e.stop_propagation(),
                            SearchBox {
                                search_text: source,
                                sort_criteria,
                                sort_ascending,
                                input_focus: FocusComponent::PreviewPane,
                            }
                        }
                    }
                }
            }
        }
        div { class: "bar-preview-content",
            match &*preview_content.read() {
                Some(content) => {
                    match content {
                        PreviewContent::None => rsx! {
                            div { class: "info" }
                        },
                        PreviewContent::Note(markdown_note) => rsx! {
                            Markdown {
                                vault: vault.clone(),
                                note_md: markdown_note.text.clone(),
                                note_links: markdown_note.links.clone(),
                                modal_type,
                            }
                        },
                        PreviewContent::Err(e) => rsx! {
                            div { class: "info", "{e}" }
                        },
                    }
                }
                None => rsx! {
                    div { class: "info", "Loading..." }
                },
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct NoHoverAction {
    active_path: Signal<VaultPath>,
}

impl NoteElementActions for NoHoverAction {
    fn on_hover(&self, _entry: &NoteBrowseEntry) -> Element {
        rsx! {}
    }

    fn on_select(&mut self, entry: &NoteBrowseEntry) {
        self.active_path.set(entry.get_path().to_owned());
    }
}
