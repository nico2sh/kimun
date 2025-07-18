use dioxus::{logger::tracing::info, prelude::*, signals::SyncSignal};
use kimun_core::{nfs::VaultPath, note::NoteContentData};

#[derive(Clone, Eq, PartialEq)]
pub enum SortCriteria {
    Title,
    FileName,
}

pub trait RowItem: PartialEq + Eq + Clone {
    fn on_select(&self) -> Box<dyn FnMut() -> bool>;
    fn get_view(&self) -> Element;
}

#[derive(Clone, Eq, PartialEq)]
pub enum NoteSelectEntry {
    Note {
        path: VaultPath,
        title: String,
        search_str: String,
    },
    Directory {
        path: VaultPath,
        name: String,
        browse_path_signal: SyncSignal<VaultPath>,
    },
    Create {
        new_note_path: VaultPath,
        name: String,
    },
}

impl NoteSelectEntry {
    pub fn from_note_details(path: VaultPath, content: NoteContentData) -> Self {
        let path_str = format!("{} {}", content.title, path.get_name());
        let title = if content.title.is_empty() {
            "<No title>".to_string()
        } else {
            content.title
        };
        Self::Note {
            path: path.clone(),
            title,
            search_str: path_str,
        }
    }

    pub fn from_directory_details(
        path: VaultPath,
        base_path_signal: SyncSignal<VaultPath>,
    ) -> Self {
        let name = path.get_name();
        Self::Directory {
            path,
            name,
            browse_path_signal: base_path_signal,
        }
    }
    pub fn create_from_name(name: String, base_path: VaultPath) -> Self {
        let note_path = VaultPath::note_path_from(name);
        let new_note_path = base_path.append(&note_path);
        let name = new_note_path.to_string();
        Self::Create {
            new_note_path,
            name,
        }
    }

    pub fn sort_string(&self) -> String {
        match &self {
            NoteSelectEntry::Note {
                path,
                title: _,
                search_str: _,
            } => format!("2-{}", path),
            NoteSelectEntry::Directory {
                path,
                name: _,
                browse_path_signal: _,
            } => format!("1-{}", path),
            NoteSelectEntry::Create {
                name: _,
                new_note_path: _,
            } => format!("0"),
        }
    }

    pub fn sort_string_for(&self, criteria: &SortCriteria) -> String {
        match &self {
            NoteSelectEntry::Note {
                path,
                title,
                search_str: _,
            } => format!(
                "2-{}",
                match criteria {
                    SortCriteria::Title => title.to_owned(),
                    SortCriteria::FileName => path.to_string(),
                }
            ),
            NoteSelectEntry::Directory {
                path,
                name: _,
                browse_path_signal: _,
            } => format!("1-{}", path),
            NoteSelectEntry::Create {
                name: _,
                new_note_path: _,
            } => format!("0"),
        }
    }
}

impl AsRef<str> for NoteSelectEntry {
    fn as_ref(&self) -> &str {
        match self {
            NoteSelectEntry::Note {
                path: _,
                title: _,
                search_str,
            } => search_str.as_str(),
            NoteSelectEntry::Directory {
                path: _,
                name,
                browse_path_signal: _,
            } => name.as_str(),
            NoteSelectEntry::Create {
                new_note_path: _,
                name,
            } => name.as_str(),
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
            } => {
                let path = path.to_owned();
                Box::new(move || {
                    navigator().replace(crate::Route::Editor {
                        note_path: path.clone(),
                        create: false,
                    });
                    true
                })
            }
            NoteSelectEntry::Directory {
                path,
                name: _,
                browse_path_signal: base_path_signal,
            } => {
                let p = path.clone();
                let mut s = *base_path_signal;
                info!("Selected dir: {}", p);
                Box::new(move || {
                    s.set(p.clone());
                    false
                })
            }
            NoteSelectEntry::Create {
                new_note_path,
                name: _,
            } => {
                let path = new_note_path.to_owned();
                Box::new(move || {
                    navigator().replace(crate::Route::Editor {
                        note_path: path.clone(),
                        create: true,
                    });
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
            } => {
                rsx! {
                    div { class: "element",
                        div { class: "icon-note note-title", "{title}" }
                        div { class: "note-meta", "{path.get_name()}" }
                    }
                }
            }
            NoteSelectEntry::Directory {
                path: _,
                name,
                browse_path_signal: _,
            } => {
                rsx! {
                    div { class: "element",
                        div { class: "icon-folder title", "{name}" }
                    }
                }
            }
            NoteSelectEntry::Create {
                new_note_path: _,
                name,
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
