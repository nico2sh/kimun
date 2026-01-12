use chrono::NaiveDate;
use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, note::NoteContentData};

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub enum SortCriteria {
    #[default]
    None,
    Title,
    FileName,
}

pub trait RowItem: PartialEq + Eq + Clone {
    // fn on_select(&self) -> bool;
    fn get_view(&self) -> Element;
}

#[derive(Clone, Eq, PartialEq)]
pub enum NoteSelectEntryListStatus {
    Loading,
    Loaded(Vec<NoteBrowseEntry>),
}

impl NoteSelectEntryListStatus {
    pub fn len(&self) -> usize {
        match self {
            NoteSelectEntryListStatus::Loading => 0,
            NoteSelectEntryListStatus::Loaded(items) => items.len(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NoteBrowseEntry {
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
    Directory {
        path: VaultPath,
        name: String,
    },
    Create {
        new_note_path: VaultPath,
        name: String,
    },
}

impl NoteBrowseEntry {
    pub fn from_note_details(path: VaultPath, content: NoteContentData) -> Self {
        let path_str = format!("{} {}", content.title, path.get_name());
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

    pub fn is_up_dir(&self) -> bool {
        match self {
            NoteBrowseEntry::Directory { path: _, name } => name.eq(".."),
            _ => false,
        }
    }

    pub fn from_note_journal(path: VaultPath, content: NoteContentData, date: NaiveDate) -> Self {
        let path_str = format!("{} {}", content.title, path.get_name());
        let title = if content.title.trim().is_empty() {
            "<No title>".to_string()
        } else {
            content.title
        };

        // let weekday = match date.weekday() {
        //     chrono::Weekday::Mon => "Monday",
        //     chrono::Weekday::Tue => "Tuesday",
        //     chrono::Weekday::Wed => "Wednesday",
        //     chrono::Weekday::Thu => "Thursday",
        //     chrono::Weekday::Fri => "Friday",
        //     chrono::Weekday::Sat => "Saturday",
        //     chrono::Weekday::Sun => "Sunday",
        // };
        let date_string = format!("{}", date.format("%a, %b %e %Y"));

        Self::Journal {
            path: path.clone(),
            title,
            date_string,
            search_str: path_str,
        }
    }

    pub fn from_directory_details(path: VaultPath) -> Self {
        let name = path.get_name();
        Self::Directory { path, name }
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
            NoteBrowseEntry::Note {
                path,
                title,
                search_str: _,
            } => format!(
                "2-{}",
                match criteria {
                    SortCriteria::Title => title.to_owned(),
                    SortCriteria::FileName => path.to_string(),
                    SortCriteria::None => "".to_string(),
                }
            ),
            NoteBrowseEntry::Journal {
                path,
                title,
                date_string: _,
                search_str: _,
            } => format!(
                "2-{}",
                match criteria {
                    SortCriteria::Title => title.to_owned(),
                    SortCriteria::FileName => path.to_string(),
                    SortCriteria::None => "".to_string(),
                }
            ),
            NoteBrowseEntry::Directory { path, name: _ } => format!("1-{}", path),
            NoteBrowseEntry::Create {
                name: _,
                new_note_path: _,
            } => "0".to_string(),
        }
    }

    pub fn get_path(&self) -> &VaultPath {
        match self {
            NoteBrowseEntry::Note {
                path,
                title: _,
                search_str: _,
            } => path,
            NoteBrowseEntry::Journal {
                path,
                title: _,
                date_string: _,
                search_str: _,
            } => path,
            NoteBrowseEntry::Directory { path, name: _ } => path,
            NoteBrowseEntry::Create {
                new_note_path,
                name: _,
            } => new_note_path,
        }
    }
}

impl AsRef<str> for NoteBrowseEntry {
    fn as_ref(&self) -> &str {
        match self {
            NoteBrowseEntry::Note {
                path: _,
                title: _,
                search_str,
            } => search_str.as_str(),
            NoteBrowseEntry::Journal {
                path: _,
                title: _,
                date_string: _,
                search_str,
            } => search_str.as_str(),
            NoteBrowseEntry::Directory { path: _, name } => name.as_str(),
            NoteBrowseEntry::Create {
                new_note_path: _,
                name,
            } => name.as_str(),
        }
    }
}

impl RowItem for NoteBrowseEntry {
    // fn on_select(&self) -> bool {
    //     let mut app_state: Signal<AppState> = use_context();
    //     match self {
    //         NoteBrowseEntry::Note {
    //             path,
    //             title: _,
    //             search_str: _,
    //         } => {
    //             app_state.write().set_path(&path, false);
    //             true
    //         }
    //         NoteBrowseEntry::Journal {
    //             path,
    //             title: _,
    //             date_string: _,
    //             search_str: _,
    //         } => {
    //             app_state.write().set_path(&path, false);
    //             true
    //         }
    //         NoteBrowseEntry::Directory {
    //             path,
    //             name: _,
    //             browse_path_signal: base_path_signal,
    //         } => {
    //             let p = path.clone();
    //             let mut s = *base_path_signal;
    //             info!("Selected dir: {}", p);
    //             s.set(p.clone());
    //             false
    //         }
    //         NoteBrowseEntry::Create {
    //             new_note_path,
    //             name: _,
    //         } => {
    //             app_state.write().set_path(&new_note_path, true);
    //             true
    //         }
    //     }
    // }

    fn get_view(&self) -> Element {
        match self {
            NoteBrowseEntry::Note {
                path,
                title,
                search_str: _,
            } => {
                rsx! {
                    div { class: "note-item-content",
                        div { class: "icon-note note-title", "{title}" }
                        div { class: "note-meta", "{path}" }
                    }
                }
            }
            NoteBrowseEntry::Journal {
                path,
                title,
                date_string,
                search_str: _,
            } => {
                rsx! {
                    div { class: "note-item-content",
                        div { class: "icon-note note-title", "{title}" }
                        div { class: "note-meta", "{path.get_name()}" }
                        div { class: "note-journal", "{date_string}" }
                    }
                }
            }
            NoteBrowseEntry::Directory { path: _, name } => {
                rsx! {
                    div { class: "note-item-content",
                        div { class: "icon-folder note-title", "{name}" }
                    }
                }
            }
            NoteBrowseEntry::Create {
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
