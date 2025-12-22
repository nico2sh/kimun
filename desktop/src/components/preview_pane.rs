use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{
    nfs::VaultPath, note::MarkdownNote, NoteVault, ResultType, VaultBrowseOptionsBuilder,
};

use crate::components::{
    modal::ModalType,
    note_browser::note_list::{NoteElementActions, NoteList},
    note_select_entry::NoteBrowseEntry,
    preview::Markdown,
};

#[derive(Clone, PartialEq)]
pub enum PreviewList {
    FromPath(VaultPath),
    FromList(Vec<NoteBrowseEntry>),
    FromQuery(String),
}

enum PreviewContent {
    None,
    Note(MarkdownNote),
    Err(String),
}

#[derive(Clone, PartialEq, Props)]
pub struct PreviewPaneProps {
    vault: Arc<NoteVault>,
    source: PreviewList,
    modal_type: Signal<ModalType>,
}

#[component]
pub fn PreviewPane(props: PreviewPaneProps) -> Element {
    let vault = props.vault;
    let source = props.source;
    let active_path = use_signal(|| VaultPath::root());
    let modal_type = props.modal_type;

    let vault_list = vault.clone();
    let list = use_resource(move || {
        let vault = vault_list.clone();
        let source = source.clone();
        async move {
            match source {
                PreviewList::FromPath(vault_path) => {
                    let browsing_directory = if vault_path.is_note() {
                        vault_path.get_parent_path().0
                    } else {
                        vault_path.to_owned()
                    };
                    let mut entries = vec![];
                    let (search_options, rx) = VaultBrowseOptionsBuilder::new(&browsing_directory)
                        .full_validation()
                        .non_recursive()
                        .build();
                    let browsing_vault = vault.clone();
                    let _ = tokio::spawn(async move {
                        browsing_vault
                            .browse_vault(search_options)
                            .expect("Error fetching Entries");
                    })
                    .await;

                    while let Ok(entry) = rx.recv() {
                        match &entry.rtype {
                            ResultType::Note(note_details) => {
                                let e = if let Some(date) = vault.journal_date(&entry.path) {
                                    NoteBrowseEntry::from_note_journal(
                                        entry.path,
                                        note_details.to_owned(),
                                        date,
                                    )
                                } else {
                                    NoteBrowseEntry::from_note_details(
                                        entry.path,
                                        note_details.to_owned(),
                                    )
                                };
                                entries.push(e)
                            }
                            ResultType::Directory => {
                                // Do nothing
                            }
                            ResultType::Attachment => {
                                // Do nothing
                            }
                        };
                    }
                    entries
                }
                PreviewList::FromList(items) => items
                    .into_iter()
                    .filter(|e| match e {
                        NoteBrowseEntry::Note {
                            path: _,
                            title: _,
                            search_str: _,
                        } => true,
                        _ => false,
                    })
                    .collect(),
                PreviewList::FromQuery(query) => todo!(),
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
            button { class: "bar-preview-toggle",
                span { "Quick Browser" }
                span { "▼" }
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
                            div { class: "info" }
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
