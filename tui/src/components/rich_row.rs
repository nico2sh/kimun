//! The shared **rich list row** format every drawer list uses (spec §4):
//!
//! ```text
//! ▤ Auth Flow Meeting              04-08
//!   attendees: maria, david              ← optional secondary line
//!   2026-04-08.md                        ← dim italic filename line
//! ```
//!
//! A `RichRow` is a declarative description; `into_list_item` renders it with
//! the theme's roles. Selection background is applied by the `SearchList`
//! engine's highlight style — rows only choose foregrounds.
//!
//! `meta` currently renders inline after the title; right-aligning it needs
//! the row width, which the `SearchRow` seam does not carry yet — that pass
//! lands with the telescope alignment work (Phase 08).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::ListItem;

use crate::settings::themes::Theme;

#[derive(Default)]
pub struct RichRow {
    glyph: String,
    glyph_style: Option<Style>,
    title: String,
    title_style: Option<Style>,
    /// Dim metadata after the title (count, date, …).
    meta: Option<String>,
    /// Optional secondary line with its own style.
    secondary: Option<(String, Option<Style>)>,
    /// Dim italic filename line.
    filename: Option<String>,
}

impl RichRow {
    pub fn new(glyph: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            glyph: glyph.into(),
            title: title.into(),
            ..Self::default()
        }
    }

    pub fn glyph_style(mut self, style: Style) -> Self {
        self.glyph_style = Some(style);
        self
    }

    pub fn title_style(mut self, style: Style) -> Self {
        self.title_style = Some(style);
        self
    }

    pub fn meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn secondary(mut self, text: impl Into<String>, style: Option<Style>) -> Self {
        self.secondary = Some((text.into(), style));
        self
    }

    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Terminal rows this row occupies when rendered.
    pub fn height(&self) -> u16 {
        1 + u16::from(self.secondary.is_some()) + u16::from(self.filename.is_some())
    }

    pub fn into_list_item(self, theme: &Theme) -> ListItem<'static> {
        let fg = Style::default().fg(theme.fg.to_ratatui());
        let gray = Style::default().fg(theme.gray.to_ratatui());
        let secondary_default = Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::ITALIC);

        let mut main = vec![
            Span::styled(format!("{} ", self.glyph), self.glyph_style.unwrap_or(fg)),
            Span::styled(self.title, self.title_style.unwrap_or(fg)),
        ];
        if let Some(meta) = self.meta {
            main.push(Span::styled(format!("  {meta}"), gray));
        }

        let mut lines = vec![Line::from(main)];
        if let Some((text, style)) = self.secondary {
            lines.push(Line::from(Span::styled(
                format!("  {text}"),
                style.unwrap_or(secondary_default),
            )));
        }
        if let Some(filename) = self.filename {
            lines.push(Line::from(Span::styled(
                format!("  {filename}"),
                secondary_default,
            )));
        }
        ListItem::new(Text::from(lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_counts_optional_lines() {
        let theme = Theme::default();
        let row = RichRow::new("X", "title");
        assert_eq!(row.height(), 1);
        let row = RichRow::new("X", "title").filename("a.md");
        assert_eq!(row.height(), 2);
        let row = RichRow::new("X", "title")
            .secondary("sub", None)
            .filename("a.md");
        assert_eq!(row.height(), 3);
        // Renders without panicking.
        let _ = RichRow::new("X", "t").meta("42").into_list_item(&theme);
    }
}
