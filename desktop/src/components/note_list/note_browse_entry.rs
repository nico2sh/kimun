use chrono::NaiveDate;
use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, note::NoteContentData};

use crate::themes::Theme;

#[derive(Default, Debug, Clone, Eq, PartialEq)]
enum EntryStyle {
    #[default]
    NoIcon,
    WithIcon,
}

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub enum SortCriteria {
    #[default]
    None,
    Title,
    FileName,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoteBrowseEntry {
    pub path: VaultPath,
    pub e_type: NoteEntryType,
    style: EntryStyle,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NoteEntryType {
    Note {
        title: String,
        search_str: String,
    },
    Journal {
        title: String,
        date_string: String,
        search_str: String,
    },
    Directory {
        name: String,
    },
    Create {
        name: String,
    },
}

impl NoteBrowseEntry {
    pub fn with_style_icon(mut self) -> Self {
        self.style = EntryStyle::WithIcon;
        self
    }

    pub fn from_note_details(path: VaultPath, content: NoteContentData) -> Self {
        let path_str = format!("{} {}", content.title, path.get_name());
        let title = if content.title.trim().is_empty() {
            "<No title>".to_string()
        } else {
            content.title
        };

        Self {
            path: path.clone(),
            e_type: NoteEntryType::Note {
                title,
                search_str: path_str,
            },
            style: EntryStyle::default(),
        }
    }

    pub fn new_note(path: VaultPath, title: String) -> Self {
        let search_str = format!("{} {}", title, path.get_name());
        Self {
            path,
            e_type: NoteEntryType::Note { title, search_str },
            style: EntryStyle::default(),
        }
    }

    pub fn is_up_dir(&self) -> bool {
        match &self.e_type {
            NoteEntryType::Directory { name } => name.eq(".."),
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

        let date_string = format!("{}", date.format("%a, %b %e %Y"));

        Self {
            path: path.clone(),
            e_type: NoteEntryType::Journal {
                title,
                date_string,
                search_str: path_str,
            },
            style: EntryStyle::default(),
        }
    }

    pub fn from_directory_details(path: VaultPath) -> Self {
        let name = path.get_name();
        Self {
            path,
            e_type: NoteEntryType::Directory { name },
            style: EntryStyle::default(),
        }
    }

    pub fn create_from_name(name: String, base_path: VaultPath) -> Self {
        let note_path = VaultPath::note_path_from(name);
        let new_note_path = base_path.append(&note_path);
        let name = new_note_path.to_string();

        Self {
            path: note_path,
            e_type: NoteEntryType::Create { name },
            style: EntryStyle::default(),
        }
    }

    pub fn up_dir_from(from_path: VaultPath) -> Self {
        Self {
            path: from_path.get_parent_path().0,
            e_type: NoteEntryType::Directory {
                name: "..".to_string(),
            },
            style: EntryStyle::default(),
        }
    }

    pub fn sort_string_for(&self, criteria: &SortCriteria) -> String {
        match &self.e_type {
            NoteEntryType::Note {
                title,
                search_str: _,
            } => format!(
                "2-{}",
                match criteria {
                    SortCriteria::Title => title.to_owned(),
                    SortCriteria::FileName => self.path.to_string(),
                    SortCriteria::None => "".to_string(),
                }
            ),
            NoteEntryType::Journal {
                title,
                date_string: _,
                search_str: _,
            } => format!(
                "2-{}",
                match criteria {
                    SortCriteria::Title => title.to_owned(),
                    SortCriteria::FileName => self.path.to_string(),
                    SortCriteria::None => "".to_string(),
                }
            ),
            NoteEntryType::Directory { name: _ } => format!("1-{}", self.path),
            NoteEntryType::Create { name: _ } => "0".to_string(),
        }
    }

    pub fn get_path(&self) -> &VaultPath {
        &self.path
    }

    pub fn get_view(&self, theme: &Theme) -> Element {
        match &self.e_type {
            NoteEntryType::Note {
                title,
                search_str: _,
            } => {
                rsx! {
                    div { class: "note-item-content",
                        if self.style == EntryStyle::WithIcon {
                            div { class: "icon-note note-title", "{title}" }
                        } else {
                            div {
                                class: "note-title",
                                color: "{theme.text_primary}",
                                "» {title}"
                            }
                        }
                        div { class: "note-meta", color: "{theme.text_light}", "{self.path}" }
                    }
                }
            }
            NoteEntryType::Journal {
                title,
                date_string,
                search_str: _,
            } => {
                rsx! {
                    div { class: "note-item-content",
                        if self.style == EntryStyle::WithIcon {
                            div { class: "icon-journal note-title", "{title}" }
                        } else {
                            div {
                                class: "note-title",
                                color: "{theme.text_primary}",
                                "◦ {title}"
                            }
                        }
                        div { class: "note-meta", color: "{theme.text_light}",
                            "{self.path.get_name()}"
                        }
                        div {
                            class: "note-journal",
                            color: "{theme.text_muted}",
                            "{date_string}"
                        }
                    }
                }
            }
            NoteEntryType::Directory { name } => {
                rsx! {
                    div { class: "note-item-content",
                        if self.style == EntryStyle::WithIcon {
                            div { class: "icon-folder note-title", "{name}" }
                        } else {
                            div {
                                class: "note-title",
                                color: "{theme.text_primary}",
                                "■ {name}"
                            }
                        }
                    }
                }
            }
            NoteEntryType::Create { name } => {
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

impl AsRef<str> for NoteBrowseEntry {
    fn as_ref(&self) -> &str {
        match &self.e_type {
            NoteEntryType::Note {
                title: _,
                search_str,
            } => search_str.as_str(),
            NoteEntryType::Journal {
                title: _,
                date_string: _,
                search_str,
            } => search_str.as_str(),
            NoteEntryType::Directory { name } => name.as_str(),
            NoteEntryType::Create { name } => name.as_str(),
        }
    }
}
