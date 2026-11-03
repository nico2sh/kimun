//! The **line field**: a one-line text value and its cursor, toolkit-free.
//!
//! The editing half of a single-line input (the find bar's fields, the TUI's
//! dialogs and search boxes). It owns the value and a byte cursor and answers
//! keys in the terminal-neutral [`KeyInput`]; drawing it — scrolling, caret
//! placement — is the front end's job, done over [`LineField::value`] and
//! [`LineField::cursor_byte`].
//!
//! [`LineField::handle_key`] returns a [`FieldOutcome`] so callers can branch
//! on submit / cancel / textual change without re-matching the key.

use crate::logic::input::{Key, KeyInput};
use unicode_width::UnicodeWidthStr;

/// What [`LineField::handle_key`] did with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldOutcome {
    /// Key was consumed but the value did not change (cursor move, no-op).
    Consumed,
    /// Value (and possibly cursor) changed.
    Changed,
    /// User pressed Enter.
    Submit,
    /// User pressed Esc.
    Cancel,
    /// Key was not recognised by the field.
    NotConsumed,
}

/// A one-line text value with a char-boundary byte cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineField {
    value: String,
    /// Byte offset into `value`, always on a char boundary.
    cursor: usize,
}

impl LineField {
    pub fn new() -> Self {
        Self::default()
    }

    /// A field holding `value`, cursor at the end.
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

    /// Current cursor position as a byte offset.
    pub fn cursor_byte(&self) -> usize {
        self.cursor
    }

    /// Codepoint count to the left of the cursor. Not a display column: use
    /// [`Self::cursor_display_col`] for caret placement in a cell grid.
    pub fn cursor_char_offset(&self) -> usize {
        self.value[..self.cursor].chars().count()
    }

    /// Display columns to the left of the cursor — wide (CJK, emoji)
    /// characters count two.
    pub fn cursor_display_col(&self) -> usize {
        self.value[..self.cursor].width()
    }

    /// Total display width of the value — wide characters count two.
    pub fn display_width(&self) -> usize {
        self.value.width()
    }

    /// Overwrite a byte `range` of the value with `new_text`, then place the
    /// cursor at byte offset `new_cursor_byte` in the updated value. All three
    /// positions must be on char boundaries — checked via debug_assert.
    pub fn replace_range_bytes(
        &mut self,
        range: std::ops::Range<usize>,
        new_text: &str,
        new_cursor_byte: usize,
    ) {
        debug_assert!(self.value.is_char_boundary(range.start));
        debug_assert!(self.value.is_char_boundary(range.end));
        self.value.replace_range(range, new_text);
        let clamped = new_cursor_byte.min(self.value.len());
        debug_assert!(
            self.value.is_char_boundary(clamped),
            "new_cursor_byte must land on a char boundary"
        );
        self.cursor = clamped;
    }

    /// Insert `text` at the cursor and move the cursor past it.
    pub fn insert_str(&mut self, text: &str) {
        let at = self.cursor;
        self.replace_range_bytes(at..at, text, at + text.len());
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

    /// Take the current value, leaving the field empty.
    pub fn take_text(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

    /// Apply one key. Enter submits and Esc cancels whatever the modifiers;
    /// editing and motion keys ignore modifiers; a char is typed unless Ctrl
    /// or Alt is held — those chords are shortcuts and bubble up as
    /// [`FieldOutcome::NotConsumed`].
    pub fn handle_key(&mut self, key: KeyInput) -> FieldOutcome {
        match key.key {
            Key::Enter => FieldOutcome::Submit,
            Key::Esc => FieldOutcome::Cancel,
            Key::Backspace => {
                if self.cursor == 0 {
                    return FieldOutcome::Consumed;
                }
                let prev = prev_char_boundary(&self.value, self.cursor);
                self.value.drain(prev..self.cursor);
                self.cursor = prev;
                FieldOutcome::Changed
            }
            Key::Delete => {
                if self.cursor >= self.value.len() {
                    return FieldOutcome::Consumed;
                }
                let next = next_char_boundary(&self.value, self.cursor);
                self.value.drain(self.cursor..next);
                FieldOutcome::Changed
            }
            Key::Left => {
                if self.cursor > 0 {
                    self.cursor = prev_char_boundary(&self.value, self.cursor);
                }
                FieldOutcome::Consumed
            }
            Key::Right => {
                if self.cursor < self.value.len() {
                    self.cursor = next_char_boundary(&self.value, self.cursor);
                }
                FieldOutcome::Consumed
            }
            Key::Home => {
                self.cursor = 0;
                FieldOutcome::Consumed
            }
            Key::End => {
                self.cursor = self.value.len();
                FieldOutcome::Consumed
            }
            Key::Char(c) if !key.mods.ctrl && !key.mods.alt => {
                self.value.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                FieldOutcome::Changed
            }
            _ => FieldOutcome::NotConsumed,
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
    use crate::logic::input::Mods;

    fn k(key: Key) -> KeyInput {
        KeyInput::plain(key)
    }

    #[test]
    fn new_is_empty_cursor_zero() {
        let f = LineField::new();
        assert!(f.is_empty());
        assert_eq!(f.cursor_char_offset(), 0);
    }

    #[test]
    fn typing_chars_appends_and_advances_cursor() {
        let mut f = LineField::new();
        assert_eq!(f.handle_key(k(Key::Char('a'))), FieldOutcome::Changed);
        assert_eq!(f.handle_key(k(Key::Char('b'))), FieldOutcome::Changed);
        assert_eq!(f.value(), "ab");
        assert_eq!(f.cursor_char_offset(), 2);
    }

    #[test]
    fn left_then_insert_inserts_mid_string() {
        let mut f = LineField::with_value("ac");
        f.handle_key(k(Key::Left));
        f.handle_key(k(Key::Char('b')));
        assert_eq!(f.value(), "abc");
        assert_eq!(f.cursor_char_offset(), 2);
    }

    #[test]
    fn backspace_and_delete_at_the_edges_are_noops() {
        let mut f = LineField::with_value("abc");
        assert_eq!(f.handle_key(k(Key::Delete)), FieldOutcome::Consumed);
        f.handle_key(k(Key::Home));
        assert_eq!(f.handle_key(k(Key::Backspace)), FieldOutcome::Consumed);
        assert_eq!(f.value(), "abc");
    }

    #[test]
    fn delete_removes_the_char_after_the_cursor() {
        let mut f = LineField::with_value("abc");
        f.handle_key(k(Key::Home));
        assert_eq!(f.handle_key(k(Key::Delete)), FieldOutcome::Changed);
        assert_eq!(f.value(), "bc");
        assert_eq!(f.cursor_byte(), 0);
    }

    #[test]
    fn unicode_is_edited_by_codepoint_and_measured_by_width() {
        let mut f = LineField::new();
        f.handle_key(k(Key::Char('あ')));
        f.handle_key(k(Key::Char('い')));
        assert_eq!(f.cursor_char_offset(), 2);
        assert_eq!(f.cursor_display_col(), 4);
        assert_eq!(f.display_width(), 4);
        f.handle_key(k(Key::Left));
        f.handle_key(k(Key::Backspace));
        assert_eq!(f.value(), "い");
    }

    #[test]
    fn enter_submits_and_esc_cancels_whatever_the_modifiers() {
        let mut f = LineField::with_value("x");
        assert_eq!(f.handle_key(k(Key::Enter)), FieldOutcome::Submit);
        assert_eq!(
            f.handle_key(KeyInput::new(Key::Enter, Mods::SHIFT)),
            FieldOutcome::Submit
        );
        assert_eq!(f.handle_key(k(Key::Esc)), FieldOutcome::Cancel);
    }

    #[test]
    fn ctrl_and_alt_chars_bubble_shift_and_super_type() {
        let mut f = LineField::new();
        assert_eq!(f.handle_key(KeyInput::ctrl('a')), FieldOutcome::NotConsumed);
        assert_eq!(
            f.handle_key(KeyInput::new(Key::Char('a'), Mods::ALT)),
            FieldOutcome::NotConsumed
        );
        assert!(f.is_empty());
        assert_eq!(
            f.handle_key(KeyInput::new(Key::Char('A'), Mods::SHIFT)),
            FieldOutcome::Changed
        );
        assert_eq!(f.value(), "A");
    }

    #[test]
    fn tab_and_vertical_keys_are_not_the_fields() {
        let mut f = LineField::with_value("x");
        for key in [Key::Tab, Key::BackTab, Key::Up, Key::Down] {
            assert_eq!(f.handle_key(k(key)), FieldOutcome::NotConsumed);
        }
    }

    #[test]
    fn insert_str_lands_at_the_cursor() {
        let mut f = LineField::with_value("ad");
        f.handle_key(k(Key::Left));
        f.insert_str("bc");
        assert_eq!(f.value(), "abcd");
        assert_eq!(f.cursor_byte(), 3);
    }

    #[test]
    fn take_text_and_set_value_and_clear() {
        let mut f = LineField::with_value("abc");
        f.handle_key(k(Key::Home));
        f.set_value("xyz!");
        assert_eq!(f.cursor_char_offset(), 4);
        assert_eq!(f.take_text(), "xyz!");
        assert!(f.is_empty());
        f.set_value("q");
        f.clear();
        assert_eq!(f.cursor_byte(), 0);
    }
}
