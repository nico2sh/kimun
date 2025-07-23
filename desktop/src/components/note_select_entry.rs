use chrono::Datelike;
use chrono::NaiveDate;
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
    Journal {
        path: VaultPath,
        title: String,
        date_string: String,
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
            NoteSelectEntry::Directory {
                path: _,
                name,
                browse_path_signal: _,
            } => name.eq(".."),
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
            NoteSelectEntry::Journal {
                path,
                title,
                date_string: _,
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
            NoteSelectEntry::Directory {
                path,
                name: _,
                browse_path_signal: _,
            } => path,
            NoteSelectEntry::Create {
                new_note_path,
                name: _,
            } => new_note_path,
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
                        editor_path: path.clone(),
                        create: false,
                    });
                    true
                })
            }
            NoteSelectEntry::Journal {
                path,
                title: _,
                date_string: _,
                search_str: _,
            } => {
                let path = path.to_owned();
                Box::new(move || {
                    navigator().replace(crate::Route::Editor {
                        editor_path: path.clone(),
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
                        editor_path: path.clone(),
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
            NoteSelectEntry::Journal {
                path,
                title,
                date_string,
                search_str: _,
            } => {
                rsx! {
                    div { class: "element",
                        div { class: "icon-note note-title", "{title}" }
                        div { class: "note-meta", "{path.get_name()}" }
                        div { class: "note-journal", "{date_string}" }
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
