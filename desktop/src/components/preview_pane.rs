use std::{fmt::Display, sync::Arc};

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, note::MarkdownNote, NoteVault};

use crate::components::{
    focus_manager::FocusComponent,
    modal::ModalType,
    note_browser::note_list::{NoteElementActions, NoteList},
    note_select_entry::{NoteBrowseEntry, SortCriteria},
    preview::Markdown,
    search_box::{SearchBox, StringSearch},
};

#[derive(Clone, PartialEq)]
pub enum PreviewList {
    FromList(String, Vec<NoteBrowseEntry>),
    FromQuery(String),
}

impl StringSearch for PreviewList {
    fn on_string_change(&mut self, value: String) {
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

#[component]
pub fn PreviewPane(props: PreviewPaneProps) -> Element {
    let vault = props.vault;
    let active_path = use_signal(|| VaultPath::root());
    let modal_type = props.modal_type;
    let mut show_search = use_signal(|| false);

    let source = use_signal(|| props.source);

    let sort_criteria = use_signal(|| props.sort_criteria);
    let sort_ascending = use_signal(|| props.sort_ascending);

    let vault_list = vault.clone();
    let list = use_resource(move || {
        let vault = vault_list.clone();
        async move {
            match source() {
                PreviewList::FromList(_query, items) => items,
                PreviewList::FromQuery(query) => {
                    let result = tokio::spawn(async move {
                        match vault.search_notes(query) {
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
                    })
                    .await;
                    result.unwrap_or_default()
                }
            }
        }
    });

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
            if let Some(entries) = &*list.read() {
                NoteList {
                    entries: entries.clone(),
                    active_path: active_path.read().to_owned(),
                    element_action: NoHoverAction { active_path },
                    compact: true,
                }
            }
        }
        if show_search() {
            {
                rsx! {
                    div {
                        class: "bar-preview-search-popup-overlay",
                        onclick: move |e| show_search.set(false),
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
    fn on_hover(&self, _entry: NoteBrowseEntry) -> Element {
        rsx! {}
    }

    fn on_select(&mut self, entry: NoteBrowseEntry) {
        self.active_path.set(entry.get_path().to_owned());
    }
}
