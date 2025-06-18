use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, error},
    prelude::*,
};
use kimun_core::{
    nfs::{NoteEntryData, VaultPath},
    note::NoteContentData,
    NoteVault,
};

use super::{Modal, RowItem, SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SearchProps {
    modal: Signal<Modal>,
    vault: Arc<NoteVault>,
    filter_text: String,
}

#[derive(Clone, PartialEq)]
struct SearchFunctions {
    vault: Arc<NoteVault>,
}

impl SelectorFunctions<NoteSearchEntry> for SearchFunctions {
    fn init(&self) -> Vec<NoteSearchEntry> {
        debug!("Opening Note Search");
        vec![]
    }

    fn filter(&self, filter_text: String, _items: &Vec<NoteSearchEntry>) -> Vec<NoteSearchEntry> {
        match self.vault.search_notes(filter_text) {
            Ok(res) => res
                .into_iter()
                .map(|p| NoteSearchEntry::from_note_details(p))
                .collect::<Vec<NoteSearchEntry>>(),
            Err(e) => {
                error!("Error searching notes: {}", e);
                vec![]
            }
        }
    }

    fn preview(&self, element: &NoteSearchEntry) -> Option<String> {
        let preview = self
            .vault
            .load_note(&element.note_path)
            .map_or_else(|_e| "Error loading preview...".to_string(), |d| d.raw_text);
        Some(preview)
    }
}

#[allow(non_snake_case)]
pub fn NoteSearch(props: SearchProps) -> Element {
    let vault = props.vault;

    let search_functions = SearchFunctions {
        vault: vault.clone(),
    };

    SelectorView(
        "Select a note, use up and down to select, <Return> selects the first result.".to_string(),
        props.filter_text,
        props.modal,
        search_functions,
    )
}

#[derive(Clone, Eq, PartialEq)]
pub struct NoteSearchEntry {
    note_path: VaultPath,
    note_title: String,
    search_str: String,
}

impl NoteSearchEntry {
    pub fn from_note_details(note: (NoteEntryData, NoteContentData)) -> Self {
        let entry = note.0;
        let content = note.1;
        let note_path = entry.path.clone();
        let note_title = content.title;
        let path_str = format!("{} {}", note_path, note_title);
        Self {
            note_path,
            note_title,
            search_str: path_str,
        }
    }
}

impl AsRef<str> for NoteSearchEntry {
    fn as_ref(&self) -> &str {
        self.search_str.as_str()
    }
}

impl RowItem for NoteSearchEntry {
    fn on_select(&self) -> Box<dyn FnMut() -> bool> {
        let path = self.note_path.to_owned();
        Box::new(move || {
            navigator().replace(crate::Route::Editor {
                note_path: path.clone(),
                create: false,
            });
            true
        })
    }

    fn get_view(&self) -> Element {
        rsx! {
            div { class: "title", "{self.note_title}" }
            div { class: "details", "{self.note_path.to_string()}" }
        }
    }
}
