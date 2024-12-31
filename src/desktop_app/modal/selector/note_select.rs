use crate::{
    core_notes::{
        nfs::NotePath, DirectoryDetails, NoteDetails, NoteVault, SearchResult, VaultBrowseOptions,
        VaultBrowseOptionsBuilder,
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
    current_base_path: SyncSignal<NotePath>,
}

impl SelectFunctions {
    fn open(&self) -> Vec<NoteSelectEntry> {
        let (search_options, rx) = VaultBrowseOptionsBuilder::new(&self.current_base_path.read())
            .no_validation()
            .non_recursive()
            .build();
        let res = self.vault.browse_vault(search_options);

        let mut result = vec![];
        while let Ok(sr) = rx.recv() {
            match sr {
                SearchResult::Note(note_details) => {
                    result.push(NoteSelectEntry::from_note_details(
                        note_details,
                        self.current_note_path,
                    ));
                }
                SearchResult::Directory(directory_details) => {
                    result.push(NoteSelectEntry::from_directory_details(
                        directory_details,
                        self.current_base_path,
                    ));
                }
                _ => {}
            }
        }
        result
    }
}

impl SelectorFunctions<NoteSelectEntry> for SelectFunctions {
    fn init(&self) -> Vec<NoteSelectEntry> {
        debug!("Opening Note Selector");

        let items = self.open().into_iter().collect::<Vec<NoteSelectEntry>>();
        debug!("Loaded {} items", items.len());
        items
    }

    fn filter(&self, filter_text: String, items: Vec<NoteSelectEntry>) -> Vec<NoteSelectEntry> {
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

    fn preview(&self, element: &NoteSelectEntry) -> Option<String> {
        let preview = if let NoteSelectEntry::Note {
            note,
            search_str: _,
            path_signal: _,
        } = element
        {
            self.vault
                .load_note(&note.path)
                .unwrap_or_else(|_e| "Error loading preview...".to_string())
        } else {
            "".to_string()
        };
        Some(preview)
    }
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
    let current_base_path = use_signal_sync(|| {
        current_note_path
            .read()
            .to_owned()
            .map_or_else(NotePath::root, |path| {
                if path.is_note() {
                    path.get_parent_path().0
                } else {
                    path
                }
            })
    });
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;

    let moved_vault = vault.clone();

    let select_functions = SelectFunctions {
        vault: moved_vault,
        current_note_path,
        current_base_path,
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
    Directory {
        dir: DirectoryDetails,
        name: String,
        base_path_signal: SyncSignal<NotePath>,
    },
    Create {
        name: String,
        path_signal: SyncSignal<Option<NotePath>>,
    },
}

impl NoteSelectEntry {
    pub fn from_note_details(note: NoteDetails, path_signal: SyncSignal<Option<NotePath>>) -> Self {
        let path_str = format!("{} {}", note.get_title(), note.path);
        Self::Note {
            note,
            search_str: path_str,
            path_signal,
        }
    }

    pub fn from_directory_details(
        dir: DirectoryDetails,
        base_path_signal: SyncSignal<NotePath>,
    ) -> Self {
        let name = dir.path.get_parent_path().1;
        Self::Directory {
            dir,
            name,
            base_path_signal,
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
            NoteSelectEntry::Directory {
                dir: _,
                name,
                base_path_signal: _,
            } => name.as_str(),
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
            NoteSelectEntry::Directory {
                dir,
                name: _,
                base_path_signal,
            } => {
                let p = dir.path.clone();
                let mut s = *base_path_signal;
                Box::new(move || s.set(p.clone()))
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
                        "{note.get_title()}"
                    }
                    div {
                        class: "details",
                        "{note.path.to_string()}"
                    }
                }
            }
            NoteSelectEntry::Directory {
                dir: _,
                name,
                base_path_signal: _,
            } => {
                rsx! {
                    div {
                        class: "title",
                        "{name}"
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
