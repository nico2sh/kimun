use crate::{
    core_notes::{
        nfs::{NoteDetails, NotePath},
        NoteVault,
    },
    desktop_app::AppContext,
};

use dioxus::prelude::*;
use dioxus_logger::tracing::{debug, error};

use super::{Modal, RowItem, SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SearchProps {
    modal: Signal<Modal>,
    filter_text: String,
    note_path: SyncSignal<Option<NotePath>>,
}

#[derive(Clone, PartialEq)]
struct SearchFunctions {
    vault: NoteVault,
    current_note_path: SyncSignal<Option<NotePath>>,
}

impl SelectorFunctions<NoteSearchEntry> for SearchFunctions {
    fn init(&self) -> Vec<NoteSearchEntry> {
        debug!("Opening Note Search");
        vec![]
    }

    fn filter(&self, filter_text: String, _items: Vec<NoteSearchEntry>) -> Vec<NoteSearchEntry> {
        match self.vault.search_notes(filter_text, true) {
            Ok(res) => res
                .into_iter()
                .map(|p| NoteSearchEntry::from_note_details(p, self.current_note_path))
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
            .load_note(&element.note.path)
            .unwrap_or_else(|_e| "Error loading preview...".to_string());
        Some(preview)
    }
}

#[allow(non_snake_case)]
pub fn NoteSearch(props: SearchProps) -> Element {
    let current_note_path = props.note_path;
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;

    let moved_vault = vault.clone();

    let search_functions = SearchFunctions {
        vault: moved_vault,
        current_note_path,
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
