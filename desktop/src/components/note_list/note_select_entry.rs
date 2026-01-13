use chrono::NaiveDate;
use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, note::NoteContentData};

use crate::components::note_browse_entry::SortCriteria;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NoteSelectEntry {
    Note {
        path: VaultPath,
        title: String,
        search_str: String,
    },
    Journal {
        path: VaultPath,
        title: String,
        date_string: String,
        search_str: String,
    },
    Create {
        new_note_path: VaultPath,
        name: String,
    },
}

impl NoteSelectEntry {
    pub fn from_note_details(path: VaultPath, content: NoteContentData) -> Self {
        let path_str = format!("{} {}", path.get_clean_name(), content.title);
        let title = if content.title.trim().is_empty() {
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

    pub fn from_note_journal(path: VaultPath, content: NoteContentData, date: NaiveDate) -> Self {
        let path_str = format!("{} {}", content.title, path.get_name());
        let title = if content.title.trim().is_empty() {
            "<No title>".to_string()
        } else {
            content.title
        };

        let date_string = format!("{}", date.format("%a, %b %e %Y"));

        Self::Journal {
            path: path.clone(),
            title,
            date_string,
            search_str: path_str,
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
                    SortCriteria::FileName => path.get_name(),
                    SortCriteria::None => "".to_string(),
                }
            ),
            NoteSelectEntry::Journal {
                path,
                title,
                date_string: _,
                search_str: _,
            } => format!(
                "2-{}",
                match criteria {
                    SortCriteria::Title => title.to_owned(),
                    SortCriteria::FileName => path.get_name(),
                    SortCriteria::None => "".to_string(),
                }
            ),
            NoteSelectEntry::Create {
                name: _,
                new_note_path: _,
            } => "0".to_string(),
        }
    }

    pub fn get_path(&self) -> &VaultPath {
        match self {
            NoteSelectEntry::Note {
                path,
                title: _,
                search_str: _,
            } => path,
            NoteSelectEntry::Journal {
                path,
                title: _,
                date_string: _,
                search_str: _,
            } => path,
            NoteSelectEntry::Create {
                new_note_path,
                name: _,
            } => new_note_path,
        }
    }

    pub fn get_view(&self) -> Element {
        match self {
            NoteSelectEntry::Note {
                path,
                title,
                search_str: _,
            } => {
                rsx! {
                    div { class: "note-item-content",
                        div { class: "note-title", "» {title}" }
                        div { class: "note-meta", "{path}" }
                    }
                }
            }
            NoteSelectEntry::Journal {
                path,
                title,
                date_string,
                search_str: _,
            } => {
                rsx! {
                    div { class: "note-item-content",
                        div { class: "note-title", "» {title}" }
                        div { class: "note-meta", "{path.get_name()}" }
                        div { class: "note-journal", "{date_string}" }
                    }
                }
            }
            NoteSelectEntry::Create {
                new_note_path: _,
                name,
            } => {
                rsx! {
                    div { class: "note-item-content",
                        span { class: "emphasized", "Create new Note " }
                        span { class: "strong", "`{name}`" }
                    }
                }
            }
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
            NoteSelectEntry::Journal {
                path: _,
                title: _,
                date_string: _,
                search_str,
            } => search_str.as_str(),
            NoteSelectEntry::Create {
                new_note_path: _,
                name,
            } => name.as_str(),
        }
    }
}
