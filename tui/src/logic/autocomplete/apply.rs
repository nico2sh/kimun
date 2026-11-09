//! The accept edit: an [`AcceptAction`] (byte offsets into the joined text)
//! applied to the **rope buffer** (`(row, char_col)` coordinates).
//!
//! The conversion between the two is [`crate::ropetext::Text::position_at_byte`]:
//! the offsets the engine reports index the same bytes the rope does, so no
//! joined copy of the note has to exist for them to be resolved.

use crate::logic::editor::rope_buffer::{CursorMove, RopeBuffer};
use unicode_segmentation::UnicodeSegmentation;

use super::AcceptAction;

/// Apply an `AcceptAction` from the autocomplete controller to a
/// textarea. Positions the cursor at the start of the trigger range,
/// deletes forward by the range's char count, inserts the
/// replacement, then moves the cursor to the requested
/// post-replacement byte offset.
///
/// Both `delete_str` and `cut` write the removed text into the
/// buffer's register (see `RopeBuffer::delete_str`
/// — `self.yank = removed.clone().into()`). To avoid clobbering
/// anything the user had previously yanked (Ctrl+X / Ctrl+C), this
/// function snapshots the yank buffer before the delete and restores
/// it afterwards.
pub fn apply_accept(ta: &mut RopeBuffer, action: &AcceptAction) {
    // Cloning the text is a pointer copy, which is what lets the byte offsets
    // be resolved against the buffer while the buffer is being mutated. The
    // range materialised below is the replaced text, not the note.
    let before = ta.text().clone();
    let (Some(start), Some(end)) = (
        before.position_at_byte(action.range.start),
        before.position_at_byte(action.range.end),
    ) else {
        return;
    };

    ta.cancel_selection();
    ta.move_cursor(CursorMove::Jump(start.row(), start.column().get()));
    if action.range.end > action.range.start {
        let preserved_yank = ta.yank_text();
        // Clusters, matching what `delete_str` spends: the controller reports
        // byte offsets, and counting the scalars between them over a flag or a
        // ZWJ sequence would delete past the range it named.
        let Some(clusters) = before
            .span(start, end)
            .and_then(|span| before.slice(span))
            .map(|removed| removed.graphemes(true).count())
        else {
            return;
        };
        ta.delete_str(clusters);
        ta.set_yank_text(preserved_yank);
    }
    ta.insert_str(&action.new_text);

    // Place the cursor at the requested post-replacement byte offset, against
    // the text as the edits above left it.
    let after = ta.text().clone();
    if let Some(position) = after.position_at_byte(action.new_cursor_byte) {
        ta.move_cursor(CursorMove::Jump(position.row(), position.column().get()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_accept_replaces_and_positions_cursor() {
        let mut ta = RopeBuffer::new(crate::ropetext::Text::from("see [[me"));
        // Move cursor to end so the textarea matches the accept scenario.
        ta.move_cursor(CursorMove::End);
        let action = AcceptAction {
            range: 6..8,
            new_text: "meeting]]".to_string(),
            new_cursor_byte: 15,
            saved_search_name: None,
        };
        apply_accept(&mut ta, &action);
        let result: String = ta.text().to_string();
        assert_eq!(result, "see [[meeting]]");
        // After insert, cursor should be at end of `]]`.
        let (row, col) = ta.cursor();
        assert_eq!((row, col), (0, 15));
    }

    #[test]
    fn apply_accept_over_a_multi_scalar_cluster_stops_at_the_range() {
        // The trigger range covers `[[` plus a flag: 4 scalars but 3 clusters.
        // `delete_str` steps clusters, so handing it the scalar count spent one
        // step past the range and swallowed the character after it.
        let mut ta = RopeBuffer::new(crate::ropetext::Text::from("see [[\u{1F1EA}\u{1F1F8} rest"));
        ta.move_cursor(CursorMove::End);
        let action = AcceptAction {
            // bytes 4..14 — `[[` (2) plus the two 4-byte regional indicators.
            range: 4..14,
            new_text: "[[Spain]]".to_string(),
            new_cursor_byte: 13,
            saved_search_name: None,
        };
        apply_accept(&mut ta, &action);
        assert_eq!(
            ta.text().to_string(),
            "see [[Spain]] rest",
            "the space after the flag is outside the range and must survive"
        );
    }

    #[test]
    fn apply_accept_preserves_textarea_yank_buffer() {
        // User Ctrl+X's some text into the yank ring, then accepts an
        // autocomplete suggestion. The yank buffer must survive —
        // `RopeBuffer::delete_str` overwrites it by default.
        let mut ta = RopeBuffer::new(crate::ropetext::Text::from("see [[me"));
        ta.set_yank_text("previously yanked text");
        ta.move_cursor(CursorMove::End);
        let action = AcceptAction {
            range: 6..8,
            new_text: "meeting]]".to_string(),
            new_cursor_byte: 15,
            saved_search_name: None,
        };
        apply_accept(&mut ta, &action);
        assert_eq!(ta.yank_text(), "previously yanked text");
    }

    #[test]
    fn apply_accept_replaces_across_multiple_lines_unaffected() {
        // Sanity: a single-line replacement on a multi-line buffer leaves
        // the other lines untouched.
        let mut ta = RopeBuffer::new(crate::ropetext::Text::from("alpha\nsee [[me"));
        let action = AcceptAction {
            range: 12..14, // bytes 12..14 in the joined "alpha\nsee [[me"
            new_text: "meeting]]".to_string(),
            new_cursor_byte: 21,
            saved_search_name: None,
        };
        apply_accept(&mut ta, &action);
        let result: String = ta.text().to_string();
        assert_eq!(result, "alpha\nsee [[meeting]]");
    }
}
