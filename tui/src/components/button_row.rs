//! A row of clickable `[ Label ]` buttons. Rects are recorded at render time
//! and hit-tested against later mouse events (same pattern as
//! `search_list`): no render, no hits.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::settings::themes::Theme;

pub struct ButtonRow {
    labels: Vec<String>,
    enabled: Vec<bool>,
    focused: Option<usize>,
    rects: Vec<Rect>,
}

impl ButtonRow {
    pub fn new(labels: &[&str]) -> Self {
        Self {
            labels: labels.iter().map(|l| l.to_string()).collect(),
            enabled: vec![true; labels.len()],
            focused: None,
            rects: Vec::new(),
        }
    }

    pub fn set_enabled(&mut self, idx: usize, enabled: bool) {
        if let Some(e) = self.enabled.get_mut(idx) {
            *e = enabled;
        }
        if !enabled && self.focused == Some(idx) {
            self.focused = None;
        }
    }

    pub fn is_enabled(&self, idx: usize) -> bool {
        self.enabled.get(idx).copied().unwrap_or(false)
    }

    pub fn set_focused(&mut self, idx: Option<usize>) {
        self.focused = idx.filter(|&i| self.is_enabled(i));
    }

    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    /// Moves focus to the next enabled button; returns false at the end.
    pub fn focus_next(&mut self) -> bool {
        let start = self.focused.map_or(0, |i| i + 1);
        match (start..self.labels.len()).find(|&i| self.is_enabled(i)) {
            Some(i) => {
                self.focused = Some(i);
                true
            }
            None => false,
        }
    }

    /// Moves focus to the previous enabled button; returns false at the start.
    pub fn focus_prev(&mut self) -> bool {
        let end = self.focused.unwrap_or(self.labels.len());
        match (0..end).rev().find(|&i| self.is_enabled(i)) {
            Some(i) => {
                self.focused = Some(i);
                true
            }
            None => false,
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme) {
        self.rects.clear();
        let normal = Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_panel.to_ratatui());
        let dim = Style::default()
            .fg(theme.gray.to_ratatui())
            .bg(theme.bg_panel.to_ratatui())
            .add_modifier(Modifier::DIM);
        let focus = Style::default()
            .fg(theme.selection_fg.to_ratatui())
            .bg(theme.selection_bg.to_ratatui())
            .add_modifier(Modifier::BOLD);
        let mut spans = vec![Span::styled(" ", normal)];
        let mut x = rect.x + 1;
        for (i, label) in self.labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ", normal));
                x += 2;
            }
            let text = format!("[ {label} ]");
            let w = text.width() as u16;
            let style = if !self.enabled[i] {
                dim
            } else if self.focused == Some(i) {
                focus
            } else {
                normal.patch(theme.action())
            };
            // Clip to the row: a button past the right edge is not clickable.
            let visible = rect.right().saturating_sub(x).min(w);
            let r = Rect {
                x,
                y: rect.y,
                width: visible,
                height: 1,
            };
            self.rects.push(r);
            if self.enabled[i] {
                crate::components::clickable::register(r);
            }
            spans.push(Span::styled(text, style));
            x += w;
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(normal), rect);
    }

    /// Index of the enabled button under a left press, from the last
    /// render. Other buttons never press a button — a right-click on
    /// `Discard` must not discard.
    pub fn hit(&self, m: &ratatui::crossterm::event::MouseEvent) -> Option<usize> {
        if !crate::components::clickable::is_left_press(m) {
            return None;
        }
        self.hit_at(m.column, m.row)
    }

    /// Index of the enabled button at (col,row) from the last render — for
    /// callers that have already decided the event is a left press.
    pub fn hit_at(&self, col: u16, row: u16) -> Option<usize> {
        let pos = Position { x: col, y: row };
        self.rects
            .iter()
            .position(|r| r.contains(pos))
            .filter(|&i| self.is_enabled(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn drawn(row: &mut ButtonRow) {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(40, 1)).unwrap();
        t.draw(|f| row.render(f, f.area(), &theme)).unwrap();
    }

    #[test]
    fn hit_finds_each_button_after_render() {
        let mut row = ButtonRow::new(&["Save", "Cancel"]);
        drawn(&mut row);
        // Layout: " [ Save ]  [ Cancel ]" → Save at cols 1..=8, Cancel at 11..=20.
        assert_eq!(row.hit_at(1, 0), Some(0));
        assert_eq!(row.hit_at(8, 0), Some(0));
        assert_eq!(row.hit_at(11, 0), Some(1));
        assert_eq!(row.hit_at(20, 0), Some(1));
    }

    #[test]
    fn gap_between_buttons_hits_nothing() {
        let mut row = ButtonRow::new(&["Save", "Cancel"]);
        drawn(&mut row);
        assert_eq!(row.hit_at(9, 0), None);
        assert_eq!(row.hit_at(0, 0), None);
        assert_eq!(row.hit_at(30, 0), None);
    }

    #[test]
    fn disabled_buttons_are_never_hit_or_focused() {
        let mut row = ButtonRow::new(&["Add", "Edit", "Delete"]);
        row.set_enabled(1, false);
        drawn(&mut row);
        assert_eq!(row.hit_at(10, 0), None, "Edit is disabled");
        row.set_focused(Some(0));
        assert!(row.focus_next());
        assert_eq!(row.focused(), Some(2), "focus skips the disabled button");
        assert!(!row.focus_next());
    }

    #[test]
    fn focus_prev_skips_disabled_and_stops_at_start() {
        let mut row = ButtonRow::new(&["Add", "Edit", "Delete"]);
        row.set_enabled(1, false);
        row.set_focused(Some(2));
        assert!(row.focus_prev());
        assert_eq!(row.focused(), Some(0));
        assert!(!row.focus_prev());
    }

    #[test]
    fn disabling_focused_button_clears_focus() {
        let mut row = ButtonRow::new(&["Add", "Edit"]);
        row.set_focused(Some(1));
        row.set_enabled(1, false);
        assert_eq!(row.focused(), None);
        row.set_focused(Some(1));
        assert_eq!(row.focused(), None);
    }

    #[test]
    fn no_hit_before_first_render() {
        let row = ButtonRow::new(&["Save"]);
        assert_eq!(row.hit_at(1, 0), None);
    }
}
