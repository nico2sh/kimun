//! When one keystroke continues the last one's **undo group**.
//!
//! The engine holds no clock and no policy (adr/0041): it offers a group that can
//! span keystrokes, and the backend says where one ends. This is the **plain**
//! backend's answer. The **vim** backend has a different one — a group is an
//! Insert session — which is why the rule lives here rather than in the buffer.
//!
//! Two rules, because each covers the other's blind spot:
//!
//! - **A word boundary ends a run.** Undo then takes back a word, which is how
//!   people describe what they typed. Whitespace attaches to the word it follows,
//!   so typing `hello world` leaves `hello ` and `world` — undo removes `world`,
//!   then `hello `.
//! - **A pause ends a run.** A gap mid-word is a new thought, and resuming should
//!   not undo back through it.
//!
//! Insert runs and delete runs never merge. Typing a word and then backspacing to
//! fix it is two actions, and one undo should not take back both.
//!
//! No timer is needed. Time only has to be read when a key arrives, and every
//! non-typing action — a motion, a click, a save, an undo — ends the run
//! explicitly. Undo is itself one of those, so by the time anyone can observe the
//! grouping, the pause has already been accounted for.

use std::time::{Duration, Instant};

/// How long a gap has to be before the next keystroke starts a new group.
///
/// Below about half a second this fires mid-word for ordinary typing; much above
/// a second and a genuine pause stops registering as one.
pub const IDLE: Duration = Duration::from_millis(750);

/// What a keystroke did to the text, for deciding whether the next one belongs
/// with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stroke {
    /// A character was typed.
    Insert(char),
    /// Text was removed — backspace, or a forward delete.
    Delete,
}

/// The run of keystrokes currently sharing an undo group.
#[derive(Debug, Default)]
pub struct TypingRun {
    last: Option<(Stroke, Instant)>,
}

impl TypingRun {
    /// Whether `stroke` at `now` continues the run, and record it either way.
    ///
    /// The caller passes the time rather than this reading a clock, so a test can
    /// describe a pause instead of sleeping through one.
    pub fn continues(&mut self, stroke: Stroke, now: Instant) -> bool {
        let carries_on = match self.last {
            Some((previous, at)) => now.duration_since(at) < IDLE && follows(previous, stroke),
            None => false,
        };
        self.last = Some((stroke, now));
        carries_on
    }

    /// Whether `stroke` continues a run that neither a word boundary nor a pause
    /// may break — vim's Insert session, which `u` undoes whole.
    ///
    /// Still not the *first* stroke of one: the session's start is marked by
    /// [`Self::end`], called when the mode changes.
    pub fn continues_session(&mut self, stroke: Stroke, now: Instant) -> bool {
        let carries_on = self.last.is_some();
        self.last = Some((stroke, now));
        carries_on
    }

    /// End the run: the next keystroke starts a new group.
    ///
    /// Called for everything that is not typing — a cursor move, a click, a
    /// paste, a save, an undo.
    pub fn end(&mut self) {
        self.last = None;
    }
}

/// Whether `next` belongs with `previous`.
fn follows(previous: Stroke, next: Stroke) -> bool {
    match (previous, next) {
        (Stroke::Delete, Stroke::Delete) => true,
        (Stroke::Insert(before), Stroke::Insert(now)) => {
            // A new word begins a new group; whitespace stays with the word it
            // follows, so the break lands between `hello ` and `world`.
            !(before.is_whitespace() && !now.is_whitespace())
        }
        // Typing and deleting are different actions.
        _ => false,
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
    fn the_first_keystroke_starts_a_group() {
        let mut run = TypingRun::default();
        assert!(!run.continues(Stroke::Insert('a'), at(0)));
    }

    #[test]
    fn typing_on_continues() {
        let mut run = TypingRun::default();
        run.continues(Stroke::Insert('h'), at(0));
        assert!(run.continues(Stroke::Insert('e'), at(50)));
        assert!(run.continues(Stroke::Insert('y'), at(100)));
    }

    #[test]
    fn a_new_word_starts_a_new_group() {
        let mut run = TypingRun::default();
        for (index, c) in "hello".chars().enumerate() {
            run.continues(Stroke::Insert(c), at(index as u64 * 50));
        }
        assert!(
            run.continues(Stroke::Insert(' '), at(300)),
            "the space belongs with the word it follows"
        );
        assert!(
            !run.continues(Stroke::Insert('w'), at(350)),
            "the next word is a new group"
        );
    }

    #[test]
    fn a_pause_ends_a_run() {
        let mut run = TypingRun::default();
        run.continues(Stroke::Insert('a'), at(0));
        assert!(run.continues(Stroke::Insert('b'), at(700)));
        assert!(
            !run.continues(Stroke::Insert('c'), at(700 + IDLE.as_millis() as u64)),
            "a gap of exactly the threshold is already too long"
        );
    }

    #[test]
    fn deletes_coalesce_among_themselves() {
        let mut run = TypingRun::default();
        run.continues(Stroke::Delete, at(0));
        assert!(run.continues(Stroke::Delete, at(50)));
    }

    #[test]
    fn typing_and_deleting_never_merge() {
        let mut run = TypingRun::default();
        run.continues(Stroke::Insert('a'), at(0));
        assert!(
            !run.continues(Stroke::Delete, at(50)),
            "fixing what you typed is a second action"
        );
        assert!(
            !run.continues(Stroke::Insert('b'), at(100)),
            "and typing again is a third"
        );
    }

    #[test]
    fn a_session_ignores_boundaries_and_pauses() {
        // Vim's `u` takes back a whole Insert session, spaces and thinking time
        // included.
        let mut run = TypingRun::default();
        assert!(
            !run.continues_session(Stroke::Insert('h'), at(0)),
            "the first"
        );
        assert!(run.continues_session(Stroke::Insert('i'), at(50)));
        assert!(
            run.continues_session(Stroke::Insert(' '), at(10_000)),
            "a long pause does not break a session"
        );
        assert!(
            run.continues_session(Stroke::Insert('t'), at(10_050)),
            "nor does a word boundary"
        );
        assert!(
            run.continues_session(Stroke::Delete, at(10_100)),
            "nor does backspacing to fix a typo mid-session"
        );
    }

    #[test]
    fn anything_else_ends_the_run() {
        let mut run = TypingRun::default();
        run.continues(Stroke::Insert('a'), at(0));
        run.end();
        assert!(
            !run.continues(Stroke::Insert('b'), at(50)),
            "a motion between two keystrokes separates them"
        );
    }
}
