use std::sync::Arc;

use dioxus::{hooks::use_signal, html::div, logger::tracing::info, prelude::*};
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, SearchResult, VaultBrowseOptionsBuilder};

#[component]
pub fn NoteBrowser(vault: Arc<NoteVault>, base_path: VaultPath) -> Element {
    info!("Open Note Browser");
    let mut browsing_directory = use_signal(move || {
        if base_path.is_note() {
            base_path.get_parent_path().0
        } else {
            base_path.to_owned()
        }
    });
    let notes_and_dirs = NotesAndDirs::new(vault, browsing_directory);
    let current_path = notes_and_dirs.get_current();

    rsx! {
        div { class: "sidebar-header",
            div { class: "sidebar-title", "{current_path.to_string()}" }
            button { class: "sidebar-toggle", onclick: move |_| {},
                svg {
                    width: 16,
                    height: 16,
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: 2,
                }
            }
        }
        div { class: "sidebar-search",
            input {
                r#type: "text",
                class: "search-input",
                placeholder: "search",
            }
        }
        div { class: "entry-list", id: "entryList",
            if !current_path.is_root_or_empty() {
                div {
                    class: "entry-item",
                    onclick: move |_| {
                        let parent_path = browsing_directory.read().get_parent_path().0;
                        browsing_directory.set(parent_path);
                    },
                    div { class: "icon-folder title", ".." }
                }
            }
            if let Some(entries) = notes_and_dirs.entries.value().read().clone() {
                for entry in entries {
                    match entry.rtype {
                        ResultType::Note(data) => {
                            let (_directory, file) = entry.path.get_parent_path();
                            rsx! {
                                div { class: "entry-item", onclick: move |_| {},
                                    div { class: "icon-note title", "{data.title}" }
                                    div { class: "details", "{file}" }
                                }
                            }
                        }
                        ResultType::Directory => {
                            let (_directory, path) = entry.path.get_parent_path();
                            rsx! {
                                div {
                                    class: "entry-item",
                                    onclick: move |_| browsing_directory.set(entry.path.to_owned()),
                                    div { class: "icon-folder title", "{path}" }
                                }
                            }
                        }
                        ResultType::Attachment => {
                            rsx! {
                                div { "This shouldn't show up" }
                            }
                        }
                    }
                }
            } else {
                div { "Loading..." }
            }
        }
    }
}

#[derive(Clone)]
struct NotesAndDirs {
    current_path: Signal<VaultPath>,
    entries: Resource<Vec<SearchResult>>,
}

impl NotesAndDirs {
    fn new(vault: Arc<NoteVault>, path: Signal<VaultPath>) -> Self {
        // Since this is a resource that depends on the current_path
        // the entries change every time the current_path is changed
        let entries = use_resource(move || {
            let vault = vault.clone();
            let mut entries = vec![];
            async move {
                let current_path = path.read().clone();
                let (search_options, rx) = VaultBrowseOptionsBuilder::new(&current_path)
                    .full_validation()
                    .non_recursive()
                    .build();
                let _ = tokio::spawn(async move {
                    vault
                        .browse_vault(search_options)
                        .expect("Error fetching Entries");
                })
                .await;
                let current_path = path.read().clone();
                while let Ok(entry) = rx.recv() {
                    match &entry.rtype {
                        ResultType::Note(_note_details) => entries.push(entry.to_owned()),
                        ResultType::Directory => {
                            if entry.path != current_path {
                                info!("entry: {} - current: {}", entry.path, current_path);
                                entries.push(entry.to_owned())
                            }
                        }
                        ResultType::Attachment => {
                            // Do nothing
                        }
                    };
                }
                entries.sort_by_key(|b| std::cmp::Reverse(sort_string(b)));
                entries
            }
        });
        Self {
            current_path: path,
            entries,
        }
    }

    fn get_current(&self) -> VaultPath {
        self.current_path.read().clone()
    }
}

fn sort_string(entry: &SearchResult) -> String {
    match &entry.rtype {
        ResultType::Directory => format!("1-{}", entry.path),
        ResultType::Note(_data) => format!("2-{}", entry.path),
        ResultType::Attachment => format!("3-{}", entry.path),
    }
}
