use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{nfs::VaultPath, NoteVault};
use nucleo::Matcher;

use crate::components::note_list::note_browse_entry::NoteBrowseEntry;

use super::{SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    vault: Arc<NoteVault>,
    filter_text: String,
    note_path: VaultPath,
}

#[derive(Clone, PartialEq)]
struct SelectFunctions {
    vault: Arc<NoteVault>,
    current_browse_path: SyncSignal<VaultPath>,
}

impl SelectFunctions {
    fn open(&self) -> Vec<NoteBrowseEntry> {
        match self.vault.get_all_notes() {
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

impl SelectorFunctions<String> for SelectFunctions {
    fn init(&self) -> Vec<NoteBrowseEntry> {
        debug!("Opening Note Selector");

        let items = self.open().into_iter().collect::<Vec<NoteBrowseEntry>>();
        debug!("Loaded {} items", items.len());
        items
    }

    fn filter(&self, filter_text: String, items: &[NoteBrowseEntry]) -> Vec<NoteBrowseEntry> {
        if !items.is_empty() {
            let mut result = Vec::new();
            if !filter_text.is_empty() {
                result.push(NoteBrowseEntry::create_from_name(
                    filter_text.to_owned(),
                    self.current_browse_path.read().to_owned(),
                ));
            }
            debug!("Filtering {}", filter_text);
            let mut fi = filter_items(items, filter_text);
            debug!("Filtered {} items", fi.len());
            result.append(&mut fi);
            result
        } else {
            vec![]
        }
    }
}

fn filter_items(items: &[NoteBrowseEntry], filter_text: String) -> Vec<NoteBrowseEntry> {
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let filtered = nucleo::pattern::Pattern::parse(
        filter_text.as_ref(),
        nucleo::pattern::CaseMatching::Ignore,
        nucleo::pattern::Normalization::Smart,
    )
    .match_list(items, &mut matcher)
    .iter()
    .map(|e| e.0.to_owned())
    .collect::<Vec<NoteBrowseEntry>>();
    filtered
}

#[allow(non_snake_case)]
pub fn NoteSelector(props: SelectorProps) -> Element {
    let current_note_path = props.note_path;
    info!("Current note path: {:?}", current_note_path);
    let current_base_path = use_signal_sync(|| {
        let mut p = if current_note_path.is_note() {
            current_note_path.get_parent_path().0
        } else {
            current_note_path
        };
        if p.is_relative() {
            p.to_absolute();
        }
        p
    });
    let vault = props.vault;

    let select_functions = SelectFunctions {
        vault: vault.clone(),
        current_browse_path: current_base_path,
    };
    SelectorView(
        "Use keywords to find notes, search is case insensitive and special characters are ignored.".to_string(),
        props.filter_text,
        vault,
        select_functions,
        false
    )
}
