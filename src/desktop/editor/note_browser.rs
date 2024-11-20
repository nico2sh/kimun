use std::sync::mpsc;

use dioxus::{hooks::use_signal, prelude::*};

use crate::noters::{
    nfs::{EntryData, NotePath},
    NoteVault,
};

#[derive(Props, Clone, PartialEq)]
pub struct NoteBrowserProps {
    note_vault: NoteVault,
    note_path: Signal<Option<NotePath>>,
}

#[allow(non_snake_case)]
pub fn NoteBrowser(props: NoteBrowserProps) -> Element {
    let mut note_path = props.note_path;
    let mut notes_and_dirs = use_signal(|| NotesAndDirs::new(props.note_vault, &note_path.read()));
    let entries = notes_and_dirs.read().entries.clone();
    let current_path = notes_and_dirs.read().get_current();
    rsx! {
        div {
            class: "flex flex-col h-full border border-solid border-2 border-blue-400",
            div {
                class: "flex shrink-0",
                "Files: " {current_path.to_string()}
            }
            div {
                class: "flex flex-col flex-1 overflow-hidden hover:overflow-auto border border-solid border-2 border-lime-400",
                if current_path != NotePath::root() {
                    div {
                        onclick: move |_| notes_and_dirs.write().go_up(),
                        "[UP]"
                    }
                }
                for entry in entries {
                    {
                        match entry {
                            NavEntry::Note(path) => rsx! {
                                div {
                                    onclick: move |_| *note_path.write() = Some(path.clone()),
                                    { path.get_name() }
                                }
                            },
                            NavEntry::Directory(path) => rsx! {
                                div {
                                    onclick: move |_| notes_and_dirs.write().enter_dir(&path),
                                    { path.get_name() }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
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

struct NotesAndDirs {
    vault: NoteVault,
    current_path: NotePath,
    entries: Vec<NavEntry>,
    err: Option<String>,
}

impl NotesAndDirs {
    fn new(vault: NoteVault, path: &Option<NotePath>) -> Self {
        let current_path = if let Some(path) = path {
            if path.is_note() {
                path.get_parent_path().0
            } else {
                path.to_owned()
            }
        } else {
            NotePath::root()
        };
        let entries = vec![];
        let err = None;
        let mut notes_and_dirs = Self {
            vault,
            current_path,
            entries,
            err,
        };
        notes_and_dirs.load_current_dir();
        notes_and_dirs
    }

    fn open_note(&self, path: &NotePath) -> anyhow::Result<String> {
        self.vault.load_note(path)
    }

    fn load_current_dir(&mut self) {
        let (tx, rx) = mpsc::channel();
        // TODO: manage error
        self.vault
            .get_notes_at(&self.current_path, tx, false)
            .expect("Error fetching Entries");
        self.entries.clear();
        while let Ok(entry) = rx.recv() {
            let nav_entry = match &entry.data {
                EntryData::Note(note_data) => Some(NavEntry::Note(note_data.path.clone())),
                EntryData::Directory(directory_data) => {
                    if directory_data.path == self.current_path {
                        None
                    } else {
                        Some(NavEntry::Directory(directory_data.path.clone()))
                    }
                }
                EntryData::Attachment => None,
            };
            if let Some(e) = nav_entry {
                self.entries.push(e);
            }
        }
        self.entries
            .sort_by_key(|b| std::cmp::Reverse(b.sort_string()));
    }

    fn get_current(&self) -> NotePath {
        self.current_path.clone()
    }

    fn go_up(&mut self) {
        self.current_path = self.current_path.get_parent_path().0;
        self.load_current_dir();
    }

    fn enter_dir(&mut self, path: &NotePath) {
        self.current_path = path.to_owned();
        self.load_current_dir();
    }

    fn cleat_err(&mut self) {
        self.err = None;
    }
}
