use std::num::NonZeroU64;

use crate::components::text_editor::snapshot::EditorSnapshot;

/// Read-only view of the input surface (editor or search box) for the
/// autocomplete controller. The controller derives byte offsets and
/// the joined buffer text from the snapshot — neither needs to be
/// pre-computed by the host.
///
/// The trait is intentionally read-only: the controller computes what
/// to insert and returns an `AcceptAction` (see `controller`), and the
/// host applies it. That split keeps borrow-checker contention out of
/// the way when the controller is held as a field of the host itself.
pub trait AutocompleteHost {
    /// Atomic borrow of `(lines, cursor, content_revision)`. The
    /// snapshot's lifetime ties to `&self`, so the editor's textarea
    /// or the search-box buffer is borrowed without an intermediate
    /// `Vec<String>` clone in the common case — perf #8.
    fn buffer_snapshot(&self) -> EditorSnapshot<'_>;

    /// Host opt-in to the controller's per-text-revision cache for
    /// `(joined_text, ExclusionZones)`. Returning `None` makes every
    /// reconcile rebuild both; use for hosts whose buffer is tiny
    /// enough that the rebuild cost is negligible (e.g. the
    /// single-line search box). Hosts that participate must bump
    /// this value on every content change — sharing the editor's
    /// `content_revision` is the easy way (see `EditorSnapshot`).
    fn cache_key(&self) -> Option<NonZeroU64>;

    /// Screen position adjacent to the trigger byte (the byte right
    /// after `[[` or `#`). Returned as `(col, row)` cells. `None`
    /// when the byte is currently off-screen; the controller hides
    /// the popup in that case.
    fn screen_anchor_for(&self, byte_offset: usize) -> Option<(u16, u16)>;
}
