//! Glue between the autocomplete popup and the terminal: where the cursor is
//! on screen. The accept edit itself is
//! [`crate::logic::autocomplete::apply_accept`].

use ratatui::layout::Rect;

/// Cursor screen position given a `rect` (col, row in cells), or `None`
/// when the cursor is scrolled off-screen. The popup uses this as its
/// anchor — a small liberty over "just after the sigil" (we anchor
/// at the cursor, which sits at the end of the typed prefix), but the
/// popup ends up adjacent to the typed text either way.
pub fn cursor_screen_pos(
    rendered_col: usize,
    cursor_vrow: usize,
    visual_scroll_offset: usize,
    rect: Rect,
) -> Option<(u16, u16)> {
    if cursor_vrow < visual_scroll_offset {
        return None;
    }
    let vrow_in_view = cursor_vrow - visual_scroll_offset;
    if vrow_in_view as u16 >= rect.height {
        return None;
    }
    Some((rect.x + rendered_col as u16, rect.y + vrow_in_view as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_screen_pos_scrolled_off_top() {
        let rect = Rect::new(2, 5, 80, 24);
        assert!(cursor_screen_pos(0, 0, 5, rect).is_none());
    }

    #[test]
    fn cursor_screen_pos_scrolled_off_bottom() {
        let rect = Rect::new(0, 0, 80, 10);
        assert!(cursor_screen_pos(0, 100, 0, rect).is_none());
    }

    #[test]
    fn cursor_screen_pos_in_view() {
        let rect = Rect::new(2, 5, 80, 24);
        assert_eq!(cursor_screen_pos(7, 12, 10, rect), Some((9, 7)));
    }
}
