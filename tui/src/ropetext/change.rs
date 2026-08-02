//! What an edit did, told rather than diffed.

use std::ops::Range;

use crate::ropetext::position::Revision;

/// One replaced region, in the coordinates of the text *after* the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    inserted: Range<usize>,
    removed_bytes: usize,
}

impl Edit {
    pub(crate) fn new(inserted: Range<usize>, removed_bytes: usize) -> Self {
        Self {
            inserted,
            removed_bytes,
        }
    }

    /// Byte range the new text occupies, in the new text.
    pub fn inserted(&self) -> Range<usize> {
        self.inserted.clone()
    }

    /// How many bytes stood there before.
    pub fn removed_bytes(&self) -> usize {
        self.removed_bytes
    }
}

/// The outcome of a committed transaction, an undo, or a redo.
///
/// A consumer holding a derived cache — a parse, a layout — is *told* what moved
/// and re-does that much. Nothing has to compare two buffers to find out, which
/// is the cost this type exists to remove, and nothing has to guess how far the
/// damage spread, which is the correctness problem underneath that cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    revision: Revision,
    edits: Option<Vec<Edit>>,
    rows: Range<usize>,
    line_delta: isize,
}

impl Change {
    pub(crate) fn new(
        revision: Revision,
        edits: Option<Vec<Edit>>,
        rows: Range<usize>,
        line_delta: isize,
    ) -> Self {
        Self {
            revision,
            edits,
            rows,
            line_delta,
        }
    }

    /// Which state of the text this change produced.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The replaced regions, in the new text's coordinates.
    ///
    /// `None` when the change cannot be described that precisely: undoing or
    /// redoing a group that several transactions were folded into means several
    /// coordinate spaces, and a list of ranges that were each correct at a
    /// different moment is worse than no list at all. A committed transaction
    /// always reports its edits; only a coalesced group's undo declines to.
    /// [`Self::rows`] is always populated.
    pub fn edits(&self) -> Option<&[Edit]> {
        self.edits.as_deref()
    }

    /// Rows of the new text whose content is not what it was, so a consumer
    /// holding a derived cache re-derives that much. Rows *after* it are
    /// unchanged in content but have moved by [`Self::line_delta`].
    ///
    /// Exact for a single transaction. Widened, never narrowed, for a coalesced
    /// group — over-reporting costs work, under-reporting costs correctness.
    pub fn rows(&self) -> Range<usize> {
        self.rows.clone()
    }

    /// How the text's row count changed. Positive when rows were added.
    pub fn line_delta(&self) -> isize {
        self.line_delta
    }

    /// Whether the damage reaches past a single row.
    ///
    /// The named form of `rows().len() > 1 || line_delta() != 0`, for the caches
    /// whose cheap path is only valid for a change confined to one row.
    pub fn is_bulk(&self) -> bool {
        self.line_delta != 0 || self.rows.end.saturating_sub(self.rows.start) > 1
    }
}
