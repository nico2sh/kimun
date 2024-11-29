use dioxus::prelude::*;
use log::{debug, error};

use crate::{
    desktop::{modal::selector::SelectorView, AppContext},
    noters::{nfs::NotePath, NoteVault},
};

use super::{Modal, PathEntry};

#[derive(Props, Clone, PartialEq)]
pub struct SearchProps {
    modal: Signal<Modal>,
    filter_text: String,
    note_path: Signal<Option<NotePath>>,
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
    let filter = move |filter_text: String, _items: Vec<PathEntry>| match moved_vault
        .search_notes(filter_text, true)
    {
        Ok(res) => res
            .into_iter()
            .map(|p| PathEntry::from_note_path(p, current_note_path))
            .collect::<Vec<PathEntry>>(),
        Err(e) => {
            error!("Error searching notes: {}", e);
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
        "Select a note, use up and down to select, <Return> selects the first result.".to_string(),
        props.filter_text,
        props.modal,
        Box::new(init),
        Box::new(filter),
        Some(preview),
    )
}
