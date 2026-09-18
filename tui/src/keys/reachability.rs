//! Whether a chord can reach the app on *this* terminal.
//!
//! A terminal without the kitty keyboard protocol does not send keys; it sends
//! bytes. `Ctrl` plus a letter is one byte in `0x01..=0x1A`, and several of
//! those bytes were spoken for by real keys decades before anyone wanted them
//! as chords: `0x09` is Tab, `0x0D` is Enter, `0x1B` is Escape. Press `Ctrl+I`
//! on such a terminal and what arrives is Tab — identical, no side channel, no
//! timing tell. Shift fares no better: a `Ctrl+Shift+letter` chord transmits
//! without the shift bit, arriving as the plain `Ctrl+letter`.
//!
//! This module answers, for one [`KeyCombo`] and one terminal, which of three
//! things happens. It is pure and takes the terminal's traits as data, so the
//! answer can be asserted in a test rather than discovered by pressing keys —
//! which is the point: [`crate::settings`] uses it to keep the *default* keymap
//! free of chords that cannot arrive, and nothing about that check needs a
//! terminal.
//!
//! Every rule here is two small tables: the ASCII control byte a chord packs
//! to (`control_byte`) and what crossterm's legacy decoder
//! (`event::sys::unix::parse`) makes of that byte (`legacy_decode`). Where it
//! resolves a byte to a named key before it reaches the `Ctrl`+letter range —
//! `\t`, `\r`, `\x1B`, `\x7F` each have their own arm — that named key is
//! what wins, and the chord is what loses.

use super::KeyBindings;
use super::action_shortcuts::ActionShortcuts;
use super::key_combo::{KeyCombo, KeyModifiers};
use super::key_strike::KeyStrike;

/// What this terminal does to keys. Both facts are session-scoped and known at
/// startup; neither is a user preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalKeys {
    /// The kitty keyboard-enhancement flags went out, so keys arrive as keys
    /// and none of the byte collisions below exist.
    pub enhanced: bool,
    /// `0x08` is being delivered as Backspace (see `app::ctrl_h`), so the
    /// `Ctrl+H` chord never arrives at all.
    pub ctrl_h_is_backspace: bool,
}

impl TerminalKeys {
    /// The terminal the default keymap is held to: no protocol, so every byte
    /// collision applies.
    ///
    /// `Ctrl+H` is left as a chord rather than made pessimistic here, because
    /// the rewrite is conditional on one user's tty erase character rather
    /// than on terminals in general (see `app::ctrl_h`). Holding the defaults
    /// to it would demand `FocusSidebar` give up `Ctrl+H` for everyone to
    /// serve the few — which is the trade the `ctrl_h` setting exists to let
    /// those users make for themselves.
    pub const LEGACY: Self = Self {
        enhanced: false,
        ctrl_h_is_backspace: false,
    };

    /// The traits of a live session.
    ///
    /// `kitty_pushed` is whether the enhancement flags went out
    /// (`terminal::TerminalSession::keyboard_enhanced`). It is not the whole
    /// answer: on Windows crossterm never asks the terminal — its
    /// `supports_keyboard_enhancement` is a hard-coded `Ok(false)` — yet the
    /// console API reports keys rather than bytes, so none of the collisions
    /// this module tracks exist there (see `app::ctrl_h::tty_erase_char`).
    /// Folding that in here, rather than at each caller, keeps the startup
    /// notice and `kimun doctor` agreeing on the `enhanced` axis for the same
    /// terminal: both derive it as `cfg!(windows) || kitty_pushed`.
    ///
    /// `ctrl_h_is_backspace` carries no such guarantee — it is a plain
    /// pass-through, and the two callers deliberately pass different values
    /// (see the comment at the `app/mod.rs` call site). This constructor
    /// cannot make them agree on that axis; it only relays what each caller
    /// decides.
    pub fn detected(kitty_pushed: bool, ctrl_h_is_backspace: bool) -> Self {
        Self {
            enhanced: cfg!(windows) || kitty_pushed,
            ctrl_h_is_backspace,
        }
    }
}

/// What becomes of a chord on the way in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Arrives as itself. The binding works.
    Ok,
    /// Arrives as a *different* combo. Pressing the chord fires whatever that
    /// one is bound to, and the binding is dead — silently, which is what
    /// makes this worth a test.
    Shadowed(KeyCombo),
    /// Never arrives. The terminal has no distinct encoding for it, so the
    /// binding is dead but nothing else fires either.
    Untransmitted,
}

impl Reach {
    pub fn is_ok(self) -> bool {
        self == Reach::Ok
    }
}

/// The byte a `Ctrl` chord on `key` packs to on a legacy terminal, or `None`
/// for a key most terminals send no distinct code for. Letters and the
/// punctuation that shares their column are the only keys with a byte.
///
/// The rule is `& 0x1F` of the teletype-shifted symbol historically wired to
/// each key — not the US-keyboard shifted symbol: `2`→`@`, `3`→`[`, `4`→`\`,
/// `5`→`]`, `6`→`^`, `7`→`_`, `8`→DEL. `ascii & 0x1F` happens to agree for the
/// letters and for `[`/`\`/`]`/Space, but not for the digits or `-`/`/`
/// (`b'-' & 0x1F` is `0x0D`, Enter — not the `0x1F` this table assigns it).
fn control_byte(key: KeyStrike) -> Option<u8> {
    use KeyStrike::*;
    let byte = match key {
        // This arithmetic depends on `KeyStrike` (key_strike.rs) declaring
        // KeyA..KeyZ contiguously and in that order, so it lines up with
        // `LETTERS` below — nothing in key_strike.rs enforces that.
        // `key_combo.rs`'s `is_letter_chord` (its own KeyA..=KeyZ range
        // check) and `is_valid_binding` (which calls `is_letter_chord`, and
        // separately range-checks Digit0..=Digit9 the same way) share the
        // assumption. `letters_round_trip_through_both_control_tables` below
        // is what actually checks it.
        k if (KeyA..=KeyZ).contains(&k) => (k as u8) - (KeyA as u8) + 0x01,
        Space | Digit2 => 0x00,
        Digit3 | BracketLeft => 0x1B,
        Digit4 | Backslash => 0x1C,
        Digit5 | BracketRight => 0x1D,
        Digit6 => 0x1E,
        Digit7 | Minus | Slash => 0x1F,
        Digit8 => 0x7F,
        // `use KeyStrike::*` above brings `KeyStrike::None` into scope, which
        // would otherwise shadow the prelude's `Option::None` here.
        _ => return Option::None,
    };
    Some(byte)
}

/// What crossterm's legacy decoder (`event::sys::unix::parse`, 0.29) makes
/// of one control byte: `(ctrl, key)`. Mirrors its arms in order — the named
/// keys first, then the two `Ctrl` ranges — so a crossterm bump has one
/// table to re-check.
fn legacy_decode(byte: u8) -> (bool, KeyStrike) {
    use KeyStrike::*;
    match byte {
        0x09 => (false, Tab),
        0x0D => (false, Enter),
        0x1B => (false, Escape),
        0x7F => (false, Backspace),
        0x00 => (true, Space),
        // No `0x0A => Enter` arm, and that is correct: crossterm's own
        // `b'\n' => Enter` arm is guarded by `!is_raw_mode_enabled()`, and
        // this TUI is always in raw mode. So 0x0A falls through to the
        // Ctrl+letter range just below and arrives as Ctrl+J (`NewJournal`
        // in the default keymap) — adding the arm to "complete" this table
        // would silently break that binding instead.
        0x01..=0x1A => (true, LETTERS[usize::from(byte - 0x01)]),
        0x1C..=0x1F => (
            true,
            [Digit4, Digit5, Digit6, Digit7][usize::from(byte - 0x1C)],
        ),
        // Every byte `control_byte` can produce is matched above.
        _ => unreachable!("not a control byte: {byte:#04x}"),
    }
}

const LETTERS: [KeyStrike; 26] = {
    use KeyStrike::*;
    [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ]
};

/// What happens to `combo` on a terminal with these traits.
pub fn reach(combo: KeyCombo, keys: TerminalKeys) -> Reach {
    // Checked before the protocol, because this one is not the terminal's
    // doing: `app::ctrl_h` rewrites `Ctrl+H` at kimün's own input seam, after
    // the terminal has spoken, and an explicit `ctrl_h = "backspace"` is
    // obeyed even where the protocol keeps the two keys apart. So the chord
    // is gone under that setting on *any* terminal — and a diagnostic that
    // said otherwise would contradict the session it describes.
    //
    // Exactly the bare chord: `CtrlHPolicy::apply` matches `CONTROL` alone,
    // so `Ctrl+Shift+H` and `Ctrl+Alt+H` are untouched.
    let bare_ctrl_h = KeyCombo::new(KeyModifiers::new().and_ctrl(), KeyStrike::KeyH);
    if keys.ctrl_h_is_backspace && combo == bare_ctrl_h {
        return Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace));
    }
    // Past here every collision is an artefact of packing chords into single
    // bytes, which the protocol does away with.
    if keys.enhanced {
        return Reach::Ok;
    }
    // Only Ctrl collapses a chord into a control byte. `Alt` is sent as an
    // Esc prefix followed by the unmodified key, which keeps the key intact;
    // bare F-keys have their own escape sequences.
    if !combo.modifiers.is_ctrl() {
        return Reach::Ok;
    }

    // Pack the chord into its byte, decode the byte the way crossterm does,
    // and compare. The byte carries no shift bit and no Ctrl once it is
    // decoded as a named key, so a shadow keeps only Alt — the Esc prefix is
    // sent separately and survives independently.
    let Some(byte) = control_byte(combo.key) else {
        // Ctrl plus punctuation or a remaining digit: most terminals send no
        // distinct code for these at all. `Ctrl+,` is the one the default
        // keymap cares about, which is why `OpenPreferences` leads with F4.
        return Reach::Untransmitted;
    };
    let (ctrl, key) = legacy_decode(byte);
    if ctrl && key == combo.key && !combo.modifiers.is_shift() {
        return Reach::Ok;
    }
    let mut modifiers = KeyModifiers::new();
    if combo.modifiers.is_alt() {
        modifiers = modifiers.and_alt();
    }
    if ctrl {
        modifiers = modifiers.and_ctrl();
    }
    Reach::Shadowed(KeyCombo::new(modifiers, key))
}

/// An action with no chord this terminal can send, and what becomes of each
/// chord it does have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreachable {
    pub action: ActionShortcuts,
    pub combos: Vec<(KeyCombo, Reach)>,
}

/// Every action in `bindings` that cannot be reached at all on this terminal,
/// ordered by action name so output is stable between runs.
///
/// Empty for the default keymap on any terminal with `ctrl_h_is_backspace:
/// false` — an invariant test in [`crate::settings`] holds it to that (it
/// scans against [`TerminalKeys::LEGACY`], which sets that field `false`). So
/// a non-empty answer usually means the user's own `[key_bindings]` picked a
/// chord their terminal cannot deliver, which is worth telling them about
/// precisely because nothing else would: the chord simply does nothing, or
/// quietly fires whatever shadows it.
///
/// The one exception is the Ctrl-H rewrite: under `ctrl_h_is_backspace: true`
/// the *default* keymap is not empty either, because `FocusSidebar`'s only
/// chord is the bare `Ctrl+H` the rewrite shadows with Backspace —
/// `the_rewrite_strands_exactly_focus_sidebar_in_the_default_keymap` below
/// proves it. That case is kimün's own doing rather than a user mistake, and
/// `notice_text` in `app/mod.rs` words it accordingly.
///
/// Actions the user has deliberately *unbound* never appear: they hold no
/// combos, and only bound actions are listed at all.
pub fn unreachable_actions(bindings: &KeyBindings, keys: TerminalKeys) -> Vec<Unreachable> {
    let mut found: Vec<Unreachable> = bindings
        .to_hashmap()
        .into_iter()
        .filter(|(_, combos)| !combos.iter().any(|c| reach(*c, keys).is_ok()))
        .map(|(action, combos)| Unreachable {
            combos: combos.into_iter().map(|c| (c, reach(c, keys))).collect(),
            action,
        })
        .collect();
    found.sort_by_key(|u| u.action.to_string());
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(key: KeyStrike) -> KeyCombo {
        KeyCombo::new(KeyModifiers::new().and_ctrl(), key)
    }

    fn ctrl_shift(key: KeyStrike) -> KeyCombo {
        KeyCombo::new(KeyModifiers::new().and_ctrl().and_shift(), key)
    }

    /// The protocol is the one cure: it reports keys, not bytes.
    #[test]
    fn the_kitty_protocol_makes_everything_reachable() {
        let enhanced = TerminalKeys {
            enhanced: true,
            ctrl_h_is_backspace: false,
        };
        for combo in [
            ctrl(KeyStrike::KeyI),
            ctrl(KeyStrike::KeyM),
            ctrl(KeyStrike::Comma),
            ctrl_shift(KeyStrike::KeyL),
            ctrl(KeyStrike::BracketLeft),
        ] {
            assert_eq!(reach(combo, enhanced), Reach::Ok, "{combo}");
        }
    }

    /// The three bytes crossterm hands to a named key before it considers the
    /// Ctrl+letter range — verified against its `parse_event`.
    #[test]
    fn named_keys_win_their_bytes() {
        assert_eq!(
            reach(ctrl(KeyStrike::KeyI), TerminalKeys::LEGACY),
            Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Tab))
        );
        assert_eq!(
            reach(ctrl(KeyStrike::KeyM), TerminalKeys::LEGACY),
            Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Enter))
        );
        assert_eq!(
            reach(ctrl(KeyStrike::BracketLeft), TerminalKeys::LEGACY),
            Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Escape))
        );
    }

    /// Shift never reaches the app on a legacy terminal, so the chord arrives
    /// as its unshifted twin — and fires whatever *that* is bound to.
    #[test]
    fn shift_is_lost_from_a_ctrl_chord() {
        assert_eq!(
            reach(ctrl_shift(KeyStrike::KeyL), TerminalKeys::LEGACY),
            Reach::Shadowed(ctrl(KeyStrike::KeyL))
        );
        // The collision wins over the shift rule: the byte is 0x09 either way.
        assert_eq!(
            reach(ctrl_shift(KeyStrike::KeyI), TerminalKeys::LEGACY),
            Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Tab))
        );
    }

    /// Ctrl+H's reachability is the one answer that depends on policy rather
    /// than on the terminal alone.
    #[test]
    fn ctrl_h_follows_the_session_policy() {
        assert_eq!(
            reach(ctrl(KeyStrike::KeyH), TerminalKeys::LEGACY),
            Reach::Ok
        );
        let backspace = Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace));
        for enhanced in [false, true] {
            let rewritten = TerminalKeys {
                enhanced,
                ctrl_h_is_backspace: true,
            };
            assert_eq!(
                reach(ctrl(KeyStrike::KeyH), rewritten),
                backspace,
                "the rewrite is kimün's own, not the terminal's (enhanced={enhanced})"
            );
            // Only the *bare* chord is rewritten. On a protocol terminal
            // Ctrl+Shift+H is its own event and survives; on a legacy one it
            // still loses its shift bit to the ordinary rule, landing on the
            // chord that the rewrite then claims — reported one hop at a
            // time, since chaining would assert more than is known.
            assert_eq!(
                reach(ctrl_shift(KeyStrike::KeyH), rewritten),
                if enhanced {
                    Reach::Ok
                } else {
                    Reach::Shadowed(ctrl(KeyStrike::KeyH))
                }
            );
        }
    }

    /// `0x1C..=0x1F` arrive as Ctrl+4..7, so the digits are the reachable
    /// half of that alias and the punctuation is not.
    #[test]
    fn the_control_digits_alias_punctuation() {
        assert_eq!(
            reach(ctrl(KeyStrike::Backslash), TerminalKeys::LEGACY),
            Reach::Shadowed(ctrl(KeyStrike::Digit4))
        );
        assert_eq!(
            reach(ctrl(KeyStrike::BracketRight), TerminalKeys::LEGACY),
            Reach::Shadowed(ctrl(KeyStrike::Digit5))
        );
        for digit in [KeyStrike::Digit4, KeyStrike::Digit7] {
            assert_eq!(reach(ctrl(digit), TerminalKeys::LEGACY), Reach::Ok);
        }
        assert_eq!(
            reach(ctrl(KeyStrike::Digit1), TerminalKeys::LEGACY),
            Reach::Untransmitted
        );
        assert_eq!(
            reach(ctrl(KeyStrike::Comma), TerminalKeys::LEGACY),
            Reach::Untransmitted
        );
    }

    /// Alt is an Esc prefix, which leaves the key itself untouched — so the
    /// byte collisions are a Ctrl problem and F-keys are never affected.
    #[test]
    fn alt_chords_and_fkeys_always_arrive() {
        for combo in [
            KeyCombo::new(KeyModifiers::new().and_alt(), KeyStrike::KeyI),
            KeyCombo::new(KeyModifiers::new().and_alt(), KeyStrike::KeyH),
            KeyCombo::new(KeyModifiers::new(), KeyStrike::F4),
        ] {
            assert_eq!(reach(combo, TerminalKeys::LEGACY), Reach::Ok, "{combo}");
        }
    }

    /// Ctrl+Alt still packs a control byte, so the collision holds — and the
    /// Esc prefix survives into the shadow, because it is sent separately.
    #[test]
    fn a_ctrl_alt_collision_keeps_its_esc_prefix() {
        let combo = KeyCombo::new(KeyModifiers::new().and_ctrl().and_alt(), KeyStrike::KeyI);
        assert_eq!(
            reach(combo, TerminalKeys::LEGACY),
            Reach::Shadowed(KeyCombo::new(KeyModifiers::new().and_alt(), KeyStrike::Tab))
        );
    }

    /// The scan the startup warning and `kimun doctor` both run.
    #[test]
    fn the_scan_finds_an_action_with_no_usable_chord() {
        let mut kb = KeyBindings::empty();
        kb.batch_add()
            .with_ctrl()
            // Only chord, and it is Tab's byte: unreachable.
            .add(KeyStrike::KeyI, ActionShortcuts::QuickNote)
            // Unreachable, but this action has a second chord below.
            .add(KeyStrike::KeyM, ActionShortcuts::Quit)
            .add(KeyStrike::KeyQ, ActionShortcuts::Quit);

        let found = unreachable_actions(&kb, TerminalKeys::LEGACY);
        assert_eq!(found.len(), 1, "only QuickNote is stranded: {found:?}");
        assert_eq!(found[0].action, ActionShortcuts::QuickNote);
        assert_eq!(
            found[0].combos,
            vec![(
                ctrl(KeyStrike::KeyI),
                Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Tab))
            )]
        );
    }

    /// The protocol resolves every collision, so nothing is ever stranded on
    /// a terminal that speaks it.
    #[test]
    fn the_scan_finds_nothing_under_the_kitty_protocol() {
        let mut kb = KeyBindings::empty();
        kb.batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyI, ActionShortcuts::QuickNote);
        let enhanced = TerminalKeys {
            enhanced: true,
            ctrl_h_is_backspace: false,
        };
        assert!(unreachable_actions(&kb, enhanced).is_empty());
    }

    /// The default keymap must never trip the warning — otherwise it fires
    /// for every user on a legacy terminal, which is a nag, not a warning.
    #[test]
    fn the_scan_is_silent_for_the_default_keymap() {
        let kb = crate::settings::AppSettings::default().key_bindings;
        for keys in [
            TerminalKeys::LEGACY,
            TerminalKeys {
                enhanced: true,
                ctrl_h_is_backspace: false,
            },
        ] {
            let found = unreachable_actions(&kb, keys);
            assert!(found.is_empty(), "{keys:?} stranded {found:?}");
        }
    }

    /// The ordinary case, so the rules above read as exceptions rather than
    /// the norm.
    #[test]
    fn an_ordinary_ctrl_letter_arrives() {
        for key in [KeyStrike::KeyB, KeyStrike::KeyJ, KeyStrike::KeyQ] {
            assert_eq!(reach(ctrl(key), TerminalKeys::LEGACY), Reach::Ok);
        }
    }

    /// The Windows console reports keys, not bytes: crossterm's
    /// `supports_keyboard_enhancement` is a hard-coded `Ok(false)` there, but
    /// Ctrl+I and Tab still arrive as different events. The session
    /// constructor is the one place that knows this.
    #[test]
    fn a_live_session_is_enhanced_on_windows_without_the_protocol() {
        let keys = TerminalKeys::detected(false, false);
        assert_eq!(keys.enhanced, cfg!(windows));
        assert!(!keys.ctrl_h_is_backspace);
        // The protocol answer is honoured everywhere.
        assert!(TerminalKeys::detected(true, true).enhanced);
        assert!(TerminalKeys::detected(true, true).ctrl_h_is_backspace);
    }

    /// The bytes the hand-written table used to miss. Each one is a real
    /// keypress that crossterm's legacy decoder resolves to *something*, so
    /// "never arrives" was wrong — the chord fires another key.
    #[test]
    fn every_control_byte_is_accounted_for() {
        let plain = |key| KeyCombo::new(KeyModifiers::new(), key);
        // 0x00: Ctrl+2 and Ctrl+Space share a byte; crossterm names it Ctrl+Space.
        assert_eq!(
            reach(ctrl(KeyStrike::Space), TerminalKeys::LEGACY),
            Reach::Ok
        );
        assert_eq!(
            reach(ctrl(KeyStrike::Digit2), TerminalKeys::LEGACY),
            Reach::Shadowed(ctrl(KeyStrike::Space))
        );
        // 0x1B: Ctrl+3 is Esc, like Ctrl+[.
        assert_eq!(
            reach(ctrl(KeyStrike::Digit3), TerminalKeys::LEGACY),
            Reach::Shadowed(plain(KeyStrike::Escape))
        );
        // 0x7F: Ctrl+8 is the *other* Backspace byte — the same class of
        // shadow as the Ctrl+H bug, and it deletes a character.
        assert_eq!(
            reach(ctrl(KeyStrike::Digit8), TerminalKeys::LEGACY),
            Reach::Shadowed(plain(KeyStrike::Backspace))
        );
        // 0x1E / 0x1F: Ctrl+6 arrives as itself; Ctrl+- and Ctrl+/ as Ctrl+7.
        assert_eq!(
            reach(ctrl(KeyStrike::Digit6), TerminalKeys::LEGACY),
            Reach::Ok
        );
        for key in [KeyStrike::Minus, KeyStrike::Slash] {
            assert_eq!(
                reach(ctrl(key), TerminalKeys::LEGACY),
                Reach::Shadowed(ctrl(KeyStrike::Digit7)),
                "{key}"
            );
        }
        // And the keys that genuinely have no control byte stay untransmitted.
        for key in [
            KeyStrike::Digit0,
            KeyStrike::Digit1,
            KeyStrike::Digit9,
            KeyStrike::Comma,
            KeyStrike::Period,
            KeyStrike::Equal,
            KeyStrike::Semicolon,
            KeyStrike::Quote,
            KeyStrike::Backquote,
        ] {
            assert_eq!(
                reach(ctrl(key), TerminalKeys::LEGACY),
                Reach::Untransmitted,
                "{key}"
            );
        }
    }

    /// Locks `LETTERS`, `control_byte`'s arithmetic and `legacy_decode`'s
    /// `0x01..=0x1A` arm to each other and to the `KeyStrike` discriminants —
    /// the invariant `control_byte`'s doc comment states but that nothing
    /// else in this file checks. Covers the boundaries KeyA -> 0x01 and
    /// KeyZ -> 0x1A, which the hand-picked B/J/Q cases in
    /// `an_ordinary_ctrl_letter_arrives` do not reach. An insertion into
    /// `KeyStrike` between two letters (`KeyStrike` is declared
    /// alphabetically, so a non-letter variant can land mid-run) would shift
    /// this arithmetic out of step with `LETTERS` and fail here first.
    #[test]
    fn letters_round_trip_through_both_control_tables() {
        for (i, k) in LETTERS.iter().enumerate() {
            let byte = i as u8 + 1;
            assert_eq!(control_byte(*k), Some(byte), "{k:?} -> control_byte");
            // 0x09 (I) and 0x0D (M) are intercepted by legacy_decode's
            // named-key arms (Tab, Enter) before the Ctrl+letter range —
            // `named_keys_win_their_bytes` already covers that half of the
            // story. Every other byte in range round-trips to its own
            // letter, which is the part nothing else here checks.
            if byte != 0x09 && byte != 0x0D {
                assert_eq!(
                    legacy_decode(byte),
                    (true, *k),
                    "{byte:#04x} -> legacy_decode"
                );
            }
        }
    }

    /// Under the rewrite the default keymap *does* strand one action —
    /// `FocusSidebar`, whose only chord is Ctrl+H. The scan reports it; the
    /// callers word it as a `ctrl_h` decision rather than a user mistake.
    #[test]
    fn the_rewrite_strands_exactly_focus_sidebar_in_the_default_keymap() {
        let kb = crate::settings::AppSettings::default().key_bindings;
        let keys = TerminalKeys {
            enhanced: false,
            ctrl_h_is_backspace: true,
        };
        let found = unreachable_actions(&kb, keys);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].action, ActionShortcuts::FocusSidebar);
        assert_eq!(
            found[0].combos,
            vec![(
                ctrl(KeyStrike::KeyH),
                Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace))
            )]
        );
    }
}
