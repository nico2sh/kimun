use core_notes::{
    nfs::{NoteDetails, NotePath},
    NoteVault,
};

use dioxus::prelude::*;
use log::{debug, error};

use crate::AppContext;

use super::{Modal, RowItem, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SearchProps {
    modal: Signal<Modal>,
    filter_text: String,
    note_path: SyncSignal<Option<NotePath>>,
}

#[allow(non_snake_case)]
pub fn NoteSearch(props: SearchProps) -> Element {
    let current_note_path = props.note_path;
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;

    let init = move || {
        debug!("Opening Note Search");
        vec![]
    };

    let moved_vault = vault.clone();
    let filter = move |filter_text: String, _items: Vec<NoteSearchEntry>| match moved_vault
        .search_notes(filter_text, true)
    {
        Ok(res) => res
            .into_iter()
            .map(|p| NoteSearchEntry::from_note_details(p, current_note_path))
            .collect::<Vec<NoteSearchEntry>>(),
        Err(e) => {
            error!("Error searching notes: {}", e);
            vec![]
        }
    };

    let moved_vault = vault.clone();
    let preview = move |entry: &NoteSearchEntry| {
        // sleep(Duration::from_millis(1000));
        moved_vault
            .load_note(&entry.note.path)
            .unwrap_or_else(|_e| "Error loading preview...".to_string())
    };

    SelectorView(
        "Select a note, use up and down to select, <Return> selects the first result.".to_string(),
        props.filter_text,
        props.modal,
        Box::new(init),
        Box::new(filter),
        Some(preview),
    )
}

#[derive(Clone, Eq, PartialEq)]
pub struct NoteSearchEntry {
    note: NoteDetails,
    search_str: String,
    path_signal: SyncSignal<Option<NotePath>>,
}

impl NoteSearchEntry {
    pub fn from_note_details(note: NoteDetails, path_signal: SyncSignal<Option<NotePath>>) -> Self {
        let path_str = format!("{} {}", note.path, note.title);
        Self {
            note,
            search_str: path_str,
            path_signal,
        }
    }
}

impl AsRef<str> for NoteSearchEntry {
    fn as_ref(&self) -> &str {
        self.search_str.as_str()
    }
}

impl RowItem for NoteSearchEntry {
    fn on_select(&self) -> Box<dyn FnMut()> {
        let p = self.note.path.clone();
        let mut s = self.path_signal;
        Box::new(move || s.set(Some(p.clone())))
    }

    fn get_view(&self) -> Element {
        rsx! {
            div {
                class: "title",
                "{self.note.title}"
            }
            div {
                class: "details",
                "{self.note.path.to_string()}"
            }
        }
    }
}
