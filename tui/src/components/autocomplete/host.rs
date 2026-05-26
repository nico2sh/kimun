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

    /// Monotonic counter the host bumps every time `buffer_text()` would
    /// return different bytes. The controller uses this as the cache key
    /// for `ExclusionZones`, so cursor-only reconciles never repay the
    /// full-buffer pulldown-cmark parse + regex scans. Hosts with tiny or
    /// trivial buffers (e.g. a single-line search box) can return `0` to
    /// effectively disable the cache — the controller still works
    /// correctly because exclusion-zone checks are skipped there.
    fn text_revision(&self) -> u64;
}
