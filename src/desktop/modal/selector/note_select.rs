use dioxus::prelude::*;
use log::{debug, error, warn};
use nucleo::Matcher;

use crate::{
    desktop::{modal::selector::SelectorView, AppContext},
    noters::{
        nfs::{NoteEntry, NotePath},
        NoteVault, NotesGetterOptions,
    },
};

use super::{Modal, PathEntry};

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    modal: Signal<Modal>,
    filter_text: String,
    note_path: Signal<Option<NotePath>>,
}

fn open(note_path: NotePath, vault: &NoteVault) -> Vec<NoteEntry> {
    let path = if note_path.is_note() {
        note_path.get_parent_path().0
    } else {
        note_path
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let options = NotesGetterOptions::default()
        .no_validation()
        .set_sender(tx)
        .recursive();
    if let Err(e) = vault.get_notes(path, options) {
        error!("{}", e);
    }

    let mut items = vec![];
    while let Ok(row) = rx.recv() {
        if let crate::noters::nfs::EntryData::Note(_) = row.data {
            items.push(row);
        }
    }

    // let options = NotesGetterOptions::default().recursive();
    // let items = vault.get_notes(path, options).unwrap().unwrap();
    items
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
    warn!("Opening Selector");
    let current_note_path = props.note_path;
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;

    let moved_vault = vault.clone();
    let init = move || {
        let items = open(NotePath::root(), &moved_vault)
            .iter()
            .map(|e| PathEntry::from_note_path(e.path.clone(), current_note_path))
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
    let preview = move |path: &PathEntry| {
        // sleep(Duration::from_millis(1000));
        moved_vault
            .load_note(&path.path)
            .unwrap_or_else(|_e| "Error loading preview...".to_string())
    };

    SelectorView(
        "Use keywords to find notes, search is case insensitive and special characters are ignored.".to_string(),
        props.filter_text,
        props.modal,
        Box::new(init),
        Box::new(filter),
        Box::new(preview),
    )
}
