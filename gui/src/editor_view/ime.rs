//! The platform text input contract: the preedit of a composition, the
//! commit of its text, the marked range and the caret bounds that the IME
//! reads. Text lands in the buffer, or in the find bar's focused field
//! while the bar is open. This module does not handle key-downs.

use std::ops::Range;

use gpui_kit::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};
use kimun_notes::logic::editor::Editor;
use kimun_notes::logic::input::{Key, KeyInput};

use super::EditorView;
use crate::find_bar;

impl EditorView {
    /// The input handler's whole "document": the cursor row, with the preedit
    /// spliced in at the caret. The IME only ever needs the text around the
    /// caret, and a row-local document keeps every query O(row) instead of
    /// O(buffer). The preedit is in the document but not in the buffer.
    ///
    /// While the find bar is open the document is its focused field instead.
    fn document(&self) -> Document {
        if let Some(bar) = self.editor.find_bar() {
            let field = find_bar::focused_field(bar);
            return Document::new(
                0,
                field.value().to_string(),
                field.cursor_char_offset(),
                self.preedit.as_deref(),
            );
        }
        let (row, caret) = self.editor.cursor();
        let buffer_line = self
            .editor
            .buffer()
            .row(row)
            .map(|r| r.into_owned())
            .unwrap_or_default();
        Document::new(row, buffer_line, caret, self.preedit.as_deref())
    }

    /// Commit `text`: over `range` of the document when given, else at the
    /// cursor.
    ///
    /// Composition rule: a preedit never touches the buffer. A selection made
    /// before composing stays as it is while composing; *cancelling* (empty
    /// text over the preedit, or an empty preedit update) only drops the
    /// preedit, so nothing the user selected is ever lost to an abandoned
    /// composition; *committing* text acts exactly like typing it, replacing
    /// that selection.
    fn commit(&mut self, range: Option<Range<usize>>, text: &str) {
        let doc = self.document();
        let marked = doc.marked();
        let had_preedit = self.preedit.take().is_some();
        let over_preedit = had_preedit && (range.is_none() || range == marked);
        if over_preedit && text.is_empty() {
            return;
        }
        let target = range.filter(|r| Some(r) != marked.as_ref()).map(|r| {
            (
                doc.row,
                doc.to_buffer(r.start)..doc.to_buffer(r.end.max(r.start)),
            )
        });
        if self.editor.find_open() {
            self.commit_to_bar(target.map(|(_, cols)| cols), text);
        } else {
            self.editor.commit_text(target, text);
        }
    }

    /// Committed text into the find bar's focused field: over `cols` (chars
    /// of the field) when given, else at its caret. The field is the logic
    /// bar's, reached only through its key and paste API, so a range is
    /// walked to with Left / Right, cleared with Backspace, and the text goes
    /// in as the one key it is, or as a paste (first line only; a field is
    /// one line).
    fn commit_to_bar(&mut self, cols: Option<Range<usize>>, text: &str) {
        let press = |editor: &mut Editor, key: Key, n: usize| {
            for _ in 0..n {
                editor.handle_find_key(KeyInput::plain(key));
            }
        };
        if let Some(cols) = cols
            && let Some(bar) = self.editor.find_bar()
        {
            let caret = find_bar::focused_field(bar).cursor_char_offset();
            let len = find_bar::focused_field(bar).value().chars().count();
            let end = cols.end.min(len);
            let start = cols.start.min(end);
            if caret > end {
                press(&mut self.editor, Key::Left, caret - end);
            } else {
                press(&mut self.editor, Key::Right, end - caret);
            }
            press(&mut self.editor, Key::Backspace, end - start);
        }
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (None, _) => {}
            (Some(c), None) if c.is_control() => {}
            (Some(c), None) => {
                self.editor.handle_find_key(KeyInput::plain(Key::Char(c)));
            }
            _ => {
                self.editor.paste_into_find(text);
            }
        }
    }
}

/// The cursor row as the input handler sees it (see [`EditorView::document`]).
struct Document {
    row: usize,
    /// The row with the preedit spliced in at the caret.
    line: String,
    /// UTF-16 offset of the caret (= the preedit's start) in `line`.
    caret: usize,
    /// The caret's char column in the buffer row.
    caret_col: usize,
    /// UTF-16 length of the preedit, 0 without one.
    preedit_len: usize,
}

impl Document {
    fn new(row: usize, buffer_line: String, caret_col: usize, preedit: Option<&str>) -> Self {
        let byte = buffer_line
            .char_indices()
            .nth(caret_col)
            .map_or(buffer_line.len(), |(i, _)| i);
        let preedit = preedit.unwrap_or("");
        let mut line = buffer_line;
        line.insert_str(byte, preedit);
        let caret = char_to_utf16(&line, caret_col);
        Self {
            row,
            line,
            caret,
            caret_col,
            preedit_len: preedit.encode_utf16().count(),
        }
    }

    /// Where the preedit sits, when there is one.
    fn marked(&self) -> Option<Range<usize>> {
        (self.preedit_len > 0).then(|| self.caret..self.caret + self.preedit_len)
    }

    /// A document UTF-16 offset as a buffer char column. Offsets inside the
    /// preedit land on the caret: none of it is in the buffer.
    fn to_buffer(&self, utf16: usize) -> usize {
        if utf16 <= self.caret {
            utf16_to_char(&self.line, utf16)
        } else if utf16 >= self.caret + self.preedit_len {
            utf16_to_char(&self.line, utf16) - self.preedit_chars()
        } else {
            self.caret_col
        }
    }

    fn preedit_chars(&self) -> usize {
        let end = utf16_to_char(&self.line, self.caret + self.preedit_len);
        end - self.caret_col
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let line = self.document().line;
        let (a, b) = (
            utf16_to_char(&line, range.start),
            utf16_to_char(&line, range.end.max(range.start)),
        );
        *adjusted_range = Some(char_to_utf16(&line, a)..char_to_utf16(&line, b));
        Some(line.chars().skip(a).take(b - a).collect())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let doc = self.document();
        // While composing the caret is drawn after the preedit.
        if let Some(marked) = doc.marked() {
            return Some(UTF16Selection {
                range: marked.end..marked.end,
                reversed: false,
            });
        }
        let (row, line) = (doc.row, &doc.line);
        let col = doc.caret_col;
        let caret = doc.caret;
        // A find bar field has a caret and no selection.
        let selection = self
            .editor
            .selection()
            .filter(|_| !self.editor.find_open())
            .filter(|((sr, _), (er, _))| *sr == row && *er == row);
        Some(match selection {
            Some(((_, sc), (_, ec))) => UTF16Selection {
                range: char_to_utf16(line, sc)..char_to_utf16(line, ec),
                reversed: col == sc && sc != ec,
            },
            None => UTF16Selection {
                range: caret..caret,
                reversed: false,
            },
        })
    }

    /// The preedit's place in the document. It is not in the buffer, but the
    /// platform must see it: Linux cancels a composition by deleting the
    /// marked range, and macOS reads "composing" from its presence (so Enter
    /// commits rather than inserting a newline).
    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.document().marked()
    }

    /// Turns the platform's text services (IME) off outside plain mode and vim
    /// Insert. Linux and Windows honour it each frame; the methods below gate
    /// on the same rule for a platform that does not.
    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.accepts_text()
    }

    /// Ending composition keeps what was composed (GPUI's contract): with the
    /// preedit outside the buffer, that means committing it now.
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.preedit.take() {
            if self.accepts_text() && !self.input_blocked {
                self.commit(None, &text);
            }
            self.settle(window, cx);
        }
    }

    /// Committed text. Outside plain mode and vim Insert it is not typing: a
    /// single char the platform composed (a dead-key `é`, say) goes to vim as
    /// the keystroke it is, so `r` then `é` still replaces; anything longer
    /// is dropped.
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked {
            self.preedit = None;
        } else if self.accepts_text() {
            self.commit(range, text);
        } else {
            self.preedit = None;
            let mut chars = text.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                self.editor
                    .handle_key(KeyInput::plain(Key::Char(c)), &self.tx);
            }
        }
        self.settle(window, cx);
    }

    /// A new or updated preedit. An active selection is left alone (see the
    /// composition rule on `commit`). A `range` other than the current preedit
    /// is buffer text the platform explicitly hands to the composition (macOS
    /// reconversion): it is removed and the preedit starts where it was.
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // No composing where the editor does not type.
        if self.input_blocked || !self.accepts_text() {
            self.preedit = None;
            self.settle(window, cx);
            return;
        }
        let doc = self.document();
        if let Some(r) = range.filter(|r| Some(r) != doc.marked().as_ref()) {
            let cols = doc.to_buffer(r.start)..doc.to_buffer(r.end.max(r.start));
            // The preedit is not in the field/buffer: drop it before editing.
            self.preedit = None;
            if self.editor.find_open() {
                self.commit_to_bar(Some(cols), "");
            } else {
                self.editor.commit_text(Some((doc.row, cols)), "");
            }
        }
        self.preedit = (!new_text.is_empty()).then(|| new_text.to_string());
        self.settle(window, cx);
    }

    /// Where the IME should put its candidate window: the caret cell (the
    /// find bar's while it is open).
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.painted.as_ref().and_then(|p| p.caret)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// The char index `utf16` code units into `s`, clamped to its end. An offset
/// inside a surrogate pair rounds down to the char that starts it.
fn utf16_to_char(s: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in s.chars().enumerate() {
        let next = units + c.len_utf16();
        if next > utf16 {
            return i;
        }
        units = next;
    }
    s.chars().count()
}

/// UTF-16 code units in the first `chars` chars of `s`, clamped to its end.
fn char_to_utf16(s: &str, chars: usize) -> usize {
    s.chars().take(chars).map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_round_trip_through_chars() {
        // 'a' (1 unit), 'é' (1), '😀' (2), 'b' (1).
        let s = "aé😀b";
        assert_eq!(char_to_utf16(s, 0), 0);
        assert_eq!(char_to_utf16(s, 3), 4);
        assert_eq!(char_to_utf16(s, 99), 5);
        assert_eq!(utf16_to_char(s, 4), 3);
        assert_eq!(utf16_to_char(s, 2), 2);
        // Inside the surrogate pair: the emoji's own index.
        assert_eq!(utf16_to_char(s, 3), 2);
        assert_eq!(utf16_to_char(s, 99), 4);
    }
}
