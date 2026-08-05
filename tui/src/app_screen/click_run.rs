//! When two clicks are one gesture.
//!
//! Following a link with the mouse is a double-click, not the Ctrl+click every
//! IDE uses, because no modifier-click survives all three platforms:
//!
//! - **Cmd+click cannot exist.** xterm mouse reporting encodes three modifier
//!   bits — shift, alt, control. There is no super bit for a terminal to set
//!   or a parser to read, and macOS terminals consume Cmd+click for their own
//!   URL opening anyway.
//! - **Ctrl+click is macOS's right-click**, synthesised by Terminal.app and
//!   iTerm2 *before* mouse reporting. The application receives a right press,
//!   which kimün already answers with the note context menu.
//! - **Shift+click never arrives.** Most terminals use shift to bypass
//!   application mouse reporting entirely.
//! - **Alt+click** works on Linux and Windows, and fails silently on macOS
//!   unless the user has turned on "Use Option as Meta key".
//!
//! A double-click needs no modifier bit and no terminal cooperation beyond the
//! mouse reporting kimün already requires, so it behaves the same everywhere.
//! Nothing is lost by not having a chord: the bound `FollowLink` shortcut
//! (Ctrl-N by default) follows from the keyboard, and the footer advertises it
//! whenever the cursor sits on a link.
//!
//! This module is the rule for what counts as one gesture.
//!
//! Two conditions, and the second is why the first can be strict:
//!
//! - **The same cell.** Not a neighbourhood. The terminal grid is coarse
//!   enough to absorb ordinary hand drift for free, and a tolerance would make
//!   two deliberate clicks on adjacent characters — nudging the cursor one
//!   column over — follow a link, which is the accident this rule exists to
//!   stop.
//! - **Within [`WINDOW`].** A burst, not a pause. Without it, any two clicks
//!   on a cell would pair up however far apart, and a link cell gets clicked
//!   repeatedly while its text is being edited.
//!
//! Firing ends the run: a third click must not pair with the second, because
//! by then the double has opened a different note under the pointer.
//!
//! Like [`TypingRun`](crate::components::text_editor::typing_run::TypingRun),
//! this holds no clock — the caller passes `now`, so a test describes a gap
//! instead of sleeping through one.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::time::{Duration, Instant};

/// How close together two clicks on a cell have to be to read as one gesture.
///
/// Shorter than `TypingRun::IDLE`, which measures something else: that is a
/// *pause* (how long before a human stops feeling they are still typing), this
/// is a *deliberate burst*. macOS and Windows default to 500ms, GNOME to 400ms.
const WINDOW: Duration = Duration::from_millis(400);

/// The clicks currently forming one gesture.
#[derive(Debug, Default)]
pub struct ClickRun {
    last: Option<(u16, u16, Instant)>,
}

impl ClickRun {
    /// Feed one mouse event to the run and report whether it completes a
    /// double-click. `in_editor` is the screen's answer to "did this land in
    /// the editor column" — the run does no hit-testing of its own.
    ///
    /// The policy lives here rather than in `EditorScreen` so it can be tested
    /// against a real event sequence. It could not be, and a release ending
    /// the run went unnoticed: every press is followed by one.
    pub fn observe(&mut self, event: &MouseEvent, in_editor: bool, now: Instant) -> bool {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if in_editor => {
                self.completes_double(event.column, event.row, now)
            }
            // Neither is a new action, and ending the run on either would mean
            // no double-click ever completes — see [`Self::end`].
            MouseEventKind::Up(_) | MouseEventKind::Moved => false,
            _ => {
                self.end();
                false
            }
        }
    }

    /// Whether a press on `(column, row)` at `now` completes a double-click,
    /// recording it either way.
    fn completes_double(&mut self, column: u16, row: u16, now: Instant) -> bool {
        let doubled = match self.last {
            Some((c, r, at)) => c == column && r == row && now.duration_since(at) < WINDOW,
            None => false,
        };
        // A fired double starts over rather than becoming the first half of the
        // next one — see the module note on the third click.
        self.last = if doubled {
            None
        } else {
            Some((column, row, now))
        };
        doubled
    }

    /// End the run: the next press starts a new gesture.
    ///
    /// Called for everything that is not a press in the editor — a key, a
    /// paste, a drag, a scroll, a press on another panel or on the divider.
    ///
    /// Two mouse events are deliberately *not* among them, both because they
    /// are noise rather than actions, and ending the run on either would mean
    /// no double-click could ever complete:
    ///
    /// - **The release.** Every press is followed by one. SGR reports any
    ///   release as button 3, so crossterm surfaces it as `Up(Left)` whichever
    ///   button was let go — the button carries no information and the event
    ///   is simply the tail of the press before it.
    /// - **Pointer motion.** `EnableMouseCapture` turns on any-event tracking
    ///   (`?1003h`), so a `Moved` arrives for every pixel of travel. Drift is
    ///   already handled by requiring the same cell.
    ///
    /// A scroll, by contrast, must end it: the viewport moved, so the same
    /// cell now shows different text, and a second press there would follow a
    /// link the user never saw.
    pub fn end(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(millis: u64) -> Instant {
        // A fixed origin so the tests describe gaps rather than wall-clock time.
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(millis)
    }

    #[test]
    fn one_press_is_not_a_double() {
        let mut run = ClickRun::default();
        assert!(!run.completes_double(4, 2, at(0)));
    }

    #[test]
    fn two_quick_presses_on_a_cell_are_a_double() {
        let mut run = ClickRun::default();
        run.completes_double(4, 2, at(0));
        assert!(run.completes_double(4, 2, at(120)));
    }

    #[test]
    fn a_slow_second_press_is_a_fresh_first() {
        let mut run = ClickRun::default();
        run.completes_double(4, 2, at(0));
        assert!(
            !run.completes_double(4, 2, at(WINDOW.as_millis() as u64)),
            "a gap of exactly the window is already too long"
        );
        assert!(
            run.completes_double(4, 2, at(WINDOW.as_millis() as u64 + 100)),
            "and it counts as the first of the next pair"
        );
    }

    #[test]
    fn a_neighbouring_cell_is_a_different_target() {
        let mut run = ClickRun::default();
        run.completes_double(4, 2, at(0));
        assert!(
            !run.completes_double(5, 2, at(50)),
            "one column over is a second cursor placement, not a double-click"
        );
        run.completes_double(4, 2, at(100));
        assert!(!run.completes_double(4, 3, at(150)), "nor is one row down");
    }

    #[test]
    fn a_third_press_does_not_pair_with_the_second() {
        let mut run = ClickRun::default();
        run.completes_double(4, 2, at(0));
        assert!(run.completes_double(4, 2, at(100)));
        assert!(
            !run.completes_double(4, 2, at(200)),
            "the double already fired and opened something else under the pointer"
        );
    }

    fn ev(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 4,
            row: 2,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        }
    }

    fn press() -> MouseEvent {
        ev(MouseEventKind::Down(MouseButton::Left))
    }

    /// The sequence a real double-click actually produces. A press is always
    /// followed by its release, and SGR reports every release as button 3 —
    /// which crossterm surfaces as `Up(Left)` whatever was let go. Treating
    /// that as "something else happened" ended the run between the two
    /// presses, so no double-click could ever complete.
    #[test]
    fn a_release_does_not_end_the_run() {
        let mut run = ClickRun::default();
        assert!(!run.observe(&press(), true, at(0)));
        assert!(!run.observe(&ev(MouseEventKind::Up(MouseButton::Left)), true, at(10)));
        assert!(
            run.observe(&press(), true, at(120)),
            "the release between the two presses is part of the gesture, not an interruption"
        );
    }

    /// Motion is exempt for the same reason and a different cause: any-event
    /// tracking means a `Moved` per pixel of travel.
    #[test]
    fn motion_does_not_end_the_run() {
        let mut run = ClickRun::default();
        run.observe(&press(), true, at(0));
        run.observe(&ev(MouseEventKind::Moved), true, at(30));
        assert!(run.observe(&press(), true, at(120)));
    }

    /// A scroll moves the viewport, so the same cell now shows different text.
    /// A second press there would follow a link the user never saw.
    #[test]
    fn a_scroll_ends_the_run() {
        let mut run = ClickRun::default();
        run.observe(&press(), true, at(0));
        run.observe(&ev(MouseEventKind::ScrollDown), true, at(30));
        assert!(!run.observe(&press(), true, at(120)));
    }

    /// A drag is a selection, not half of a double-click.
    #[test]
    fn a_drag_ends_the_run() {
        let mut run = ClickRun::default();
        run.observe(&press(), true, at(0));
        run.observe(&ev(MouseEventKind::Drag(MouseButton::Left)), true, at(30));
        assert!(!run.observe(&press(), true, at(120)));
    }

    /// The screen decides what "in the editor" means; the run only obeys it.
    /// A press on the drawer or the divider is not half of an editor gesture.
    #[test]
    fn a_press_outside_the_editor_ends_the_run() {
        let mut run = ClickRun::default();
        run.observe(&press(), true, at(0));
        run.observe(&press(), false, at(30));
        assert!(!run.observe(&press(), true, at(120)));
    }

    /// Another button is a deliberate other action — the right-click context
    /// menu, most obviously.
    #[test]
    fn another_button_ends_the_run() {
        let mut run = ClickRun::default();
        run.observe(&press(), true, at(0));
        run.observe(&ev(MouseEventKind::Down(MouseButton::Right)), true, at(30));
        assert!(!run.observe(&press(), true, at(120)));
    }

    #[test]
    fn anything_else_ends_the_run() {
        let mut run = ClickRun::default();
        run.completes_double(4, 2, at(0));
        run.end();
        assert!(
            !run.completes_double(4, 2, at(50)),
            "a key, a drag or a scroll between the two presses separates them"
        );
    }
}
