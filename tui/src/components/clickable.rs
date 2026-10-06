//! The one visual language for click targets: an **action** colour at rest
//! ([`Theme::action`]) and a **hover** highlight under the pointer
//! ([`Theme::hover`]). No underlines — a terminal draws them as a heavy rule
//! that reads as noise, not as "link".
//!
//! Rest styling is each widget's own (it picks `theme.action()` for the part
//! that is clickable). Hover is central: every widget that records a click
//! rect for its own hit-testing also [`register`]s it here while rendering,
//! and the screen calls [`apply_hover`] once at the end of the frame. So a
//! new target gets hover by registering — no per-widget pointer tracking.
//!
//! The registry is per frame and per thread (rendering is single-threaded;
//! tests on separate threads stay isolated). A modal overlay calls [`clear`]
//! before drawing itself: the panels under it are not clickable, so they
//! must not light up through it either.
//!
//! [`Theme::action`]: crate::settings::themes::Theme::action
//! [`Theme::hover`]: crate::settings::themes::Theme::hover

use std::cell::RefCell;

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

use crate::settings::themes::Theme;

thread_local! {
    static TARGETS: RefCell<Vec<Rect>> = const { RefCell::new(Vec::new()) };
}

/// Forget every registered target — at the start of a frame, and before a
/// modal layer draws over the rest.
pub fn clear() {
    TARGETS.with(|t| t.borrow_mut().clear());
}

/// Record a click target drawn this frame. Empty rects are ignored.
pub fn register(rect: Rect) {
    if !rect.is_empty() {
        TARGETS.with(|t| t.borrow_mut().push(rect));
    }
}

/// The target under (col,row) in a widget's own `(rect, target)` list —
/// the one hit-test every recorded-rect click target uses.
pub fn target_at<T: Copy>(targets: &[(Rect, T)], col: u16, row: u16) -> Option<T> {
    let pos = Position::new(col, row);
    targets
        .iter()
        .find(|(r, _)| r.contains(pos))
        .map(|(_, t)| *t)
}

/// An opaque layer was drawn over `rect` (the which-key panel, an
/// autocomplete popup): drop every target it covers, so hover cannot show
/// through it. A layer that has targets of its own registers them after.
pub fn occlude(rect: Rect) {
    TARGETS.with(|t| t.borrow_mut().retain(|r| !r.intersects(rect)));
}

/// The target under `pointer`, if any. Later registrations win: whatever
/// was drawn last is on top.
pub fn hovered(pointer: Position) -> Option<Rect> {
    TARGETS.with(|t| {
        t.borrow()
            .iter()
            .rev()
            .find(|r| r.contains(pointer))
            .copied()
    })
}

/// Paint the hover highlight over the target under `pointer` — the last
/// step of a frame, so it lands on top of whatever the widget drew.
pub fn apply_hover(buf: &mut Buffer, pointer: Option<Position>, theme: &Theme) {
    if let Some(rect) = pointer.and_then(hovered) {
        buf.set_style(rect.intersection(buf.area), theme.hover());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_registered_target_wins_and_clear_forgets() {
        clear();
        register(Rect::new(0, 0, 10, 1));
        register(Rect::new(5, 0, 2, 1));
        assert_eq!(hovered(Position::new(5, 0)), Some(Rect::new(5, 0, 2, 1)));
        assert_eq!(hovered(Position::new(1, 0)), Some(Rect::new(0, 0, 10, 1)));
        assert_eq!(hovered(Position::new(1, 3)), None);
        clear();
        assert_eq!(hovered(Position::new(1, 0)), None);
    }

    #[test]
    fn an_opaque_layer_hides_what_it_covers() {
        clear();
        register(Rect::new(0, 0, 4, 1));
        register(Rect::new(10, 0, 4, 1));
        occlude(Rect::new(2, 0, 3, 3));
        assert_eq!(hovered(Position::new(1, 0)), None, "covered target gone");
        assert!(hovered(Position::new(11, 0)).is_some(), "others stay");
        clear();
    }

    #[test]
    fn hover_restyles_only_the_target() {
        clear();
        let theme = Theme::gruvbox_dark();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
        register(Rect::new(2, 1, 3, 1));
        apply_hover(&mut buf, Some(Position::new(3, 1)), &theme);
        assert_eq!(buf[(2, 1)].bg, theme.hover().bg.unwrap());
        assert_ne!(buf[(1, 1)].bg, theme.hover().bg.unwrap());
        assert_ne!(buf[(2, 0)].bg, theme.hover().bg.unwrap());
        clear();
    }
}
