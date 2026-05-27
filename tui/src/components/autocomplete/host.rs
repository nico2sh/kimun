use crate::components::text_editor::treesitter_parser::EditorTree;

/// Read-only view of the input surface (editor or search box) for the
/// autocomplete controller. All offsets are **byte offsets** into
/// `buffer_text()`.
///
/// The trait is intentionally read-only: the controller computes what to
/// insert and returns an `AcceptAction` (see `controller`), and the host
/// applies it. That split keeps borrow-checker contention out of the way
/// when the controller is held as a field of the host itself.
pub trait AutocompleteHost {
    /// The full buffer text. Allocates on each call; the controller calls
    /// this at most once per keystroke.
    fn buffer_text(&self) -> String;

    /// Cursor position as a byte offset into `buffer_text()`.
    fn cursor_byte_offset(&self) -> usize;

    /// Screen position adjacent to the trigger byte (the byte right after
    /// `[[` or `#`). Returned as `(col, row)` cells. `None` when the byte
    /// is currently off-screen; the controller hides the popup in that
    /// case.
    fn screen_anchor_for(&self, byte_offset: usize) -> Option<(u16, u16)>;

    /// Monotonic-on-text-change counter the host bumps every time
    /// `buffer_text()` would return different bytes. The controller uses
    /// this as the cache key for the joined buffer text + `ExclusionZones`,
    /// so cursor-only reconciles never repay the full-buffer pulldown-cmark
    /// parse + regex scans (nor the join itself).
    ///
    /// Return the literal `0` to opt out of the cache entirely — the
    /// controller treats `0` as an unconditional miss, so every reconcile
    /// rebuilds. Use this only for hosts whose buffer is tiny enough that
    /// the rebuild cost is negligible (e.g. a single-line search box).
    fn text_revision(&self) -> u64;

    /// Optional handle to the editor's incremental tree-sitter parse.
    /// Editor hosts return `Some(&tree)`; the search-box returns `None`
    /// because no tree exists for that input. The controller passes the
    /// tree to the trigger detector in editor mode, bypassing the
    /// pulldown-based `ExclusionZones::from_text` on the typing path.
    fn editor_tree(&self) -> Option<&EditorTree> {
        None
    }
}
