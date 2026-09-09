//! Markdown-aware edits over the **rope buffer**: list continuation on Enter,
//! **auto-surround**, the emphasis markers behind Bold / Italic / Strikethrough,
//! and the heading jump the OUTLINE drawer asks for.
//!
//! Every operation here takes `&mut RopeBuffer` and nothing else — the same
//! shape as the plain key table and the vim engine — so each is tested against
//! a bare buffer, not an editor. The two lookups beside them (`surround_pair`,
//! `emphasis_marker`) map a key or a `TextAction` to its markers and touch no
//! buffer at all. The buffer knows text; this module knows
//! markdown; the component that calls both knows which backend is live, what a
//! vim Visual selection includes, and what an edit owes it afterwards (the
//! selection mirror, the typing run, the outcome drain). None of that is here.

use super::markdown;
use super::rope_buffer::{CursorMove, RopeBuffer};
use crate::keys::action_shortcuts::TextAction;

/// Auto-surround pair for `c`: typing an opening pair character or a
/// symmetric one while a selection is active wraps the selection instead of
/// replacing it. Closing characters return `None` — they replace, like any
/// other key. See CONTEXT.md **Auto-surround**.
pub fn surround_pair(c: char) -> Option<(&'static str, &'static str)> {
    match c {
        '(' => Some(("(", ")")),
        '[' => Some(("[", "]")),
        '{' => Some(("{", "}")),
        '<' => Some(("<", ">")),
        '"' => Some(("\"", "\"")),
        '\'' => Some(("'", "'")),
        '`' => Some(("`", "`")),
        '*' => Some(("*", "*")),
        '_' => Some(("_", "_")),
        '~' => Some(("~", "~")),
        _ => None,
    }
}

/// The marker an emphasis action wraps a selection in, or `None` for the
/// actions that are not emphasis — Link, Image and the headers, which nothing
/// here handles.
pub fn emphasis_marker(action: TextAction) -> Option<&'static str> {
    match action {
        TextAction::Bold => Some("**"),
        TextAction::Italic => Some("*"),
        TextAction::Strikethrough => Some("~~"),
        _ => None,
    }
}

/// Wrap the selection in `open` … `close` and reselect the inner text, so
/// wraps chain — `[` `[` builds a wikilink. One **undo group**: the replace is
/// a single transaction. `false`, touching nothing, without a selection of
/// some width.
pub fn wrap_selection(buf: &mut RopeBuffer, open: &str, close: &str) -> bool {
    let Some(((sr, sc), (er, ec))) = buf.selection_range() else {
        return false;
    };
    let Some(text) = buf.selection_text() else {
        return false;
    };
    buf.insert_str(format!("{open}{text}{close}"));
    // The open marker shifts columns on the first selected row only; columns
    // are chars, matching `selection_range`.
    let shift = open.chars().count();
    let inner_end_col = if sr == er { ec + shift } else { ec };
    buf.set_selection((sr, sc + shift), (er, inner_end_col));
    true
}

/// Insert `open` and `close` at the cursor and leave the cursor between them —
/// what Bold does with nothing selected: `****`, cursor in the middle.
pub fn insert_pair(buf: &mut RopeBuffer, open: &str, close: &str) {
    buf.insert_str(format!("{open}{close}"));
    for _ in 0..close.chars().count() {
        buf.move_cursor(CursorMove::Back);
    }
}

/// Smart Enter at the end of a row: continue a list marker (an ordered one
/// incremented), carry the indent, dedent an indent-only row, and clear an
/// empty list item — dedenting it first while it is indented. `true` when
/// handled, so the caller does not insert a plain newline; `false` mid-row,
/// with a selection of some width, or on a row that is neither indented nor a
/// list item.
pub fn smart_enter(buf: &mut RopeBuffer) -> bool {
    enum Action {
        ClearRow { chars: usize },
        InsertPrefix(String),
        Dedent,
    }
    // A mouse click leaves a zero-width selection active, so only a selection
    // of some width declines.
    if buf
        .selection_range()
        .is_some_and(|(start, end)| start != end)
    {
        return false;
    }
    let (row, action) = {
        let (row, col) = buf.cursor();
        let Some(line) = buf.row(row) else {
            return false;
        };
        let total_chars = line.chars().count();
        if col != total_chars {
            return false;
        }
        // ASCII whitespace, so byte index == char index here.
        let ws_end = markdown::leading_ws_byte_len(&line);
        let (ws, after_ws) = line.split_at(ws_end);
        let action = if let Some(marker_len) = markdown::list_marker_len(after_ws) {
            if after_ws.len() == marker_len {
                // An empty item: dedent while indented, then clear the marker
                // once fully unindented.
                if ws_end > 0 {
                    Action::Dedent
                } else {
                    Action::ClearRow { chars: total_chars }
                }
            } else {
                let marker = &after_ws[..marker_len];
                let next = increment_ordered_marker(marker).unwrap_or_else(|| marker.to_string());
                Action::InsertPrefix(format!("{ws}{next}"))
            }
        } else if ws_end > 0 && total_chars == ws_end {
            Action::Dedent
        } else if ws_end > 0 {
            Action::InsertPrefix(ws.to_string())
        } else {
            return false;
        };
        (row, action)
    };
    match action {
        // A dedent that removes nothing (a zero-width step) is not handled:
        // declining lets Enter fall through to a plain newline instead of
        // being swallowed.
        Action::Dedent => return buf.indent_rows(row..=row, true),
        Action::ClearRow { chars } => {
            buf.move_cursor(CursorMove::Head);
            buf.delete_str(chars);
        }
        Action::InsertPrefix(prefix) => {
            // Newline plus prefix is two history entries; one `edit()` scope
            // makes continuing a list one undo.
            buf.edit(|buf| {
                buf.insert_newline();
                buf.insert_str(prefix);
            });
        }
    }
    true
}

/// Move the cursor to the first heading whose text equals `heading`, at any
/// level — the OUTLINE drawer's jump. `false`, cursor untouched, when none
/// matches.
///
/// OUTLINE entries carry the extractor-rendered heading text (inline markup
/// resolved, closing ATX `#` dropped), so both sides are normalised before
/// comparing: ATX markers stripped, the common inline-emphasis characters
/// removed.
pub fn jump_to_heading(buf: &mut RopeBuffer, heading: &str) -> bool {
    fn normalise(text: &str) -> String {
        text.trim()
            .trim_end_matches('#')
            .trim()
            .replace(['*', '_', '`'], "")
    }
    let wanted = normalise(heading);
    let row = (0..buf.row_count()).find(|&row| {
        let Some(line) = buf.row(row) else {
            return false;
        };
        let t = line.trim_start();
        let stripped = t.trim_start_matches('#');
        stripped.len() != t.len() && normalise(stripped) == wanted
    });
    match row {
        Some(row) => buf.jump_to(row, 0),
        None => false,
    }
}

/// If `marker` is an ordered-list marker like `"3. "`, the next one (`"4. "`).
/// `None` for unordered markers or unrecognised input.
fn increment_ordered_marker(marker: &str) -> Option<String> {
    let trimmed = marker.trim_end_matches(' ');
    let dot = trimmed.strip_suffix('.')?;
    let n: u32 = dot.parse().ok()?;
    Some(format!("{}. ", n + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ropetext::Text;

    fn buffer(text: &str) -> RopeBuffer {
        RopeBuffer::new(Text::from(text))
    }

    /// A buffer with the cursor at the very end of its text.
    fn at_end(text: &str) -> RopeBuffer {
        let mut buf = buffer(text);
        buf.move_cursor(CursorMove::Bottom);
        buf.move_cursor(CursorMove::End);
        buf
    }

    fn text(buf: &RopeBuffer) -> String {
        buf.text().to_string()
    }

    // ── surround / wrap ───────────────────────────────────────────────────

    #[test]
    fn surround_pair_maps_open_and_symmetric_chars() {
        assert_eq!(surround_pair('('), Some(("(", ")")));
        assert_eq!(surround_pair('['), Some(("[", "]")));
        assert_eq!(surround_pair('{'), Some(("{", "}")));
        assert_eq!(surround_pair('<'), Some(("<", ">")));
        assert_eq!(surround_pair('"'), Some(("\"", "\"")));
        assert_eq!(surround_pair('\''), Some(("'", "'")));
        assert_eq!(surround_pair('`'), Some(("`", "`")));
        assert_eq!(surround_pair('*'), Some(("*", "*")));
        assert_eq!(surround_pair('_'), Some(("_", "_")));
        assert_eq!(surround_pair('~'), Some(("~", "~")));
        assert_eq!(surround_pair(')'), None);
        assert_eq!(surround_pair(']'), None);
        assert_eq!(surround_pair('}'), None);
        assert_eq!(surround_pair('>'), None);
        assert_eq!(surround_pair('a'), None);
    }

    #[test]
    fn emphasis_marker_covers_bold_italic_and_strikethrough_only() {
        assert_eq!(emphasis_marker(TextAction::Bold), Some("**"));
        assert_eq!(emphasis_marker(TextAction::Italic), Some("*"));
        assert_eq!(emphasis_marker(TextAction::Strikethrough), Some("~~"));
        assert_eq!(emphasis_marker(TextAction::Underline), None);
        assert_eq!(emphasis_marker(TextAction::Link), None);
        assert_eq!(emphasis_marker(TextAction::Image), None);
        assert_eq!(emphasis_marker(TextAction::ToggleHeader), None);
        assert_eq!(emphasis_marker(TextAction::Header(1)), None);
    }

    #[test]
    fn wrap_reselects_the_inner_text_so_wraps_chain() {
        let mut buf = buffer("my note");
        assert!(buf.set_selection((0, 0), (0, 7)));
        assert!(wrap_selection(&mut buf, "[", "]"));
        assert_eq!(text(&buf), "[my note]");
        assert_eq!(buf.selection_range(), Some(((0, 1), (0, 8))));
        assert!(wrap_selection(&mut buf, "[", "]"));
        assert_eq!(text(&buf), "[[my note]]");
        assert_eq!(buf.selection_range(), Some(((0, 2), (0, 9))));
    }

    #[test]
    fn wrap_spans_a_multi_row_selection() {
        let mut buf = buffer("abc\ndef");
        assert!(buf.set_selection((0, 0), (1, 3)));
        assert!(wrap_selection(&mut buf, "(", ")"));
        assert_eq!(text(&buf), "(abc\ndef)");
        // The open marker shifts only the first row.
        assert_eq!(buf.selection_range(), Some(((0, 1), (1, 3))));
    }

    #[test]
    fn wrap_counts_columns_in_chars() {
        let mut buf = buffer("héllo🦀 x");
        assert!(buf.set_selection((0, 0), (0, 6))); // "héllo🦀" = 6 chars
        assert!(wrap_selection(&mut buf, "`", "`"));
        assert_eq!(text(&buf), "`héllo🦀` x");
        assert_eq!(buf.selection_range(), Some(((0, 1), (0, 7))));
    }

    #[test]
    fn wrap_handles_a_selection_made_right_to_left() {
        let mut buf = buffer("hello world");
        assert!(buf.jump_to(0, 5));
        buf.start_selection();
        assert!(buf.jump_to(0, 0));
        assert!(wrap_selection(&mut buf, "(", ")"));
        assert_eq!(text(&buf), "(hello) world");
        assert_eq!(buf.selection_range(), Some(((0, 1), (0, 6))));
    }

    #[test]
    fn wrap_uses_a_word_selection_as_it_stands() {
        let mut buf = buffer("hello world");
        buf.move_cursor(CursorMove::Head);
        buf.start_selection();
        buf.move_cursor(CursorMove::WordForward);
        assert!(wrap_selection(&mut buf, "~~", "~~"));
        assert_eq!(text(&buf), "~~hello ~~world");
    }

    #[test]
    fn wrap_handles_a_non_ascii_selection() {
        let mut buf = buffer("hello 你好 world");
        buf.move_cursor(CursorMove::Head);
        buf.move_cursor(CursorMove::WordForward);
        buf.start_selection();
        buf.move_cursor(CursorMove::WordForward);
        assert!(wrap_selection(&mut buf, "**", "**"));
        assert_eq!(text(&buf), "hello **你好 **world");
    }

    #[test]
    fn wrap_without_a_selection_touches_nothing() {
        let mut buf = at_end("hello");
        assert!(!wrap_selection(&mut buf, "(", ")"));
        assert_eq!(text(&buf), "hello");
        assert!(!buf.take_outcome().changed);
    }

    #[test]
    fn a_zero_width_selection_does_not_wrap() {
        let mut buf = buffer("hello");
        assert!(buf.jump_to(0, 2));
        buf.start_selection();
        assert!(!wrap_selection(&mut buf, "(", ")"));
        assert_eq!(text(&buf), "hello");
    }

    #[test]
    fn a_wrap_is_one_undo_group() {
        // The replace is a single transaction, so the gesture is one history
        // entry. Asserting what each undo *returns* is what makes this a claim
        // about grouping rather than about the final text.
        let mut buf = buffer("hello world");
        assert!(buf.set_selection((0, 0), (0, 5)));
        assert!(wrap_selection(&mut buf, "(", ")"));
        assert_eq!(text(&buf), "(hello) world");
        assert!(buf.undo(), "the wrap is one entry");
        assert_eq!(text(&buf), "hello world");
        assert!(!buf.undo(), "and has no second half left to take back");
    }

    #[test]
    fn insert_pair_leaves_the_cursor_between_the_markers() {
        let mut buf = at_end("hello");
        insert_pair(&mut buf, "**", "**");
        assert_eq!(text(&buf), "hello****");
        assert_eq!(buf.cursor(), (0, 7));
    }

    #[test]
    fn insert_pair_with_a_single_char_marker() {
        let mut buf = buffer("");
        insert_pair(&mut buf, "*", "*");
        assert_eq!(text(&buf), "**");
        assert_eq!(buf.cursor(), (0, 1));
    }

    // ── smart enter ───────────────────────────────────────────────────────

    #[test]
    fn smart_enter_continues_an_unordered_list() {
        let mut buf = at_end("- foo");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "- foo\n- ");
    }

    #[test]
    fn smart_enter_increments_an_ordered_list() {
        let mut buf = at_end("1. foo");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "1. foo\n2. ");
    }

    #[test]
    fn smart_enter_continues_an_indented_list() {
        let mut buf = at_end("  - foo");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "  - foo\n  - ");
    }

    #[test]
    fn smart_enter_continues_a_list_with_non_ascii_content() {
        let mut buf = at_end("- 你好");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "- 你好\n- ");
    }

    #[test]
    fn smart_enter_on_an_empty_list_marker_clears_the_row() {
        let mut buf = at_end("- ");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "");
    }

    #[test]
    fn smart_enter_on_an_empty_indented_list_marker_dedents_keeping_the_marker() {
        let mut buf = at_end("    - ");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "- ");
    }

    #[test]
    fn smart_enter_clears_an_empty_list_marker_once_fully_dedented() {
        let mut buf = at_end("    - ");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "- ");
        buf.move_cursor(CursorMove::End);
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "");
    }

    #[test]
    fn smart_enter_carries_the_indent() {
        let mut buf = at_end("    body");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "    body\n    ");
    }

    #[test]
    fn smart_enter_carries_an_existing_tab_indent() {
        // Carrying is not inserting: a tab the row already has is kept as it is.
        let mut buf = at_end("\tbody");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "\tbody\n\t");
    }

    #[test]
    fn smart_enter_on_an_indent_only_row_dedents() {
        let mut buf = at_end("    ");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "");
    }

    #[test]
    fn smart_enter_on_a_tab_only_row_dedents_one_tab() {
        let mut buf = at_end("\t\t");
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "\t");
    }

    #[test]
    fn a_zero_width_step_makes_smart_enter_decline_rather_than_swallow_enter() {
        let mut buf = at_end("    ");
        buf.set_indent_width(0);
        assert!(!smart_enter(&mut buf));
        assert_eq!(text(&buf), "    ");
    }

    #[test]
    fn smart_enter_declines_a_plain_row() {
        let mut buf = at_end("plain");
        assert!(!smart_enter(&mut buf));
        assert_eq!(text(&buf), "plain");
    }

    #[test]
    fn smart_enter_declines_mid_row() {
        let mut buf = buffer("- foo");
        assert!(buf.jump_to(0, 2));
        assert!(!smart_enter(&mut buf));
        assert_eq!(text(&buf), "- foo");
    }

    #[test]
    fn smart_enter_declines_a_selection_of_width() {
        let mut buf = at_end("- foo");
        assert!(buf.set_selection((0, 0), (0, 2)));
        assert!(!smart_enter(&mut buf));
        assert_eq!(text(&buf), "- foo");
    }

    #[test]
    fn a_zero_width_selection_does_not_stop_smart_enter() {
        // A mouse click leaves one behind.
        let mut buf = at_end("- foo");
        buf.start_selection();
        assert!(smart_enter(&mut buf));
        assert_eq!(text(&buf), "- foo\n- ");
    }

    #[test]
    fn continuing_a_list_is_one_undo_group() {
        let mut buf = at_end("- foo");
        assert!(smart_enter(&mut buf));
        assert!(buf.undo(), "newline plus marker is one entry");
        assert_eq!(text(&buf), "- foo");
        assert!(!buf.undo());
    }

    // ── heading jump ──────────────────────────────────────────────────────

    #[test]
    fn jump_to_heading_finds_any_level_and_normalises_markup() {
        let mut buf = buffer("intro\n# Top\nbody\n## **Sub** One ##\nmore\n");
        assert!(jump_to_heading(&mut buf, "Sub One"));
        assert_eq!(buf.cursor(), (3, 0));
        assert!(jump_to_heading(&mut buf, "Top"));
        assert_eq!(buf.cursor(), (1, 0));
    }

    #[test]
    fn an_unknown_heading_leaves_the_cursor_where_it_was() {
        let mut buf = buffer("intro\n# Top\nbody");
        assert!(buf.jump_to(2, 1));
        assert!(!jump_to_heading(&mut buf, "Nope"));
        assert_eq!(buf.cursor(), (2, 1));
    }

    #[test]
    fn a_hash_inside_a_row_is_not_a_heading() {
        let mut buf = buffer("see #tag here\n# Real");
        assert!(jump_to_heading(&mut buf, "Real"));
        assert_eq!(buf.cursor(), (1, 0));
        assert!(!jump_to_heading(&mut buf, "tag here"));
    }
}
