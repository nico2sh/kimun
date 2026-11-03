//! The TUI half of a file-listing row: how a [`FileListEntry`] (the logic
//! row, see [`crate::logic::files`]) draws in a terminal list.

use ratatui::style::{Modifier, Style};
use ratatui::widgets::ListItem;

use crate::components::rich_row::RichRow;
pub use crate::logic::files::{FileListEntry, PropertyValues, entry_order};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

impl FileListEntry {
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
}

#[cfg(test)]
mod open_marker_tests {
    use super::*;
    use ratatui::style::Style;
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::ListItem;

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
        assert_eq!(actual, expected, "ListItem mismatch for is_open={is_open}");
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
