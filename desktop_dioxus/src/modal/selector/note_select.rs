use core_notes::{
    nfs::{NoteDetails, NotePath},
    NoteVault, SearchResult,
};

use dioxus::prelude::*;
use log::debug;
use nucleo::Matcher;

use crate::AppContext;

use super::{Modal, PathEntry, SelectorView};

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

fn filter_items(items: Vec<PathEntry>, filter_text: String) -> Vec<PathEntry> {
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let filtered = nucleo::pattern::Pattern::parse(
        filter_text.as_ref(),
        nucleo::pattern::CaseMatching::Ignore,
        nucleo::pattern::Normalization::Smart,
    )
    .match_list(items, &mut matcher)
    .iter()
    .map(|e| e.0.to_owned())
    .collect::<Vec<PathEntry>>();
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
            .map(|e| PathEntry::from_note_details(e, current_note_path))
            .collect::<Vec<PathEntry>>();
        debug!("Loaded {} items", items.len());
        items
    };

    let filter = |filter_text: String, items: Vec<PathEntry>| {
        // dependencies
        if !items.is_empty() {
            debug!("Filtering {}", filter_text);
            let fi = filter_items(items, filter_text);
            debug!("Filtered {} items", fi.len());
            fi
        } else {
            vec![]
        }
    };

    let moved_vault = vault.clone();
    let preview = move |entry: &PathEntry| {
        // sleep(Duration::from_millis(2000));
        moved_vault
            .load_note(&entry.note.path)
            .unwrap_or_else(|_e| "Error loading preview...".to_string())
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
