use core_notes::{
    nfs::{NoteDetails, NotePath},
    NoteVault, SearchResult,
};

use dioxus::prelude::*;
use log::debug;
use nucleo::Matcher;

use crate::AppContext;

use super::{Modal, RowItem, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    modal: Signal<Modal>,
    filter_text: String,
    note_path: SyncSignal<Option<NotePath>>,
}

fn open(note_path: NotePath, vault: &NoteVault) -> Vec<NoteDetails> {
    let path = if note_path.is_note() {
        note_path.get_parent_path().0
    } else {
        note_path
    };
    vault
        .get_notes(path, true)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|sr| {
            if let SearchResult::Note(note) = sr {
                Some(note)
            } else {
                None
            }
        })
        .collect()
}

fn filter_items(items: Vec<NoteSelectEntry>, filter_text: String) -> Vec<NoteSelectEntry> {
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let filtered = nucleo::pattern::Pattern::parse(
        filter_text.as_ref(),
        nucleo::pattern::CaseMatching::Ignore,
        nucleo::pattern::Normalization::Smart,
    )
    .match_list(items, &mut matcher)
    .iter()
    .map(|e| e.0.to_owned())
    .collect::<Vec<NoteSelectEntry>>();
    filtered
}

#[allow(non_snake_case)]
pub fn NoteSelector(props: SelectorProps) -> Element {
    let current_note_path = props.note_path;
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;

    let moved_vault = vault.clone();
    let init = move || {
        debug!("Opening Note Selector");
        let items = open(NotePath::root(), &moved_vault)
            .into_iter()
            .map(|e| NoteSelectEntry::from_note_details(e, current_note_path))
            .collect::<Vec<NoteSelectEntry>>();
        debug!("Loaded {} items", items.len());
        items
    };

    let filter = move |filter_text: String, items: Vec<NoteSelectEntry>| {
        // dependencies
        if !items.is_empty() {
            let mut result = Vec::new();
            if !filter_text.is_empty() {
                result.push(NoteSelectEntry::create_from_name(
                    filter_text.to_owned(),
                    current_note_path,
                ));
                result.push(NoteSelectEntry::Separator);
            }
            debug!("Filtering {}", filter_text);
            let mut fi = filter_items(items, filter_text);
            debug!("Filtered {} items", fi.len());
            result.append(&mut fi);
            result
        } else {
            vec![]
        }
    };

    let moved_vault = vault.clone();
    let preview = move |entry: &NoteSelectEntry| {
        // sleep(Duration::from_millis(2000));
        if let NoteSelectEntry::Note {
            note,
            search_str: _,
            path_signal: _,
        } = entry
        {
            moved_vault
                .load_note(&note.path)
                .unwrap_or_else(|_e| "Error loading preview...".to_string())
        } else {
            "".to_string()
        }
    };

    SelectorView(
        "Use keywords to find notes, search is case insensitive and special characters are ignored.".to_string(),
        props.filter_text,
        props.modal,
        Box::new(init),
        Box::new(filter),
        Some(preview),
    )
}

#[derive(Clone, Eq, PartialEq)]
pub enum NoteSelectEntry {
    Note {
        note: NoteDetails,
        search_str: String,
        path_signal: SyncSignal<Option<NotePath>>,
    },
    Create {
        name: String,
        path_signal: SyncSignal<Option<NotePath>>,
    },
    Separator,
}

impl NoteSelectEntry {
    pub fn from_note_details(note: NoteDetails, path_signal: SyncSignal<Option<NotePath>>) -> Self {
        let path_str = format!("{} {}", note.path, note.title);
        Self::Note {
            note,
            search_str: path_str,
            path_signal,
        }
    }

    pub fn create_from_name(name: String, path_signal: SyncSignal<Option<NotePath>>) -> Self {
        Self::Create { name, path_signal }
    }
}

impl AsRef<str> for NoteSelectEntry {
    fn as_ref(&self) -> &str {
        match self {
            NoteSelectEntry::Note {
                note: _,
                search_str,
                path_signal: _,
            } => search_str.as_str(),
            NoteSelectEntry::Create { name, path_signal } => name,
            NoteSelectEntry::Separator => "",
        }
    }
}

impl RowItem for NoteSelectEntry {
    fn on_select(&self) -> Box<dyn FnMut()> {
        match self {
            NoteSelectEntry::Note {
                note,
                search_str: _,
                path_signal,
            } => {
                let p = note.path.clone();
                let mut s = *path_signal;
                Box::new(move || s.set(Some(p.clone())))
            }
            NoteSelectEntry::Create { name, path_signal } => {
                let p = NotePath::new(name);
                let mut s = *path_signal;
                Box::new(move || s.set(Some(p.clone())))
            }
            NoteSelectEntry::Separator => Box::new(|| {}),
        }
    }

    fn get_view(&self) -> Element {
        match self {
            NoteSelectEntry::Note {
                note,
                search_str: _,
                path_signal: _,
            } => {
                rsx! {
                    div {
                        class: "title",
                        "{note.title}"
                    }
                    div {
                        class: "details",
                        "{note.path.to_string()}"
                    }
                }
            }
            NoteSelectEntry::Create {
                name,
                path_signal: _,
            } => {
                rsx! {
                    div {
                        class: "note_create",
                        span {
                            class: "emphasized",
                            "Create new Note "
                        },
                        span {
                            class: "strong",
                            "`{name}`"
                        }
                    }
                }
            }
            NoteSelectEntry::Separator => {
                rsx! {
                    div {
                        class: "separator"
                    }
                }
            }
        }
    }
}
