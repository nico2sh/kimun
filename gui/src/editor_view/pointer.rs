//! Presses, drags, releases and wheel steps, and the double-click rule that
//! follows a link (ADR-0043). It also drives the read-only drag and the
//! release that follows a link. The read-only press itself is in the
//! [`mod@super::read_only`] module.

use std::time::{Duration, Instant};

use gpui_kit::{
    Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollEvent,
    Window,
};
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::editor::find_bar::BarFocus;
use kimun_notes::logic::input::{Button, Key, KeyInput, Mods, PointerInput, PointerKind};
use kimun_notes::logic::message::AppMessage;

use super::{EditorEvent, EditorView};
use crate::grid::CellMetrics;
use crate::input;
use crate::keymap;

/// How close together two presses on a cell must be to follow (ADR-0043;
/// the TUI's `DOUBLE_CLICK`).
pub(crate) const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The presses forming one double-click (ADR-0043): two plain left presses on
/// the same cell within [`DOUBLE_CLICK`]. Not a neighbourhood, so two clicks
/// on adjacent chars never follow; a fired double starts over, so a third
/// press does not pair with the second. Holds no clock: the caller passes
/// `now`. GPUI's own `click_count` is not used: its timing and slop are the
/// platform's, not this rule.
#[derive(Debug, Default)]
pub(crate) struct ClickRun {
    last: Option<(u16, u16, Instant)>,
}

impl ClickRun {
    /// Whether a press on (`col`, `row`) at `now` completes a double-click,
    /// recording it either way.
    pub(crate) fn completes_double(&mut self, col: u16, row: u16, now: Instant) -> bool {
        let doubled = self.last.is_some_and(|(c, r, at)| {
            c == col && r == row && now.saturating_duration_since(at) < DOUBLE_CLICK
        });
        self.last = (!doubled).then_some((col, row, now));
        doubled
    }

    /// The next press starts a new gesture: a key, a scroll, another
    /// button, a modifier held, a press off the text.
    pub(crate) fn end(&mut self) {
        self.last = None;
    }
}

/// A clock that never moves (tests): every press happens at the same instant.
#[cfg(test)]
pub(crate) fn frozen_clock() -> Instant {
    static AT: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *AT.get_or_init(Instant::now)
}

impl EditorView {
    /// Run the double-click rule on `clock` instead of the real one (tests).
    #[cfg(test)]
    pub(crate) fn set_clock(&mut self, clock: fn() -> Instant) {
        self.clock = clock;
    }

    pub(crate) fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.pressed_at = event.position;
        // A press on the text (the popup's own rows never get here) dismisses
        // the popup.
        if let Some(engine) = self.autocomplete.as_mut() {
            engine.close();
        }
        let Some(pointer) = input::pointer_down(event, origin, cells) else {
            return;
        };
        // The find bar and the mode line are not text: a press on them only
        // focuses (and, on the bar's other field, moves the keyboard there).
        // Without either (plain mode) a press below the last full row still
        // reaches the editor, as in M1.
        if let Some(p) = &self.painted
            && (p.mode_line.is_some() || p.find_bar.is_some())
            && pointer.row >= p.rows
        {
            self.clicks.end();
            if pointer.kind == PointerKind::Down(Button::Left) {
                self.focus_bar_row(pointer.row - p.rows);
                self.settle(window, cx);
            }
            return;
        }
        let left = pointer.kind == PointerKind::Down(Button::Left);
        if self.read_only {
            self.read_only_press(pointer, left, window, cx);
            return;
        }
        // A right press with no selection on a follow target asks for the
        // Link menu instead of the note menu. The caret stays: the target is
        // read from the cell, not from the caret. With a selection, or off
        // any target, the editor handles it below (copy, or the note menu).
        if let Some(target) = self.link_menu_target(&pointer) {
            self.clicks.end();
            // A right press is an action, as in `handle_pointer`: it ends
            // the typing run.
            self.editor.interrupt_typing();
            self.dragging = false;
            cx.emit(EditorEvent::LinkMenu {
                at: event.position,
                target,
            });
            self.settle(window, cx);
            return;
        }
        // A press on a link (the hand cursor) only places the caret, as the
        // TUI's first press does. The primary modifier (Cmd on macOS, Ctrl
        // elsewhere, as the keymap's `primary`; Ctrl+click is a right-click on
        // a Mac) follows in one press: place the caret, then follow what is
        // under it. The find bar owns the pointer while open, so there it is
        // an ordinary press.
        if left && pointer.mods == keymap::primary(cx) && self.editor.find_bar().is_none() {
            self.clicks.end();
            self.feed_pointer(&pointer, true);
            // Stay armed even when it follows: the release must still reach
            // the editor to end the press `handle_pointer` started.
            self.follow_at_caret();
            self.dragging = true;
            self.settle(window, cx);
            return;
        }
        // A second plain left press on the same cell soon after follows the
        // **follow target** the first one put the caret on (ADR-0043): the
        // workspace does it, as for the chord. With nothing to follow there,
        // it is an ordinary press.
        if left && pointer.mods == Mods::NONE {
            if self
                .clicks
                .completes_double(pointer.col, pointer.row, (self.clock)())
                && self.follow_at_caret()
            {
                self.dragging = false;
                self.settle(window, cx);
                return;
            }
        } else {
            self.clicks.end();
        }
        self.feed_pointer(&pointer, left);
        self.dragging = left;
        self.settle(window, cx);
    }

    /// Hand `pointer` to the editor, then settle the editor after it.
    /// `settle` is true for a left press: it ends vim's Visual mode.
    pub(super) fn feed_pointer(&mut self, pointer: &PointerInput, settle: bool) {
        self.editor.handle_pointer(pointer, &self.tx);
        self.editor.settle_pointer(settle);
    }

    /// The follow target (a link, a label) drawn in `pointer`'s cell, or
    /// `None`. It reads the editor's last layout: the caret and the
    /// selection stay where they are, target or not.
    pub(crate) fn target_under(&mut self, pointer: &PointerInput) -> Option<FollowTarget> {
        self.editor
            .follow_target_at_screen(pointer.row, pointer.col)
    }

    /// The target a right press asks the Link menu for: the one under the
    /// pointer, when the press is a right press and nothing is selected (an
    /// empty selection counts as none, as in `Editor::handle_pointer`). The
    /// find bar owns the pointer while open, so there a press never acts on
    /// a link: no target, and the right press asks for the note menu.
    pub(super) fn link_menu_target(&mut self, pointer: &PointerInput) -> Option<FollowTarget> {
        let unselected = self
            .editor
            .selection()
            .is_none_or(|(start, end)| start == end);
        if pointer.kind != PointerKind::Down(Button::Right)
            || !unselected
            || self.editor.find_bar().is_some()
        {
            return None;
        }
        self.target_under(pointer)
    }

    /// Ask the window's owner to follow the follow target under the caret
    /// (a link, a label). False when there is none. On an **Inline image**'s
    /// line it asks to open the file the image draws: a bare `![[name]]`
    /// finds it by the closest-file rule, which the link lookup does not
    /// know.
    fn follow_at_caret(&self) -> bool {
        if let Some(path) = self.image_at_cursor() {
            self.tx.send(AppMessage::OpenAttachment(path)).ok();
            return true;
        }
        let message = match self.editor.follow_target_at_cursor() {
            Some(FollowTarget::Link { target, kind }) => AppMessage::FollowLink { target, kind },
            Some(FollowTarget::Label(name)) => AppMessage::FollowLabel(name),
            None => return false,
        };
        self.tx.send(message).ok();
        true
    }

    /// A press on find bar row `row` (0 the query, 1 the replacement) gives
    /// that field the keyboard. The bar's own Tab is the one way it moves
    /// focus, so the press uses it.
    fn focus_bar_row(&mut self, row: u16) {
        let Some(bar) = self.editor.find_bar() else {
            return;
        };
        let want = match row {
            0 => BarFocus::Find,
            1 if bar.is_replacing() => BarFocus::Replace,
            _ => return,
        };
        if bar.focus() != want {
            self.preedit = None;
            self.editor.handle_find_key(KeyInput::plain(Key::Tab));
        }
    }

    pub(crate) fn pointer_move(
        &mut self,
        event: &MouseMoveEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging {
            return;
        }
        // The release happened where we could not see it (outside the window).
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = false;
            self.clicks_ro.pressed = None;
            self.drop_unreachable_selection(window, cx);
            return;
        }
        let Some(pointer) = input::pointer_move(event, origin, cells) else {
            return;
        };
        self.feed_pointer(&pointer, false);
        // Off the pressed target, or moved past the slop: a drag, not a
        // click.
        if !self.on_pressed_target() || self.moved_past_slop(event.position) {
            self.clicks_ro.pressed = None;
        }
        self.settle(window, cx);
    }

    pub(crate) fn pointer_up(
        &mut self,
        event: &MouseUpEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging || event.button != MouseButton::Left {
            return;
        }
        self.dragging = false;
        let pressed = self.clicks_ro.pressed.take();
        if let Some(pointer) = input::pointer_up(event, origin, cells) {
            self.feed_pointer(&pointer, false);
            self.settle(window, cx);
            // A read-only click: released on the target it pressed.
            if let Some(pressed) = pressed
                && self.caret_on(&pressed)
            {
                self.clicks_ro
                    .followed_at(pressed.col, pressed.row, pressed.at);
                cx.emit(EditorEvent::ReadOnlyFollow {
                    target: pressed.target,
                });
            }
        }
        // The keyboard left during the drag: what it selected is out of reach.
        self.drop_unreachable_selection(window, cx);
    }

    pub(crate) fn scroll_wheel(
        &mut self,
        event: &ScrollEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let steps = input::pointer_scroll(event, origin, cells, &mut self.scroll);
        if steps.is_empty() {
            return;
        }
        // The same cell now shows other text.
        self.clicks.end();
        for step in &steps {
            self.editor.handle_pointer(step, &self.tx);
        }
        self.editor.settle_pointer(false);
        self.settle(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(millis: u64) -> Instant {
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(millis)
    }

    #[test]
    fn two_quick_presses_on_one_cell_are_a_double_and_end_the_run() {
        let mut run = ClickRun::default();
        assert!(!run.completes_double(4, 2, at(0)), "one press is not");
        assert!(run.completes_double(4, 2, at(399)));
        // A third press starts over rather than pairing with the second.
        assert!(!run.completes_double(4, 2, at(450)));
    }

    #[test]
    fn a_slow_pair_or_another_cell_is_no_double() {
        let mut run = ClickRun::default();
        assert!(!run.completes_double(4, 2, at(0)));
        assert!(
            !run.completes_double(4, 2, at(400)),
            "the window is exclusive"
        );
        assert!(!run.completes_double(5, 2, at(500)), "a neighbour cell");
        assert!(!run.completes_double(5, 3, at(600)), "another row");
        assert!(run.completes_double(5, 3, at(700)));
    }

    #[test]
    fn ending_the_run_forgets_the_first_press() {
        let mut run = ClickRun::default();
        assert!(!run.completes_double(1, 1, at(0)));
        run.end();
        assert!(!run.completes_double(1, 1, at(100)));
    }
}
