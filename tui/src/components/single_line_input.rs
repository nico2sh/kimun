//! Reusable single-line text input.
//!
//! Used by the editor find bar, dialogs (rename, move, quick-note), the sidebar
//! / note-browser search boxes, and the settings workspace name field. The
//! widget owns its value and char cursor; callers add titles, hints, validation
//! visuals, and submit/cancel semantics on top.
//!
//! `handle_key` returns [`InputOutcome`] so callers can branch on Submit /
//! Cancel / textual mutation without re-matching the raw key.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// Outcome of [`SingleLineInput::handle_key`] — lets callers branch without
/// re-parsing the key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    /// Key was consumed but the value did not change (cursor move, no-op).
    Consumed,
    /// Value (and possibly cursor) changed.
    Changed,
    /// User pressed Enter.
    Submit,
    /// User pressed Esc.
    Cancel,
    /// Key was not recognised by the widget.
    NotConsumed,
}

#[derive(Default)]
pub struct SingleLineInput {
    value: String,
    /// Byte offset into `value`.
    cursor: usize,
}

impl SingleLineInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_value(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Replace the value; cursor jumps to end.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.value.len();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Codepoint count to the left of the cursor. Test-only: callers must use
    /// [`cursor_display_col`](Self::cursor_display_col) for caret placement,
    /// since codepoint count differs from display width for CJK / emoji.
    #[cfg(test)]
    pub(crate) fn cursor_char_offset(&self) -> usize {
        self.value[..self.cursor].chars().count()
    }

    /// Display column to the left of the cursor — accounts for wide (CJK,
    /// emoji) characters via `unicode-width`. Use this for caret placement.
    pub fn cursor_display_col(&self) -> usize {
        self.value[..self.cursor].width()
    }

    /// Total display width of the value — accounts for wide characters.
    pub fn display_width(&self) -> usize {
        self.value.width()
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> InputOutcome {
        match (key.modifiers, key.code) {
            (_, KeyCode::Enter) => InputOutcome::Submit,
            (_, KeyCode::Esc) => InputOutcome::Cancel,
            (_, KeyCode::Backspace) => {
                if self.cursor == 0 {
                    return InputOutcome::Consumed;
                }
                let prev = prev_char_boundary(&self.value, self.cursor);
                self.value.drain(prev..self.cursor);
                self.cursor = prev;
                InputOutcome::Changed
            }
            (_, KeyCode::Delete) => {
                if self.cursor >= self.value.len() {
                    return InputOutcome::Consumed;
                }
                let next = next_char_boundary(&self.value, self.cursor);
                self.value.drain(self.cursor..next);
                InputOutcome::Changed
            }
            (_, KeyCode::Left) => {
                if self.cursor == 0 {
                    return InputOutcome::Consumed;
                }
                self.cursor = prev_char_boundary(&self.value, self.cursor);
                InputOutcome::Consumed
            }
            (_, KeyCode::Right) => {
                if self.cursor >= self.value.len() {
                    return InputOutcome::Consumed;
                }
                self.cursor = next_char_boundary(&self.value, self.cursor);
                InputOutcome::Consumed
            }
            (_, KeyCode::Home) => {
                self.cursor = 0;
                InputOutcome::Consumed
            }
            (_, KeyCode::End) => {
                self.cursor = self.value.len();
                InputOutcome::Consumed
            }
            // Accept only plain or Shift-modified chars; Ctrl/Alt combos are
            // not text input and must bubble up to the caller for shortcuts.
            (m, KeyCode::Char(c)) if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.value.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                InputOutcome::Changed
            }
            _ => InputOutcome::NotConsumed,
        }
    }

    /// Render the value text at `rect` using `style`. Caller is responsible for
    /// any surrounding chrome (borders, prompt prefix, validation glyphs).
    /// Place the terminal cursor when `focused`. `value_offset_x` is the
    /// display-column offset within `rect` where the value text starts (e.g.
    /// when the caller renders a "Find: " prefix separately, pass its
    /// display width via `UnicodeWidthStr::width`).
    pub fn render(
        &self,
        f: &mut Frame,
        rect: Rect,
        style: Style,
        value_offset_x: u16,
        focused: bool,
    ) {
        let inner = Rect {
            x: rect.x.saturating_add(value_offset_x),
            width: rect.width.saturating_sub(value_offset_x),
            ..rect
        };
        f.render_widget(Paragraph::new(self.value.as_str()).style(style), inner);
        if focused {
            let caret_x = inner
                .x
                .saturating_add(self.cursor_display_col() as u16)
                .min(inner.x + inner.width.saturating_sub(1));
            f.set_cursor_position(Position {
                x: caret_x,
                y: inner.y,
            });
        }
    }
}

fn prev_char_boundary(s: &str, from: usize) -> usize {
    s[..from]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn next_char_boundary(s: &str, from: usize) -> usize {
    s[from..]
        .char_indices()
        .nth(1)
        .map(|(i, _)| from + i)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn new_is_empty_cursor_zero() {
        let i = SingleLineInput::new();
        assert!(i.is_empty());
        assert_eq!(i.cursor_char_offset(), 0);
    }

    #[test]
    fn with_value_places_cursor_at_end() {
        let i = SingleLineInput::with_value("hello");
        assert_eq!(i.value(), "hello");
        assert_eq!(i.cursor_char_offset(), 5);
    }

    #[test]
    fn typing_chars_appends_and_advances_cursor() {
        let mut i = SingleLineInput::new();
        assert_eq!(i.handle_key(&k(KeyCode::Char('a'))), InputOutcome::Changed);
        assert_eq!(i.handle_key(&k(KeyCode::Char('b'))), InputOutcome::Changed);
        assert_eq!(i.value(), "ab");
        assert_eq!(i.cursor_char_offset(), 2);
    }

    #[test]
    fn left_then_insert_inserts_mid_string() {
        let mut i = SingleLineInput::with_value("ac");
        i.handle_key(&k(KeyCode::Left));
        assert_eq!(i.cursor_char_offset(), 1);
        i.handle_key(&k(KeyCode::Char('b')));
        assert_eq!(i.value(), "abc");
        assert_eq!(i.cursor_char_offset(), 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut i = SingleLineInput::with_value("abc");
        i.handle_key(&k(KeyCode::Home));
        assert_eq!(i.handle_key(&k(KeyCode::Backspace)), InputOutcome::Consumed);
        assert_eq!(i.value(), "abc");
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut i = SingleLineInput::with_value("abc");
        assert_eq!(i.handle_key(&k(KeyCode::Delete)), InputOutcome::Consumed);
        assert_eq!(i.value(), "abc");
    }

    #[test]
    fn home_end_jump_cursor() {
        let mut i = SingleLineInput::with_value("abc");
        i.handle_key(&k(KeyCode::Home));
        assert_eq!(i.cursor_char_offset(), 0);
        i.handle_key(&k(KeyCode::End));
        assert_eq!(i.cursor_char_offset(), 3);
    }

    #[test]
    fn unicode_chars_count_by_codepoint_not_bytes() {
        let mut i = SingleLineInput::new();
        i.handle_key(&k(KeyCode::Char('あ')));
        i.handle_key(&k(KeyCode::Char('い')));
        assert_eq!(i.value(), "あい");
        assert_eq!(i.cursor_char_offset(), 2);
        i.handle_key(&k(KeyCode::Left));
        assert_eq!(i.cursor_char_offset(), 1);
        i.handle_key(&k(KeyCode::Backspace));
        assert_eq!(i.value(), "い");
    }

    #[test]
    fn enter_returns_submit_esc_returns_cancel() {
        let mut i = SingleLineInput::with_value("x");
        assert_eq!(i.handle_key(&k(KeyCode::Enter)), InputOutcome::Submit);
        assert_eq!(i.handle_key(&k(KeyCode::Esc)), InputOutcome::Cancel);
    }

    #[test]
    fn ctrl_char_is_not_consumed_as_text() {
        let mut i = SingleLineInput::new();
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(i.handle_key(&key), InputOutcome::NotConsumed);
        assert!(i.is_empty());
    }

    #[test]
    fn alt_char_is_not_consumed_as_text() {
        let mut i = SingleLineInput::new();
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(i.handle_key(&key), InputOutcome::NotConsumed);
        assert!(i.is_empty());
    }

    #[test]
    fn cjk_chars_count_two_display_cols_per_char() {
        let mut i = SingleLineInput::new();
        i.handle_key(&k(KeyCode::Char('あ')));
        i.handle_key(&k(KeyCode::Char('い')));
        // 2 codepoints, but each is 2 cells wide.
        assert_eq!(i.cursor_char_offset(), 2);
        assert_eq!(i.cursor_display_col(), 4);
        assert_eq!(i.display_width(), 4);
    }

    #[test]
    fn mixed_ascii_and_cjk_caret_column() {
        let mut i = SingleLineInput::with_value("ab猫");
        // Caret at end of "ab猫" → 1+1+2 display cols.
        assert_eq!(i.cursor_display_col(), 4);
        i.handle_key(&k(KeyCode::Left));
        // Caret moved before 猫 → 2 cells.
        assert_eq!(i.cursor_display_col(), 2);
    }

    #[test]
    fn shift_char_inserts() {
        let mut i = SingleLineInput::new();
        let key = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert_eq!(i.handle_key(&key), InputOutcome::Changed);
        assert_eq!(i.value(), "A");
    }

    #[test]
    fn set_value_resets_cursor_to_end() {
        let mut i = SingleLineInput::with_value("abc");
        i.handle_key(&k(KeyCode::Home));
        i.set_value("xyz!");
        assert_eq!(i.value(), "xyz!");
        assert_eq!(i.cursor_char_offset(), 4);
    }

    #[test]
    fn clear_resets_both() {
        let mut i = SingleLineInput::with_value("abc");
        i.clear();
        assert!(i.is_empty());
        assert_eq!(i.cursor_char_offset(), 0);
    }
}
