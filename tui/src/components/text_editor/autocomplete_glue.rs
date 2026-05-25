//! Glue between the autocomplete controller (which works in byte offsets
//! against a single joined buffer string) and `ratatui_textarea::TextArea`
//! (which works in `(row, char_col)` per-line coordinates).

use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

use crate::components::autocomplete::AcceptAction;

/// Convert a byte offset in `lines.join("\n")` to `(row, char_col)`.
/// Returns `None` if the offset is past the end of the joined text or
/// lands inside a multi-byte char boundary.
pub fn byte_to_row_char_col(lines: &[String], byte_offset: usize) -> Option<(usize, usize)> {
    let mut byte_running = 0;
    for (row, line) in lines.iter().enumerate() {
        let line_end = byte_running + line.len();
        if byte_offset >= byte_running && byte_offset <= line_end {
            let col_bytes = byte_offset - byte_running;
            if !line.is_char_boundary(col_bytes) {
                return None;
            }
            let char_col = line[..col_bytes].chars().count();
            return Some((row, char_col));
        }
        byte_running = line_end + 1; // +1 for the `\n` separator
    }
    None
}

/// Inverse of `byte_to_row_char_col`. Clamps `char_col` to the line length
/// (textarea's own `Jump` behaviour).
pub fn row_char_col_to_byte(lines: &[String], row: usize, char_col: usize) -> usize {
    let mut byte_offset = 0;
    for (r, line) in lines.iter().enumerate().take(row) {
        byte_offset += line.len() + 1;
        let _ = r;
    }
    let Some(line) = lines.get(row) else {
        return byte_offset;
    };
    byte_offset
        + line
            .char_indices()
            .nth(char_col)
            .map(|(b, _)| b)
            .unwrap_or(line.len())
}

/// Apply an `AcceptAction` from the autocomplete controller to a
/// textarea. Positions the cursor at the start of the trigger range,
/// deletes forward by the range's char count, inserts the
/// replacement, then moves the cursor to the requested
/// post-replacement byte offset.
///
/// Both `delete_str` and `cut` write the removed text into the
/// textarea's yank buffer (see `ratatui_textarea::TextArea::delete_str`
/// — `self.yank = removed.clone().into()`). To avoid clobbering
/// anything the user had previously yanked (Ctrl+X / Ctrl+C →
/// ratatui-textarea yank ring), this function snapshots the yank
/// buffer before the delete and restores it afterwards.
pub fn apply_accept_to_textarea(ta: &mut TextArea<'_>, action: &AcceptAction) {
    let before: Vec<String> = ta.lines().iter().map(|l| l.to_string()).collect();
    let Some((start_row, start_col)) = byte_to_row_char_col(&before, action.range.start) else {
        return;
    };
    if byte_to_row_char_col(&before, action.range.end).is_none() {
        return;
    }

    ta.cancel_selection();
    ta.move_cursor(CursorMove::Jump(start_row as u16, start_col as u16));
    if action.range.end > action.range.start {
        let preserved_yank = ta.yank_text();
        let joined: String = before.join("\n");
        let char_count = joined[action.range.clone()].chars().count();
        ta.delete_str(char_count);
        ta.set_yank_text(preserved_yank);
    }
    ta.insert_str(&action.new_text);

    // Place the cursor at the requested post-replacement byte offset. The
    // textarea's lines may now be different so we re-read them.
    let after: Vec<String> = ta.lines().iter().map(|l| l.to_string()).collect();
    if let Some((row, col)) = byte_to_row_char_col(&after, action.new_cursor_byte) {
        ta.move_cursor(CursorMove::Jump(row as u16, col as u16));
    }
}

/// Cursor screen position given a `rect` (col, row in cells), or `None`
/// when the cursor is scrolled off-screen. The popup uses this as its
/// anchor — a small spec liberty over "just after the sigil" (we anchor
/// at the cursor, which sits at the end of the typed prefix), but the
/// popup ends up adjacent to the typed text either way.
pub fn cursor_screen_pos(
    rendered_col: usize,
    cursor_vrow: usize,
    visual_scroll_offset: usize,
    rect: Rect,
) -> Option<(u16, u16)> {
    if cursor_vrow < visual_scroll_offset {
        return None;
    }
    let vrow_in_view = cursor_vrow - visual_scroll_offset;
    if vrow_in_view as u16 >= rect.height {
        return None;
    }
    Some((rect.x + rendered_col as u16, rect.y + vrow_in_view as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_to_row_col_single_line() {
        let lines = vec!["hello".to_string()];
        assert_eq!(byte_to_row_char_col(&lines, 0), Some((0, 0)));
        assert_eq!(byte_to_row_char_col(&lines, 3), Some((0, 3)));
        assert_eq!(byte_to_row_char_col(&lines, 5), Some((0, 5)));
        assert_eq!(byte_to_row_char_col(&lines, 6), None);
    }

    #[test]
    fn byte_to_row_col_across_newlines() {
        let lines = vec!["hi".to_string(), "world".to_string()];
        // "hi\nworld" — bytes: h=0 i=1 \n=2 w=3 o=4 r=5 l=6 d=7
        assert_eq!(byte_to_row_char_col(&lines, 2), Some((0, 2))); // end of line 0
        assert_eq!(byte_to_row_char_col(&lines, 3), Some((1, 0))); // start of line 1
        assert_eq!(byte_to_row_char_col(&lines, 7), Some((1, 4)));
        assert_eq!(byte_to_row_char_col(&lines, 8), Some((1, 5))); // end of buffer
    }

    #[test]
    fn byte_to_row_col_multi_byte_chars() {
        // "héllo" — é is 2 bytes (0xc3 0xa9); valid byte boundaries are
        // at 0, 1, 3, 4, 5, 6. char_col counts chars.
        let lines = vec!["héllo".to_string()];
        assert_eq!(byte_to_row_char_col(&lines, 0), Some((0, 0)));
        assert_eq!(byte_to_row_char_col(&lines, 1), Some((0, 1))); // after 'h'
        assert_eq!(byte_to_row_char_col(&lines, 2), None); // mid 'é'
        assert_eq!(byte_to_row_char_col(&lines, 3), Some((0, 2))); // after 'é'
        assert_eq!(byte_to_row_char_col(&lines, 4), Some((0, 3))); // after 'l'
    }

    #[test]
    fn row_col_to_byte_round_trips() {
        let lines = vec!["hi".to_string(), "héllo".to_string()];
        // "hi\n" = 3 bytes. "héllo": 'h' at 0, 'é' at 1 (2 bytes), 'l' at 3.
        for (row, col, expected_byte) in [(0, 0, 0), (0, 2, 2), (1, 0, 3), (1, 2, 6)] {
            assert_eq!(row_char_col_to_byte(&lines, row, col), expected_byte);
            assert_eq!(
                byte_to_row_char_col(&lines, expected_byte),
                Some((row, col))
            );
        }
    }

    #[test]
    fn row_col_to_byte_clamps_to_line_end() {
        let lines = vec!["hi".to_string()];
        assert_eq!(row_char_col_to_byte(&lines, 0, 999), 2);
    }

    #[test]
    fn cursor_screen_pos_scrolled_off_top() {
        let rect = Rect::new(2, 5, 80, 24);
        assert!(cursor_screen_pos(0, 0, 5, rect).is_none());
    }

    #[test]
    fn cursor_screen_pos_scrolled_off_bottom() {
        let rect = Rect::new(0, 0, 80, 10);
        assert!(cursor_screen_pos(0, 100, 0, rect).is_none());
    }

    #[test]
    fn cursor_screen_pos_in_view() {
        let rect = Rect::new(2, 5, 80, 24);
        assert_eq!(cursor_screen_pos(7, 12, 10, rect), Some((9, 7)));
    }

    #[test]
    fn apply_accept_replaces_and_positions_cursor() {
        let mut ta = TextArea::from(vec!["see [[me".to_string()]);
        // Move cursor to end so the textarea matches the spec scenario.
        ta.move_cursor(CursorMove::End);
        let action = AcceptAction {
            range: 6..8,
            new_text: "meeting]]".to_string(),
            new_cursor_byte: 15,
        };
        apply_accept_to_textarea(&mut ta, &action);
        let result: String = ta.lines().join("\n");
        assert_eq!(result, "see [[meeting]]");
        // After insert, cursor should be at end of `]]`.
        let ratatui_textarea::DataCursor(row, col) = ta.cursor();
        assert_eq!((row, col), (0, 15));
    }

    #[test]
    fn apply_accept_preserves_textarea_yank_buffer() {
        // User Ctrl+X's some text into the yank ring, then accepts an
        // autocomplete suggestion. The yank buffer must survive — the
        // ratatui-textarea `delete_str` overwrites it by default.
        let mut ta = TextArea::from(vec!["see [[me".to_string()]);
        ta.set_yank_text("previously yanked text");
        ta.move_cursor(CursorMove::End);
        let action = AcceptAction {
            range: 6..8,
            new_text: "meeting]]".to_string(),
            new_cursor_byte: 15,
        };
        apply_accept_to_textarea(&mut ta, &action);
        assert_eq!(ta.yank_text(), "previously yanked text");
    }

    #[test]
    fn apply_accept_replaces_across_multiple_lines_unaffected() {
        // Sanity: a single-line replacement on a multi-line buffer leaves
        // the other lines untouched.
        let mut ta = TextArea::from(vec!["alpha".to_string(), "see [[me".to_string()]);
        let action = AcceptAction {
            range: 12..14, // bytes 12..14 in the joined "alpha\nsee [[me"
            new_text: "meeting]]".to_string(),
            new_cursor_byte: 21,
        };
        apply_accept_to_textarea(&mut ta, &action);
        let result: String = ta.lines().join("\n");
        assert_eq!(result, "alpha\nsee [[meeting]]");
    }
}
