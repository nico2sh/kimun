//! What a bare `0x08` byte from the terminal means.
//!
//! Outside the kitty keyboard protocol a terminal has one byte for two keys.
//! `0x08` is what the Ctrl-H chord sends, and it is also what the Backspace
//! *key* sends on a terminal whose keytab (or `stty erase ^H`) is set to the
//! older of the two conventions — the other, and the common one today, being
//! `0x7F`. crossterm's legacy decoder breaks the tie one way: `0x7F` is the
//! only spelling of `KeyCode::Backspace`, while every byte in `0x01..=0x1A`
//! becomes that letter plus `CONTROL`. So on such a terminal Backspace
//! arrives as Ctrl-H and fires whatever chord Ctrl-H is bound to instead of
//! deleting a character.
//!
//! There is no side channel to tell the two apart — same byte, no timing
//! tell, nothing in the escape stream. Both keys cannot work at once, so this
//! module decides once at startup and rewrites the event at the input seam,
//! leaving everything downstream — the shortcut tier, the editor backends — to
//! see one unambiguous key.
//!
//! Where the terminal *can* tell them apart nothing is rewritten and both keys
//! work: under the kitty protocol (pushed in `terminal::TerminalSession` when
//! the terminal answers the capability query) the chord arrives as
//! `CSI 104;5u` and the key keeps `0x7F`. That is why
//! [`CtrlHPolicy::resolve`] takes whether those flags went out: the ambiguity
//! this module exists for is a property of the *session*, not of the user's
//! taste, and `auto` should not spend a working chord on a terminal that never
//! had the problem.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::settings::CtrlHSetting;

/// The resolved rule, consulted per key event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CtrlHPolicy {
    /// Leave `Ctrl+h` alone: the chord's binding fires, and a Backspace key
    /// that sends `0x08` cannot delete.
    #[default]
    Chord,
    /// Rewrite `Ctrl+h` to `Backspace`: the key deletes, and the chord is
    /// unreachable (rebind the action if you need it).
    Backspace,
}

impl CtrlHPolicy {
    /// Decide the rule for this session.
    ///
    /// `keyboard_enhanced` is whether the kitty keyboard-enhancement flags
    /// were pushed — when they were, the two keys are already distinct and
    /// `Auto` keeps the chord. The explicit settings are unconditional: a user
    /// who names one has said what they want the byte to mean, and a rule that
    /// quietly did nothing on some terminals would be worse than one that is
    /// simply obeyed.
    pub fn resolve(setting: CtrlHSetting, keyboard_enhanced: bool) -> Self {
        Self::resolve_with(setting, keyboard_enhanced, tty_erase_is_bs())
    }

    /// [`Self::resolve`] with the tty probe supplied rather than performed.
    ///
    /// Two callers need this. `kimun doctor` already knows the erase
    /// character and must not read it a second time — it declines to read it
    /// at all when its output is redirected, and a hidden probe inside here
    /// would contradict the disclaimer it prints. And the `Auto` branches are
    /// only testable at all once the probe is an argument: called for real,
    /// it reads the test runner's own stdin.
    pub fn resolve_with(
        setting: CtrlHSetting,
        keyboard_enhanced: bool,
        tty_erase_is_bs: bool,
    ) -> Self {
        let policy = match setting {
            CtrlHSetting::Chord => Self::Chord,
            CtrlHSetting::Backspace => Self::Backspace,
            CtrlHSetting::Auto if keyboard_enhanced => Self::Chord,
            CtrlHSetting::Auto if tty_erase_is_bs => Self::Backspace,
            CtrlHSetting::Auto => Self::Chord,
        };
        tracing::debug!(
            "ctrl_h: setting={setting:?} keyboard_enhanced={keyboard_enhanced} \
             tty_erase_is_bs={tty_erase_is_bs} -> {policy:?}"
        );
        policy
    }

    /// Whether losing the `Ctrl+H` chord is worth telling the user about.
    ///
    /// Only when they did not ask for it. `ctrl_h = "backspace"` is a choice
    /// made against a documented consequence — the setting's own
    /// documentation says the chord becomes unreachable — so reporting it
    /// every launch says nothing they do not know, and a warning that fires
    /// on a working, chosen setup is one people learn to ignore. Under `auto`
    /// the erase-character probe can reach the same policy without anyone
    /// choosing it, and there it is news.
    pub fn loss_is_worth_reporting(self, setting: CtrlHSetting) -> bool {
        self == Self::Backspace && setting != CtrlHSetting::Backspace
    }

    /// `key`, with `Ctrl+h` rewritten to `Backspace` when that is the rule.
    ///
    /// The modifier test is exact, so `Ctrl+Shift+H` and `Ctrl+Alt+H` — chords
    /// a terminal spells differently, and which no Backspace key sends — pass
    /// through untouched. `kind` and `state` are carried over: the rewrite
    /// changes which key this is, not whether it is a press.
    pub fn apply(self, key: KeyEvent) -> KeyEvent {
        if self == Self::Backspace
            && key.code == KeyCode::Char('h')
            && key.modifiers == KeyModifiers::CONTROL
        {
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
                ..key
            }
        } else {
            key
        }
    }
}

/// This tty's erase character, or `None` when it cannot be read (not a tty,
/// or not a platform with `termios`).
///
/// `0x08` here is the one signal available about which convention the user's
/// terminal is set up for: a `^H` Backspace key and `stty erase ^H` are halves
/// of the same old-school setup, and a terminal emulator is not obliged to
/// agree with the line discipline — so this catches the correlated case and no
/// more. Raw mode does not touch `c_cc[VERASE]`, so the answer is the same
/// before or after `enable_raw_mode`, which is what lets `kimun doctor` read
/// it outside the TUI.
///
/// Asked of the descriptor crossterm reads keys from: stdin when it is a
/// terminal, `/dev/tty` otherwise (its `tty_fd`). A launcher that redirects
/// stdin still gets an interactive TUI, and probing fd 0 there would answer
/// `None` for the very user the probe exists for.
#[cfg(unix)]
pub fn tty_erase_char() -> Option<u8> {
    use std::io::IsTerminal;
    use std::os::fd::AsFd;

    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return erase_char_of(stdin.as_fd());
    }
    let tty = std::fs::File::open("/dev/tty").ok()?;
    erase_char_of(tty.as_fd())
}

/// `VERASE` of one descriptor, or `None` when it is not a terminal.
#[cfg(unix)]
fn erase_char_of(fd: std::os::fd::BorrowedFd<'_>) -> Option<u8> {
    use rustix::termios::{SpecialCodeIndex, tcgetattr};
    let termios = tcgetattr(fd).ok()?;
    Some(termios.special_codes[SpecialCodeIndex::VERASE])
}

/// Windows has no `termios`, and no ambiguity to resolve: the console API
/// reports the Backspace key and the Ctrl-H chord as different events however
/// the terminal is configured.
#[cfg(not(unix))]
pub fn tty_erase_char() -> Option<u8> {
    None
}

fn tty_erase_is_bs() -> bool {
    tty_erase_char() == Some(0x08)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_h() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL)
    }

    /// The kitty protocol makes the keys distinct, so `Auto` must not spend
    /// the chord — even when the tty's erase character says otherwise.
    #[test]
    fn auto_keeps_the_chord_under_the_kitty_protocol() {
        assert_eq!(
            CtrlHPolicy::resolve_with(CtrlHSetting::Auto, true, true),
            CtrlHPolicy::Chord
        );
    }

    /// Both explicit settings are obeyed on every terminal: naming one is the
    /// escape hatch for a session whose ambiguity `Auto` guessed wrong.
    #[test]
    fn explicit_settings_ignore_the_terminal() {
        for enhanced in [false, true] {
            for erase_is_bs in [false, true] {
                assert_eq!(
                    CtrlHPolicy::resolve_with(CtrlHSetting::Backspace, enhanced, erase_is_bs),
                    CtrlHPolicy::Backspace,
                    "explicit `backspace` must hold with enhanced={enhanced} erase_is_bs={erase_is_bs}"
                );
                assert_eq!(
                    CtrlHPolicy::resolve_with(CtrlHSetting::Chord, enhanced, erase_is_bs),
                    CtrlHPolicy::Chord,
                    "explicit `chord` must hold with enhanced={enhanced} erase_is_bs={erase_is_bs}"
                );
            }
        }
    }

    /// The `Auto` decision table, now that the probe is an argument. These
    /// four rows are the whole of what `auto` means.
    #[test]
    fn auto_reads_the_terminal() {
        use CtrlHPolicy::{Backspace, Chord};
        for (enhanced, erase_is_bs, expected) in [
            (true, false, Chord),
            (true, true, Chord),
            (false, false, Chord),
            (false, true, Backspace),
        ] {
            assert_eq!(
                CtrlHPolicy::resolve_with(CtrlHSetting::Auto, enhanced, erase_is_bs),
                expected,
                "enhanced={enhanced} erase_is_bs={erase_is_bs}"
            );
        }
    }

    /// The rule that keeps the startup warning quiet for the people who
    /// chose this, and talkative for the people who did not.
    #[test]
    fn only_an_unasked_for_loss_is_reported() {
        assert!(
            CtrlHPolicy::Backspace.loss_is_worth_reporting(CtrlHSetting::Auto),
            "auto resolved it by probe — the user never chose it"
        );
        assert!(
            !CtrlHPolicy::Backspace.loss_is_worth_reporting(CtrlHSetting::Backspace),
            "they asked for exactly this"
        );
        // Nothing is lost under the chord policy, so there is nothing to say
        // whatever the setting was.
        for setting in [
            CtrlHSetting::Auto,
            CtrlHSetting::Backspace,
            CtrlHSetting::Chord,
        ] {
            assert!(!CtrlHPolicy::Chord.loss_is_worth_reporting(setting));
        }
    }

    #[test]
    fn rewrites_ctrl_h_to_backspace() {
        let key = CtrlHPolicy::Backspace.apply(ctrl_h());
        assert_eq!(key.code, KeyCode::Backspace);
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn chord_policy_rewrites_nothing() {
        assert_eq!(CtrlHPolicy::Chord.apply(ctrl_h()), ctrl_h());
    }

    /// Only the bare chord collides with `0x08`. A shifted or alt-ed Ctrl-H is
    /// a chord of its own and must survive the rewrite.
    #[test]
    fn leaves_neighbouring_chords_alone() {
        for modifiers in [
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::NONE,
            KeyModifiers::ALT,
        ] {
            let key = KeyEvent::new(KeyCode::Char('h'), modifiers);
            assert_eq!(
                CtrlHPolicy::Backspace.apply(key),
                key,
                "{modifiers:?}+h is not the byte 0x08"
            );
        }
    }

    /// A real Backspace is already a Backspace — the rewrite must be
    /// idempotent rather than turn it into something else.
    #[test]
    fn leaves_backspace_alone() {
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(CtrlHPolicy::Backspace.apply(key), key);
    }

    /// The probe answers `None` for a descriptor that is not a terminal
    /// rather than reading garbage — `/dev/null` is the classic redirected
    /// stdin, and the reason the probe now looks past stdin at all.
    #[cfg(unix)]
    #[test]
    fn a_non_tty_descriptor_has_no_erase_char() {
        use std::os::fd::AsFd;
        let null = std::fs::File::open("/dev/null").unwrap();
        assert_eq!(erase_char_of(null.as_fd()), None);
    }
}
