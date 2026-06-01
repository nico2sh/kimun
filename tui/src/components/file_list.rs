use kimun_core::nfs::VaultPath;
use kimun_core::{ResultType, SearchResult};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::ListItem;

use crate::settings::icons::Icons;
use crate::settings::themes::Theme;
use crate::settings::{SortFieldSetting, SortOrderSetting};

// ---------------------------------------------------------------------------
// Sort options
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum SortField {
    Name,
    Title,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl From<SortFieldSetting> for SortField {
    fn from(s: SortFieldSetting) -> Self {
        match s {
            SortFieldSetting::Name => Self::Name,
            SortFieldSetting::Title => Self::Title,
        }
    }
}

impl From<SortOrderSetting> for SortOrder {
    fn from(s: SortOrderSetting) -> Self {
        match s {
            SortOrderSetting::Ascending => Self::Ascending,
            SortOrderSetting::Descending => Self::Descending,
        }
    }
}

impl SortField {
    pub fn label(self) -> char {
        match self {
            Self::Name => 'N',
            Self::Title => 'T',
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Name => Self::Title,
            Self::Title => Self::Name,
        }
    }
}

impl SortOrder {
    pub fn label(self) -> char {
        match self {
            Self::Ascending => '↑',
            Self::Descending => '↓',
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

// ---------------------------------------------------------------------------
// FileListEntry
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum FileListEntry {
    Up {
        parent: VaultPath,
    },
    Note {
        path: VaultPath,
        title: String,
        filename: String,
        journal_date: Option<String>,
    },
    Directory {
        path: VaultPath,
        name: String,
    },
    Attachment {
        path: VaultPath,
        filename: String,
    },
    CreateNote {
        filename: String,
        path: VaultPath,
    },
}

impl FileListEntry {
    pub fn from_result(result: SearchResult, journal_date: Option<String>) -> Self {
        let filename = result.path.get_parent_path().1;
        match result.rtype {
            ResultType::Note(data) => {
                let title = if data.title.trim().is_empty() {
                    "<no title>".to_string()
                } else {
                    data.title
                };
                Self::Note {
                    path: result.path,
                    title,
                    filename,
                    journal_date,
                }
            }
            ResultType::Directory => Self::Directory {
                path: result.path,
                name: filename,
            },
            ResultType::Attachment => Self::Attachment {
                path: result.path,
                filename,
            },
        }
    }

    pub fn path(&self) -> &VaultPath {
        match self {
            Self::Up { parent } => parent,
            Self::Note { path, .. } => path,
            Self::Directory { path, .. } => path,
            Self::Attachment { path, .. } => path,
            Self::CreateNote { path, .. } => path,
        }
    }

    pub fn search_str(&self) -> Option<String> {
        match self {
            Self::Up { .. } => None,
            Self::Note {
                title, filename, ..
            } => Some(format!("{} {}", title, filename)),
            Self::Directory { name, .. } => Some(name.clone()),
            Self::Attachment { filename, .. } => Some(filename.clone()),
            Self::CreateNote { filename, .. } => Some(filename.clone()),
        }
    }

    /// Sort key for the given field.
    pub(crate) fn sort_key(&self, field: SortField) -> String {
        match self {
            Self::Up { .. } => String::new(),
            Self::Note {
                title, filename, ..
            } => match field {
                SortField::Title => title.to_lowercase(),
                SortField::Name => filename.to_lowercase(),
            },
            Self::Directory { name, .. } => name.to_lowercase(),
            Self::Attachment { filename, .. } => filename.to_lowercase(),
            Self::CreateNote { filename, .. } => filename.to_lowercase(),
        }
    }

    /// Terminal rows this entry occupies when rendered.
    pub fn visual_height(&self) -> u16 {
        match self {
            Self::Note { journal_date, .. } => {
                if journal_date.is_some() {
                    3
                } else {
                    2
                }
            }
            _ => 1,
        }
    }

    pub fn to_list_item(&self, theme: &Theme, icons: &Icons) -> ListItem<'static> {
        let lines: Vec<Line> = match self {
            Self::Up { .. } => vec![Line::from(Span::styled(
                format!("{} [UP] ..", icons.directory_up),
                Style::default().fg(theme.fg_muted.to_ratatui()),
            ))],
            Self::Note {
                title,
                filename,
                journal_date,
                ..
            } => {
                let mut lines = vec![];
                if let Some(date) = journal_date {
                    lines.push(Line::from(format!("{} {}", icons.journal, title)));
                    lines.push(Line::from(Span::styled(
                        format!(" {}", date),
                        Style::default().fg(theme.color_journal_date.to_ratatui()),
                    )));
                } else {
                    lines.push(Line::from(format!("{} {}", icons.note, title)));
                }
                lines.push(Line::from(Span::styled(
                    format!(" {}", filename),
                    Style::default()
                        .add_modifier(Modifier::ITALIC)
                        .fg(theme.fg_secondary.to_ratatui()),
                )));
                lines
            }
            Self::Directory { name, .. } => vec![Line::from(Span::styled(
                format!("{} {}", icons.directory, name),
                Style::default().fg(theme.color_directory.to_ratatui()),
            ))],
            Self::Attachment { filename, .. } => vec![Line::from(Span::styled(
                format!("{} {}", icons.attachment, filename),
                Style::default()
                    .add_modifier(Modifier::ITALIC)
                    .fg(theme.fg_secondary.to_ratatui()),
            ))],
            Self::CreateNote { filename, .. } => vec![Line::from(Span::styled(
                format!("+ Create: {}", filename),
                Style::default().fg(theme.accent.to_ratatui()),
            ))],
        };
        ListItem::new(Text::from(lines))
    }
}

impl crate::components::search_list::SearchRow for FileListEntry {
    fn to_list_item(&self, theme: &Theme, icons: &Icons, _selected: bool) -> ListItem<'static> {
        // Delegate to inherent method; engine applies selection highlight via `highlight_style`.
        FileListEntry::to_list_item(self, theme, icons)
    }

    fn visual_height(&self) -> u16 {
        FileListEntry::visual_height(self)
    }

    fn match_text(&self) -> Option<&str> {
        match self {
            Self::Note { filename, .. } | Self::CreateNote { filename, .. } => Some(filename),
            _ => None,
        }
    }
}
