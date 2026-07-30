//! Differential test: this crate against the implementation it replaces.
//!
//! `ratatui-textarea` is the incumbent. Where the two define the same operation
//! they must agree on the resulting text and cursor, including across undo and
//! redo. This is the substitute for a staged migration: the replacement is
//! written whole, so something other than the replacement has to say whether it
//! behaves.
//!
//! Deliberate limits, each because comparing there would compare the *intended*
//! differences rather than find unintended ones:
//!
//! - **ASCII only.** Grapheme handling is where the two are meant to differ —
//!   this crate refuses a position inside a cluster, the incumbent does not — and
//!   that behaviour has its own properties in `src/text.rs`.
//! - **One transaction per operation.** Coalescing is a deliberate difference:
//!   the incumbent records one history entry per character and cannot express a
//!   group at all.
//! - **No selections.** The incumbent's `insert_str` deletes a live selection as
//!   a side effect; this crate makes a caller say so.
//! - **Cursor is not compared across undo/redo.** `delete_range` records the far
//!   end of the deleted range as the cursor the edit began from, though it had
//!   just set the live cursor to the near end — so the incumbent's undo of a
//!   forward delete puts the cursor somewhere it never was. This crate restores
//!   the cursor the group actually started from; `src/buffer.rs` asserts it.
//!
//! When this crate's behaviour is the intended one and the incumbent's is not,
//! the disagreement belongs in a `src/` test that states the intent — not here.

use proptest::prelude::*;
use ratatui_textarea::{CursorMove, DataCursor, TextArea};
use ropetext::{Column, EditBuffer, Text};

/// An operation both implementations define.
#[derive(Debug, Clone)]
enum Op {
    /// Put the cursor at `(row, col)` and insert.
    Insert {
        row: usize,
        col: usize,
        text: String,
    },
    /// Put the cursor at `(row, col)` and delete `chars` characters forward,
    /// counting a line break as one.
    Delete {
        row: usize,
        col: usize,
        chars: usize,
    },
    Undo,
    Redo,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0usize..6, 0usize..12, "[a-z \n]{0,6}")
            .prop_map(|(row, col, text)| Op::Insert { row, col, text }),
        3 => (0usize..6, 0usize..12, 0usize..6)
            .prop_map(|(row, col, chars)| Op::Delete { row, col, chars }),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

/// The two implementations, kept in step.
struct Pair {
    ours: EditBuffer,
    theirs: TextArea<'static>,
}

impl Pair {
    fn new(initial: &str) -> Self {
        let lines: Vec<String> = initial.split('\n').map(str::to_string).collect();
        let mut theirs = TextArea::new(lines);
        // The incumbent's history holds 50 entries by default, which a sequence
        // long enough to be interesting would exhaust. Raising it keeps the
        // comparison about behaviour rather than about its bound — a bound this
        // crate deliberately expresses in bytes instead.
        theirs.set_max_histories(10_000);
        Self {
            ours: EditBuffer::new(Text::from(initial)),
            theirs,
        }
    }

    fn our_lines(&self) -> Vec<String> {
        self.ours
            .text()
            .lines()
            .map(|line| line.to_string())
            .collect()
    }

    fn our_cursor(&self) -> (usize, usize) {
        let cursor = self.ours.cursor();
        (cursor.row(), cursor.column().get())
    }

    fn their_cursor(&self) -> (usize, usize) {
        let DataCursor(row, col) = self.theirs.cursor();
        (row, col)
    }

    /// Clamp a generated `(row, col)` onto somewhere both texts can address.
    fn resolve(&self, row: usize, col: usize) -> (usize, usize) {
        let rows = self.ours.text().line_count();
        let row = row % rows;
        let len = self
            .ours
            .text()
            .line_len_chars(row)
            .expect("row is in range");
        (row, col.min(len))
    }

    /// Where `chars` characters forward of `(row, col)` lands, clamped to the end
    /// of the text, and how many characters that actually was. A line break
    /// counts as one, as it does for the incumbent's `delete_str`.
    ///
    /// The count matters: asking the incumbent to delete more than remains is one
    /// of the places the two deliberately differ (see `apply`), so the comparison
    /// asks it for exactly what is there.
    fn forward(&self, row: usize, col: usize, chars: usize) -> (usize, usize, usize) {
        let text = self.ours.text();
        let (mut row, mut col) = (row, col);
        let mut left = chars;
        let mut consumed = 0;
        while left > 0 {
            let len = text.line_len_chars(row).expect("row is in range");
            let room = len - col;
            if left <= room {
                col += left;
                consumed += left;
                break;
            }
            consumed += room;
            left -= room;
            col = len;
            if row + 1 >= text.line_count() {
                break; // no line break left to cross
            }
            consumed += 1;
            left -= 1;
            row += 1;
            col = 0;
        }
        (row, col, consumed)
    }

    fn apply(&mut self, op: &Op) {
        match op {
            Op::Insert { row, col, text } => {
                let (row, col) = self.resolve(*row, *col);
                let at = self
                    .ours
                    .text()
                    .position(row, Column::new(col))
                    .expect("resolved position");
                self.ours.set_cursor(at);
                let mut txn = self.ours.begin();
                txn.insert(at, text);
                txn.commit();

                self.theirs
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                self.theirs.insert_str(text);
            }
            Op::Delete { row, col, chars } => {
                let (row, col) = self.resolve(*row, *col);
                let (end_row, end_col, chars) = self.forward(row, col, *chars);
                // Move first, delete second — as the keypress does. Doing it
                // before the early return below also resynchronises the
                // incumbent's cursor after an undo left it somewhere this crate
                // deliberately does not put it.
                let landing = self
                    .ours
                    .text()
                    .position(row, Column::new(col))
                    .expect("resolved position");
                self.ours.set_cursor(landing);
                self.theirs
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                if chars == 0 {
                    // A delete with nothing to delete is an intended difference,
                    // not a bug to compare: the incumbent's `delete_str` walks to
                    // a row past the end of the buffer, returns `true`, and pushes
                    // a history entry that removed nothing — so the *next* undo
                    // pops an unrelated edit. This crate records no entry, which
                    // is asserted in `src/buffer.rs` instead.
                    return;
                }
                let text = self.ours.text();
                let from = text
                    .position(row, Column::new(col))
                    .expect("resolved position");
                let to = text
                    .position(end_row, Column::new(end_col))
                    .expect("walked position");
                let span = text.span(from, to).expect("same text");
                let mut txn = self.ours.begin();
                txn.delete(span);
                txn.commit();

                self.theirs.delete_str(chars);
            }
            Op::Undo => {
                self.ours.undo();
                self.theirs.undo();
            }
            Op::Redo => {
                self.ours.redo();
                self.theirs.redo();
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Text and cursor agree after every operation in the sequence, not merely at
    /// the end — a divergence that later operations paper over is still a
    /// divergence, and the step it happened on is the useful part of the report.
    #[test]
    fn text_and_cursor_agree_with_the_incumbent(
        initial in "[a-z \n]{0,40}",
        ops in prop::collection::vec(op_strategy(), 1..14),
    ) {
        let mut pair = Pair::new(&initial);
        for (step, op) in ops.iter().enumerate() {
            pair.apply(op);
            prop_assert_eq!(
                pair.our_lines(),
                pair.theirs.lines().to_vec(),
                "text diverged at step {} ({:?}) from {:?}", step, op, initial
            );
            if !matches!(op, Op::Undo | Op::Redo) {
                prop_assert_eq!(
                    pair.our_cursor(),
                    pair.their_cursor(),
                    "cursor diverged at step {} ({:?}) from {:?}", step, op, initial
                );
            }
        }
    }

    /// Undoing every group returns the buffer to exactly what it started as. The
    /// incumbent cannot be the oracle here — its per-character history is a
    /// different shape — so this one is checked against the initial text.
    #[test]
    fn undoing_everything_returns_the_initial_text(
        initial in "[a-z \n]{0,40}",
        ops in prop::collection::vec(op_strategy(), 1..14),
    ) {
        let mut pair = Pair::new(&initial);
        for op in &ops {
            pair.apply(op);
        }
        // Redo first, so an undo in the sequence cannot leave a group unreachable.
        while pair.ours.redo().is_some() {}
        while pair.ours.undo().is_some() {}
        prop_assert_eq!(pair.ours.text().to_string(), initial);
    }

    /// A redo after an undo puts back exactly the text the undo took away.
    ///
    /// Text only. The cursor is deliberately not idempotent here: each entry
    /// records the cursor its own group started from, so undoing entry N leaves
    /// the cursor at N's starting point rather than at N-1's end — and redoing N
    /// then lands on N's end. That is the point of restoring a cursor at all, not
    /// a defect, so asserting round-trip equality would be asserting the wrong
    /// thing.
    #[test]
    fn undo_then_redo_restores_the_text(
        initial in "[a-z \n]{0,40}",
        ops in prop::collection::vec(op_strategy(), 1..14),
    ) {
        let mut pair = Pair::new(&initial);
        for op in &ops {
            pair.apply(op);
        }
        let text = pair.ours.text().to_string();
        if pair.ours.undo().is_some() {
            prop_assert!(pair.ours.redo().is_some(), "what undid must redo");
            prop_assert_eq!(pair.ours.text().to_string(), text);
        }
    }
}
