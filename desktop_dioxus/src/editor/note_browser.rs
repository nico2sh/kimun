use core_notes::{nfs::NotePath, NoteVault, NotesGetterOptions, SearchResult};
use std::sync::mpsc;

use dioxus::{hooks::use_signal, prelude::*};
use log::warn;

use crate::AppContext;

#[derive(Props, Clone, PartialEq)]
pub struct NoteBrowserProps {
    note_path: SyncSignal<Option<NotePath>>,
}

#[allow(non_snake_case)]
pub fn NoteBrowser(props: NoteBrowserProps) -> Element {
    let app_context: AppContext = use_context();
    let mut note_path = props.note_path;
    let vault: NoteVault = app_context.vault;
    let mut browsing_directory = use_signal(move || {
        if let Some(path) = &*note_path.read() {
            if path.is_note() {
                path.get_parent_path().0
            } else {
                path.to_owned()
            }
        } else {
            NotePath::root()
        }
    });
    let notes_and_dirs = NotesAndDirs::new(vault, browsing_directory);
    let current_path = notes_and_dirs.get_current();
    warn!("Notes and dirs: {:?}", browsing_directory);

    rsx! {
        div {
            class: "sideheader",
            "Files: {current_path.to_string()}"
        }
        div {
            class: "list",
            if current_path != NotePath::root() {
                div {
                    class: "icon-folder element",
                    onclick: move |_| {
                        let parent_path = browsing_directory.read().get_parent_path().0;
                        browsing_directory.set(parent_path);
                    },
                    ".."
                }
            }
            if let Some(entries) = notes_and_dirs.entries.value().read().clone() {
                for entry in entries {
                    match entry {
                        SearchResult::Note(details) => {
                            let (_directory, file) = details.path.get_parent_path();
                            rsx!{div {
                                class: "element",
                                onclick: move |_| *note_path.write() = Some(details.path.clone()),
                                div { class: "icon-note title", "{details.title}"}
                                div { class: "details", "{file}"}
                            }}
                        },
                        SearchResult::Directory(details) => {
                            let (_directory, path) = details.path.get_parent_path();
                            rsx!{div {
                                class: "element",
                                onclick: move |_| browsing_directory.set(details.path.to_owned()),
                                div { class: "icon-folder title", "{path}"}
                                // div { class: "details", "{directory}"}
                            }}
                        },
                        SearchResult::Attachment(_) => { rsx!{ div { "This shouldn't show up" } } }
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
    current_path: Signal<NotePath>,
    entries: Resource<Vec<SearchResult>>,
}

impl NotesAndDirs {
    fn new(vault: NoteVault, path: Signal<NotePath>) -> Self {
        // Since this is a resource that depends on the current_path
        // the entries change every time the current_path is changed
        let entries = use_resource(move || {
            let vault = vault.clone();
            let mut entries = vec![];
            async move {
                let current_path = path.read().clone();
                let (tx, rx) = mpsc::channel();
                let task = smol::spawn(async move {
                    vault
                        .get_notes_channel(
                            &current_path,
                            NotesGetterOptions::new(tx).full_validation(),
                        )
                        .expect("Error fetching Entries");
                });
                task.await;
                let current_path = path.read().clone();
                while let Ok(entry) = rx.recv() {
                    match &entry {
                        SearchResult::Note(_note_details) => entries.push(entry.to_owned()),
                        SearchResult::Directory(_directory_details) => {
                            if _directory_details.path != current_path {
                                entries.push(entry.to_owned())
                            }
                        }
                        SearchResult::Attachment(_) => {
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

    fn get_current(&self) -> NotePath {
        self.current_path.read().clone()
    }
}

fn sort_string(entry: &SearchResult) -> String {
    match entry {
        SearchResult::Directory(details) => format!("1-{}", details.path),
        SearchResult::Note(details) => format!("2-{}", details.path),
        SearchResult::Attachment(details) => format!("3-{}", details),
    }
}
