use core_notes::{
    nfs::{EntryData, NotePath},
    NoteVault, NotesGetterOptions,
};
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
                        NavEntry::Note(path) => {
                            rsx!{div {
                                class: "icon-note element",
                                onclick: move |_| *note_path.write() = Some(path.clone()),
                                "{path.get_name()}"
                            }}
                        },
                        NavEntry::Directory(path) => {
                            rsx!{div {
                                class: "icon-folder element",
                                onclick: move |_| browsing_directory.set(path.to_owned()),
                                "{path.get_name()}"
                            }}
                        },
                    }
                }
            } else {
                div { "Loading..." }
            }
        }
    }
}

#[derive(Clone)]
enum NavEntry {
    Note(NotePath),
    Directory(NotePath),
}

impl NavEntry {
    fn sort_string(&self) -> String {
        match self {
            NavEntry::Directory(note_path) => format!("1-{}", note_path),
            NavEntry::Note(note_path) => format!("2-{}", note_path),
        }
    }
}

#[derive(Clone)]
struct NotesAndDirs {
    current_path: Signal<NotePath>,
    entries: Resource<Vec<NavEntry>>,
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
                        .get_notes(
                            &current_path,
                            NotesGetterOptions::default()
                                .set_sender(tx)
                                .full_validation(),
                        )
                        .expect("Error fetching Entries");
                });
                task.await;
                let current_path = path.read().clone();
                while let Ok(entry) = rx.recv() {
                    match &entry.data {
                        EntryData::Note(note_data) => {
                            entries.push(NavEntry::Note(note_data.path.clone()))
                        }
                        EntryData::Directory(directory_data) => {
                            if directory_data.path != current_path {
                                entries.push(NavEntry::Directory(directory_data.path.clone()))
                            }
                        }
                        EntryData::Attachment => {
                            // Do nothing
                        }
                    };
                }
                entries.sort_by_key(|b| std::cmp::Reverse(b.sort_string()));
                entries
            }
        });
        Self {
            current_path: path,
            entries,
        }
    }

    fn get_entries(&self) -> Vec<NavEntry> {
        let res = self.entries.value().read().to_owned().unwrap_or_default();
        res
    }

    fn get_current(&self) -> NotePath {
        self.current_path.read().clone()
    }
}
