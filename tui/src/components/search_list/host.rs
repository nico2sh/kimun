//! `SearchBoxHostSnapshot` — canonical `AutocompleteHost` implementation for
//! the single-line search-query input used by `SearchList`.

use std::num::NonZeroU64;

use crate::components::autocomplete::AutocompleteHost;

/// Snapshot of the search input that satisfies `AutocompleteHost`.
/// Owned so the controller's borrow doesn't overlap with the search
/// input's `&mut` borrow during key handling and replacement. Holds
/// a single-row `Vec<String>` because `EditorSnapshot` borrows a
/// slice of lines — the search-box buffer is the one row.
pub(super) struct SearchBoxHostSnapshot {
    pub(super) lines: Vec<String>,
    /// Cursor as `(row, char_col)` — row is always 0 for the
    /// single-line search box; char_col derived from the byte
    /// cursor returned by the input widget.
    pub(super) cursor: (usize, usize),
    pub(super) caret_pos: Option<(u16, u16)>,
}

impl AutocompleteHost for SearchBoxHostSnapshot {
    fn buffer_snapshot(&self) -> crate::components::text_editor::snapshot::EditorSnapshot<'_> {
        // content_revision unused (cache_key returns None); supply a
        // placeholder so the field stays NonZeroU64.
        let dummy = NonZeroU64::new(1).unwrap();
        crate::components::text_editor::snapshot::EditorSnapshot::borrowed(
            &self.lines,
            self.cursor,
            dummy,
        )
    }
    fn cache_key(&self) -> Option<std::num::NonZeroU64> {
        // `None` opts out of the controller's per-buffer cache. The
        // search-box buffer is single-line and short, so the rebuild
        // cost per keystroke is negligible — opting out keeps the
        // modal free of per-keystroke revision bookkeeping.
        None
    }
    fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
        // Anchor at the caret — same liberty as the editor host. The
        // popup sits adjacent to the typed text either way.
        // Fall back to (0, 0) when no render has occurred yet (headless
        // unit tests and first-frame construction before any render).
        Some(self.caret_pos.unwrap_or((0, 0)))
    }
}
