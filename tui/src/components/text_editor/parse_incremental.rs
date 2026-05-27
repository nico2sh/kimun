//! Diff a `Vec<String>` buffer pair into a single `tree_sitter::InputEdit`.
//!
//! Used by `MarkdownEditorView::update` on every text-mutating frame: feed
//! the resulting `InputEdit` to `EditorTree::apply_edit` for an incremental
//! reparse, or fall back to a full `parse_full` when the diff cannot be
//! expressed as a single contiguous edit.

use tree_sitter::{InputEdit, Point};

/// Maximum line distance from `cursor_row_hint` we are willing to scan in
/// either direction before declaring "diff too far from cursor — fall back to
/// full reparse". 64 lines is generous for any realistic single edit.
const SEARCH_WINDOW_LINES: usize = 64;

/// Produce a single `InputEdit` describing the diff between `old` and `new`,
/// using `cursor_row_hint` as a seed for the scan. Returns `None` when the
/// diff is empty, the buffers don't overlap (first parse), or the diff
/// extends beyond `SEARCH_WINDOW_LINES` from the cursor — caller should run a
/// full reparse in those cases.
pub fn lines_diff_to_input_edit(
    old: &[String],
    new: &[String],
    cursor_row_hint: usize,
) -> Option<InputEdit> {
    // First parse: caller must full-reparse.
    if old.is_empty() {
        return None;
    }

    // Compose source bytes once; same encoding as EditorTree's `source` mirror.
    let old_src = join_lines(old);
    let new_src = join_lines(new);

    // Common byte prefix.
    let mut p = 0usize;
    while p < old_src.len() && p < new_src.len() && old_src[p] == new_src[p] {
        p += 1;
    }

    // No diff: tail check.
    if p == old_src.len() && p == new_src.len() {
        return None;
    }

    // Common byte suffix, capped so it cannot eat into the prefix on either
    // side.
    let mut s = 0usize;
    let max_s = (old_src.len() - p).min(new_src.len() - p);
    while s < max_s
        && old_src[old_src.len() - 1 - s] == new_src[new_src.len() - 1 - s]
    {
        s += 1;
    }

    let start_byte = p;
    let old_end_byte = old_src.len() - s;
    let new_end_byte = new_src.len() - s;

    let start_position = byte_to_point(&old_src, start_byte);
    let old_end_position = byte_to_point(&old_src, old_end_byte);
    let new_end_position = byte_to_point(&new_src, new_end_byte);

    // Diff too far from cursor — bail to full reparse.
    if start_position.row.abs_diff(cursor_row_hint) > SEARCH_WINDOW_LINES {
        return None;
    }

    Some(InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position,
        old_end_position,
        new_end_position,
    })
}

fn join_lines(lines: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        out.extend_from_slice(line.as_bytes());
        if i + 1 < lines.len() {
            out.push(b'\n');
        }
    }
    out
}

fn byte_to_point(src: &[u8], byte: usize) -> Point {
    // Walk source; count '\n' to determine row; column is bytes since the
    // last '\n' (or start of buffer).
    let mut row = 0usize;
    let mut last_newline = 0usize;
    let mut last_newline_found = false;
    for (i, &b) in src.iter().enumerate().take(byte) {
        if b == b'\n' {
            row += 1;
            last_newline = i + 1;
            last_newline_found = true;
        }
    }
    let column = if last_newline_found {
        byte - last_newline
    } else {
        byte
    };
    Point::new(row, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.split('\n').map(|x| x.to_string()).collect()
    }

    fn assert_edit(
        edit: InputEdit,
        start: (usize, usize, usize),
        old_end: (usize, usize, usize),
        new_end: (usize, usize, usize),
    ) {
        assert_eq!(edit.start_byte, start.0, "start_byte");
        assert_eq!(edit.start_position, Point::new(start.1, start.2), "start_pos");
        assert_eq!(edit.old_end_byte, old_end.0, "old_end_byte");
        assert_eq!(
            edit.old_end_position,
            Point::new(old_end.1, old_end.2),
            "old_end_pos"
        );
        assert_eq!(edit.new_end_byte, new_end.0, "new_end_byte");
        assert_eq!(
            edit.new_end_position,
            Point::new(new_end.1, new_end.2),
            "new_end_pos"
        );
    }

    // ── 5.3 typing cases ──────────────────────────────────────────────────────

    #[test]
    fn intra_line_insert() {
        let old = lines("abc");
        let new = lines("aXbc");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_edit(e, (1, 0, 1), (1, 0, 1), (2, 0, 2));
    }

    #[test]
    fn intra_line_delete() {
        let old = lines("aXbc");
        let new = lines("abc");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_edit(e, (1, 0, 1), (2, 0, 2), (1, 0, 1));
    }

    #[test]
    fn newline_insert_mid_line() {
        let old = lines("abc");
        let new = lines("a\nbc");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_edit(e, (1, 0, 1), (1, 0, 1), (2, 1, 0));
    }

    #[test]
    fn newline_insert_at_line_end() {
        let old = lines("abc");
        let new = lines("abc\n");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_edit(e, (3, 0, 3), (3, 0, 3), (4, 1, 0));
    }

    #[test]
    fn backspace_at_col_zero_line_merge() {
        let old = lines("a\nbc");
        let new = lines("abc");
        let e = lines_diff_to_input_edit(&old, &new, 1).expect("Some");
        assert_edit(e, (1, 0, 1), (2, 1, 0), (1, 0, 1));
    }

    #[test]
    fn append_at_end_of_buffer() {
        let old = lines("a");
        let new = lines("a\nb");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_edit(e, (1, 0, 1), (1, 0, 1), (3, 1, 1));
    }

    #[test]
    fn append_at_start_of_buffer() {
        let old = lines("b");
        let new = lines("a\nb");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_edit(e, (0, 0, 0), (0, 0, 0), (2, 1, 0));
    }

    // ── 5.4 fallback cases ────────────────────────────────────────────────────

    #[test]
    fn first_parse_returns_none() {
        let old: Vec<String> = vec![];
        let new = lines("hello\nworld");
        assert!(lines_diff_to_input_edit(&old, &new, 0).is_none());
    }

    #[test]
    fn no_diff_returns_none() {
        let old = lines("hello\nworld");
        let new = lines("hello\nworld");
        assert!(lines_diff_to_input_edit(&old, &new, 0).is_none());
    }

    #[test]
    fn far_from_cursor_returns_none() {
        let old = lines("a");
        let new = lines("b");
        assert!(
            lines_diff_to_input_edit(&old, &new, 1_000).is_none(),
            "diff > SEARCH_WINDOW_LINES from cursor must fall back"
        );
    }

    #[test]
    fn whole_buffer_replacement_returns_some_widened_edit() {
        // The byte-prefix diff naturally collapses non-contiguous changes
        // into a single bounding edit; tree-sitter then reparses the wider
        // range. This is documented behaviour — the diff layer does not
        // need to be minimal, only contiguous.
        let old = lines("aaa\nbbb\nccc");
        let new = lines("XXX\nYYY\nZZZ");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_eq!(e.start_position, Point::new(0, 0));
        assert_eq!(e.old_end_position.row, 2);
        assert_eq!(e.new_end_position.row, 2);
    }

    #[test]
    fn multibyte_replacement() {
        // "ä" (2 bytes, c3 a4) → "a" (1 byte) — common prefix 0, common
        // suffix 0; whole range replaced.
        let old = lines("ä");
        let new = lines("a");
        let e = lines_diff_to_input_edit(&old, &new, 0).expect("Some");
        assert_eq!(e.start_byte, 0);
        assert_eq!(e.old_end_byte, 2);
        assert_eq!(e.new_end_byte, 1);
    }
}
