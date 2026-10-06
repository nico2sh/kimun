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
//! tests on separate threads stay isolated). Every screen that draws click
//! targets calls [`clear`] first thing in its `render`, and a modal overlay
//! calls it again before drawing itself: the panels under it are not
//! clickable, so they must not light up through it either.
//!
//! [`Theme::action`]: crate::settings::themes::Theme::action
//! [`Theme::hover`]: crate::settings::themes::Theme::hover

use std::cell::RefCell;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
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

/// Drops the tail of the gesture that opened a modal layer: the second
/// press of a double-click on whatever opened it. Without this, a popup that
/// closes on a press outside it would open and close in one double-click,
/// and a dialog could take a press aimed at what was under it.
///
/// Armed only when the layer was opened by a mouse press — a keyboard open
/// is not a gesture with a tail — and only for [`DOUBLE_CLICK`] after.
///
/// [`DOUBLE_CLICK`]: crate::components::DOUBLE_CLICK
#[derive(Debug, Default)]
pub struct OpenGuard {
    armed_at: Option<std::time::Instant>,
    last_input_was_press: bool,
}

impl OpenGuard {
    /// Record each input before it is handled: an open that follows a
    /// mouse press (directly, or via the app message it sent) arms.
    pub fn observe_input(&mut self, event: &crate::components::events::InputEvent) {
        self.last_input_was_press = matches!(
            event,
            crate::components::events::InputEvent::Mouse(m)
                if matches!(m.kind, ratatui::crossterm::event::MouseEventKind::Down(_))
        );
    }

    /// A layer opened at `now`.
    pub fn opened(&mut self, now: std::time::Instant) {
        self.armed_at = self.last_input_was_press.then_some(now);
    }

    /// The layer closed.
    pub fn closed(&mut self) {
        self.armed_at = None;
    }

    /// Whether `event` is the opening gesture's tail, to be dropped.
    pub fn swallows(
        &self,
        event: &crate::components::events::InputEvent,
        now: std::time::Instant,
    ) -> bool {
        matches!(
            event,
            crate::components::events::InputEvent::Mouse(m)
                if matches!(m.kind, ratatui::crossterm::event::MouseEventKind::Down(_))
        ) && self
            .armed_at
            .is_some_and(|at| now.duration_since(at) < crate::components::DOUBLE_CLICK)
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

// ── Hit-testing primitives ──────────────────────────────────────────────────

/// A left-button press — the one mouse kind dialogs act on.
pub fn is_left_press(m: &MouseEvent) -> bool {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
}

/// The key of the `(rect, key)` target under a left press — for click
/// targets that are not chips (menu actions, launcher rows).
pub fn key_at(targets: &[(Rect, KeyCode)], m: &MouseEvent) -> Option<KeyEvent> {
    if !is_left_press(m) {
        return None;
    }
    target_at(targets, m.column, m.row).map(KeyEvent::from)
}

/// The index of the ratatui `List` row under a left press: `rect` is where
/// the rows are drawn (inside any border), `offset` the list state's first
/// visible index, `len` the item count.
pub fn list_index_at(m: &MouseEvent, rect: Rect, offset: usize, len: usize) -> Option<usize> {
    if !is_left_press(m) || !rect.contains(Position::new(m.column, m.row)) {
        return None;
    }
    let idx = offset + (m.row - rect.y) as usize;
    (idx < len).then_some(idx)
}

/// A press of any button outside `rect` — how a read-only popup is dismissed.
/// An empty `rect` (not rendered yet) is never "outside".
pub fn is_press_outside(m: &MouseEvent, rect: Rect) -> bool {
    use ratatui::crossterm::event::MouseEventKind;
    matches!(m.kind, MouseEventKind::Down(_))
        && !rect.is_empty()
        && !rect.contains(Position::new(m.column, m.row))
}

/// Which border a [`BorderChip`] sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

/// A click target drawn as a right-aligned title on a block's border — the
/// sort label (`Name ↑`), `[F1] Syntax`. One place for its geometry,
/// hover registration and hit-test, so the border chips cannot drift.
///
/// Ratatui stacks right-aligned titles leftward in the order they are
/// added, so place a chip *after* any other right-aligned title on the
/// same edge: its rect assumes it is the rightmost.
#[derive(Debug, Default)]
pub struct BorderChip {
    rect: Option<Rect>,
}

impl BorderChip {
    /// Put `line` on `block`'s `edge` border (`area` is where the block is
    /// drawn), record and register its rect, and return the block.
    pub fn place<'a>(
        &mut self,
        block: ratatui::widgets::Block<'a>,
        area: Rect,
        edge: Edge,
        line: ratatui::text::Line<'a>,
    ) -> ratatui::widgets::Block<'a> {
        // Ratatui keeps the corner cells clear of titles.
        let inner_w = area.width.saturating_sub(2);
        let w = (line.width() as u16).min(inner_w);
        let fits = w > 0 && (edge == Edge::Top || area.height >= 2);
        self.rect = fits.then(|| {
            let y = match edge {
                Edge::Top => area.y,
                Edge::Bottom => area.bottom() - 1,
            };
            Rect::new(area.right() - 1 - w, y, w, 1)
        });
        let Some(r) = self.rect else {
            return block;
        };
        register(r);
        let line = line.right_aligned();
        match edge {
            Edge::Top => block.title(line),
            Edge::Bottom => block.title_bottom(line),
        }
    }

    /// Not drawn this frame: nothing to click.
    pub fn hide(&mut self) {
        self.rect = None;
    }

    /// A left press on the chip as last drawn.
    pub fn hit(&self, m: &MouseEvent) -> bool {
        is_left_press(m)
            && self
                .rect
                .is_some_and(|r| r.contains(Position::new(m.column, m.row)))
    }

    #[cfg(test)]
    pub(crate) fn rect(&self) -> Option<Rect> {
        self.rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_border_chip_sits_rightmost_on_its_edge() {
        clear();
        let area = Rect::new(0, 0, 20, 3);
        let mut chip = BorderChip::default();
        let _ = chip.place(
            ratatui::widgets::Block::default(),
            area,
            Edge::Bottom,
            ratatui::text::Line::from("[F1] x"),
        );
        assert_eq!(chip.rect(), Some(Rect::new(13, 2, 6, 1)));
        assert!(chip.hit(&crate::test_support::left_press(14, 2)));
        assert!(!chip.hit(&crate::test_support::left_press(14, 1)));
        assert!(
            hovered(Position::new(14, 2)).is_some(),
            "registered for hover"
        );
        chip.hide();
        assert!(!chip.hit(&crate::test_support::left_press(14, 2)));
        clear();
    }

    #[test]
    fn outside_press_needs_a_rendered_rect() {
        let rect = Rect::new(10, 10, 5, 5);
        assert!(is_press_outside(
            &crate::test_support::left_press(0, 0),
            rect
        ));
        assert!(!is_press_outside(
            &crate::test_support::left_press(11, 11),
            rect
        ));
        assert!(!is_press_outside(
            &crate::test_support::left_press(0, 0),
            Rect::default()
        ));
    }

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
    fn the_open_guard_arms_only_for_mouse_opens() {
        use crate::components::events::InputEvent;
        let press = crate::test_support::mouse_down_at(1, 1);
        let key = InputEvent::Key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Enter,
        ));
        let t0 = std::time::Instant::now();
        let mut g = OpenGuard::default();

        g.observe_input(&key);
        g.opened(t0);
        assert!(!g.swallows(&press, t0), "a keyboard open has no tail");

        g.observe_input(&press);
        g.opened(t0);
        assert!(g.swallows(&press, t0 + crate::components::DOUBLE_CLICK / 2));
        assert!(!g.swallows(&key, t0), "keys are never held back");
        assert!(!g.swallows(&press, t0 + crate::components::DOUBLE_CLICK));
        g.closed();
        assert!(!g.swallows(&press, t0));
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
