use kimun_core::nfs::VaultPath;
use kimun_core::{ResultType, SearchResult};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::ListItem;

use crate::components::rich_row::RichRow;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;
use crate::settings::{SortFieldSetting, SortOrderSetting};

// ---------------------------------------------------------------------------
// Sort options
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortField {
    Name,
    Title,
}

#[derive(Clone, Copy, PartialEq, Debug)]
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

impl From<SortField> for SortFieldSetting {
    fn from(s: SortField) -> Self {
        match s {
            SortField::Name => Self::Name,
            SortField::Title => Self::Title,
        }
    }
}

impl From<SortOrder> for SortOrderSetting {
    fn from(s: SortOrder) -> Self {
        match s {
            SortOrder::Ascending => Self::Ascending,
            SortOrder::Descending => Self::Descending,
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
        /// `true` when this is the note currently open in the editor. Drives the
        /// open-note marker (accent glyph). Stamped by the sidebar after each
        /// load; always `false` from the row source and on non-sidebar surfaces.
        is_open: bool,
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
            ResultType::Note(data) => Self::Note {
                path: result.path,
                title: Self::display_title(data.title),
                filename,
                journal_date,
                is_open: false,
            },
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

    /// Map a raw note title to its display form, substituting a placeholder
    /// for an empty/whitespace title. Shared by listing construction and the
    /// sidebar's live title updates so they never diverge.
    pub fn display_title(raw: String) -> String {
        if raw.trim().is_empty() {
            "<no title>".to_string()
        } else {
            raw
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
        match self {
            Self::Up { .. } => RichRow::new(icons.directory_up, "[UP] ..")
                .glyph_style(Style::default().fg(theme.gray.to_ratatui()))
                .title_style(Style::default().fg(theme.gray.to_ratatui()))
                .into_list_item(theme),
            Self::Note {
                title,
                filename,
                journal_date,
                is_open,
                ..
            } => {
                let glyph = if journal_date.is_some() {
                    icons.journal
                } else {
                    icons.note
                };
                let mut row = RichRow::new(glyph, title.clone()).filename(filename.clone());
                if *is_open {
                    // Open-note marker: accent the type glyph (see CONTEXT.md).
                    row = row.glyph_style(Style::default().fg(theme.accent.to_ratatui()));
                }
                if let Some(date) = journal_date {
                    row = row.secondary(
                        date.clone(),
                        Some(Style::default().fg(theme.color_journal_date.to_ratatui())),
                    );
                }
                row.into_list_item(theme)
            }
            Self::Directory { name, .. } => {
                let dir_style = Style::default().fg(theme.color_directory.to_ratatui());
                RichRow::new(icons.directory, name.clone())
                    .glyph_style(dir_style)
                    .title_style(dir_style)
                    .into_list_item(theme)
            }
            Self::Attachment { filename, .. } => {
                let style = Style::default()
                    .add_modifier(Modifier::ITALIC)
                    .fg(theme.fg_secondary.to_ratatui());
                RichRow::new(icons.attachment, filename.clone())
                    .glyph_style(style)
                    .title_style(style)
                    .into_list_item(theme)
            }
            Self::CreateNote { filename, .. } => {
                let style = Style::default().fg(theme.accent.to_ratatui());
                RichRow::new("+", format!("Create: {}", filename))
                    .glyph_style(style)
                    .title_style(style)
                    .into_list_item(theme)
            }
        }
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
            // Directories participate in the fuzzy filter (matched on their
            // name). `Up` stays exempt.
            Self::Directory { name, .. } => Some(name),
            _ => None,
        }
    }
}

#[cfg(test)]
mod open_marker_tests {
    use super::*;
    use ratatui::style::Style;
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::ListItem;

    #[test]
    fn display_title_substitutes_placeholder_for_empty() {
        assert_eq!(FileListEntry::display_title("   ".to_string()), "<no title>");
        assert_eq!(FileListEntry::display_title("Real".to_string()), "Real");
    }

    /// Build a `FileListEntry::Note` with the given `is_open` flag and call
    /// `to_list_item`, then compare the resulting `ListItem` against one whose
    /// first line's glyph span carries the expected fg color.
    ///
    /// `ListItem` derives `PartialEq` (comparing the inner `Text` and item-level
    /// `Style`).  `Text` / `Line` / `Span` all have public fields, so the
    /// comparison reaches down to `span.style.fg` without needing private access
    /// to `ListItem::content`.
    fn glyph_fg_of_note(is_open: bool) -> ratatui::style::Color {
        let theme = Theme::default();
        let icons = Icons::new(false);
        let note = FileListEntry::Note {
            path: kimun_core::nfs::VaultPath::note_path_from("a.md"),
            title: "A".to_string(),
            filename: "a.md".to_string(),
            journal_date: None,
            is_open,
        };
        // Build the expected glyph span using the same logic to_list_item uses,
        // then verify by comparing the whole ListItem via PartialEq.
        let fg = theme.fg.to_ratatui();
        let accent = theme.accent.to_ratatui();
        let glyph_style = if is_open {
            Style::default().fg(accent)
        } else {
            Style::default().fg(fg)
        };
        let title_style = Style::default().fg(fg);
        let secondary_style = Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(ratatui::style::Modifier::ITALIC);

        let expected_lines = vec![
            Line::from(vec![
                Span::styled(format!("{} ", icons.note), glyph_style),
                Span::styled("A", title_style),
            ]),
            Line::from(Span::styled("  a.md", secondary_style)),
        ];
        let expected = ListItem::new(Text::from(expected_lines));
        let actual = note.to_list_item(&theme, &icons);
        assert_eq!(
            actual, expected,
            "ListItem mismatch for is_open={is_open}"
        );
        // Return the color for the simpler assertions below.
        glyph_style.fg.expect("glyph style must have an fg color")
    }

    #[test]
    fn open_note_glyph_is_accent_colored() {
        let theme = Theme::default();
        let accent = theme.accent.to_ratatui();
        let actual_fg = glyph_fg_of_note(true);
        assert_eq!(
            actual_fg, accent,
            "is_open=true: glyph span fg should be theme.accent"
        );
    }

    #[test]
    fn closed_note_glyph_is_not_accent_colored() {
        let theme = Theme::default();
        let accent = theme.accent.to_ratatui();
        let actual_fg = glyph_fg_of_note(false);
        assert_ne!(
            actual_fg, accent,
            "is_open=false: glyph span fg should NOT be theme.accent"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::search_list::SearchRow;

    #[test]
    fn directory_match_text_is_some_name() {
        let dir = FileListEntry::Directory {
            path: VaultPath::note_path_from("projects"),
            name: "projects".to_string(),
        };
        assert_eq!(SearchRow::match_text(&dir), Some("projects"));
    }

    #[test]
    fn up_match_text_is_none() {
        let up = FileListEntry::Up {
            parent: VaultPath::root(),
        };
        assert_eq!(SearchRow::match_text(&up), None);
    }

    #[test]
    fn sort_field_setting_roundtrip() {
        use crate::settings::SortFieldSetting;
        assert_eq!(
            SortFieldSetting::from(SortField::Name),
            SortFieldSetting::Name
        );
        assert_eq!(
            SortFieldSetting::from(SortField::Title),
            SortFieldSetting::Title
        );
        assert_eq!(SortField::from(SortFieldSetting::Title), SortField::Title);
    }

    #[test]
    fn sort_order_setting_roundtrip() {
        use crate::settings::SortOrderSetting;
        assert_eq!(
            SortOrderSetting::from(SortOrder::Ascending),
            SortOrderSetting::Ascending
        );
        assert_eq!(
            SortOrderSetting::from(SortOrder::Descending),
            SortOrderSetting::Descending
        );
    }
}
