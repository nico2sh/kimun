//! The edit buffer: text, cursor, selection and history as one thing.

use std::ops::Range;

use crate::change::{Change, Edit};
use crate::history::{DEFAULT_BUDGET_BYTES, Entry, History, Shape};
use crate::position::{Position, Revision, Span};
use crate::text::Text;

/// Which side of an insertion a remapped offset ends up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gravity {
    /// Text inserted exactly here pushes the offset along. What typing does to a
    /// cursor.
    Forward,
    /// Text inserted exactly here is left in front of the offset. What a caller
    /// holding a marker wants: the marker stays where it was pointing.
    Backward,
}

/// One primitive applied inside a transaction, in the coordinates it was applied
/// in.
#[derive(Debug, Clone, Copy)]
struct Applied {
    at: usize,
    removed: usize,
    inserted: usize,
    /// The text's revision immediately before this was applied, so a position
    /// made against that state can be recognised and carried forward.
    revision_before: Revision,
}

/// A text with its cursor and selection, taken at one moment.
///
/// Cheap: the text shares its structure with the buffer's, so this is not a copy
/// of the note. Hand one to a background task or keep one for a preview and the
/// cursor cannot drift away from the text it was read with, because they arrived
/// together and neither can change.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub text: Text,
    pub cursor: Position,
    pub selection: Option<Span>,
}

/// The open note's text, cursor, selection and edit history.
///
/// Every mutation goes through a transaction ([`Self::begin`]), and a
/// transaction is exactly one entry in the history. Nothing has to count history
/// entries, predict how many an operation will push, or reconstruct where a
/// group started: the entry holds the states it ran between.
#[derive(Debug)]
pub struct EditBuffer {
    text: Text,
    cursor: Position,
    anchor: Option<Position>,
    history: History,
}

impl Default for EditBuffer {
    fn default() -> Self {
        Self::new(Text::new())
    }
}

impl EditBuffer {
    pub fn new(text: Text) -> Self {
        let cursor = text.start();
        Self {
            text,
            cursor,
            anchor: None,
            history: History::new(DEFAULT_BUDGET_BYTES),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn cursor(&self) -> Position {
        self.cursor
    }

    /// The selected range, or `None` when nothing is selected.
    ///
    /// The selection lives here rather than beside the buffer so that "moving the
    /// cursor without extending drops the selection" is a property of the thing
    /// that owns both, and not a rule every caller has to remember. Reaching for
    /// a search result and forgetting to drop the anchor is how an invisible
    /// selection gets deleted by the next keystroke.
    pub fn selection(&self) -> Option<Span> {
        let anchor = self.anchor?;
        self.text.span(anchor, self.cursor)
    }

    /// Everything a reader needs, consistent by construction.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            selection: self.selection(),
        }
    }

    /// Move the cursor, dropping any selection.
    ///
    /// Returns `false` — and moves nothing — when `to` addresses a state this
    /// buffer has left. Not an assertion: a position outliving the text it was
    /// made against is a real thing that happens to real callers, an event
    /// arriving after a background edit being the obvious way. Refusing it is a
    /// keystroke that did nothing, which is recoverable in a way editing the
    /// wrong place is not.
    pub fn set_cursor(&mut self, to: Position) -> bool {
        if self.text.is_stale(to) {
            return false;
        }
        self.cursor = to;
        self.anchor = None;
        true
    }

    /// Move the cursor, keeping or starting a selection.
    ///
    /// The counterpart of [`Self::set_cursor`], and the reason that one can drop
    /// the anchor unconditionally: every caller says which it means.
    pub fn extend_to(&mut self, to: Position) -> bool {
        if self.text.is_stale(to) {
            return false;
        }
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        self.cursor = to;
        true
    }

    /// Select `span`, leaving the cursor at its end.
    pub fn select(&mut self, span: Span) -> bool {
        if span.revision() != self.text.revision() {
            return false;
        }
        self.anchor = Some(span.start());
        self.cursor = span.end();
        true
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// Replace the whole buffer and forget its history.
    ///
    /// For opening a different note, not for editing. The history describes
    /// states the new text cannot reach, so keeping it would let an undo jump
    /// from one note into another.
    pub fn set_text(&mut self, text: Text) {
        self.text = text;
        self.cursor = self.text.start();
        self.anchor = None;
        self.history.clear();
    }

    /// How many bytes of edit history to retain.
    pub fn set_history_budget(&mut self, bytes: usize) {
        self.history.set_budget(bytes);
    }

    /// Open a transaction that will record its own [undo group].
    ///
    /// [undo group]: crate::EditBuffer
    pub fn begin(&mut self) -> Txn<'_> {
        Txn::new(self, false)
    }

    /// Open a transaction that folds into the previous group.
    ///
    /// For a backend that has decided this edit continues what the last one
    /// started — the next character of a typing run. Where a group ends is the
    /// backend's judgement, because only the backend knows what the user was
    /// doing; this crate keeps no clock and no policy.
    ///
    /// Falls back to [`Self::begin`] when there is nothing to fold into.
    pub fn begin_extending(&mut self) -> Txn<'_> {
        Txn::new(self, true)
    }

    /// Undo the newest group, restoring the text, cursor and selection it began
    /// from.
    ///
    /// All of it or none of it. The entry holds the state the group started from,
    /// so there is no replay to run out of, no count to get wrong, and no way to
    /// land part-way through an action the user asked to undo whole.
    pub fn undo(&mut self) -> Option<Change> {
        let entry = self.history.undo_candidate()?;
        let text = entry.before.reidentified();
        let shape = entry.inverse.clone();
        let cursor = entry.cursor_before;
        let anchor = entry.anchor_before;
        self.history.step_back();
        Some(self.restore(text, shape, cursor, anchor))
    }

    /// Redo the group a previous [`Self::undo`] took back.
    pub fn redo(&mut self) -> Option<Change> {
        let entry = self.history.redo_candidate()?;
        let text = entry.after.reidentified();
        let shape = entry.forward.clone();
        let cursor = entry.cursor_after;
        let anchor = entry.anchor_after;
        self.history.step_forward();
        Some(self.restore(text, shape, cursor, anchor))
    }

    fn restore(
        &mut self,
        text: Text,
        shape: Shape,
        cursor: usize,
        anchor: Option<usize>,
    ) -> Change {
        self.text = text;
        self.cursor = self.text.position_at_derived_byte(cursor);
        self.anchor = anchor.map(|a| self.text.position_at_derived_byte(a));
        Change::new(
            self.text.revision(),
            shape.edits,
            shape.rows,
            shape.line_delta,
        )
    }
}

/// A set of edits that lands as one [undo group](EditBuffer).
///
/// Dropping a transaction without committing rolls it back: the buffer returns to
/// the text, cursor and selection it had when the transaction opened. That is
/// cheap for the same reason the history is — restoring a text is restoring a
/// handle — and it means a panic part-way through a compound edit cannot leave
/// half of one behind.
#[must_use = "a transaction that is dropped without commit() is rolled back"]
pub struct Txn<'a> {
    buffer: &'a mut EditBuffer,
    before: Text,
    cursor_before: usize,
    anchor_before: Option<usize>,
    applied: Vec<Applied>,
    /// Retained bytes, accumulated as edits land.
    retained: usize,
    extending: bool,
    committed: bool,
}

impl<'a> Txn<'a> {
    fn new(buffer: &'a mut EditBuffer, extending: bool) -> Self {
        let before = buffer.text.clone();
        let cursor_before = buffer.cursor.byte();
        let anchor_before = buffer.anchor.map(Position::byte);
        Self {
            buffer,
            before,
            cursor_before,
            anchor_before,
            applied: Vec::new(),
            retained: 0,
            extending,
            committed: false,
        }
    }

    /// The text as it stands part-way through the transaction.
    pub fn text(&self) -> &Text {
        &self.buffer.text
    }

    pub fn cursor(&self) -> Position {
        self.buffer.cursor
    }

    pub fn selection(&self) -> Option<Span> {
        self.buffer.selection()
    }

    /// Carry a position made earlier in — or before — this transaction forward to
    /// where it now points.
    ///
    /// `None` when the position belongs to neither this transaction nor the state
    /// it opened from. Text inserted exactly at the position is left in front of
    /// it, so a marker keeps pointing at what it was pointing at; text deleted
    /// across it collapses it to the start of what was removed.
    pub fn map(&self, position: Position) -> Option<Position> {
        let byte = self.remap(position, Gravity::Backward)?;
        Some(self.buffer.text.position_at_derived_byte(byte))
    }

    /// Insert `text` at `at`.
    ///
    /// Returns `false`, changing nothing, when `at` belongs to neither this
    /// transaction nor the state it opened from.
    pub fn insert(&mut self, at: Position, text: &str) -> bool {
        let Some(byte) = self.remap(at, Gravity::Backward) else {
            return false;
        };
        self.splice(byte..byte, text)
    }

    /// Remove the text in `span`.
    pub fn delete(&mut self, span: Span) -> bool {
        self.replace(span, "")
    }

    /// Replace the text in `span` with `text`.
    ///
    /// Deleting and inserting as one primitive, so the pair cannot be recorded as
    /// two undo steps and cannot be interrupted between them.
    pub fn replace(&mut self, span: Span, text: &str) -> bool {
        let Some(start) = self.remap(span.start(), Gravity::Backward) else {
            return false;
        };
        let Some(end) = self.remap(span.end(), Gravity::Forward) else {
            return false;
        };
        if end < start {
            debug_assert!(false, "span ends collapsed past its start");
            return false;
        }
        self.splice(start..end, text)
    }

    /// Move the cursor, dropping any selection. Not itself an edit.
    pub fn set_cursor(&mut self, to: Position) -> bool {
        self.buffer.set_cursor(to)
    }

    /// Select `span`, leaving the cursor at its end.
    ///
    /// For a compound edit that must leave a selection behind — wrapping a
    /// selection in brackets leaves the inner text selected, so the next wrap
    /// nests.
    pub fn select(&mut self, span: Span) -> bool {
        self.buffer.select(span)
    }

    pub fn clear_selection(&mut self) {
        self.buffer.clear_selection();
    }

    /// Record the transaction and report what it did.
    ///
    /// `None` when nothing changed: no history entry, no revision, nothing for a
    /// consumer to re-derive. A transaction that only moved the cursor keeps the
    /// move — cursor position is not history.
    pub fn commit(mut self) -> Option<Change> {
        self.committed = true;
        if self.applied.is_empty() {
            return None;
        }

        let forward = self.forward_shape();
        let inverse = self.inverse_shape();
        let entry = Entry {
            before: self.before.clone(),
            after: self.buffer.text.clone(),
            forward: forward.clone(),
            inverse,
            cursor_before: self.cursor_before,
            cursor_after: self.buffer.cursor.byte(),
            anchor_before: self.anchor_before,
            anchor_after: self.buffer.anchor.map(Position::byte),
            retained: self.retained,
        };

        if self.extending {
            if let Some(entry) = self.buffer.history.extend_newest(entry) {
                self.buffer.history.push(entry);
            }
        } else {
            self.buffer.history.push(entry);
        }

        Some(Change::new(
            self.buffer.text.revision(),
            forward.edits,
            forward.rows,
            forward.line_delta,
        ))
    }

    // -- internals ----------------------------------------------------------

    fn splice(&mut self, bytes: Range<usize>, text: &str) -> bool {
        if bytes.is_empty() && text.is_empty() {
            return true;
        }
        let revision_before = self.buffer.text.revision();
        let removed = bytes.len();
        let at = bytes.start;
        let inserted = self.buffer.text.splice(bytes, text);

        self.applied.push(Applied {
            at,
            removed,
            inserted,
            revision_before,
        });
        self.retained += removed + inserted;

        let edit = Applied {
            at,
            removed,
            inserted,
            revision_before,
        };
        let cursor = shift(self.buffer.cursor.byte(), &edit, Gravity::Forward);
        self.buffer.cursor = self.buffer.text.position_at_derived_byte(cursor);
        // An edit is not a selection gesture. Typing over a selection must not
        // leave the typed text selected, and a caller that does want a selection
        // afterwards — wrapping a selection in brackets, so the next wrap nests —
        // says so with `select`.
        self.buffer.anchor = None;
        true
    }

    /// Where `position` points now, or `None` if it never pointed into this
    /// transaction's lineage.
    fn remap(&self, position: Position, gravity: Gravity) -> Option<usize> {
        if position.revision() == self.buffer.text.revision() {
            return Some(position.byte());
        }
        let from = self
            .applied
            .iter()
            .position(|a| a.revision_before == position.revision())?;
        let mut byte = position.byte();
        for edit in &self.applied[from..] {
            byte = shift(byte, edit, gravity);
        }
        Some(byte)
    }

    /// What the transaction did, in the final text's coordinates.
    fn forward_shape(&self) -> Shape {
        let mut edits = Vec::with_capacity(self.applied.len());
        let mut rows: Option<Range<usize>> = None;
        for (i, edit) in self.applied.iter().enumerate() {
            let later = &self.applied[i + 1..];
            let start = later
                .iter()
                .fold(edit.at, |b, e| shift(b, e, Gravity::Backward));
            let end = later.iter().fold(edit.at + edit.inserted, |b, e| {
                shift(b, e, Gravity::Forward)
            });
            let end = end.max(start).min(self.buffer.text.len_bytes());
            let start = start.min(end);
            edits.push(Edit::new(start..end, edit.removed));
            rows = Some(union(rows, self.rows_of(&self.buffer.text, start..end)));
        }
        Shape {
            edits: Some(edits),
            rows: rows.unwrap_or(0..0),
            line_delta: line_delta(&self.before, &self.buffer.text),
        }
    }

    /// What undoing it would do, in the original text's coordinates.
    ///
    /// Computed here rather than derived at undo time, because here is where both
    /// coordinate spaces are still known.
    fn inverse_shape(&self) -> Shape {
        let mut edits = Vec::with_capacity(self.applied.len());
        let mut rows: Option<Range<usize>> = None;
        for (i, edit) in self.applied.iter().enumerate() {
            let earlier = &self.applied[..i];
            // Walk the edit's own position back through the ones before it, so
            // it lands in the coordinates the transaction opened in.
            let start = earlier.iter().rev().fold(edit.at, unshift);
            let end = (start + edit.removed).min(self.before.len_bytes());
            let start = start.min(end);
            edits.push(Edit::new(start..end, edit.inserted));
            rows = Some(union(rows, self.rows_of(&self.before, start..end)));
        }
        Shape {
            edits: Some(edits),
            rows: rows.unwrap_or(0..0),
            line_delta: line_delta(&self.buffer.text, &self.before),
        }
    }

    fn rows_of(&self, text: &Text, bytes: Range<usize>) -> Range<usize> {
        let first = text.row_of_byte(bytes.start.min(text.len_bytes()));
        let last = text.row_of_byte(bytes.end.min(text.len_bytes()));
        first..last + 1
    }
}

impl Drop for Txn<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.buffer.text = self.before.clone();
        self.buffer.cursor = self
            .buffer
            .text
            .position_at_derived_byte(self.cursor_before);
        self.buffer.anchor = self
            .anchor_before
            .map(|byte| self.buffer.text.position_at_derived_byte(byte));
    }
}

/// Carry `byte` across one applied edit.
fn shift(byte: usize, edit: &Applied, gravity: Gravity) -> usize {
    let end = edit.at + edit.removed;
    if byte < edit.at {
        byte
    } else if byte > edit.at && byte >= end {
        byte - edit.removed + edit.inserted
    } else {
        // At the edit, or inside what it removed. Gravity decides which side of
        // the new text the offset ends up on: a cursor follows what was typed,
        // a marker stays pointing where it pointed.
        match gravity {
            Gravity::Forward => edit.at + edit.inserted,
            Gravity::Backward => edit.at,
        }
    }
}

/// Carry `byte` back across one applied edit, into the coordinates before it.
fn unshift(byte: usize, edit: &Applied) -> usize {
    let end = edit.at + edit.inserted;
    if byte <= edit.at {
        byte
    } else if byte >= end {
        byte - edit.inserted + edit.removed
    } else {
        edit.at
    }
}

fn union(rows: Option<Range<usize>>, next: Range<usize>) -> Range<usize> {
    match rows {
        Some(rows) => rows.start.min(next.start)..rows.end.max(next.end),
        None => next,
    }
}

fn line_delta(from: &Text, to: &Text) -> isize {
    to.line_count() as isize - from.line_count() as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Column;

    fn buffer(text: &str) -> EditBuffer {
        EditBuffer::new(Text::from(text))
    }

    fn at(buf: &EditBuffer, row: usize, col: usize) -> Position {
        buf.text()
            .position(row, Column::new(col))
            .expect("addressable in the test fixture")
    }

    fn span(buf: &EditBuffer, from: (usize, usize), to: (usize, usize)) -> Span {
        let a = at(buf, from.0, from.1);
        let b = at(buf, to.0, to.1);
        buf.text().span(a, b).expect("same text")
    }

    // -- primitives ---------------------------------------------------------

    #[test]
    fn inserting_at_the_cursor_carries_the_cursor_along() {
        let mut buf = buffer("hello");
        let p = at(&buf, 0, 5);
        buf.set_cursor(p);
        let mut txn = buf.begin();
        assert!(txn.insert(p, ", world"));
        txn.commit();
        assert_eq!(buf.text().to_string(), "hello, world");
        assert_eq!(buf.cursor().column().get(), 12);
    }

    #[test]
    fn inserting_after_the_cursor_leaves_the_cursor_where_it_was() {
        let mut buf = buffer("hello");
        buf.set_cursor(at(&buf, 0, 1));
        let target = at(&buf, 0, 4);
        let mut txn = buf.begin();
        txn.insert(target, "XXX");
        txn.commit();
        assert_eq!(buf.cursor().byte(), 1, "an edit elsewhere is not a move");
    }

    #[test]
    fn inserting_before_the_cursor_pushes_it_along() {
        let mut buf = buffer("hello");
        buf.set_cursor(at(&buf, 0, 4));
        let target = at(&buf, 0, 0);
        let mut txn = buf.begin();
        txn.insert(target, "XX");
        txn.commit();
        assert_eq!(buf.cursor().column().get(), 6);
    }

    #[test]
    fn deleting_across_the_cursor_collapses_it_to_the_start() {
        let mut buf = buffer("hello world");
        buf.set_cursor(at(&buf, 0, 8));
        let s = span(&buf, (0, 5), (0, 11));
        let mut txn = buf.begin();
        txn.delete(s);
        txn.commit();
        assert_eq!(buf.text().to_string(), "hello");
        assert_eq!(buf.cursor().byte(), 5);
    }

    #[test]
    fn replacing_leaves_the_cursor_after_the_new_text() {
        let mut buf = buffer("hello world");
        let s = span(&buf, (0, 6), (0, 11));
        buf.set_cursor(s.end());
        let mut txn = buf.begin();
        txn.replace(s, "there");
        txn.commit();
        assert_eq!(buf.text().to_string(), "hello there");
        assert_eq!(buf.cursor().column().get(), 11);
    }

    #[test]
    fn an_edit_clears_the_selection() {
        let mut buf = buffer("hello world");
        let s = span(&buf, (0, 0), (0, 5));
        buf.select(s);
        assert!(buf.selection().is_some());
        let mut txn = buf.begin();
        txn.replace(s, "bye");
        txn.commit();
        assert_eq!(buf.text().to_string(), "bye world");
        assert!(
            buf.selection().is_none(),
            "typing over a selection must not leave the typed text selected"
        );
    }

    #[test]
    fn a_compound_edit_can_leave_a_selection_behind() {
        // Wrapping a selection in brackets keeps the inner text selected, so a
        // second wrap nests.
        let mut buf = buffer("word");
        let s = span(&buf, (0, 0), (0, 4));
        let mut txn = buf.begin();
        txn.insert(s.end(), "]");
        txn.insert(s.start(), "[");
        let inner = txn
            .text()
            .span(
                txn.text().position(0, Column::new(1)).unwrap(),
                txn.text().position(0, Column::new(5)).unwrap(),
            )
            .unwrap();
        txn.select(inner);
        txn.commit();
        assert_eq!(buf.text().to_string(), "[word]");
        assert_eq!(
            buf.text()
                .slice(buf.selection().expect("still selected"))
                .as_deref(),
            Some("word")
        );
    }

    #[test]
    fn several_primitives_are_one_undo() {
        let mut buf = buffer("word");
        let s = span(&buf, (0, 0), (0, 4));
        let mut txn = buf.begin();
        txn.insert(s.end(), "]");
        txn.insert(s.start(), "[");
        txn.commit();
        assert_eq!(buf.text().to_string(), "[word]");
        buf.undo();
        assert_eq!(buf.text().to_string(), "word", "one action, one undo");
    }

    #[test]
    fn a_position_from_before_the_transaction_still_points_at_its_text() {
        let mut buf = buffer("one two");
        let two = at(&buf, 0, 4);
        let head = at(&buf, 0, 0);
        let mut txn = buf.begin();
        txn.insert(head, "zero ");
        // `two` was made before the insert; it now sits five bytes further on.
        let moved = txn.map(two).expect("carried forward");
        assert_eq!(moved.byte(), 9);
        txn.commit();
    }

    #[test]
    fn a_position_from_another_text_is_refused() {
        let mut buf = buffer("hello");
        let other = Text::from("hello");
        let foreign = other.position(0, Column::new(2)).unwrap();
        let mut txn = buf.begin();
        assert!(!txn.insert(foreign, "x"));
        assert!(txn.map(foreign).is_none());
        txn.commit();
        assert_eq!(buf.text().to_string(), "hello", "nothing happened");
    }

    #[test]
    fn a_stale_cursor_is_refused_rather_than_approximated() {
        let mut buf = buffer("hello");
        let stale = at(&buf, 0, 4);
        let head = at(&buf, 0, 0);
        let mut txn = buf.begin();
        txn.insert(head, "xx");
        txn.commit();
        assert!(!buf.set_cursor(stale));
    }

    // -- transactions -------------------------------------------------------

    #[test]
    fn a_transaction_that_changes_nothing_reports_nothing() {
        let mut buf = buffer("hello");
        let p = at(&buf, 0, 2);
        let mut txn = buf.begin();
        txn.insert(p, "");
        assert!(txn.commit().is_none());
        assert!(buf.undo().is_none(), "and records no history");
    }

    #[test]
    fn a_transaction_that_only_moves_the_cursor_keeps_the_move() {
        let mut buf = buffer("hello");
        let p = at(&buf, 0, 3);
        let mut txn = buf.begin();
        txn.set_cursor(p);
        assert!(txn.commit().is_none(), "a cursor move is not history");
        assert_eq!(buf.cursor().byte(), 3);
    }

    #[test]
    fn dropping_a_transaction_rolls_it_back() {
        let mut buf = buffer("hello");
        let end = at(&buf, 0, 5);
        buf.set_cursor(end);
        {
            let mut txn = buf.begin();
            txn.insert(end, " world");
            assert_eq!(txn.text().to_string(), "hello world");
            // dropped without commit
        }
        assert_eq!(buf.text().to_string(), "hello");
        assert_eq!(buf.cursor().byte(), 5);
        assert!(buf.undo().is_none(), "a rollback is not an undo step");
    }

    // -- history ------------------------------------------------------------

    #[test]
    fn a_delete_that_removes_nothing_records_no_history() {
        // Pressing Delete at the end of a buffer must not consume an undo step.
        // The implementation this crate replaces does: its `delete_str` walks to a
        // row past the end, returns `true`, and pushes an entry that removed
        // nothing — so the *next* undo takes back an unrelated edit.
        let mut buf = buffer("hello");
        let start = at(&buf, 0, 0);
        let mut txn = buf.begin();
        txn.insert(start, "x");
        txn.commit();

        // Taken from the current text: a span made before that insert would be
        // stale, and refused for that reason rather than this one.
        let end = buf.text().end();
        let empty = buf.text().span(end, end).expect("same text");
        let mut txn = buf.begin();
        assert!(txn.delete(empty), "an empty delete is not a failure");
        assert!(txn.commit().is_none(), "but it is not a change either");

        buf.undo();
        assert_eq!(
            buf.text().to_string(),
            "hello",
            "the undo took back the insert, not a phantom delete"
        );
        assert!(buf.undo().is_none(), "and there was only ever one entry");
    }

    #[test]
    fn undo_of_a_forward_delete_returns_the_cursor_to_where_the_user_was() {
        // The near end of the deleted range, which is where the cursor was when
        // the delete happened — not the far end, which it never visited.
        let mut buf = buffer("hello world");
        let s = span(&buf, (0, 5), (0, 11));
        buf.set_cursor(s.start());
        let mut txn = buf.begin();
        txn.delete(s);
        txn.commit();
        assert_eq!(buf.text().to_string(), "hello");
        buf.undo();
        assert_eq!(buf.text().to_string(), "hello world");
        assert_eq!(buf.cursor().byte(), 5, "not 11");
    }

    #[test]
    fn undo_and_redo_walk_the_text_back_and_forward() {
        let mut buf = buffer("");
        for word in ["a", "b", "c"] {
            let end = buf.text().end();
            let mut txn = buf.begin();
            txn.insert(end, word);
            txn.commit();
        }
        assert_eq!(buf.text().to_string(), "abc");
        buf.undo();
        assert_eq!(buf.text().to_string(), "ab");
        buf.undo();
        assert_eq!(buf.text().to_string(), "a");
        buf.redo();
        assert_eq!(buf.text().to_string(), "ab");
        buf.redo();
        assert_eq!(buf.text().to_string(), "abc");
        assert!(buf.redo().is_none());
    }

    #[test]
    fn undo_restores_the_cursor_the_group_started_from() {
        let mut buf = buffer("hello");
        let start = at(&buf, 0, 2);
        buf.set_cursor(start);
        let mut txn = buf.begin();
        txn.insert(start, "XYZ");
        txn.commit();
        assert_eq!(buf.cursor().byte(), 5);
        buf.undo();
        assert_eq!(buf.cursor().byte(), 2);
    }

    #[test]
    fn undo_mints_a_new_revision_rather_than_reusing_the_old_one() {
        let mut buf = buffer("hello");
        let before = buf.text().revision();
        let p = at(&buf, 0, 5);
        let mut txn = buf.begin();
        txn.insert(p, "!");
        txn.commit();
        buf.undo();
        assert_eq!(buf.text().to_string(), "hello");
        assert_ne!(
            buf.text().revision(),
            before,
            "the same content at a later point in the timeline is a later revision"
        );
    }

    #[test]
    fn extending_folds_a_typing_run_into_one_undo() {
        let mut buf = buffer("");
        let first = buf.text().end();
        let mut txn = buf.begin();
        txn.insert(first, "h");
        txn.commit();
        for c in ["e", "l", "l", "o"] {
            let end = buf.text().end();
            let mut txn = buf.begin_extending();
            txn.insert(end, c);
            txn.commit();
        }
        assert_eq!(buf.text().to_string(), "hello");
        buf.undo();
        assert_eq!(
            buf.text().to_string(),
            "",
            "the whole run went back at once"
        );
        assert!(buf.undo().is_none());
    }

    #[test]
    fn extending_with_no_previous_group_records_one() {
        let mut buf = buffer("");
        let end = buf.text().end();
        let mut txn = buf.begin_extending();
        txn.insert(end, "a");
        txn.commit();
        buf.undo();
        assert_eq!(buf.text().to_string(), "");
    }

    #[test]
    fn a_new_group_after_an_undo_drops_the_redo_side() {
        let mut buf = buffer("");
        let end = buf.text().end();
        let mut txn = buf.begin();
        txn.insert(end, "a");
        txn.commit();
        buf.undo();
        let end = buf.text().end();
        let mut txn = buf.begin();
        txn.insert(end, "b");
        txn.commit();
        assert!(buf.redo().is_none());
        assert_eq!(buf.text().to_string(), "b");
    }

    #[test]
    fn opening_a_different_note_forgets_the_history() {
        let mut buf = buffer("first");
        let end = buf.text().end();
        let mut txn = buf.begin();
        txn.insert(end, "!");
        txn.commit();
        buf.set_text(Text::from("second"));
        assert!(
            buf.undo().is_none(),
            "an undo must not walk from one note into another"
        );
        assert_eq!(buf.text().to_string(), "second");
    }

    // -- what a change reports ---------------------------------------------

    #[test]
    fn a_change_reports_the_row_it_touched() {
        let mut buf = buffer("one\ntwo\nthree");
        let p = at(&buf, 1, 1);
        let mut txn = buf.begin();
        txn.insert(p, "X");
        let change = txn.commit().expect("something changed");
        assert_eq!(change.rows(), 1..2);
        assert_eq!(change.line_delta(), 0);
        assert!(!change.is_bulk());
    }

    #[test]
    fn a_change_reports_added_rows() {
        let mut buf = buffer("one\ntwo");
        let p = at(&buf, 0, 3);
        let mut txn = buf.begin();
        txn.insert(p, "\nmiddle");
        let change = txn.commit().expect("something changed");
        assert_eq!(change.line_delta(), 1);
        assert_eq!(change.rows(), 0..2);
        assert!(change.is_bulk());
    }

    #[test]
    fn a_change_reports_removed_rows() {
        let mut buf = buffer("one\ntwo\nthree");
        let s = span(&buf, (0, 3), (2, 0));
        let mut txn = buf.begin();
        txn.delete(s);
        let change = txn.commit().expect("something changed");
        assert_eq!(change.line_delta(), -2);
        assert_eq!(buf.text().to_string(), "onethree");
    }

    #[test]
    fn a_change_names_the_bytes_it_wrote() {
        let mut buf = buffer("hello");
        let p = at(&buf, 0, 5);
        let mut txn = buf.begin();
        txn.insert(p, "!!");
        let change = txn.commit().expect("something changed");
        let edits = change.edits().expect("a single transaction is precise");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].inserted(), 5..7);
        assert_eq!(edits[0].removed_bytes(), 0);
    }

    #[test]
    fn undoing_a_coalesced_group_declines_to_name_bytes() {
        let mut buf = buffer("");
        let end = buf.text().end();
        let mut txn = buf.begin();
        txn.insert(end, "a");
        txn.commit();
        let end = buf.text().end();
        let mut txn = buf.begin_extending();
        txn.insert(end, "b");
        txn.commit();
        let change = buf.undo().expect("there was a group");
        assert!(
            change.edits().is_none(),
            "two coordinate spaces cannot be reported as one list"
        );
        assert!(change.rows().start <= change.rows().end);
    }

    #[test]
    fn an_undo_reports_the_rows_of_the_text_it_restored() {
        let mut buf = buffer("one\ntwo\nthree");
        let p = at(&buf, 2, 0);
        let mut txn = buf.begin();
        txn.insert(p, "new\n");
        txn.commit();
        assert_eq!(buf.text().line_count(), 4);
        let change = buf.undo().expect("there was a group");
        assert_eq!(change.line_delta(), -1);
        assert!(
            change.rows().end <= buf.text().line_count(),
            "rows {:?} must address the restored text of {} rows",
            change.rows(),
            buf.text().line_count()
        );
    }

    // -- snapshots ----------------------------------------------------------

    #[test]
    fn a_snapshot_holds_a_text_its_cursor_cannot_drift_from() {
        let mut buf = buffer("hello");
        buf.set_cursor(at(&buf, 0, 5));
        let snap = buf.snapshot();
        let end = buf.text().end();
        let mut txn = buf.begin();
        txn.insert(end, " world");
        txn.commit();
        assert_eq!(snap.text.to_string(), "hello");
        assert_eq!(snap.cursor.byte(), 5);
        assert!(!snap.text.is_stale(snap.cursor));
        assert_eq!(
            snap.text.slice(snap.text.full_span()).as_deref(),
            Some("hello")
        );
    }
}
