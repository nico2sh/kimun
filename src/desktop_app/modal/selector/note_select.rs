use crate::{
    core_notes::{
        nfs::{NoteDetails, NotePath},
        NoteVault, SearchResult,
    },
    desktop_app::AppContext,
};

use dioxus::prelude::*;
use dioxus_logger::tracing::debug;
use nucleo::Matcher;

use super::{Modal, RowItem, SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    modal: Signal<Modal>,
    filter_text: String,
    note_path: SyncSignal<Option<NotePath>>,
}

#[derive(Clone, PartialEq)]
struct SelectFunctions {
    vault: NoteVault,
    current_note_path: SyncSignal<Option<NotePath>>,
}

impl SelectorFunctions<NoteSelectEntry> for SelectFunctions {
    async fn init(&self) -> Vec<NoteSelectEntry> {
        debug!("Opening Note Selector");
        let items = open(NotePath::root(), &self.vault)
            .await
            .into_iter()
            .map(|e| NoteSelectEntry::from_note_details(e, self.current_note_path))
            .collect::<Vec<NoteSelectEntry>>();
        debug!("Loaded {} items", items.len());
        items
    }

    async fn filter(
        &self,
        filter_text: String,
        items: Vec<NoteSelectEntry>,
    ) -> Vec<NoteSelectEntry> {
        if !items.is_empty() {
            let mut result = Vec::new();
            if !filter_text.is_empty() {
                result.push(NoteSelectEntry::create_from_name(
                    filter_text.to_owned(),
                    self.current_note_path,
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

    async fn preview(&self, element: &NoteSelectEntry) -> Option<String> {
        let preview = if let NoteSelectEntry::Note {
            note,
            search_str: _,
            path_signal: _,
        } = element
        {
            self.vault
                .load_note(&note.path)
                .await
                .unwrap_or_else(|_e| "Error loading preview...".to_string())
        } else {
            "".to_string()
        };
        Some(preview)
    }
}

async fn open(note_path: NotePath, vault: &NoteVault) -> Vec<NoteDetails> {
    let path = if note_path.is_note() {
        note_path.get_parent_path().0
    } else {
        note_path
    };
    vault
        .get_notes(path, true)
        .await
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

fn filter_items(items: Vec<NoteSelectEntry>, filter_text: String) -> Vec<NoteSelectEntry> {
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
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;

    let moved_vault = vault.clone();
    // let init = move || async {
    //     debug!("Opening Note Selector");
    //     let items = open(NotePath::root(), &moved_vault)
    //         .await
    //         .into_iter()
    //         .map(|e| NoteSelectEntry::from_note_details(e, current_note_path))
    //         .collect::<Vec<NoteSelectEntry>>();
    //     debug!("Loaded {} items", items.len());
    //     items
    // };
    //
    // let filter = move |filter_text: String, items: Vec<NoteSelectEntry>| async {
    //     // dependencies
    //     if !items.is_empty() {
    //         let mut result = Vec::new();
    //         if !filter_text.is_empty() {
    //             result.push(NoteSelectEntry::create_from_name(
    //                 filter_text.to_owned(),
    //                 current_note_path,
    //             ));
    //         }
    //         debug!("Filtering {}", filter_text);
    //         let mut fi = filter_items(items, filter_text);
    //         debug!("Filtered {} items", fi.len());
    //         result.append(&mut fi);
    //         result
    //     } else {
    //         vec![]
    //     }
    // };
    //
    // let moved_vault = vault.clone();
    // let preview = move |entry: &NoteSelectEntry| async {
    //     // sleep(Duration::from_millis(2000));
    //     if let NoteSelectEntry::Note {
    //         note,
    //         search_str: _,
    //         path_signal: _,
    //     } = entry
    //     {
    //         moved_vault
    //             .load_note(&note.path)
    //             .await
    //             .unwrap_or_else(|_e| "Error loading preview...".to_string())
    //     } else {
    //         "".to_string()
    //     }
    // };

    let select_functions = SelectFunctions {
        vault: moved_vault,
        current_note_path,
    };
    SelectorView(
        "Use keywords to find notes, search is case insensitive and special characters are ignored.".to_string(),
        props.filter_text,
        props.modal,
        select_functions
    )
}

#[derive(Clone, Eq, PartialEq)]
pub enum NoteSelectEntry {
    Note {
        note: NoteDetails,
        search_str: String,
        path_signal: SyncSignal<Option<NotePath>>,
    },
    Create {
        name: String,
        path_signal: SyncSignal<Option<NotePath>>,
    },
}

impl NoteSelectEntry {
    pub fn from_note_details(note: NoteDetails, path_signal: SyncSignal<Option<NotePath>>) -> Self {
        let path_str = format!("{} {}", note.path, note.title);
        Self::Note {
            note,
            search_str: path_str,
            path_signal,
        }
    }

    pub fn create_from_name(name: String, path_signal: SyncSignal<Option<NotePath>>) -> Self {
        Self::Create { name, path_signal }
    }
}

impl AsRef<str> for NoteSelectEntry {
    fn as_ref(&self) -> &str {
        match self {
            NoteSelectEntry::Note {
                note: _,
                search_str,
                path_signal: _,
            } => search_str.as_str(),
            NoteSelectEntry::Create {
                name,
                path_signal: _,
            } => name,
        }
    }
}

impl RowItem for NoteSelectEntry {
    fn on_select(&self) -> Box<dyn FnMut()> {
        match self {
            NoteSelectEntry::Note {
                note,
                search_str: _,
                path_signal,
            } => {
                let p = note.path.clone();
                let mut s = *path_signal;
                Box::new(move || s.set(Some(p.clone())))
            }
            NoteSelectEntry::Create { name, path_signal } => match NotePath::file_from(name) {
                Ok(p) => {
                    let mut s = *path_signal;
                    Box::new(move || s.set(Some(p.clone())))
                }
                Err(err) => {
                    let app_context: AppContext = use_context();
                    let mut error = app_context.current_error;
                    error.set(Some(format!("{}", err)));
                    Box::new(|| {})
                }
            },
        }
    }

    fn get_view(&self) -> Element {
        match self {
            NoteSelectEntry::Note {
                note,
                search_str: _,
                path_signal: _,
            } => {
                rsx! {
                    div {
                        class: "title",
                        "{note.title}"
                    }
                    div {
                        class: "details",
                        "{note.path.to_string()}"
                    }
                }
            }
            NoteSelectEntry::Create {
                name,
                path_signal: _,
            } => {
                rsx! {
                    div {
                        class: "note_create",
                        span {
                            class: "emphasized",
                            "Create new Note "
                        },
                        span {
                            class: "strong",
                            "`{name}`"
                        }
                    }
                }
            }
        }
    }
}
