use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, error},
    prelude::*,
};
use kimun_core::NoteVault;

use crate::components::note_list::note_browse_entry::NoteBrowseEntry;

use super::{SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SearchProps {
    vault: Arc<NoteVault>,
    filter_text: String,
}

#[derive(Clone, PartialEq)]
struct SearchFunctions {
    vault: Arc<NoteVault>,
}

impl SelectorFunctions<String> for SearchFunctions {
    async fn init(&self) -> Vec<NoteBrowseEntry> {
        debug!("Opening Note Search");
        vec![]
    }

    async fn filter(&self, filter_text: String, _items: &[NoteBrowseEntry]) -> Vec<NoteBrowseEntry> {
        debug!("Searching {}", filter_text);
        let vault = self.vault.clone();
        match vault.search_notes(filter_text).await {
            Ok(res) => res
                .into_iter()
                .map(|(entry, content)| NoteBrowseEntry::from_note_details(entry.path, content))
                .collect::<Vec<NoteBrowseEntry>>(),
            Err(e) => {
                error!("Error searching notes: {}", e);
                vec![]
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
        vault,
        search_functions,
        true,
    )
}
