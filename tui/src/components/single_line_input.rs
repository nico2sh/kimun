//! Reusable single-line text input: the logic [`LineField`] plus its terminal
//! drawing.
//!
//! Used by the dialogs (rename, move, quick-note), the sidebar / note-browser
//! search boxes, and the settings workspace name field. The editing — value,
//! cursor, keys — is the toolkit-free [`LineField`]; this wrapper adds the
//! crossterm key translation and the ratatui rendering ([`FieldView`]: the
//! horizontal scroll and caret placement). Callers add titles, hints,
//! validation visuals, and submit/cancel semantics on top. The editor's find
//! bar paints its logic-owned fields through [`FieldView`] directly.
//!
//! `handle_key` returns [`InputOutcome`] so callers can branch on Submit /
//! Cancel / textual mutation without re-matching the raw key.

use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use crate::logic::editor::line_field::{FieldOutcome, LineField};

/// Outcome of [`SingleLineInput::handle_key`] — the logic field's outcome.
pub type InputOutcome = FieldOutcome;

/// The terminal drawing state of one line field: the horizontal scroll that
/// keeps the caret visible, and where the caret was last placed.
#[derive(Debug, Default)]
pub struct FieldView {
    /// Caret screen position (col, row), cached after the most recent
    /// `render` call. Used by overlays anchored on the caret (e.g. the
    /// hashtag autocomplete popup). `None` until the first render.
    last_caret_pos: Option<(u16, u16)>,
    /// Horizontal scroll offset in display columns, updated on render so the
    /// caret stays inside the visible window when the value overflows the
    /// rect. Kept across renders so the cursor can move within the window
    /// without the text shifting under it.
    scroll_x: u16,
}

impl FieldView {
    /// Caret screen position from the last `render` call, or `None` if the
    /// field has not been rendered yet (or was rendered unfocused).
    pub fn last_caret_pos(&self) -> Option<(u16, u16)> {
        self.last_caret_pos
    }

    /// Render `field`'s value at `rect` using `style`. Caller is responsible
    /// for any surrounding chrome. Place the terminal cursor when `focused`.
    /// `value_offset_x` is the display-column offset within `rect` where the
    /// value text starts.
    pub fn render(
        &mut self,
        f: &mut Frame,
        field: &LineField,
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
        let scroll_x = self.scroll_into_view(field, inner.width);
        f.render_widget(
            Paragraph::new(field.value())
                .style(style)
                .scroll((0, scroll_x)),
            inner,
        );
        self.place_caret(f, field, inner, focused);
    }

    /// Like [`Self::render`], but with a pre-styled line (e.g. the query
    /// highlighter's output). The line must render the same text as the
    /// field's value, so caret math stays valid.
    #[allow(clippy::too_many_arguments)]
    pub fn render_line(
        &mut self,
        f: &mut Frame,
        field: &LineField,
        rect: Rect,
        line: ratatui::text::Line<'static>,
        base_style: Style,
        value_offset_x: u16,
        focused: bool,
    ) {
        let inner = Rect {
            x: rect.x.saturating_add(value_offset_x),
            width: rect.width.saturating_sub(value_offset_x),
            ..rect
        };
        let scroll_x = self.scroll_into_view(field, inner.width);
        f.render_widget(
            Paragraph::new(line).style(base_style).scroll((0, scroll_x)),
            inner,
        );
        self.place_caret(f, field, inner, focused);
    }

    /// Adjust the horizontal scroll so the caret falls inside a window of
    /// `width` display columns, and return the resulting offset. Scrolls only
    /// when the caret crosses a window edge, so cursor movement inside the
    /// window leaves the text in place.
    fn scroll_into_view(&mut self, field: &LineField, width: u16) -> u16 {
        if width == 0 {
            self.scroll_x = 0;
            return 0;
        }
        // The caret can sit one column past the last char, so the maximum
        // useful offset keeps that extra cell — not just the last char —
        // inside the window. Also clamps stale offsets after the value shrank.
        let total = u16::try_from(field.display_width()).unwrap_or(u16::MAX);
        let max_scroll = total.saturating_add(1).saturating_sub(width);
        self.scroll_x = self.scroll_x.min(max_scroll);
        let cursor_col = u16::try_from(field.cursor_display_col()).unwrap_or(u16::MAX);
        if cursor_col < self.scroll_x {
            self.scroll_x = cursor_col;
        } else if cursor_col >= self.scroll_x.saturating_add(width) {
            self.scroll_x = cursor_col - (width - 1);
        }
        self.scroll_x
    }

    /// Shared caret placement for both render paths.
    fn place_caret(&mut self, f: &mut Frame, field: &LineField, inner: Rect, focused: bool) {
        self.last_caret_pos = None;
        if focused {
            let caret_x = inner
                .x
                .saturating_add((field.cursor_display_col() as u16).saturating_sub(self.scroll_x))
                .min(inner.x + inner.width.saturating_sub(1));
            f.set_cursor_position(Position {
                x: caret_x,
                y: inner.y,
            });
            self.last_caret_pos = Some((caret_x, inner.y));
        }
    }
}

#[derive(Default)]
pub struct SingleLineInput {
    field: LineField,
    view: FieldView,
}

impl SingleLineInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_value(value: impl Into<String>) -> Self {
        Self {
            field: LineField::with_value(value),
            view: FieldView::default(),
        }
    }

    pub fn value(&self) -> &str {
        self.field.value()
    }

    pub fn is_empty(&self) -> bool {
        self.field.is_empty()
    }

    /// Current cursor position as a byte offset.
    pub fn cursor_byte(&self) -> usize {
        self.field.cursor_byte()
    }

    /// Caret screen position from the last `render` call, or `None` if
    /// the widget has not been rendered yet (or was rendered unfocused).
    pub fn last_caret_pos(&self) -> Option<(u16, u16)> {
        self.view.last_caret_pos()
    }

    /// Overwrite a byte `range` of the value with `new_text`, then place
    /// the cursor at byte offset `new_cursor_byte` in the updated value.
    /// Used by the hashtag autocomplete to apply an `AcceptAction`
    /// ([`LineField::replace_range_bytes`]).
    pub fn replace_range_bytes(
        &mut self,
        range: std::ops::Range<usize>,
        new_text: &str,
        new_cursor_byte: usize,
    ) {
        self.field
            .replace_range_bytes(range, new_text, new_cursor_byte);
    }

    /// Replace the value; cursor jumps to end.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.field.set_value(value);
    }

    pub fn clear(&mut self) {
        self.field.clear();
    }

    /// Take the current value, leaving the input empty — a submit path that
    /// consumes the text without a separate `clear()` call.
    pub fn take_text(&mut self) -> String {
        self.field.take_text()
    }

    /// Codepoint count to the left of the cursor. Test-only: callers must use
    /// [`cursor_display_col`](Self::cursor_display_col) for caret placement,
    /// since codepoint count differs from display width for CJK / emoji.
    #[cfg(test)]
    pub(crate) fn cursor_char_offset(&self) -> usize {
        self.field.cursor_char_offset()
    }

    /// Display column to the left of the cursor — accounts for wide (CJK,
    /// emoji) characters via `unicode-width`. Use this for caret placement.
    pub fn cursor_display_col(&self) -> usize {
        self.field.cursor_display_col()
    }

    /// Total display width of the value — accounts for wide characters.
    pub fn display_width(&self) -> usize {
        self.field.display_width()
    }

    /// Translate the crossterm key and apply it ([`LineField::handle_key`]).
    /// A key with no field meaning is [`InputOutcome::NotConsumed`].
    pub fn handle_key(&mut self, key: &KeyEvent) -> InputOutcome {
        match crate::keys::crossterm_input::field_key_input(key) {
            Some(input) => self.field.handle_key(input),
            None => InputOutcome::NotConsumed,
        }
    }

    /// Render the value text at `rect` using `style` ([`FieldView::render`]).
    pub fn render(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        style: Style,
        value_offset_x: u16,
        focused: bool,
    ) {
        self.view
            .render(f, &self.field, rect, style, value_offset_x, focused);
    }

    /// Render a pre-styled line ([`FieldView::render_line`]).
    pub fn render_line(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        line: ratatui::text::Line<'static>,
        base_style: Style,
        value_offset_x: u16,
        focused: bool,
    ) {
        self.view.render_line(
            f,
            &self.field,
            rect,
            line,
            base_style,
            value_offset_x,
            focused,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

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

    #[test]
    fn take_text_returns_value_and_empties_the_input() {
        let mut i = SingleLineInput::with_value("abc");
        assert_eq!(i.take_text(), "abc");
        assert!(i.is_empty());
        assert_eq!(i.cursor_char_offset(), 0);
    }

    mod rendering {
        use super::*;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        fn draw(
            i: &mut SingleLineInput,
            width: u16,
        ) -> (Terminal<TestBackend>, Option<(u16, u16)>) {
            let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
            terminal
                .draw(|f| {
                    i.render(f, Rect::new(0, 0, width, 1), Style::default(), 0, true);
                })
                .unwrap();
            let caret = i.last_caret_pos();
            (terminal, caret)
        }

        fn row(terminal: &Terminal<TestBackend>, width: u16) -> String {
            let buf = terminal.backend().buffer();
            (0..width).map(|x| buf[(x, 0)].symbol()).collect()
        }

        #[test]
        fn short_value_renders_from_start() {
            let mut i = SingleLineInput::with_value("abc");
            let (t, caret) = draw(&mut i, 10);
            assert_eq!(row(&t, 10), "abc       ");
            assert_eq!(caret, Some((3, 0)));
        }

        #[test]
        fn long_value_scrolls_to_keep_caret_visible() {
            // 15 chars in a 10-wide rect, cursor at end: scroll 6 cols so the
            // caret cell after the last char is the rightmost column.
            let mut i = SingleLineInput::with_value("abcdefghijklmno");
            let (t, caret) = draw(&mut i, 10);
            assert_eq!(row(&t, 10), "ghijklmno ");
            assert_eq!(caret, Some((9, 0)));
        }

        #[test]
        fn cursor_moves_inside_window_without_scrolling() {
            let mut i = SingleLineInput::with_value("abcdefghijklmno");
            draw(&mut i, 10); // establishes scroll = 6
            for _ in 0..3 {
                i.handle_key(&k(KeyCode::Left));
            }
            // Cursor col 12 still inside [6, 16) — window must not move.
            let (t, caret) = draw(&mut i, 10);
            assert_eq!(row(&t, 10), "ghijklmno ");
            assert_eq!(caret, Some((6, 0)));
        }

        #[test]
        fn cursor_past_left_edge_scrolls_back() {
            let mut i = SingleLineInput::with_value("abcdefghijklmno");
            draw(&mut i, 10); // scroll = 6
            i.handle_key(&k(KeyCode::Home));
            let (t, caret) = draw(&mut i, 10);
            assert_eq!(row(&t, 10), "abcdefghij");
            assert_eq!(caret, Some((0, 0)));
        }

        #[test]
        fn shrinking_value_clamps_scroll() {
            let mut i = SingleLineInput::with_value("abcdefghijklmno");
            draw(&mut i, 10); // scroll = 6
            i.set_value("abc");
            let (t, caret) = draw(&mut i, 10);
            assert_eq!(row(&t, 10), "abc       ");
            assert_eq!(caret, Some((3, 0)));
        }
    }
}
