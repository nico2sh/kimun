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
//! Every rule here traces to crossterm's legacy byte decoder
//! (`event::sys::unix::parse`) plus the ASCII control table. Where crossterm
//! resolves a byte to a named key before it reaches the `Ctrl`+letter range —
//! `\t`, `\r`, `\x1B` each have their own arm — that named key is what wins,
//! and the chord is what loses.

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

/// What happens to `combo` on a terminal with these traits.
pub fn reach(combo: KeyCombo, keys: TerminalKeys) -> Reach {
    // The protocol reports the key itself; every collision below is an
    // artefact of packing chords into single bytes.
    if keys.enhanced {
        return Reach::Ok;
    }
    // Only Ctrl collapses a chord into a control byte. `Alt` is sent as an
    // Esc prefix followed by the unmodified key, which keeps the key intact;
    // bare F-keys have their own escape sequences.
    if !combo.modifiers.is_ctrl() {
        return Reach::Ok;
    }

    // The byte carries no shift bit and no Ctrl once it is decoded as a named
    // key, so a shadow keeps only Alt — the Esc prefix survives independently.
    let mut alt = KeyModifiers::new();
    if combo.modifiers.is_alt() {
        alt = alt.and_alt();
    }
    let as_key = |key| Reach::Shadowed(KeyCombo::new(alt, key));

    match combo.key {
        // Bytes crossterm resolves to a named key before it ever considers
        // the Ctrl+letter range. The key wins; the chord is unreachable.
        KeyStrike::KeyI => as_key(KeyStrike::Tab), // 0x09
        KeyStrike::KeyM => as_key(KeyStrike::Enter), // 0x0D
        KeyStrike::BracketLeft => as_key(KeyStrike::Escape), // 0x1B
        // 0x08 is Ctrl+H's byte and, on a terminal set up the older way, the
        // Backspace key's. Which one wins is the session's `ctrl_h` policy.
        KeyStrike::KeyH if keys.ctrl_h_is_backspace => as_key(KeyStrike::Backspace),

        // 0x1C..=0x1F decode as Ctrl plus the digits 4-7, so these two chords
        // arrive wearing someone else's name. `Ctrl+4`..`Ctrl+7` are the
        // combos that *do* arrive, which is why they are reachable and the
        // punctuation they alias is not.
        KeyStrike::Backslash => Reach::Shadowed(KeyCombo::new(alt.and_ctrl(), KeyStrike::Digit4)),
        KeyStrike::BracketRight => {
            Reach::Shadowed(KeyCombo::new(alt.and_ctrl(), KeyStrike::Digit5))
        }
        KeyStrike::Digit4 | KeyStrike::Digit5 | KeyStrike::Digit6 | KeyStrike::Digit7 => Reach::Ok,

        // Every other Ctrl+letter has a byte to itself. Shift does not
        // survive the trip, so a shifted chord arrives as the unshifted one.
        key if (KeyStrike::KeyA..=KeyStrike::KeyZ).contains(&key) => {
            if combo.modifiers.is_shift() {
                Reach::Shadowed(KeyCombo::new(alt.and_ctrl(), key))
            } else {
                Reach::Ok
            }
        }

        // Ctrl plus punctuation or a remaining digit: most terminals send no
        // distinct code for these at all. `Ctrl+,` is the one the default
        // keymap cares about, which is why `OpenPreferences` leads with F4.
        _ => Reach::Untransmitted,
    }
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
        let rewritten = TerminalKeys {
            enhanced: false,
            ctrl_h_is_backspace: true,
        };
        assert_eq!(
            reach(ctrl(KeyStrike::KeyH), rewritten),
            Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace))
        );
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

    /// The ordinary case, so the rules above read as exceptions rather than
    /// the norm.
    #[test]
    fn an_ordinary_ctrl_letter_arrives() {
        for key in [KeyStrike::KeyB, KeyStrike::KeyJ, KeyStrike::KeyQ] {
            assert_eq!(reach(ctrl(key), TerminalKeys::LEGACY), Reach::Ok);
        }
    }
}
