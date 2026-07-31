//! The edit history: one entry per [undo group](crate::EditBuffer::begin).

use std::collections::VecDeque;
use std::ops::Range;

use crate::change::Edit;
use crate::text::Text;

/// Retained-byte budget for the history.
///
/// Bounded by bytes rather than by a count of entries. A count is the wrong unit
/// twice over: it makes how far back undo reaches depend on how small the edits
/// were, and it lets a buffer full of one-character entries cost far more memory
/// than a buffer holding a few large ones.
pub(crate) const DEFAULT_BUDGET_BYTES: usize = 4 << 20;

/// Charged against the budget for every entry, on top of the text it retains.
///
/// Without it, a million single-character edits would sit inside a byte budget
/// while costing the allocator far more than the budget describes. A rough
/// stand-in for an entry's own footprint, not a measurement.
const ENTRY_OVERHEAD_BYTES: usize = 128;

/// What changed, in one text's coordinates.
#[derive(Debug, Clone)]
pub(crate) struct Shape {
    /// `None` once two shapes in different coordinate spaces have been folded
    /// together — see [`crate::Change::edits`].
    pub edits: Option<Vec<Edit>>,
    pub rows: Range<usize>,
    pub line_delta: isize,
}

/// One user action, as the states it ran between.
///
/// Holding both sides is affordable because a [`Text`] clone shares its
/// structure with what it was cloned from: consecutive entries hold the same
/// content once between them. It is also what makes undo exact rather than
/// replayed — there is no sequence of inverse operations to get wrong, and no
/// way to arrive half-way through one.
///
/// Cursor and selection are stored as byte offsets, not positions. A position
/// carries the revision it was made against and undo deliberately produces a
/// *new* revision, so a stored position would come back stale. An offset has no
/// such opinion.
#[derive(Debug)]
pub(crate) struct Entry {
    pub before: Text,
    pub after: Text,
    /// Describes `after`, for redoing.
    pub forward: Shape,
    /// Describes `before`, for undoing.
    pub inverse: Shape,
    pub cursor_before: usize,
    pub cursor_after: usize,
    pub anchor_before: Option<usize>,
    pub anchor_after: Option<usize>,
    /// Bytes of text this entry is responsible for retaining.
    pub retained: usize,
}

impl Entry {
    fn cost(&self) -> usize {
        ENTRY_OVERHEAD_BYTES + self.retained
    }
}

/// A linear undo history.
#[derive(Debug)]
pub(crate) struct History {
    /// Oldest first. Entries at and after `index` are the redo side.
    entries: VecDeque<Entry>,
    index: usize,
    budget: usize,
    spent: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(DEFAULT_BUDGET_BYTES)
    }
}

impl History {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            index: 0,
            budget,
            spent: 0,
        }
    }

    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
        self.evict_to_budget();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index = 0;
        self.spent = 0;
    }

    /// Record an entry, discarding anything that was waiting to be redone.
    pub fn push(&mut self, entry: Entry) {
        self.drop_redo_side();
        self.spent += entry.cost();
        self.entries.push_back(entry);
        self.index = self.entries.len();
        self.evict_to_budget();
    }

    /// Fold `entry` into the newest one, so the two read as a single action.
    ///
    /// Used when a backend judges that an edit continues what the last one
    /// started — a typing run, say. The merged entry keeps the *earlier* before
    /// state and the *later* after state, which is the pair a single group would
    /// have recorded had it been opened at the start of the run.
    ///
    /// Returns the entry back when there is nothing to fold it into: an empty
    /// history, or a redo side waiting — extending across an undo would rewrite a
    /// future the user can still walk forward into.
    pub fn extend_newest(&mut self, entry: Entry) -> Option<Entry> {
        if self.index != self.entries.len() || self.entries.is_empty() {
            return Some(entry);
        }
        let newest = self.entries.back_mut().expect("checked non-empty");
        self.spent -= newest.cost();

        // The two shapes describe different coordinate spaces, so the surviving
        // row range is widened to cover both. Widening costs a consumer some
        // redundant work; narrowing would cost it correctness. For a typing run —
        // the case this exists for — both line deltas are zero and the widening
        // is a no-op.
        newest.forward.rows = hull(
            widen(&newest.forward.rows, entry.forward.line_delta),
            &entry.forward.rows,
        );
        newest.inverse.rows = hull(
            newest.inverse.rows.clone(),
            &widen(&entry.inverse.rows, newest.forward.line_delta),
        );
        newest.forward.edits = None;
        newest.inverse.edits = None;
        newest.forward.line_delta += entry.forward.line_delta;
        newest.inverse.line_delta += entry.inverse.line_delta;

        newest.after = entry.after;
        newest.cursor_after = entry.cursor_after;
        newest.anchor_after = entry.anchor_after;
        newest.retained += entry.retained;

        self.spent += newest.cost();
        self.evict_to_budget();
        None
    }

    /// The entry a call to undo would apply, without applying it.
    pub fn undo_candidate(&self) -> Option<&Entry> {
        self.entries.get(self.index.checked_sub(1)?)
    }

    /// The entry a call to redo would apply, without applying it.
    pub fn redo_candidate(&self) -> Option<&Entry> {
        self.entries.get(self.index)
    }

    pub fn step_back(&mut self) {
        self.index = self.index.saturating_sub(1);
    }

    pub fn step_forward(&mut self) {
        self.index = (self.index + 1).min(self.entries.len());
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn undo_depth(&self) -> usize {
        self.index
    }

    fn drop_redo_side(&mut self) {
        while self.entries.len() > self.index {
            if let Some(dropped) = self.entries.pop_back() {
                self.spent -= dropped.cost();
            }
        }
    }

    /// Evict oldest-first until the budget is met.
    ///
    /// Only ever drops whole entries, so an eviction cannot leave a user action
    /// half-undoable — the failure mode that made a bounded, entry-counted
    /// history unsafe for grouped edits. The newest entry is never evicted: a
    /// buffer that cannot undo what was just done would be worse than one
    /// briefly over a memory target.
    fn evict_to_budget(&mut self) {
        while self.spent > self.budget && self.entries.len() > 1 {
            let dropped = self.entries.pop_front().expect("checked non-empty");
            self.spent -= dropped.cost();
            self.index = self.index.saturating_sub(1);
        }
    }
}

fn hull(a: Range<usize>, b: &Range<usize>) -> Range<usize> {
    a.start.min(b.start)..a.end.max(b.end)
}

/// Grow `rows` by `delta` rows in both directions.
///
/// Used to bring a row range from one text's coordinates into another's without
/// knowing where in the text the shift happened. Symmetric on purpose: it is
/// always safe, and exact when nothing moved.
fn widen(rows: &Range<usize>, delta: isize) -> Range<usize> {
    let slack = delta.unsigned_abs();
    rows.start.saturating_sub(slack)..rows.end.saturating_add(slack)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(rows: Range<usize>, line_delta: isize) -> Shape {
        Shape {
            edits: Some(vec![Edit::new(0..1, 0)]),
            rows,
            line_delta,
        }
    }

    fn entry(before: &str, after: &str) -> Entry {
        Entry {
            before: Text::from(before),
            after: Text::from(after),
            forward: shape(0..1, 0),
            inverse: shape(0..1, 0),
            cursor_before: 0,
            cursor_after: after.len(),
            anchor_before: None,
            anchor_after: None,
            retained: after.len(),
        }
    }

    #[test]
    fn a_push_drops_the_redo_side() {
        let mut h = History::default();
        h.push(entry("", "a"));
        h.push(entry("a", "ab"));
        h.step_back();
        assert!(h.redo_candidate().is_some());
        h.push(entry("a", "ax"));
        assert!(h.redo_candidate().is_none());
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn extending_keeps_the_earlier_before_and_the_later_after() {
        let mut h = History::default();
        h.push(entry("", "a"));
        assert!(h.extend_newest(entry("a", "ab")).is_none());
        assert_eq!(h.len(), 1);
        let merged = h.undo_candidate().expect("one entry");
        assert_eq!(merged.before.to_string(), "");
        assert_eq!(merged.after.to_string(), "ab");
    }

    #[test]
    fn extending_forgets_the_per_edit_ranges() {
        let mut h = History::default();
        h.push(entry("", "a"));
        h.extend_newest(entry("a", "ab"));
        let merged = h.undo_candidate().expect("one entry");
        assert!(
            merged.forward.edits.is_none(),
            "two coordinate spaces cannot be reported as one list"
        );
        assert!(merged.inverse.edits.is_none());
    }

    #[test]
    fn extending_widens_rows_rather_than_narrowing_them() {
        let mut h = History::default();
        let mut first = entry("", "a\nb");
        first.forward = shape(0..2, 1);
        first.inverse = shape(0..1, -1);
        h.push(first);
        let mut second = entry("a\nb", "a\nbc");
        second.forward = shape(1..2, 0);
        second.inverse = shape(1..2, 0);
        h.extend_newest(second);
        let merged = h.undo_candidate().expect("one entry");
        assert_eq!(merged.forward.rows, 0..2);
        // The second edit's inverse rows are one line further down than the
        // first's coordinate space, so the surviving range covers both.
        assert_eq!(merged.inverse.rows, 0..3);
        assert_eq!(merged.forward.line_delta, 1);
    }

    #[test]
    fn extending_is_refused_with_nothing_to_extend() {
        let mut h = History::default();
        assert!(h.extend_newest(entry("", "a")).is_some());
    }

    #[test]
    fn extending_is_refused_across_an_undo() {
        let mut h = History::default();
        h.push(entry("", "a"));
        h.push(entry("a", "ab"));
        h.step_back();
        assert!(h.extend_newest(entry("a", "ax")).is_some());
    }

    #[test]
    fn eviction_drops_whole_entries_oldest_first() {
        let mut h = History::new(ENTRY_OVERHEAD_BYTES * 3);
        for _ in 0..10 {
            h.push(entry("a", "ab"));
        }
        assert!(h.len() <= 3, "kept {} entries", h.len());
        assert_eq!(h.undo_depth(), h.len(), "undo side stays consistent");
    }

    #[test]
    fn the_newest_entry_is_never_evicted() {
        let mut h = History::new(0);
        h.push(entry("", "a"));
        h.push(entry("a", "ab"));
        assert_eq!(h.len(), 1);
        assert_eq!(
            h.undo_candidate()
                .expect("the newest survives")
                .after
                .to_string(),
            "ab"
        );
    }

    #[test]
    fn widening_by_nothing_changes_nothing() {
        assert_eq!(widen(&(3..5), 0), 3..5);
        assert_eq!(widen(&(3..5), 2), 1..7);
        assert_eq!(widen(&(0..1), -2), 0..3);
    }
}
