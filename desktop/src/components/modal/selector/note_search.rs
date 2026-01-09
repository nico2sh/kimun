use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, error},
    prelude::*,
};
use kimun_core::NoteVault;

use crate::{
    app_state::AppState,
    components::{
        modal::{selector::PreviewData, ModalType},
        note_list_data::note_select_entry::NoteSelectEntry,
    },
};

use super::{SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SearchProps {
    modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    filter_text: String,
}

#[derive(Clone, PartialEq)]
struct SearchFunctions {
    vault: Arc<NoteVault>,
}

impl SelectorFunctions for SearchFunctions {
    fn init(&self) -> Vec<NoteSelectEntry> {
        debug!("Opening Note Search");
        vec![]
    }

    fn filter(&self, filter_text: String, _items: &[NoteSelectEntry]) -> Vec<NoteSelectEntry> {
        debug!("Searching {}", filter_text);
        match self.vault.search_notes(filter_text) {
            Ok(res) => res
                .into_iter()
                .map(|(entry, content)| NoteSelectEntry::from_note_details(entry.path, content))
                .collect::<Vec<NoteSelectEntry>>(),
            Err(e) => {
                error!("Error searching notes: {}", e);
                vec![]
            }
        }
    }

    fn preview(&self, element: &NoteSelectEntry) -> Option<PreviewData> {
        let preview = self.vault.load_note(&element.get_path()).map_or_else(
            |e| PreviewData {
                title: "Error loading preview...".to_string(),
                data: e.to_string(),
                content: "".to_string(),
            },
            |d| PreviewData {
                title: d.get_title(),
                data: d.path.to_string(),
                content: d.raw_text,
            },
        );
        Some(preview)
    }

    fn on_select(&mut self, element: &NoteSelectEntry) -> bool {
        let mut app_state: Signal<AppState> = use_context();
        match element {
            NoteSelectEntry::Note {
                path,
                title: _,
                search_str: _,
            } => {
                app_state.write().set_path(&path, false);
                true
            }
            NoteSelectEntry::Journal {
                path,
                title: _,
                date_string: _,
                search_str: _,
            } => {
                app_state.write().set_path(&path, false);
                true
            }
            NoteSelectEntry::Create {
                new_note_path,
                name: _,
            } => {
                app_state.write().set_path(&new_note_path, true);
                true
            }
        }
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
        props.modal_type,
        search_functions,
    )
}

// #[derive(Clone, Eq, PartialEq)]
// pub struct NoteSearchEntry {
//     note_path: VaultPath,
//     note_title: String,
//     search_str: String,
// }

// impl NoteSearchEntry {
//     pub fn from_note_details(note: (NoteEntryData, NoteContentData)) -> Self {
//         let entry = note.0;
//         let content = note.1;
//         let note_path = entry.path.clone();
//         let note_title = content.title;
//         let path_str = format!("{} {}", note_path, note_title);
//         Self {
//             note_path,
//             note_title,
//             search_str: path_str,
//         }
//     }
// }

// impl AsRef<str> for NoteSearchEntry {
//     fn as_ref(&self) -> &str {
//         self.search_str.as_str()
//     }
// }

// impl RowItem for NoteSearchEntry {
//     fn on_select(&self) -> bool {
//         let encoded_path = encode_path(&self.note_path);
//         navigator().replace(crate::Route::MainView {
//             encoded_path,
//             create: false,
//         });
//         true
//     }

//     fn get_view(&self) -> Element {
//         rsx! {
//             div {
//                 class: "note-item-content",
//                 div { class: "note-title", "{self.note_title}" }
//                 div { class: "note-meta", "{self.note_path.to_string()}" }
//             }
//         }
//     }
// }
