//! Where a movement lands.
//!
//! Every motion is a function of a text and a position that *returns* a position.
//! It never moves a cursor. That is what lets an operator ask where a movement
//! would end without going there first — deleting to the end of a word is a
//! delete over `cursor..word_end(text, cursor)`, not a move followed by a
//! measurement of where the cursor got to.
//!
//! A position sits *between* clusters, so the far end of a row is a place. An
//! editor whose cursor sits *on* a character has one fewer column per row and
//! adapts at its own edge; this module does not take a view on that.
//!
//! # Failure
//!
//! A motion returns [`Position`] when it is defined as going as far as it can —
//! moving right at the end of the text stays at the end. It returns
//! `Option<Position>` when finding nothing has to be distinguishable from
//! arriving somewhere, because an operator waiting on it must abort rather than
//! act on a range that means something else. `f` with no matching character on the
//! row is the plain case: deleting to it must delete nothing at all.

use crate::layout::{Cell, Layout, RowHints};
use crate::position::{Column, Position};
use crate::text::Text;

/// What kind of character a cluster starts with, for word motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Space, tab, or the line break.
    Blank,
    /// Letters, digits, underscore — a word to a word motion.
    Word,
    /// Everything else that is not blank.
    Punctuation,
}

/// How a word motion divides the text.
///
/// One motion implementation serves both by taking this: what distinguishes `w`
/// from `W` is entirely which runs count as one word, so writing them separately
/// means writing the same crossing-rows, empty-line and end-of-text logic twice
/// and getting it subtly different in each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Words {
    /// Runs of word characters and runs of punctuation are separate words.
    Small,
    /// Any run of non-blanks is one word.
    Big,
}

impl Words {
    /// How this division classifies `c`.
    ///
    /// Public because a caller building its own ranges — vim's text objects, say —
    /// must divide the text exactly as the motions do, and reimplementing the rule
    /// is how the two drift apart.
    pub fn class_of(self, c: char) -> Class {
        self.classify(c)
    }

    fn classify(self, c: char) -> Class {
        if c.is_whitespace() {
            Class::Blank
        } else if self == Words::Big || c.is_alphanumeric() || c == '_' {
            Class::Word
        } else {
            Class::Punctuation
        }
    }
}

/// Where a vertical motion aims to land.
///
/// Vim calls the remembered column `curswant`, and it is why walking down through
/// a short line and out the other side returns to the column you started in
/// rather than the short line's end. Passed in rather than remembered here: this
/// module holds no state, and the caller already knows whether the last motion was
/// vertical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    /// Land in this column, or at the end of the row if it is shorter.
    Column(Column),
    /// Land at the end of the row, whatever its length.
    RowEnd,
}

// -- within a row ------------------------------------------------------------

/// One cluster right, stopping at the end of the row.
pub fn right(text: &Text, from: Position) -> Position {
    let end = row_end(text, from);
    if from.byte() >= end.byte() {
        return from;
    }
    position(text, text.next_cluster_byte(from.byte()))
}

/// One cluster left, stopping at the start of the row.
pub fn left(text: &Text, from: Position) -> Position {
    let start = row_start(text, from);
    if from.byte() <= start.byte() {
        return from;
    }
    position(text, text.prev_cluster_byte(from.byte()))
}

/// One cluster forward, crossing into the next row at the end of this one.
pub fn next_cluster(text: &Text, from: Position) -> Position {
    position(text, text.next_cluster_byte(from.byte()))
}

/// One cluster back, crossing into the previous row at the start of this one.
pub fn prev_cluster(text: &Text, from: Position) -> Position {
    position(text, text.prev_cluster_byte(from.byte()))
}

/// Column zero of this row.
pub fn row_start(text: &Text, from: Position) -> Position {
    text.position(from.row(), Column::ZERO)
        .unwrap_or_else(|| text.start())
}

/// Just past the last cluster of this row, before its line break.
pub fn row_end(text: &Text, from: Position) -> Position {
    let len = text.line_len_chars(from.row()).unwrap_or(0);
    text.position(from.row(), Column::new(len)).unwrap_or(from)
}

/// The first non-blank of this row, or its end if the row is all blanks.
pub fn first_non_blank(text: &Text, from: Position) -> Position {
    let Some(line) = text.line(from.row()) else {
        return from;
    };
    let blanks = line.chars().take_while(|c| c.is_whitespace()).count();
    text.position(from.row(), Column::new(blanks))
        .unwrap_or_else(|| row_end(text, from))
}

/// Just past the last non-blank of this row, or its start if all blanks.
pub fn last_non_blank(text: &Text, from: Position) -> Position {
    let Some(line) = text.line(from.row()) else {
        return from;
    };
    let trimmed = line.trim_end();
    let chars = trimmed.chars().count();
    text.position(from.row(), Column::new(chars))
        .unwrap_or_else(|| row_end(text, from))
}

// -- across rows -------------------------------------------------------------

/// The start of the text.
pub fn text_start(text: &Text) -> Position {
    text.start()
}

/// The end of the text.
pub fn text_end(text: &Text) -> Position {
    text.end()
}

/// The first non-blank of row `row`, clamped to the last row.
pub fn goto_row(text: &Text, row: usize) -> Position {
    let row = row.min(text.line_count().saturating_sub(1));
    let at = text
        .position(row, Column::ZERO)
        .unwrap_or_else(|| text.start());
    first_non_blank(text, at)
}

/// `rows` rows down (or up, when negative), aiming at `goal`.
///
/// Saturates at the first and last row rather than failing: holding a cursor key
/// down at the end of a buffer should rest there.
pub fn vertical(text: &Text, from: Position, rows: isize, goal: Goal) -> Position {
    let last = text.line_count().saturating_sub(1);
    let row = if rows >= 0 {
        from.row().saturating_add(rows.unsigned_abs()).min(last)
    } else {
        from.row().saturating_sub(rows.unsigned_abs())
    };
    let len = text.line_len_chars(row).unwrap_or(0);
    let column = match goal {
        Goal::RowEnd => len,
        Goal::Column(column) => column.get().min(len),
    };
    // A column landing inside a cluster is not addressable, so walk back to the
    // cluster that contains it — the row below may join characters the row above
    // kept apart.
    let mut column = column;
    loop {
        if let Some(at) = text.position(row, Column::new(column)) {
            return at;
        }
        if column == 0 {
            return text.position(row, Column::ZERO).unwrap_or(from);
        }
        column -= 1;
    }
}

/// The next paragraph break at or after `from`: a blank row, or the end of the
/// text.
///
/// Saturates. Vim's `}`.
pub fn paragraph_forward(text: &Text, from: Position) -> Position {
    let last = text.line_count().saturating_sub(1);
    let mut row = from.row() + 1;
    while row <= last {
        if is_blank_row(text, row) {
            return text
                .position(row, Column::ZERO)
                .unwrap_or_else(|| text.end());
        }
        row += 1;
    }
    text.end()
}

/// The previous paragraph break before `from`. Vim's `{`.
pub fn paragraph_back(text: &Text, from: Position) -> Position {
    let mut row = from.row();
    while row > 0 {
        row -= 1;
        if is_blank_row(text, row) {
            return text
                .position(row, Column::ZERO)
                .unwrap_or_else(|| text.start());
        }
    }
    text.start()
}

// -- words -------------------------------------------------------------------

/// The start of the next word. Vim's `w` and `W`.
///
/// Saturates at the end of the text. An empty row is a word of its own, as it is
/// in vim, so walking forward through a blank line stops on it.
pub fn word_start_forward(text: &Text, from: Position, words: Words) -> Position {
    let len = text.len_bytes();
    let mut at = from.byte();
    if at >= len {
        return text.end();
    }
    let start_class = class_at(text, at, words);
    let start_row = from.row();

    // Leave the run the cursor is in.
    if start_class != Class::Blank {
        while at < len && class_at(text, at, words) == start_class {
            at = text.next_cluster_byte(at);
        }
    }
    // Then skip blanks — but an empty row is a stop in its own right.
    while at < len && class_at(text, at, words) == Class::Blank {
        // Not the row we started on, or a cursor already sitting on an empty row
        // would be told to stay there and the motion would never move.
        if is_empty_row_at(text, at) && text.row_of_byte(at) != start_row {
            return position(text, at);
        }
        at = text.next_cluster_byte(at);
    }
    position(text, at)
}

/// The start of the current or previous word. Vim's `b` and `B`.
pub fn word_start_back(text: &Text, from: Position, words: Words) -> Position {
    let mut at = from.byte();
    if at == 0 {
        return text.start();
    }
    // Step off the cursor, then back over blanks. An empty row is a stop.
    at = text.prev_cluster_byte(at);
    while at > 0 && class_at(text, at, words) == Class::Blank {
        if is_empty_row_at(text, at) {
            return position(text, at);
        }
        at = text.prev_cluster_byte(at);
    }
    let run = class_at(text, at, words);
    if run == Class::Blank {
        return position(text, at);
    }
    // Walk to the front of the run.
    while at > 0 {
        let previous = text.prev_cluster_byte(at);
        if class_at(text, previous, words) != run {
            break;
        }
        at = previous;
    }
    position(text, at)
}

/// Just past the end of the current or next word. Vim's `e` and `E`.
///
/// `None` when there is no word ahead, so `de` at the end of a buffer deletes
/// nothing rather than deleting to the end.
pub fn word_end_forward(text: &Text, from: Position, words: Words) -> Option<Position> {
    let len = text.len_bytes();
    let mut at = text.next_cluster_byte(from.byte());
    while at < len && class_at(text, at, words) == Class::Blank {
        at = text.next_cluster_byte(at);
    }
    if at >= len {
        return None;
    }
    let run = class_at(text, at, words);
    while at < len && class_at(text, at, words) == run {
        at = text.next_cluster_byte(at);
    }
    Some(position(text, at))
}

/// Just past the end of the word at or after `from`.
///
/// Distinct from [`word_end_forward`] on purpose. That one is vim's `e`, which
/// refuses the position it starts on — so from the single-character word in
/// `"a "` it looks *past* it and finds nothing. This one accepts it, which is
/// what "delete to the end of the word" means: the caller is naming a range that
/// starts where the cursor is, not asking where `e` would land.
///
/// `None` when there is no word at or after `from`.
pub fn word_end_at_or_after(text: &Text, from: Position, words: Words) -> Option<Position> {
    let len = text.len_bytes();
    let mut at = from.byte();
    while at < len && class_at(text, at, words) == Class::Blank {
        at = text.next_cluster_byte(at);
    }
    if at >= len {
        return None;
    }
    let run = class_at(text, at, words);
    while at < len && class_at(text, at, words) == run {
        at = text.next_cluster_byte(at);
    }
    Some(position(text, at))
}

/// Just past the end of the previous word. Vim's `ge` and `gE`.
///
/// `None` when there is no word behind.
pub fn word_end_back(text: &Text, from: Position, words: Words) -> Option<Position> {
    // Stated as a search for a place rather than a walk through runs. Stepping
    // off the cursor and looking for the previous non-blank lands *inside* the
    // word the cursor is already in, which is not behind it at all.
    let mut at = text.prev_cluster_byte(from.byte());
    while at > 0 {
        if is_word_end(text, at, words) {
            return Some(position(text, at));
        }
        at = text.prev_cluster_byte(at);
    }
    None
}

/// Whether `byte` is just past the end of a word: something non-blank behind it,
/// and something of a different class at it.
fn is_word_end(text: &Text, byte: usize, words: Words) -> bool {
    let behind = class_at(text, text.prev_cluster_byte(byte), words);
    behind != Class::Blank && class_at(text, byte, words) != behind
}

// -- visual lines ------------------------------------------------------------

/// Where a vertical motion over *drawn* lines aims to land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualGoal {
    /// Land in this cell, counting the row's gutter, or at the line's end if it
    /// is shorter.
    Cell(usize),
    /// Land at the end of the drawn line.
    LineEnd,
}

/// `lines` drawn lines down (or up, when negative), aiming at `goal`.
///
/// The motion a cursor key should perform in a wrapped editor: one press moves
/// one drawn line, not past the whole remainder of a soft-wrapped paragraph.
/// Expressible only because the same crate owns the cursor and the layout —
/// splitting those two across a library boundary is what made this unfixable
/// before.
pub fn visual_vertical(
    text: &Text,
    layout: &Layout,
    hints: &[RowHints<'_>],
    from: Position,
    lines: isize,
    goal: VisualGoal,
) -> Position {
    let last = layout.visual_line_count().saturating_sub(1);
    let current = layout.visual_row_of(from);
    let row = if lines >= 0 {
        current.saturating_add(lines.unsigned_abs()).min(last)
    } else {
        current.saturating_sub(lines.unsigned_abs())
    };
    let column = match goal {
        VisualGoal::Cell(column) => column,
        VisualGoal::LineEnd => usize::MAX,
    };
    layout
        .position_at_cell(text, hints, Cell { row, column })
        .unwrap_or(from)
}

/// The start of the drawn line the cursor is on.
///
/// What Home should do where rows wrap: the start of what the reader sees as this
/// line, not of the logical row it belongs to.
pub fn visual_line_start(
    text: &Text,
    layout: &Layout,
    hints: &[RowHints<'_>],
    from: Position,
) -> Position {
    let row = layout.visual_row_of(from);
    layout
        .position_at_cell(text, hints, Cell { row, column: 0 })
        .unwrap_or(from)
}

/// The end of the drawn line the cursor is on.
pub fn visual_line_end(
    text: &Text,
    layout: &Layout,
    hints: &[RowHints<'_>],
    from: Position,
) -> Position {
    let row = layout.visual_row_of(from);
    layout
        .position_at_cell(
            text,
            hints,
            Cell {
                row,
                column: usize::MAX,
            },
        )
        .unwrap_or(from)
}

// -- searching within a row --------------------------------------------------

/// The next occurrence of `needle` after `from`, on `from`'s row only.
///
/// `till` stops just before it rather than on it — vim's `t` against `f`. `None`
/// when the row holds no further occurrence, which is what makes `dt,` on a row
/// without a comma do nothing.
pub fn find_char_forward(
    text: &Text,
    from: Position,
    needle: char,
    till: bool,
) -> Option<Position> {
    let line = text.line(from.row())?;
    let row_start = text.row_start_byte(from.row());
    let from_offset = from.byte().saturating_sub(row_start);
    let mut found = None;
    for (offset, cluster) in indices(&line) {
        if offset > from_offset && cluster.starts_with(needle) {
            found = Some(offset);
            break;
        }
    }
    let offset = found?;
    let byte = row_start + offset;
    Some(position(
        text,
        if till {
            text.prev_cluster_byte(byte)
        } else {
            byte
        },
    ))
}

/// The previous occurrence of `needle` before `from`, on `from`'s row only.
///
/// `till` stops just after it. Vim's `F` and `T`.
pub fn find_char_back(text: &Text, from: Position, needle: char, till: bool) -> Option<Position> {
    let line = text.line(from.row())?;
    let row_start = text.row_start_byte(from.row());
    let from_offset = from.byte().saturating_sub(row_start);
    let mut found = None;
    for (offset, cluster) in indices(&line) {
        if offset < from_offset && cluster.starts_with(needle) {
            found = Some(offset);
        }
    }
    let offset = found?;
    let byte = row_start + offset;
    Some(position(
        text,
        if till {
            text.next_cluster_byte(byte)
        } else {
            byte
        },
    ))
}

// -- brackets ----------------------------------------------------------------

/// The bracket matching the one at or after the cursor on its row. Vim's `%`.
///
/// Scans for the first bracket at or after the cursor on the row — as vim does —
/// then walks the whole text for its partner, counting nesting. `None` when there
/// is no bracket ahead on the row, or it is unbalanced.
pub fn matching_bracket(text: &Text, from: Position) -> Option<Position> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

    let len = text.len_bytes();
    let row_end = row_end(text, from).byte();
    let mut at = from.byte();
    let (open, close, forward) = loop {
        if at >= row_end {
            return None;
        }
        let c = text.scalar_at(at)?;
        if let Some(&(open, close)) = PAIRS.iter().find(|(open, _)| *open == c) {
            break (open, close, true);
        }
        if let Some(&(open, close)) = PAIRS.iter().find(|(_, close)| *close == c) {
            break (open, close, false);
        }
        at = text.next_cluster_byte(at);
    };

    let mut depth = 0i32;
    loop {
        let c = text.scalar_at(at)?;
        if c == open {
            depth += if forward { 1 } else { -1 };
        } else if c == close {
            depth += if forward { -1 } else { 1 };
        }
        if depth == 0 {
            return Some(position(text, at));
        }
        if forward {
            at = text.next_cluster_byte(at);
            if at >= len {
                return None;
            }
        } else {
            if at == 0 {
                return None;
            }
            at = text.prev_cluster_byte(at);
        }
    }
}

// -- helpers -----------------------------------------------------------------

fn position(text: &Text, byte: usize) -> Position {
    text.position_at_derived_byte(byte)
}

fn class_at(text: &Text, byte: usize, words: Words) -> Class {
    text.scalar_at(byte)
        .map(|c| words.classify(c))
        .unwrap_or(Class::Blank)
}

fn is_blank_row(text: &Text, row: usize) -> bool {
    text.line(row)
        .map(|line| line.trim().is_empty())
        .unwrap_or(true)
}

/// Whether `byte` is the line break of an otherwise empty row, or the start of
/// one.
///
/// Vim treats an empty line as a word, so a forward or backward word motion stops
/// there instead of skipping it with the surrounding blanks.
fn is_empty_row_at(text: &Text, byte: usize) -> bool {
    let row = text.row_of_byte(byte);
    text.line_len_chars(row) == Some(0)
}

fn indices(line: &str) -> impl Iterator<Item = (usize, &str)> {
    use unicode_segmentation::UnicodeSegmentation;
    line.grapheme_indices(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "e" plus a combining acute, then "x": three scalars, two clusters.
    const COMBINING: &str = "e\u{301}x";

    fn text(s: &str) -> Text {
        Text::from(s)
    }

    fn at(text: &Text, row: usize, col: usize) -> Position {
        text.position(row, Column::new(col))
            .expect("addressable in the test fixture")
    }

    /// Where a motion landed, as `(row, column)`.
    fn rc(position: Position) -> (usize, usize) {
        (position.row(), position.column().get())
    }

    // -- stepping -----------------------------------------------------------

    #[test]
    fn stepping_within_a_row_stops_at_its_ends() {
        let t = text("ab\ncd");
        assert_eq!(rc(right(&t, at(&t, 0, 2))), (0, 2), "row end holds");
        assert_eq!(rc(left(&t, at(&t, 1, 0))), (1, 0), "row start holds");
        assert_eq!(rc(right(&t, at(&t, 0, 1))), (0, 2));
        assert_eq!(rc(left(&t, at(&t, 0, 1))), (0, 0));
    }

    #[test]
    fn stepping_across_rows_crosses_the_break() {
        let t = text("ab\ncd");
        assert_eq!(rc(next_cluster(&t, at(&t, 0, 2))), (1, 0));
        assert_eq!(rc(prev_cluster(&t, at(&t, 1, 0))), (0, 2));
    }

    #[test]
    fn stepping_at_the_ends_of_the_text_holds() {
        let t = text("ab");
        assert_eq!(rc(next_cluster(&t, at(&t, 0, 2))), (0, 2));
        assert_eq!(rc(prev_cluster(&t, at(&t, 0, 0))), (0, 0));
    }

    #[test]
    fn one_step_crosses_a_whole_cluster() {
        // Two scalars, one cluster: stepping right from the start must land past
        // the combining mark, not on it.
        let t = text(COMBINING);
        let stepped = right(&t, at(&t, 0, 0));
        assert_eq!(stepped.column().get(), 2);
        assert_eq!(rc(left(&t, stepped)), (0, 0));
    }

    // -- row bounds ---------------------------------------------------------

    #[test]
    fn row_bounds_ignore_the_line_break() {
        let t = text("one\ntwo");
        assert_eq!(rc(row_start(&t, at(&t, 0, 2))), (0, 0));
        assert_eq!(rc(row_end(&t, at(&t, 0, 1))), (0, 3));
    }

    #[test]
    fn non_blank_bounds_skip_the_padding() {
        let t = text("  padded  ");
        assert_eq!(rc(first_non_blank(&t, at(&t, 0, 0))), (0, 2));
        assert_eq!(
            rc(last_non_blank(&t, at(&t, 0, 0))),
            (0, 8),
            "just past the last non-blank, since a position sits between clusters"
        );
    }

    #[test]
    fn an_all_blank_row_has_no_non_blank_to_find() {
        let t = text("    ");
        assert_eq!(rc(first_non_blank(&t, at(&t, 0, 0))), (0, 4));
        assert_eq!(rc(last_non_blank(&t, at(&t, 0, 2))), (0, 0));
    }

    // -- vertical -----------------------------------------------------------

    #[test]
    fn a_short_row_does_not_eat_the_goal_column() {
        let t = text("long enough\nab\nlong enough");
        let start = at(&t, 0, 9);
        let goal = Goal::Column(start.column());
        let middle = vertical(&t, start, 1, goal);
        assert_eq!(rc(middle), (1, 2), "clamped to the short row");
        let bottom = vertical(&t, middle, 1, goal);
        assert_eq!(
            rc(bottom),
            (2, 9),
            "and back out to the column the caller still wants"
        );
    }

    #[test]
    fn aiming_at_the_row_end_follows_the_row() {
        let t = text("long enough\nab");
        let landed = vertical(&t, at(&t, 0, 11), 1, Goal::RowEnd);
        assert_eq!(rc(landed), (1, 2));
    }

    #[test]
    fn vertical_saturates_at_the_first_and_last_row() {
        let t = text("a\nb\nc");
        let goal = Goal::Column(Column::ZERO);
        assert_eq!(rc(vertical(&t, at(&t, 0, 0), -3, goal)), (0, 0));
        assert_eq!(rc(vertical(&t, at(&t, 0, 0), 99, goal)), (2, 0));
    }

    #[test]
    fn vertical_will_not_land_inside_a_cluster() {
        // Row 1's first two scalars are one cluster, so column 1 is not a place.
        let t = text(&format!("abc\n{COMBINING}"));
        let landed = vertical(&t, at(&t, 0, 1), 1, Goal::Column(Column::new(1)));
        assert_eq!(rc(landed), (1, 0));
    }

    // -- paragraphs ---------------------------------------------------------

    #[test]
    fn paragraph_motions_stop_on_blank_rows() {
        let t = text("one\ntwo\n\nthree\n\nfour");
        assert_eq!(rc(paragraph_forward(&t, at(&t, 0, 0))), (2, 0));
        assert_eq!(rc(paragraph_forward(&t, at(&t, 3, 0))), (4, 0));
        assert_eq!(rc(paragraph_back(&t, at(&t, 5, 0))), (4, 0));
        assert_eq!(rc(paragraph_back(&t, at(&t, 3, 0))), (2, 0));
    }

    #[test]
    fn paragraph_motions_saturate_at_the_ends() {
        let t = text("one\ntwo");
        assert_eq!(paragraph_forward(&t, at(&t, 0, 0)).byte(), t.len_bytes());
        assert_eq!(rc(paragraph_back(&t, at(&t, 1, 0))), (0, 0));
    }

    #[test]
    fn a_row_of_only_blanks_counts_as_a_paragraph_break() {
        let t = text("one\n   \ntwo");
        assert_eq!(rc(paragraph_forward(&t, at(&t, 0, 0))), (1, 0));
    }

    // -- words --------------------------------------------------------------

    #[test]
    fn a_small_word_ends_where_the_character_class_changes() {
        let t = text("foo.bar");
        assert_eq!(
            rc(word_start_forward(&t, at(&t, 0, 0), Words::Small)),
            (0, 3),
            "the punctuation run is its own word"
        );
    }

    #[test]
    fn a_big_word_is_any_run_of_non_blanks() {
        let t = text("foo.bar baz");
        assert_eq!(
            rc(word_start_forward(&t, at(&t, 0, 0), Words::Big)),
            (0, 8),
            "punctuation does not divide a WORD"
        );
    }

    #[test]
    fn a_word_motion_crosses_rows() {
        let t = text("foo\nbar");
        assert_eq!(
            rc(word_start_forward(&t, at(&t, 0, 0), Words::Small)),
            (1, 0)
        );
        assert_eq!(rc(word_start_back(&t, at(&t, 1, 0), Words::Small)), (0, 0));
    }

    #[test]
    fn an_empty_row_is_a_word_of_its_own() {
        // Vim stops on a blank line rather than skipping it with the whitespace
        // around it.
        let t = text("a\n\nb");
        assert_eq!(
            rc(word_start_forward(&t, at(&t, 0, 0), Words::Small)),
            (1, 0)
        );
        assert_eq!(rc(word_start_back(&t, at(&t, 2, 0), Words::Small)), (1, 0));
    }

    #[test]
    fn a_forward_word_motion_saturates_at_the_end_of_the_text() {
        let t = text("one two");
        let last = word_start_forward(&t, at(&t, 0, 4), Words::Small);
        assert_eq!(last.byte(), t.len_bytes());
    }

    #[test]
    fn a_backward_word_motion_finds_the_front_of_the_run_it_is_in() {
        let t = text("one two three");
        assert_eq!(rc(word_start_back(&t, at(&t, 0, 6), Words::Small)), (0, 4));
        assert_eq!(rc(word_start_back(&t, at(&t, 0, 4), Words::Small)), (0, 0));
        assert_eq!(rc(word_start_back(&t, at(&t, 0, 0), Words::Small)), (0, 0));
    }

    #[test]
    fn a_word_end_is_just_past_the_last_cluster_of_the_word() {
        let t = text("one two");
        assert_eq!(
            rc(word_end_forward(&t, at(&t, 0, 0), Words::Small).expect("a word ahead")),
            (0, 3)
        );
        assert_eq!(
            rc(word_end_forward(&t, at(&t, 0, 3), Words::Small).expect("a word ahead")),
            (0, 7)
        );
    }

    #[test]
    fn a_word_end_with_no_word_ahead_finds_nothing() {
        // Not "the end of the text": an operator waiting on this must abort, so
        // `de` at the end of a buffer deletes nothing.
        let t = text("one   ");
        assert!(word_end_forward(&t, at(&t, 0, 3), Words::Small).is_none());
        assert!(word_end_forward(&t, at(&t, 0, 6), Words::Small).is_none());
    }

    #[test]
    fn a_word_end_at_or_after_accepts_the_word_the_cursor_is_in() {
        let t = text("a bb");
        // `e` looks past the single-character word and finds the next one.
        assert_eq!(
            rc(word_end_forward(&t, at(&t, 0, 0), Words::Small).expect("a word ahead")),
            (0, 4)
        );
        // A delete-to-word-end names the word the cursor is in.
        assert_eq!(
            rc(word_end_at_or_after(&t, at(&t, 0, 0), Words::Small).expect("a word here")),
            (0, 1)
        );
    }

    #[test]
    fn a_word_end_at_or_after_skips_leading_blanks() {
        let t = text("  ab");
        assert_eq!(
            rc(word_end_at_or_after(&t, at(&t, 0, 0), Words::Small).expect("a word ahead")),
            (0, 4)
        );
    }

    #[test]
    fn a_word_end_at_or_after_crosses_rows_to_find_one() {
        let t = text("\nab");
        assert_eq!(
            rc(word_end_at_or_after(&t, at(&t, 0, 0), Words::Small).expect("a word ahead")),
            (1, 2)
        );
    }

    #[test]
    fn a_word_end_at_or_after_finds_nothing_in_blanks() {
        let t = text("a   ");
        assert!(word_end_at_or_after(&t, at(&t, 0, 2), Words::Small).is_none());
    }

    #[test]
    fn a_backward_word_end_finds_the_previous_word() {
        let t = text("one two");
        assert_eq!(
            rc(word_end_back(&t, at(&t, 0, 5), Words::Small).expect("a word behind")),
            (0, 3)
        );
        assert!(word_end_back(&t, at(&t, 0, 0), Words::Small).is_none());
    }

    #[test]
    fn word_classes_are_the_ones_the_motions_use() {
        assert_eq!(Words::Small.class_of('a'), Class::Word);
        assert_eq!(Words::Small.class_of('_'), Class::Word);
        assert_eq!(Words::Small.class_of('.'), Class::Punctuation);
        assert_eq!(Words::Big.class_of('.'), Class::Word);
        assert_eq!(Words::Small.class_of(' '), Class::Blank);
        assert_eq!(Words::Big.class_of('\n'), Class::Blank);
    }

    // -- find char ----------------------------------------------------------

    #[test]
    fn finding_a_character_lands_on_it_or_before_it() {
        let t = text("hello");
        assert_eq!(
            rc(find_char_forward(&t, at(&t, 0, 0), 'l', false).expect("found")),
            (0, 2)
        );
        assert_eq!(
            rc(find_char_forward(&t, at(&t, 0, 0), 'l', true).expect("found")),
            (0, 1),
            "till stops one short"
        );
    }

    #[test]
    fn finding_backwards_takes_the_nearest_one_behind() {
        let t = text("hello");
        assert_eq!(
            rc(find_char_back(&t, at(&t, 0, 4), 'l', false).expect("found")),
            (0, 3)
        );
    }

    #[test]
    fn finding_a_character_never_leaves_the_row() {
        let t = text("abc\nxbz");
        assert!(find_char_forward(&t, at(&t, 0, 0), 'x', false).is_none());
        assert!(find_char_back(&t, at(&t, 1, 2), 'a', false).is_none());
    }

    #[test]
    fn finding_a_character_that_is_not_there_finds_nothing() {
        let t = text("hello");
        assert!(find_char_forward(&t, at(&t, 0, 0), 'q', false).is_none());
        assert!(
            find_char_forward(&t, at(&t, 0, 4), 'h', false).is_none(),
            "the search starts past the cursor"
        );
    }

    // -- brackets -----------------------------------------------------------

    #[test]
    fn a_bracket_matches_its_partner() {
        let t = text("(a[b]c)");
        assert_eq!(
            rc(matching_bracket(&t, at(&t, 0, 0)).expect("balanced")),
            (0, 6),
            "the inner pair of a different kind is not the partner"
        );
    }

    #[test]
    fn nesting_is_counted() {
        let t = text("((x))");
        assert_eq!(
            rc(matching_bracket(&t, at(&t, 0, 0)).expect("balanced")),
            (0, 4)
        );
        assert_eq!(
            rc(matching_bracket(&t, at(&t, 0, 1)).expect("balanced")),
            (0, 3)
        );
    }

    #[test]
    fn a_closing_bracket_matches_backwards() {
        let t = text("((x))");
        assert_eq!(
            rc(matching_bracket(&t, at(&t, 0, 4)).expect("balanced")),
            (0, 0)
        );
    }

    #[test]
    fn the_search_starts_at_the_first_bracket_on_the_row() {
        // Vim's `%` from anywhere before a bracket on the row uses that bracket.
        let t = text("if cond {\n}");
        assert_eq!(
            rc(matching_bracket(&t, at(&t, 0, 0)).expect("balanced")),
            (1, 0)
        );
    }

    #[test]
    fn a_partner_may_be_on_another_row() {
        let t = text("fn a(\n  b,\n) {}");
        assert_eq!(
            rc(matching_bracket(&t, at(&t, 0, 4)).expect("balanced")),
            (2, 0)
        );
    }

    #[test]
    fn an_unbalanced_bracket_matches_nothing() {
        let opening = text("(a");
        assert!(matching_bracket(&opening, opening.start()).is_none());
        let closing = text("a)");
        assert!(matching_bracket(&closing, closing.start()).is_none());
    }

    #[test]
    fn a_row_without_a_bracket_matches_nothing() {
        let t = text("plain text\n()");
        assert!(
            matching_bracket(&t, at(&t, 0, 0)).is_none(),
            "the bracket on the next row is not this row's"
        );
    }

    #[test]
    fn a_word_motion_leaves_an_empty_row_it_starts_on() {
        // The empty-row stop must not apply to the row the cursor is already on,
        // or the motion tells the cursor to stay where it is.
        let t = text("a\n\nb");
        assert_eq!(
            rc(word_start_forward(&t, at(&t, 1, 0), Words::Small)),
            (2, 0)
        );
    }

    #[test]
    fn a_backward_word_end_crosses_a_punctuation_run() {
        let t = text("foo.bar");
        assert_eq!(
            rc(word_end_back(&t, at(&t, 0, 5), Words::Small).expect("a run behind")),
            (0, 4),
            "the punctuation run's end, not the word before it"
        );
        assert!(
            word_end_back(&t, at(&t, 0, 5), Words::Big).is_none(),
            "to a WORD, foo.bar is one run, so no word ends behind the cursor"
        );
    }

    // -- visual lines -------------------------------------------------------

    mod visual {
        use super::*;
        use crate::layout::{Layout, RowHints, Viewport};
        use crate::width::Metrics;

        fn layout(text: &Text, width: usize) -> Layout {
            Layout::compute(text, width, Metrics::default(), &[])
        }

        #[test]
        fn down_moves_one_drawn_line_not_one_row() {
            // The whole point. "aaaa bbbb cccc" wraps into three drawn lines at
            // width 5, so pressing down once must stay inside the logical row
            // instead of skipping the rest of the paragraph.
            let t = text("aaaa bbbb cccc\nnext");
            let l = layout(&t, 5);
            let start = at(&t, 0, 0);
            let one = visual_vertical(&t, &l, &[], start, 1, VisualGoal::Cell(0));
            assert_eq!(rc(one), (0, 5), "the second drawn line of the same row");
            let two = visual_vertical(&t, &l, &[], one, 1, VisualGoal::Cell(0));
            assert_eq!(rc(two), (0, 10));
            let three = visual_vertical(&t, &l, &[], two, 1, VisualGoal::Cell(0));
            assert_eq!(rc(three), (1, 0), "and only now the next row");
        }

        #[test]
        fn up_and_down_are_symmetric() {
            let t = text("aaaa bbbb cccc");
            let l = layout(&t, 5);
            let middle = at(&t, 0, 5);
            assert_eq!(
                rc(visual_vertical(
                    &t,
                    &l,
                    &[],
                    middle,
                    -1,
                    VisualGoal::Cell(0)
                )),
                (0, 0)
            );
            assert_eq!(
                rc(visual_vertical(&t, &l, &[], middle, 1, VisualGoal::Cell(0))),
                (0, 10)
            );
        }

        #[test]
        fn the_goal_cell_is_kept_across_a_short_drawn_line() {
            let t = text("aaaaa\nb\nccccc");
            let l = layout(&t, 10);
            let start = at(&t, 0, 4);
            let goal = VisualGoal::Cell(4);
            let middle = visual_vertical(&t, &l, &[], start, 1, goal);
            assert_eq!(rc(middle), (1, 1), "clamped to the short row");
            let bottom = visual_vertical(&t, &l, &[], middle, 1, goal);
            assert_eq!(rc(bottom), (2, 4), "and back out to the cell still wanted");
        }

        #[test]
        fn aiming_at_the_line_end_follows_the_wrap() {
            let t = text("aaaa bbbb\nxy");
            let l = layout(&t, 5);
            let landed = visual_vertical(&t, &l, &[], at(&t, 0, 0), 1, VisualGoal::LineEnd);
            assert_eq!(rc(landed), (0, 9), "the end of the second drawn line");
        }

        #[test]
        fn vertical_movement_saturates_at_the_first_and_last_drawn_line() {
            let t = text("aaaa bbbb");
            let l = layout(&t, 5);
            let goal = VisualGoal::Cell(0);
            assert_eq!(
                rc(visual_vertical(&t, &l, &[], at(&t, 0, 0), -9, goal)),
                (0, 0)
            );
            assert_eq!(
                rc(visual_vertical(&t, &l, &[], at(&t, 0, 0), 9, goal)),
                (0, 5)
            );
        }

        #[test]
        fn line_bounds_are_the_drawn_line_not_the_row() {
            let t = text("aaaa bbbb cccc");
            let l = layout(&t, 5);
            let inside = at(&t, 0, 7);
            assert_eq!(rc(visual_line_start(&t, &l, &[], inside)), (0, 5));
            assert_eq!(rc(visual_line_end(&t, &l, &[], inside)), (0, 9));
            // The logical row's bounds are elsewhere, and both are wanted: Home in a
            // wrapped editor means the drawn line, `0` in vim means the row.
            assert_eq!(rc(row_start(&t, inside)), (0, 0));
            assert_eq!(rc(row_end(&t, inside)), (0, 14));
        }

        #[test]
        fn a_gutter_shifts_the_cells_a_visual_motion_aims_at() {
            let t = text("abcd\nefgh");
            let hints = [
                RowHints {
                    visible: &[],
                    inset: 2,
                },
                RowHints {
                    visible: &[],
                    inset: 2,
                },
            ];
            let l = Layout::compute(&t, 10, Metrics::default(), &hints);
            let landed = visual_vertical(&t, &l, &hints, at(&t, 0, 0), 1, VisualGoal::Cell(3));
            assert_eq!(
                rc(landed),
                (1, 1),
                "cell 3 is the second character past a two-cell gutter"
            );
        }

        #[test]
        fn a_viewport_can_follow_a_visual_motion() {
            let t = text("aaaa bbbb cccc dddd");
            let l = layout(&t, 5);
            let mut view = Viewport::new(2);
            let mut position = at(&t, 0, 0);
            for _ in 0..3 {
                position = visual_vertical(&t, &l, &[], position, 1, VisualGoal::Cell(0));
                view.follow(&l, position);
            }
            assert_eq!(view.top(), 2, "scrolled by drawn lines, not by rows");
        }
    }

    // -- properties ---------------------------------------------------------

    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn addressable(t: &Text) -> Vec<Position> {
            (0..=t.len_bytes())
                .filter_map(|byte| t.position_at_byte(byte))
                .collect()
        }

        /// Every motion, from every addressable place.
        fn all(t: &Text, from: Position) -> Vec<Position> {
            let mut landed = vec![
                right(t, from),
                left(t, from),
                next_cluster(t, from),
                prev_cluster(t, from),
                row_start(t, from),
                row_end(t, from),
                first_non_blank(t, from),
                last_non_blank(t, from),
                paragraph_forward(t, from),
                paragraph_back(t, from),
                goto_row(t, from.row()),
                vertical(t, from, 1, Goal::Column(from.column())),
                vertical(t, from, -1, Goal::RowEnd),
            ];
            for words in [Words::Small, Words::Big] {
                landed.push(word_start_forward(t, from, words));
                landed.push(word_start_back(t, from, words));
                landed.extend(word_end_forward(t, from, words));
                landed.extend(word_end_at_or_after(t, from, words));
                landed.extend(word_end_back(t, from, words));
            }
            landed.extend(matching_bracket(t, from));
            landed.extend(find_char_forward(t, from, 'a', false));
            landed.extend(find_char_forward(t, from, 'a', true));
            landed.extend(find_char_back(t, from, 'a', false));
            landed.extend(find_char_back(t, from, 'a', true));
            landed
        }

        proptest! {
            // Every case walks every motion from every boundary, so the case count
            // is kept low deliberately: the cost is quadratic in the corpus and
            // the interesting shapes are small.
            #![proptest_config(ProptestConfig::with_cases(64))]

            /// No motion can produce a position the text cannot address. This is
            /// what stops `position_at_derived_byte`'s assertion from being the
            /// thing that finds it, in a user's note.
            #[test]
            fn every_motion_lands_somewhere_addressable(s in ".{0,40}") {
                let t = Text::from(s.as_str());
                for from in addressable(&t) {
                    for landed in all(&t, from) {
                        prop_assert!(
                            t.position_at_byte(landed.byte()).is_some(),
                            "byte {} of {:?} is not addressable", landed.byte(), s
                        );
                        prop_assert!(!t.is_stale(landed));
                        prop_assert_eq!(
                            t.position_at_byte(landed.byte()),
                            Some(landed),
                            "row and column disagree with the byte"
                        );
                    }
                }
            }

            /// Forward motions never go backwards, and backward ones never
            /// forwards. A motion that overshoots into the other direction is how
            /// an operator range comes out inverted.
            #[test]
            fn motions_keep_their_direction(s in ".{0,40}") {
                let t = Text::from(s.as_str());
                for from in addressable(&t) {
                    for words in [Words::Small, Words::Big] {
                        prop_assert!(word_start_forward(&t, from, words).byte() >= from.byte());
                        prop_assert!(word_start_back(&t, from, words).byte() <= from.byte());
                        if let Some(end) = word_end_forward(&t, from, words) {
                            prop_assert!(end.byte() > from.byte());
                        }
                        if let Some(end) = word_end_at_or_after(&t, from, words) {
                            prop_assert!(end.byte() > from.byte());
                        }
                        if let Some(end) = word_end_back(&t, from, words) {
                            prop_assert!(end.byte() < from.byte());
                        }
                    }
                    prop_assert!(right(&t, from).byte() >= from.byte());
                    prop_assert!(left(&t, from).byte() <= from.byte());
                    prop_assert!(next_cluster(&t, from).byte() >= from.byte());
                    prop_assert!(prev_cluster(&t, from).byte() <= from.byte());
                    prop_assert!(paragraph_forward(&t, from).byte() >= from.byte());
                    prop_assert!(paragraph_back(&t, from).byte() <= from.byte());
                }
            }

            /// Walking by words reaches the end of the text and stops there. A
            /// motion that returns where it started is a motion the caller can
            /// loop on forever.
            #[test]
            fn walking_by_words_terminates(s in ".{0,60}") {
                let t = Text::from(s.as_str());
                for words in [Words::Small, Words::Big] {
                    let mut at = t.start();
                    let mut steps = 0;
                    loop {
                        let next = word_start_forward(&t, at, words);
                        if next.byte() == at.byte() {
                            break;
                        }
                        at = next;
                        steps += 1;
                        prop_assert!(steps <= t.len_bytes() + 2, "no progress in {:?}", s);
                    }
                    prop_assert_eq!(at.byte(), t.len_bytes(), "stopped short in {:?}", s);
                }
            }
        }
    }
}
