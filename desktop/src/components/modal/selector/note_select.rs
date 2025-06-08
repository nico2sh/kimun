use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{
    nfs::VaultPath, note::NoteContentData, NoteVault, ResultType, VaultBrowseOptionsBuilder,
};
use nucleo::Matcher;

use super::{Modal, RowItem, SelectorFunctions, SelectorView};

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    modal: Signal<Modal>,
    vault: Arc<NoteVault>,
    filter_text: String,
    note_path: SyncSignal<Option<VaultPath>>,
}

#[derive(Clone, PartialEq)]
struct SelectFunctions {
    vault: Arc<NoteVault>,
    current_note_path: SyncSignal<Option<VaultPath>>,
    current_base_path: SyncSignal<VaultPath>,
}

impl SelectFunctions {
    fn open(&self) -> Vec<NoteSelectEntry> {
        let current_base_path = self.current_base_path.read().to_owned();
        let (search_options, rx) = VaultBrowseOptionsBuilder::new(&current_base_path)
            .no_validation()
            .non_recursive()
            .build();
        let _res = self.vault.browse_vault(search_options);

        let mut result = vec![];

        if !current_base_path.is_root_or_empty() {
            result.push(NoteSelectEntry::Directory {
                path: current_base_path.get_parent_path().0,
                name: "..".to_string(),
                base_path_signal: self.current_base_path,
            });
        }
        while let Ok(sr) = rx.recv() {
            match sr.rtype {
                ResultType::Note(note_content_data) => {
                    result.push(NoteSelectEntry::from_note_details(
                        sr.path,
                        note_content_data,
                        self.current_note_path,
                    ));
                }
                ResultType::Directory => {
                    if sr.path != current_base_path {
                        result.push(NoteSelectEntry::from_directory_details(
                            sr.path,
                            self.current_base_path,
                        ));
                    }
                }
                _ => {}
            }
        }
        result.sort_by_key(|b| sort_string(b));
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
            path,
            title: _,
            search_str: _,
            path_signal: _,
        } = element
        {
            self.vault
                .load_note(&path)
                .map_or_else(|_e| "Error loading preview...".to_string(), |d| d.raw_text)
        } else {
            "".to_string()
        };
        Some(preview)
    }
}

fn sort_string(entry: &NoteSelectEntry) -> String {
    match &entry {
        NoteSelectEntry::Note {
            path,
            title: _,
            search_str: _,
            path_signal: _,
        } => format!("2-{}", path),
        NoteSelectEntry::Directory {
            path,
            name: _,
            base_path_signal: _,
        } => format!("1-{}", path),
        NoteSelectEntry::Create {
            name: _,
            path_signal: _,
        } => format!("0"),
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
        let path = match current_note_path.read().to_owned() {
            Some(path) => path.to_owned(),
            None => VaultPath::root(),
        };
        if path.is_note() {
            path.get_parent_path().0
        } else {
            path
        }
    });
    let vault = props.vault;

    let select_functions = SelectFunctions {
        vault: vault.clone(),
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
        path: VaultPath,
        title: String,
        search_str: String,
        path_signal: SyncSignal<Option<VaultPath>>,
    },
    Directory {
        path: VaultPath,
        name: String,
        base_path_signal: SyncSignal<VaultPath>,
    },
    Create {
        name: String,
        path_signal: SyncSignal<Option<VaultPath>>,
    },
}

impl NoteSelectEntry {
    pub fn from_note_details(
        path: VaultPath,
        content: NoteContentData,
        path_signal: SyncSignal<Option<VaultPath>>,
    ) -> Self {
        let path_str = format!("{} {}", content.title, path);
        Self::Note {
            path: path.clone(),
            title: content.title,
            search_str: path_str,
            path_signal,
        }
    }

    pub fn from_directory_details(
        path: VaultPath,
        base_path_signal: SyncSignal<VaultPath>,
    ) -> Self {
        let name = path.get_parent_path().1;
        Self::Directory {
            path,
            name,
            base_path_signal,
        }
    }
    pub fn create_from_name(name: String, path_signal: SyncSignal<Option<VaultPath>>) -> Self {
        Self::Create { name, path_signal }
    }
}

impl AsRef<str> for NoteSelectEntry {
    fn as_ref(&self) -> &str {
        match self {
            NoteSelectEntry::Note {
                path: _,
                title: _,
                search_str,
                path_signal: _,
            } => search_str.as_str(),
            NoteSelectEntry::Directory {
                path: _,
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
    fn on_select(&self) -> Box<dyn FnMut() -> bool> {
        match self {
            NoteSelectEntry::Note {
                path,
                title: _,
                search_str: _,
                path_signal,
            } => {
                let p = path.clone();
                let mut s = *path_signal;
                Box::new(move || {
                    s.set(Some(p.clone()));
                    true
                })
            }
            NoteSelectEntry::Directory {
                path,
                name: _,
                base_path_signal,
            } => {
                let p = path.clone();
                let mut s = *base_path_signal;
                info!("Selected dir: {}", p);
                Box::new(move || {
                    s.set(p.clone());
                    false
                })
            }
            NoteSelectEntry::Create { name, path_signal } => {
                let path = VaultPath::note_path_from(name);
                let mut s = *path_signal;
                Box::new(move || {
                    s.set(Some(path.clone()));
                    true
                })
            }
        }
    }

    fn get_view(&self) -> Element {
        match self {
            NoteSelectEntry::Note {
                path,
                title,
                search_str: _,
                path_signal: _,
            } => {
                rsx! {
                    div { class: "title", "{title}" }
                    div { class: "details", "{path.to_string()}" }
                }
            }
            NoteSelectEntry::Directory {
                path: _,
                name,
                base_path_signal: _,
            } => {
                rsx! {
                    div { class: "title", "{name}" }
                }
            }
            NoteSelectEntry::Create {
                name,
                path_signal: _,
            } => {
                rsx! {
                    div { class: "note_create",
                        span { class: "emphasized", "Create new Note " }
                        span { class: "strong", "`{name}`" }
                    }
                }
            }
        }
    }
}
