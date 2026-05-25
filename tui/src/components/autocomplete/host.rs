use std::ops::Range;

/// What the controller needs to know about the input surface (editor or
/// search box) it is layered on.
///
/// All offsets are **byte offsets** into `buffer_text()`. `screen_anchor_for`
/// converts a byte offset to a `(col, row)` cell position so the popup can
/// render adjacent to the trigger character; returning `None` means the
/// trigger byte is currently scrolled off-screen, in which case the
/// controller hides the popup until it scrolls back into view.
pub trait AutocompleteHost {
    /// The full buffer text the user is editing. Allocates on each call;
    /// the controller calls this at most once per keystroke.
    fn buffer_text(&self) -> String;

    /// Current cursor position as a byte offset into `buffer_text()`.
    fn cursor_byte_offset(&self) -> usize;

    /// Overwrite `range` with `new_text` and place the cursor at the
    /// absolute byte offset `new_cursor_byte` in the updated buffer.
    ///
    /// The host is responsible for any backend-specific conversion
    /// (`ratatui_textarea::TextArea` exposes cursor as `(row, col)`, not
    /// bytes — the host translates).
    fn apply_replacement(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        new_cursor_byte: usize,
    );

    /// Screen position adjacent to the trigger character (the byte right
    /// after `[[` or `#`). Returned as `(col, row)` cells. The host returns
    /// `None` if that byte is currently off-screen due to scroll.
    fn screen_anchor_for(&self, byte_offset: usize) -> Option<(u16, u16)>;
}
