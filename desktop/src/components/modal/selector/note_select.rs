use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{nfs::VaultPath, NoteVault};
use nucleo::Matcher;

use crate::{
    app_state::AppState,
    components::{
        modal::{selector::PreviewData, ModalType},
        note_list_data::note_select_entry::NoteSelectEntry,
    },
};

use super::{SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    modal_type: Signal<ModalType>,
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
    fn open(&self) -> Vec<NoteSelectEntry> {
        match self.vault.get_all_notes() {
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
}

impl SelectorFunctions for SelectFunctions {
    fn init(&self) -> Vec<NoteSelectEntry> {
        debug!("Opening Note Selector");

        let items = self.open().into_iter().collect::<Vec<NoteSelectEntry>>();
        debug!("Loaded {} items", items.len());
        items
    }

    fn filter(&self, filter_text: String, items: &[NoteSelectEntry]) -> Vec<NoteSelectEntry> {
        if !items.is_empty() {
            let mut result = Vec::new();
            if !filter_text.is_empty() {
                result.push(NoteSelectEntry::create_from_name(
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

    fn preview(&self, element: &NoteSelectEntry) -> Option<PreviewData> {
        if let NoteSelectEntry::Note {
            path,
            title: _,
            search_str: _,
        } = element
        {
            let p = self.vault.load_note(path).map_or_else(
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
            Some(p)
        } else {
            None
        }
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

fn filter_items(items: &[NoteSelectEntry], filter_text: String) -> Vec<NoteSelectEntry> {
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
        props.modal_type,
        select_functions
    )
}
