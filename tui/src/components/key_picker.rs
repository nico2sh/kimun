//! A text field with a filtered list of known property keys under it. Free
//! text is always allowed; a suggestion is a shortcut, never a constraint.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Paragraph};

use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

const MAX_ROWS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Consumed,
    Changed,
    Accepted(String),
    Submit,
    Cancel,
    NotConsumed,
}

pub struct KeyPicker {
    input: SingleLineInput,
    keys: Vec<String>,
    open: bool,
    highlighted: usize,
    /// First suggestion shown; follows `highlighted` so it stays on screen.
    offset: usize,
    /// Rows the last render could show (the real window for `offset`).
    window: usize,
    field_rect: Option<Rect>,
    /// (rect, suggestion) of each visible row from the last render.
    row_rects: Vec<(Rect, String)>,
}

impl KeyPicker {
    pub fn new(initial: &str) -> Self {
        Self {
            input: SingleLineInput::with_value(initial),
            keys: Vec::new(),
            open: false,
            highlighted: 0,
            offset: 0,
            window: MAX_ROWS,
            field_rect: None,
            row_rects: Vec::new(),
        }
    }

    pub fn set_keys(&mut self, keys: Vec<String>) {
        self.keys = keys;
        let count = self.suggestions().len();
        if count == 0 {
            self.open = false;
        }
        self.highlighted = self.highlighted.min(count.saturating_sub(1));
        self.follow_highlight();
    }

    /// Scrolls `offset` just enough to keep `highlighted` inside the window.
    fn follow_highlight(&mut self) {
        let window = self.window.max(1);
        if self.highlighted < self.offset {
            self.offset = self.highlighted;
        } else if self.highlighted >= self.offset + window {
            self.offset = self.highlighted + 1 - window;
        }
    }

    pub fn value(&self) -> &str {
        self.input.value()
    }

    /// Moves the caret to the end of the text (a click into the field).
    pub fn cursor_to_end(&mut self) {
        let text = self.input.value().to_string();
        self.input.set_value(text);
    }

    pub fn is_list_open(&self) -> bool {
        self.open
    }

    pub fn field_rect(&self) -> Option<Rect> {
        self.field_rect
    }

    pub(crate) fn suggestions(&self) -> Vec<&str> {
        let needle = self.input.value().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.keys
            .iter()
            .filter(|k| k.to_lowercase().contains(&needle))
            .map(String::as_str)
            .collect()
    }

    fn refresh_open(&mut self) {
        let value = self.input.value().to_lowercase();
        let s = self.suggestions();
        self.open = !s.is_empty() && !(s.len() == 1 && s[0].to_lowercase() == value);
        self.highlighted = 0;
        self.offset = 0;
    }

    fn accept(&mut self, key: String) -> PickerOutcome {
        self.input.set_value(key.clone());
        self.open = false;
        self.row_rects.clear();
        PickerOutcome::Accepted(key)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PickerOutcome {
        if self.open {
            let count = self.suggestions().len();
            match key.code {
                KeyCode::Up => {
                    self.highlighted = self.highlighted.saturating_sub(1);
                    self.follow_highlight();
                    return PickerOutcome::Consumed;
                }
                KeyCode::Down => {
                    self.highlighted = (self.highlighted + 1).min(count.saturating_sub(1));
                    self.follow_highlight();
                    return PickerOutcome::Consumed;
                }
                KeyCode::Enter | KeyCode::Right => {
                    if let Some(k) = self.suggestions().get(self.highlighted) {
                        let k = k.to_string();
                        return self.accept(k);
                    }
                }
                KeyCode::Esc => {
                    self.open = false;
                    return PickerOutcome::Consumed;
                }
                _ => {}
            }
        }
        match self.input.handle_key(key) {
            InputOutcome::Changed => {
                self.refresh_open();
                PickerOutcome::Changed
            }
            InputOutcome::Submit => PickerOutcome::Submit,
            InputOutcome::Cancel => PickerOutcome::Cancel,
            InputOutcome::Consumed => PickerOutcome::Consumed,
            InputOutcome::NotConsumed => PickerOutcome::NotConsumed,
        }
    }

    /// Field row is `field`; the suggestion list renders below it inside
    /// `area`, at most 5 rows. Render the picker last so the list sits on top.
    pub fn render(&mut self, f: &mut Frame, field: Rect, area: Rect, theme: &Theme, focused: bool) {
        self.field_rect = Some(field);
        let style = Style::default()
            .fg(theme.fg_bright.to_ratatui())
            .bg(theme.bg.to_ratatui());
        self.input.render(f, field, style, 0, focused);
        self.row_rects.clear();
        if !self.open || !focused {
            return;
        }
        let room = area.bottom().saturating_sub(field.y + 1) as usize;
        self.window = MAX_ROWS.min(room).max(1);
        self.follow_highlight();
        let rows: Vec<String> = self
            .suggestions()
            .into_iter()
            .skip(self.offset)
            .take(MAX_ROWS.min(room))
            .map(str::to_string)
            .collect();
        for (i, key) in rows.into_iter().enumerate() {
            let y = field.y + 1 + i as u16;
            let rect = Rect {
                x: field.x,
                y,
                width: field.width,
                height: 1,
            };
            let st = if self.offset + i == self.highlighted {
                Style::default()
                    .fg(theme.selection_fg.to_ratatui())
                    .bg(theme.selection_bg.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.fg.to_ratatui())
                    .bg(theme.bg_panel.to_ratatui())
            };
            f.render_widget(Clear, rect);
            f.render_widget(Paragraph::new(format!(" {key}")).style(st), rect);
            self.row_rects.push((rect, key));
        }
    }

    /// Click at (col,row): on the field → focus (`Consumed`); on a suggestion
    /// → `Accepted(key)`; elsewhere → `NotConsumed`.
    pub fn handle_click(&mut self, col: u16, row: u16) -> PickerOutcome {
        let pos = Position { x: col, y: row };
        if let Some((_, key)) = self.row_rects.iter().find(|(r, _)| r.contains(pos)) {
            let key = key.clone();
            return self.accept(key);
        }
        if self.field_rect.is_some_and(|r| r.contains(pos)) {
            return PickerOutcome::Consumed;
        }
        PickerOutcome::NotConsumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::{Terminal, backend::TestBackend};

    fn picker() -> KeyPicker {
        let mut p = KeyPicker::new("");
        p.set_keys(vec!["due".into(), "status".into(), "priority".into()]);
        p
    }
    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }
    fn type_str(p: &mut KeyPicker, s: &str) {
        for c in s.chars() {
            p.handle_key(&k(KeyCode::Char(c)));
        }
    }

    #[test]
    fn cursor_to_end_moves_the_caret_after_the_text() {
        let mut p = KeyPicker::new("status");
        p.handle_key(&k(KeyCode::Home));
        p.cursor_to_end();
        type_str(&mut p, "x");
        assert_eq!(p.value(), "statusx");
    }

    #[test]
    fn typing_filters_case_insensitively() {
        let mut p = picker();
        type_str(&mut p, "PRI");
        assert!(p.is_list_open());
        assert_eq!(p.suggestions(), vec!["priority"]);
    }

    #[test]
    fn enter_accepts_highlighted_then_submits() {
        let mut p = picker();
        type_str(&mut p, "t");
        assert_eq!(p.suggestions(), vec!["status", "priority"]);
        p.handle_key(&k(KeyCode::Down));
        assert_eq!(
            p.handle_key(&k(KeyCode::Enter)),
            PickerOutcome::Accepted("priority".into())
        );
        assert_eq!(p.value(), "priority");
        assert!(!p.is_list_open());
        assert_eq!(p.handle_key(&k(KeyCode::Enter)), PickerOutcome::Submit);
    }

    #[test]
    fn esc_closes_list_before_cancelling() {
        let mut p = picker();
        type_str(&mut p, "d");
        assert_eq!(p.handle_key(&k(KeyCode::Esc)), PickerOutcome::Consumed);
        assert!(!p.is_list_open());
        assert_eq!(p.handle_key(&k(KeyCode::Esc)), PickerOutcome::Cancel);
    }

    #[test]
    fn free_text_is_kept() {
        let mut p = picker();
        type_str(&mut p, "brand new");
        assert!(!p.is_list_open());
        assert_eq!(p.handle_key(&k(KeyCode::Enter)), PickerOutcome::Submit);
        assert_eq!(p.value(), "brand new");
    }

    fn draw(p: &mut KeyPicker, h: u16) {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(30, h)).unwrap();
        t.draw(|f| {
            let area = f.area();
            let field = Rect { height: 1, ..area };
            p.render(f, field, area, &theme, true);
        })
        .unwrap();
    }

    #[test]
    fn highlight_stays_in_rendered_window() {
        let mut p = KeyPicker::new("");
        p.set_keys((0..8).map(|i| format!("key{i}")).collect());
        type_str(&mut p, "key");
        for _ in 0..6 {
            p.handle_key(&k(KeyCode::Down));
        }
        draw(&mut p, 10);
        let rows: Vec<&str> = p.row_rects.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(rows.len(), 5);
        assert!(rows.contains(&"key6"), "rendered {rows:?}");
        assert_eq!(
            p.handle_key(&k(KeyCode::Enter)),
            PickerOutcome::Accepted("key6".into())
        );
    }

    #[test]
    fn highlight_follows_in_clipped_area() {
        let mut p = KeyPicker::new("");
        p.set_keys((0..8).map(|i| format!("key{i}")).collect());
        type_str(&mut p, "key");
        draw(&mut p, 3); // field + 2 rows
        for _ in 0..4 {
            p.handle_key(&k(KeyCode::Down));
        }
        draw(&mut p, 3);
        let rows: Vec<&str> = p.row_rects.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(rows, vec!["key3", "key4"]);
    }

    #[test]
    fn right_accepts_and_up_moves_back() {
        let mut p = picker();
        type_str(&mut p, "t");
        p.handle_key(&k(KeyCode::Down));
        p.handle_key(&k(KeyCode::Up));
        assert_eq!(
            p.handle_key(&k(KeyCode::Right)),
            PickerOutcome::Accepted("status".into())
        );
    }

    #[test]
    fn click_on_field_is_consumed_and_outside_is_not() {
        let mut p = picker();
        draw(&mut p, 8);
        assert_eq!(p.handle_click(3, 0), PickerOutcome::Consumed);
        assert_eq!(p.handle_click(3, 5), PickerOutcome::NotConsumed);
    }

    #[test]
    fn second_click_before_rerender_is_ignored() {
        let mut p = picker();
        type_str(&mut p, "t");
        draw(&mut p, 8);
        assert!(matches!(p.handle_click(3, 1), PickerOutcome::Accepted(_)));
        assert_eq!(p.handle_click(3, 1), PickerOutcome::NotConsumed);
    }

    #[test]
    fn set_keys_clamps_stale_state() {
        let mut p = picker();
        type_str(&mut p, "t");
        p.handle_key(&k(KeyCode::Down));
        p.set_keys(vec!["status".into()]);
        assert_eq!(
            p.handle_key(&k(KeyCode::Enter)),
            PickerOutcome::Accepted("status".into())
        );
        let mut p = picker();
        type_str(&mut p, "t");
        p.set_keys(vec![]);
        assert!(!p.is_list_open());
    }

    #[test]
    fn click_on_suggestion_accepts_it() {
        let mut p = picker();
        type_str(&mut p, "t");
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(30, 8)).unwrap();
        t.draw(|f| {
            let area = f.area();
            let field = Rect { height: 1, ..area };
            p.render(f, field, area, &theme, true);
        })
        .unwrap();
        // Suggestions start on the row below the field: row 1 = "status", row 2 = "priority".
        assert_eq!(
            p.handle_click(3, 2),
            PickerOutcome::Accepted("priority".into())
        );
        assert_eq!(p.value(), "priority");
    }
}
