//! Addresses into a [`Text`](crate::ropetext::Text): revisions, columns, positions, spans.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

/// Which state of a text a value refers to.
///
/// Revisions are unique across the whole process, not per text. Per-text
/// counters would collide in the case this type exists to catch: an editor's
/// buffer and a preview clone of it both advance from the same value, so a
/// [`Position`] made against one would be accepted by the other and read as a
/// different place in a different buffer.
///
/// Cloning a text does not mint a revision — a clone *is* the same text. Only a
/// committed change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Revision(NonZeroU64);

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

impl Revision {
    /// Mint a revision no other text has held.
    pub(crate) fn fresh() -> Self {
        let n = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
        // Exhausting u64 would wrap to zero. At a billion revisions a second
        // that takes about 584 years, so this is a statement of intent rather
        // than a case to handle.
        Self(NonZeroU64::new(n).expect("revision counter wrapped"))
    }

    pub fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// A column within one row, counted in Unicode scalars.
///
/// Characters, not grapheme clusters. The cursor *steps* by cluster — motions
/// and cell mapping only ever land on cluster boundaries — but indexing by
/// cluster would mean segmenting a row from its start on every lookup, where a
/// char index is a rope operation. Nothing wants cluster ordinals: vim's `|`
/// counts characters, an external editor's cursor arrives in bytes, and a
/// markdown parser reports byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Column(usize);

impl Column {
    pub const ZERO: Column = Column(0);

    pub fn new(chars: usize) -> Self {
        Self(chars)
    }

    pub fn get(self) -> usize {
        self.0
    }
}

impl From<usize> for Column {
    fn from(chars: usize) -> Self {
        Self(chars)
    }
}

/// A place in a text.
///
/// Built only by [`Text::position`](crate::ropetext::Text::position) and its siblings, so
/// a position that exists is one the text could address when it was made. Row,
/// column and byte offset are all resolved at construction and the type is
/// `Copy`, so reading them is free while *making* one costs a rope lookup —
/// loops should carry a position forward rather than rebuild it from `(row,
/// col)` each time round.
///
/// Deliberately not `Ord`. Comparing positions from two revisions is
/// meaningless, and a comparison operator that quietly answers `false` for
/// incomparable operands is the kind of wrong answer this crate exists to avoid.
/// Use [`Text::span`](crate::ropetext::Text::span), which checks both operands and orders
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Position {
    byte: usize,
    row: usize,
    column: Column,
    revision: Revision,
}

impl Position {
    pub(crate) fn new(byte: usize, row: usize, column: Column, revision: Revision) -> Self {
        Self {
            byte,
            row,
            column,
            revision,
        }
    }

    /// Byte offset from the start of the text.
    pub fn byte(self) -> usize {
        self.byte
    }

    /// Zero-based logical row.
    pub fn row(self) -> usize {
        self.row
    }

    /// Column within [`Self::row`], in Unicode scalars.
    pub fn column(self) -> Column {
        self.column
    }

    /// The text state this position addresses.
    pub fn revision(self) -> Revision {
        self.revision
    }
}

/// An ordered, single-revision range between two [`Position`]s.
///
/// The ordering and the revision agreement are established once, when the span
/// is built, so nothing downstream has to re-establish either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    start: Position,
    end: Position,
}

impl Span {
    /// `start` and `end` must share a revision, and `start` must not be after
    /// `end`. Both are guaranteed by [`Text::span`](crate::ropetext::Text::span), the only
    /// caller.
    pub(crate) fn new(start: Position, end: Position) -> Self {
        debug_assert_eq!(start.revision(), end.revision());
        debug_assert!(start.byte() <= end.byte());
        Self { start, end }
    }

    pub fn start(self) -> Position {
        self.start
    }

    pub fn end(self) -> Position {
        self.end
    }

    pub fn revision(self) -> Revision {
        self.start.revision()
    }

    pub fn is_empty(self) -> bool {
        self.start.byte() == self.end.byte()
    }

    pub fn byte_range(self) -> std::ops::Range<usize> {
        self.start.byte()..self.end.byte()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_revisions_never_repeat() {
        let a = Revision::fresh();
        let b = Revision::fresh();
        assert_ne!(a, b);
    }

    #[test]
    fn column_round_trips() {
        assert_eq!(Column::from(7).get(), 7);
        assert_eq!(Column::ZERO.get(), 0);
    }
}
