use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, VaultBrowseOptionsBuilder};

use crate::components::{
    note_browser::note_list::{NoteElementActions, NoteList},
    note_select_entry::NoteSelectEntry,
};

#[derive(Clone, PartialEq)]
pub enum PreviewList {
    FromPath(VaultPath),
    FromList(Vec<NoteSelectEntry>),
}

#[derive(Clone, PartialEq, Props)]
pub struct PreviewPaneProps {
    vault: Arc<NoteVault>,
    source: PreviewList,
}

#[component]
pub fn PreviewPane(props: PreviewPaneProps) -> Element {
    let vault = props.vault;
    let source = props.source;
    let list = use_resource(move || {
        let vault = vault.clone();
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
                                    NoteSelectEntry::from_note_journal(
                                        entry.path,
                                        note_details.to_owned(),
                                        date,
                                    )
                                } else {
                                    NoteSelectEntry::from_note_details(
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
                        NoteSelectEntry::Note {
                            path: _,
                            title: _,
                            search_str: _,
                        } => true,
                        _ => false,
                    })
                    .collect(),
            }
        }
    });

    rsx! {
        div { class: "bar-preview-header",
            button { class: "bar-preview-toggle",
                span { "Preview" }
                span { "▼" }
            }
        }
        div { class: "bar-preview-browser",
            if let Some(entries) = &*list.read() {
                NoteList {
                    entries: entries.clone(),
                    active_path: VaultPath::root(),
                    element_action: NoHoverAction {},
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct NoHoverAction {}

impl NoteElementActions for NoHoverAction {
    fn on_hover(&self, _entry: NoteSelectEntry) -> Element {
        rsx! {}
    }
}
