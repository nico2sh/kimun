//! When two clicks are one gesture.
//!
//! Following a link with the mouse is a double-click rather than a
//! modifier-click, because no modifier-click survives all three platforms —
//! see `adr/0043`. This is the rule for what counts as one.
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

use std::time::{Duration, Instant};

/// How close together two clicks on a cell have to be to read as one gesture.
///
/// Shorter than `TypingRun::IDLE`, which measures something else: that is a
/// *pause* (how long before a human stops feeling they are still typing), this
/// is a *deliberate burst*. macOS and Windows default to 500ms, GNOME to 400ms.
pub const WINDOW: Duration = Duration::from_millis(400);

/// The clicks currently forming one gesture.
#[derive(Debug, Default)]
pub struct ClickRun {
    last: Option<(u16, u16, Instant)>,
}

impl ClickRun {
    /// Whether a press on `(column, row)` at `now` completes a double-click,
    /// recording it either way.
    pub fn completes_double(&mut self, column: u16, row: u16, now: Instant) -> bool {
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
    /// Pointer *motion* is deliberately not one of them. `EnableMouseCapture`
    /// turns on any-event tracking (`?1003h`), so a `Moved` arrives for every
    /// pixel of travel; ending the run on those would mean no double-click
    /// ever completes. Drift is already handled by requiring the same cell.
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
