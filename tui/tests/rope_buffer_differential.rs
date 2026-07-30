//! The rope-backed **edit buffer** against the one it replaces.
//!
//! `RopeBuffer` keeps the surface `vim.rs`, `find_bar.rs` and the component
//! already call, so the swap can happen without rewriting them (adr/0039). This
//! holds it to the incumbent's behaviour operation by operation, before anything
//! is rewired — so a disagreement is found here rather than as a misplaced
//! keystroke in the editor.
//!
//! Where the two are *meant* to differ, the case is asserted against the intended
//! answer directly instead of against `EditBuffer`, and says why:
//!
//! - A delete that removes nothing records no history (adr/0039 — the incumbent
//!   pushes an entry that removed nothing, so the next undo takes back an
//!   unrelated edit).
//! - Vertical movement keeps its goal column (change #8).
//! - `Jump` refuses a position it cannot address rather than clamping it
//!   (adr/0038).
//! - Pasting an empty register does nothing, rather than deleting the selection
//!   and inserting nothing — adr/0038's first surprise, which cost a note once
//!   already.
//! - Undo returns the cursor to where the action was taken from, not to a
//!   position the implementation moved it to on the way. The incumbent records
//!   the latter in at least two paths — `delete_range` stores the far end of a
//!   deleted range, and `delete_next_word` moves to the next row before joining —
//!   so its cursors after undo are not compared here. Ours are pinned by test.
//! - An operation that changes nothing leaves the cursor alone, **and leaves the
//!   history alone**. The incumbent can lose entries: its no-op sweep undoes until
//!   the content matches what the operation started from, and when an *earlier*
//!   state happens to match, the entries in between are consumed. Typing `a`,
//!   deleting it, and then pressing a key that does nothing leaves nothing to
//!   undo. A no-op delete also *fills the register* with what it did not remove.
//!   Undo, redo and paste are therefore not compared after a no-op. The incumbent
//!   moves it: `EditBuffer::edit` detects a no-op by comparing content, and when
//!   the content is unchanged it undoes to sweep up orphaned history entries and
//!   redoes if that overshot — and the redo restores the text while setting the
//!   cursor to *that earlier edit's* position. Cursors are therefore compared only
//!   after an operation that actually changed the text.

use kimun_notes::components::text_editor::edit_buffer::EditBuffer;
use kimun_notes::components::text_editor::rope_buffer::{CursorMove as RopeMove, RopeBuffer};
use proptest::prelude::*;
use proptest::strategy::Union;
use ratatui_textarea::{CursorMove, DataCursor, TextArea};
use ropetext::Text;

/// An operation both buffers define.
#[derive(Debug, Clone)]
enum Op {
    Jump(usize, usize),
    Move(Movement),
    StartSelection,
    CancelSelection,
    SelectAll,
    Insert(String),
    InsertChar(char),
    InsertNewline,
    DeleteChar,
    DeleteNextChar,
    DeleteStr(usize),
    DeleteWord,
    DeleteNextWord,
    Cut,
    Paste,
    Undo,
    Redo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Movement {
    Forward,
    Back,
    Head,
    End,
    Top,
    Bottom,
    WordForward,
    WordBack,
    WordEnd,
}

impl Movement {
    fn incumbent(self) -> CursorMove {
        match self {
            Movement::Forward => CursorMove::Forward,
            Movement::Back => CursorMove::Back,
            Movement::Head => CursorMove::Head,
            Movement::End => CursorMove::End,
            Movement::Top => CursorMove::Top,
            Movement::Bottom => CursorMove::Bottom,
            Movement::WordForward => CursorMove::WordForward,
            Movement::WordBack => CursorMove::WordBack,
            Movement::WordEnd => CursorMove::WordEnd,
        }
    }

    fn rope(self) -> RopeMove {
        match self {
            Movement::Forward => RopeMove::Forward,
            Movement::Back => RopeMove::Back,
            Movement::Head => RopeMove::Head,
            Movement::End => RopeMove::End,
            Movement::Top => RopeMove::Top,
            Movement::Bottom => RopeMove::Bottom,
            Movement::WordForward => RopeMove::WordForward,
            Movement::WordBack => RopeMove::WordBack,
            Movement::WordEnd => RopeMove::WordEnd,
        }
    }
}

/// `words` includes the word motions, which agree with the incumbent within a row
/// but deliberately differ across rows — see
/// `a_word_motion_crossing_rows_lands_on_the_word`.
fn movement(words: bool) -> impl Strategy<Value = Movement> {
    let mut pool = vec![
        Movement::Forward,
        Movement::Back,
        Movement::Head,
        Movement::End,
        Movement::Top,
        Movement::Bottom,
    ];
    if words {
        pool.extend([Movement::WordForward, Movement::WordBack, Movement::WordEnd]);
    }
    prop::sample::select(pool)
}

fn op(words: bool, newlines: bool) -> BoxedStrategy<Op> {
    // Built as a weighted union rather than `prop_oneof!` because the branches are
    // conditional, and a zero-weight branch is a panic rather than an omission.
    let mut choices: Vec<(u32, BoxedStrategy<Op>)> = vec![
        (
            6,
            (0usize..5, 0usize..10)
                .prop_map(|(r, c)| Op::Jump(r, c))
                .boxed(),
        ),
        (6, movement(words).prop_map(Op::Move).boxed()),
        (2, Just(Op::StartSelection).boxed()),
        (1, Just(Op::CancelSelection).boxed()),
        (1, Just(Op::SelectAll).boxed()),
        (4, "[a-z ]{0,4}".prop_map(Op::Insert).boxed()),
        (
            3,
            prop_oneof![Just('a'), Just('z'), Just(' ')]
                .prop_map(Op::InsertChar)
                .boxed(),
        ),
        (3, Just(Op::DeleteChar).boxed()),
        (3, Just(Op::DeleteNextChar).boxed()),
        (2, (0usize..4).prop_map(Op::DeleteStr).boxed()),
        (2, Just(Op::DeleteWord).boxed()),
        (2, Just(Op::DeleteNextWord).boxed()),
        (2, Just(Op::Cut).boxed()),
        (2, Just(Op::Paste).boxed()),
        (3, Just(Op::Undo).boxed()),
        (2, Just(Op::Redo).boxed()),
    ];
    if newlines {
        choices.push((2, Just(Op::InsertNewline).boxed()));
    }
    Union::new_weighted(choices).boxed()
}

struct Pair {
    rope: RopeBuffer,
    incumbent: EditBuffer,
    /// Cleared once a no-op operation has run against the incumbent. Such an
    /// operation can eat history entries *and* fill the register with what it
    /// pretended to delete, so neither is comparable afterwards — see the module
    /// docs.
    oracle_trustworthy: bool,
}

impl Pair {
    fn new(initial: &str) -> Self {
        let lines: Vec<String> = initial.split('\n').map(str::to_string).collect();
        let mut textarea = TextArea::new(lines);
        textarea.set_max_histories(10_000);
        Self {
            rope: RopeBuffer::new(Text::from(initial)),
            incumbent: EditBuffer::new(textarea),
            oracle_trustworthy: true,
        }
    }

    fn incumbent_cursor(&self) -> (usize, usize) {
        let DataCursor(row, col) = self.incumbent.cursor();
        (row, col)
    }

    /// Characters from the cursor to the end of the text, a line break counting as
    /// one — the unit `delete_str` takes.
    fn remaining(&self) -> usize {
        let (row, col) = self.rope.cursor();
        let lines = self.rope.lines();
        let mut left = lines[row].chars().count().saturating_sub(col);
        for line in &lines[row + 1..] {
            left += 1 + line.chars().count();
        }
        left
    }

    /// Clamp a generated position onto somewhere the *rope* buffer can address,
    /// so a refused jump is not mistaken for a disagreement. The incumbent
    /// clamps rather than refusing, which is change #6 and asserted separately.
    fn resolve(&self, row: usize, col: usize) -> (usize, usize) {
        let rows = self.rope.lines().len();
        let row = row % rows.max(1);
        let len = self.rope.lines()[row].chars().count();
        (row, col.min(len))
    }

    /// Returns whether the cursor is worth comparing afterwards — see the module
    /// docs on no-op cursor drift.
    fn apply(&mut self, op: &Op) -> bool {
        let before: Vec<String> = self.rope.lines().to_vec();
        self.dispatch(op);
        if matches!(op, Op::Undo | Op::Redo) {
            // The incumbent's recorded cursors are unreliable — see the module
            // docs. Text still has to agree, so the incumbent is put back on our
            // cursor: left to drift, a documented cursor difference turns the next
            // insert into a *text* difference and the rest of the sequence stops
            // meaning anything.
            let (row, col) = self.rope.cursor();
            self.incumbent.jump_to(row, col);
            return false;
        }
        let mutating = !matches!(
            op,
            Op::Jump(..) | Op::Move(..) | Op::StartSelection | Op::CancelSelection | Op::SelectAll
        );
        if mutating && self.rope.lines() == before.as_slice() {
            // Nothing changed, so the incumbent has just drifted its cursor (see
            // the module docs). Put it back on ours: left alone, it makes the
            // *next* operation act somewhere else and the divergence resurfaces as
            // a text difference several steps later, pointing at the wrong op.
            let (row, col) = self.rope.cursor();
            self.incumbent.jump_to(row, col);
            self.oracle_trustworthy = false;
            return false;
        }
        true
    }

    fn dispatch(&mut self, op: &Op) {
        match op {
            Op::Jump(row, col) => {
                let (row, col) = self.resolve(*row, *col);
                self.rope.jump_to(row, col);
                self.incumbent.jump_to(row, col);
            }
            Op::Move(movement) => {
                self.rope.move_cursor(movement.rope());
                self.incumbent.move_cursor(movement.incumbent());
            }
            Op::StartSelection => {
                self.rope.start_selection();
                self.incumbent.start_selection();
            }
            Op::CancelSelection => {
                self.rope.cancel_selection();
                self.incumbent.cancel_selection();
            }
            Op::SelectAll => {
                self.rope.select_all();
                self.incumbent.select_all();
            }
            Op::Insert(text) => {
                self.rope.insert_str(text);
                self.incumbent.insert_str(text);
            }
            Op::InsertChar(c) => {
                self.rope.insert_char(*c);
                self.incumbent.insert_char(*c);
            }
            Op::InsertNewline => {
                self.rope.insert_newline();
                self.incumbent.insert_newline();
            }
            Op::DeleteChar => {
                self.rope.delete_char();
                self.incumbent.delete_char();
            }
            Op::DeleteNextChar => {
                self.rope.delete_next_char();
                self.incumbent.delete_next_char();
            }
            Op::DeleteStr(chars) => {
                // Clamped to what is actually there. Asking the incumbent for more
                // sends it down the branch that walks to a row past the end of the
                // buffer and yanks a chunk for text it never removed — the same
                // phantom-row path adr/0039 records. Comparing there would compare
                // that bug, not this code.
                let chars = (*chars).min(self.remaining());
                if chars == 0 {
                    return;
                }
                self.rope.delete_str(chars);
                self.incumbent.delete_str(chars);
            }
            Op::DeleteWord => {
                self.rope.delete_word();
                self.incumbent.delete_word();
            }
            Op::DeleteNextWord => {
                self.rope.delete_next_word();
                self.incumbent.delete_next_word();
            }
            Op::Cut => {
                self.rope.cut();
                self.incumbent.cut();
            }
            Op::Paste => {
                if !self.oracle_trustworthy {
                    // A no-op delete filled the incumbent's register with the text
                    // it did not remove, so what it pastes is no longer comparable.
                    return;
                }
                if self.rope.yank_text().is_empty() {
                    // Skipped, not compared: the incumbent deletes the selection
                    // and inserts nothing. Pinned by
                    // `pasting_an_empty_register_does_nothing`.
                    return;
                }
                self.rope.paste();
                self.incumbent.paste();
            }
            Op::Undo | Op::Redo => {
                if !self.oracle_trustworthy {
                    // The incumbent may have lost entries to a no-op sweep, so
                    // this is skipped on both sides rather than compared against a
                    // history that no longer describes what happened.
                    return;
                }
                if matches!(op, Op::Undo) {
                    self.rope.undo();
                    self.incumbent.undo();
                } else {
                    self.rope.redo();
                    self.incumbent.redo();
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Within one row, everything agrees — including the word motions.
    #[test]
    fn single_row_operations_agree_with_the_incumbent(
        initial in "[a-z ]{0,30}",
        ops in prop::collection::vec(op(true, false), 1..16),
    ) {
        let mut pair = Pair::new(&initial);
        for (step, op) in ops.iter().enumerate() {
            let compare_cursor = pair.apply(op);
            prop_assert_eq!(
                pair.rope.lines(),
                pair.incumbent.lines(),
                "text diverged at step {} ({:?}) from {:?}", step, op, initial
            );
            if compare_cursor {
                prop_assert_eq!(
                    pair.rope.cursor(),
                    pair.incumbent_cursor(),
                    "cursor diverged at step {} ({:?}) from {:?}", step, op, initial
                );
                prop_assert_eq!(
                    pair.rope.selection_range(),
                    pair.incumbent.selection_range(),
                    "selection diverged at step {} ({:?}) from {:?}", step, op, initial
                );
            }
        }
    }

    /// Across rows, everything agrees except the word motions, whose cross-row
    /// behaviour is deliberately vim's rather than the incumbent's.
    #[test]
    fn multi_row_operations_agree_with_the_incumbent(
        initial in "[a-z \n]{0,30}",
        ops in prop::collection::vec(op(false, true), 1..16),
    ) {
        let mut pair = Pair::new(&initial);
        for (step, op) in ops.iter().enumerate() {
            let compare_cursor = pair.apply(op);
            prop_assert_eq!(
                pair.rope.lines(),
                pair.incumbent.lines(),
                "text diverged at step {} ({:?}) from {:?}", step, op, initial
            );
            if compare_cursor {
                prop_assert_eq!(
                    pair.rope.cursor(),
                    pair.incumbent_cursor(),
                    "cursor diverged at step {} ({:?}) from {:?}", step, op, initial
                );
                prop_assert_eq!(
                    pair.rope.selection_range(),
                    pair.incumbent.selection_range(),
                    "selection diverged at step {} ({:?}) from {:?}", step, op, initial
                );
            }
        }
    }
}

// -- where the two are meant to differ ---------------------------------------

#[test]
fn pasting_an_empty_register_does_nothing() {
    let mut rope = RopeBuffer::new(Text::from("keep me"));
    rope.select_all();
    assert!(!rope.paste(), "there is nothing to paste");
    assert_eq!(
        rope.lines(),
        ["keep me"],
        "an empty register must not eat the selection"
    );
}

#[test]
fn undo_returns_the_cursor_to_where_the_action_was_taken() {
    // Joining two rows with a forward word delete: the cursor was at the end of
    // the first row, so that is where undo puts it back. The incumbent moves the
    // cursor to the *next* row before deleting the break and records that, so its
    // undo lands somewhere the cursor never was.
    let mut rope = RopeBuffer::new(Text::from("ab\ncd"));
    rope.jump_to(0, 2);
    assert!(rope.delete_next_word(), "joins the rows");
    assert_eq!(rope.lines(), ["abcd"]);
    assert!(rope.undo());
    assert_eq!(rope.lines(), ["ab", "cd"]);
    assert_eq!(rope.cursor(), (0, 2), "where the key was pressed");
}

#[test]
fn a_word_motion_crossing_rows_lands_on_the_word() {
    // The incumbent's `w` steps to the *start* of the next row, so it stops on
    // leading whitespace; vim's lands on the word, which is what this does. The
    // incumbent's own answer here is (1, 0).
    let mut rope = RopeBuffer::new(Text::from("abc\n   def"));
    rope.move_cursor(RopeMove::WordForward);
    assert_eq!(rope.cursor(), (1, 3));
}

#[test]
fn a_backward_word_motion_crossing_rows_lands_on_the_word() {
    let mut rope = RopeBuffer::new(Text::from("abc def\nghi"));
    rope.jump_to(1, 0);
    rope.move_cursor(RopeMove::WordBack);
    assert_eq!(
        rope.cursor(),
        (0, 4),
        "the start of `def`, not the row's end"
    );
}

#[test]
fn a_delete_that_removes_nothing_records_no_history() {
    // The incumbent's `delete_str` at the end of a buffer pushes an entry that
    // removed nothing, so the next undo takes back an unrelated edit.
    let mut rope = RopeBuffer::new(Text::from("hello"));
    rope.jump_to(0, 0);
    rope.insert_str("x");
    rope.jump_to(0, 6);
    assert!(!rope.delete_str(1), "nothing to delete");
    assert!(rope.undo(), "the insert is still the newest entry");
    assert_eq!(rope.lines(), ["hello"]);
    assert!(!rope.undo(), "and it was the only one");
}

#[test]
fn an_operation_that_changes_nothing_leaves_the_cursor_alone() {
    // The incumbent moves it. `EditBuffer::edit` treats "content unchanged" as
    // "there may be orphaned history to sweep up", undoes, finds it overshot, and
    // redoes — and the redo carries that older edit's cursor. Pressing Ctrl+Delete
    // at the end of a line therefore teleports the cursor to wherever the previous
    // edit finished.
    let mut rope = RopeBuffer::new(Text::from("a "));
    rope.delete_next_char();
    rope.jump_to(0, 1);
    assert!(!rope.delete_next_word(), "nothing ahead to delete");
    assert_eq!(rope.cursor(), (0, 1), "so nothing moved");
}

#[test]
fn vertical_movement_keeps_its_goal_column() {
    let mut rope = RopeBuffer::new(Text::from("long enough\nab\nlong enough"));
    rope.jump_to(0, 9);
    rope.move_cursor(RopeMove::Down);
    assert_eq!(rope.cursor(), (1, 2), "clamped to the short row");
    rope.move_cursor(RopeMove::Down);
    assert_eq!(
        rope.cursor(),
        (2, 9),
        "and back out to the column it started in"
    );
}

#[test]
fn a_non_vertical_move_forgets_the_goal_column() {
    let mut rope = RopeBuffer::new(Text::from("long enough\nab\nlong enough"));
    rope.jump_to(0, 9);
    rope.move_cursor(RopeMove::Down);
    rope.move_cursor(RopeMove::Head);
    rope.move_cursor(RopeMove::Down);
    assert_eq!(
        rope.cursor(),
        (2, 0),
        "the goal was the last horizontal move"
    );
}

#[test]
fn a_jump_the_buffer_cannot_address_is_refused() {
    let mut rope = RopeBuffer::new(Text::from("hello"));
    assert!(!rope.jump_to(9, 0), "no such row");
    assert_eq!(rope.cursor(), (0, 0), "and nothing moved");
    assert!(!rope.jump_to(0, 99), "no such column");
    assert_eq!(rope.cursor(), (0, 0));
}

#[test]
fn a_move_past_the_end_of_the_text_is_refused_not_clamped() {
    let mut rope = RopeBuffer::new(Text::from("hello"));
    rope.move_cursor(RopeMove::Jump(0, 99));
    assert_eq!(rope.cursor(), (0, 0), "a refusal moves nothing");
}

// -- groups ------------------------------------------------------------------

#[test]
fn a_compound_edit_is_one_undo() {
    // What `edit()` is for: several primitives, one user action. The incumbent
    // needs hash-based grouping to achieve this; here it is the transaction.
    let mut rope = RopeBuffer::new(Text::from("word"));
    rope.edit(|buf| {
        buf.jump_to(0, 4);
        buf.insert_str("]");
        buf.jump_to(0, 0);
        buf.insert_str("[");
    });
    assert_eq!(rope.lines(), ["[word]"]);
    assert!(rope.undo());
    assert_eq!(rope.lines(), ["word"], "one action, one undo");
    assert!(!rope.undo());
}

#[test]
fn nested_groups_belong_to_the_outermost() {
    let mut rope = RopeBuffer::new(Text::from(""));
    rope.edit(|buf| {
        buf.insert_str("a");
        buf.edit(|inner| {
            inner.insert_str("b");
            inner.insert_str("c");
        });
    });
    assert_eq!(rope.lines(), ["abc"]);
    assert!(rope.undo());
    assert_eq!(rope.lines(), [""]);
}

#[test]
fn an_edit_reports_its_outcome_once() {
    let mut rope = RopeBuffer::new(Text::from("hello"));
    rope.jump_to(0, 5);
    rope.insert_str("!");
    let outcome = rope.take_outcome();
    assert!(outcome.changed);
    assert!(
        !rope.take_outcome().changed,
        "draining leaves nothing behind"
    );
}

#[test]
fn a_multi_row_edit_reports_bulk_damage() {
    let mut rope = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    rope.select_all();
    rope.insert_str("flat");
    let outcome = rope.take_outcome();
    assert!(outcome.changed);
    assert!(
        outcome.bulk,
        "the damage is not confined to the cursor's row"
    );
}

// -- search ------------------------------------------------------------------

#[test]
fn search_steps_forward_and_wraps() {
    let mut rope = RopeBuffer::new(Text::from("one two\nthree two"));
    rope.set_search_pattern("two").expect("valid pattern");
    assert!(rope.search_forward(false));
    assert_eq!(rope.cursor(), (0, 4));
    assert!(rope.search_forward(false));
    assert_eq!(rope.cursor(), (1, 6));
    assert!(rope.search_forward(false), "wraps to the first match");
    assert_eq!(rope.cursor(), (0, 4));
}

#[test]
fn search_never_extends_a_selection() {
    // adr/0038's first invariant, now structural: `set_cursor` drops the anchor
    // and only `extend_to` keeps it.
    let mut rope = RopeBuffer::new(Text::from("one two three"));
    rope.set_search_pattern("t").expect("valid pattern");
    rope.jump_to(0, 0);
    rope.start_selection();
    assert!(rope.search_forward(false));
    assert!(
        rope.selection_range().is_none(),
        "a search is not a selection gesture"
    );
}

#[test]
fn a_match_at_the_cursor_is_reported_only_when_it_starts_there() {
    let mut rope = RopeBuffer::new(Text::from("one two"));
    rope.set_search_pattern("two").expect("valid pattern");
    rope.jump_to(0, 4);
    assert_eq!(rope.match_at_cursor(), Some(((0, 4), (0, 7))));
    rope.jump_to(0, 5);
    assert_eq!(rope.match_at_cursor(), None, "it starts one column back");
}

#[test]
fn an_uncompilable_pattern_is_reported_rather_than_searched_for() {
    let mut rope = RopeBuffer::new(Text::from("text"));
    assert!(rope.set_search_pattern("(unclosed").is_err());
}

// -- selection and clipboard -------------------------------------------------

#[test]
fn cutting_takes_the_selection_and_pasting_puts_it_back() {
    let mut rope = RopeBuffer::new(Text::from("hello world"));
    assert!(rope.set_selection((0, 0), (0, 6)));
    assert!(rope.cut());
    assert_eq!(rope.lines(), ["world"]);
    assert_eq!(rope.yank_text(), "hello ");
    rope.move_cursor(RopeMove::End);
    assert!(rope.paste());
    assert_eq!(rope.lines(), ["worldhello "]);
}

#[test]
fn copying_leaves_the_text_alone() {
    let mut rope = RopeBuffer::new(Text::from("hello"));
    assert!(rope.set_selection((0, 0), (0, 5)));
    rope.copy();
    assert_eq!(rope.yank_text(), "hello");
    assert_eq!(rope.lines(), ["hello"]);
}

#[test]
fn typing_over_a_selection_replaces_it_and_leaves_nothing_selected() {
    let mut rope = RopeBuffer::new(Text::from("hello world"));
    assert!(rope.set_selection((0, 0), (0, 5)));
    rope.insert_str("bye");
    assert_eq!(rope.lines(), ["bye world"]);
    assert!(rope.selection_range().is_none());
}

#[test]
fn a_directional_move_extends_a_live_selection() {
    // Kept deliberately: this is how vim's Visual mode extends.
    let mut rope = RopeBuffer::new(Text::from("hello"));
    rope.jump_to(0, 1);
    rope.start_selection();
    rope.move_cursor(RopeMove::Forward);
    rope.move_cursor(RopeMove::Forward);
    assert_eq!(rope.selection_range(), Some(((0, 1), (0, 3))));
}

// -- tabs --------------------------------------------------------------------

#[test]
fn a_soft_tab_fills_to_the_next_stop() {
    let mut rope = RopeBuffer::new(Text::from("ab"));
    rope.set_tab_length(4);
    rope.move_cursor(RopeMove::End);
    assert!(rope.insert_tab());
    assert_eq!(rope.lines(), ["ab  "], "two spaces reach column four");
}

#[test]
fn a_hard_tab_inserts_a_tab() {
    let mut rope = RopeBuffer::new(Text::from("ab"));
    rope.set_hard_tab_indent(true);
    rope.move_cursor(RopeMove::End);
    assert!(rope.insert_tab());
    assert_eq!(rope.lines(), ["ab\t"]);
}
