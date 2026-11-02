//! The crossterm edge of the terminal-neutral [`KeyInput`].
//!
//! Distinct from [`super::key_event_to_combo`]: a `KeyCombo` names the physical
//! key (and lowercases it, and has no entry for `$`, `:` or `é`), which is right
//! for shortcut lookup and wrong for editing, where the typed character *is* the
//! input. This adapter keeps it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::logic::input::{Key, KeyInput, Mods};

/// Convert a crossterm key event to a [`KeyInput`], or `None` when no editing
/// engine could act on it: keys with no editing meaning (`Null`, lock keys, media
/// keys) and any chord carrying META or HYPER. `KeyInput` has no slot for those
/// two modifiers, and dropping them silently would make Meta+d behave as `d`;
/// before the `KeyInput` split the engines matched modifiers exactly and so did
/// nothing with such chords, and `None` preserves that. `Release` and `Repeat`
/// kinds are deliberately not filtered here: that is the event loop's concern,
/// and the adapter stays a pure code-and-modifier mapping.
pub fn key_input(event: &KeyEvent) -> Option<KeyInput> {
    if event
        .modifiers
        .intersects(KeyModifiers::META | KeyModifiers::HYPER)
    {
        return None;
    }
    let key = match event.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Insert => Key::Insert,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::F(n) => Key::F(n),
        _ => return None,
    };
    let m = event.modifiers;
    Some(KeyInput::new(
        key,
        Mods {
            ctrl: m.contains(KeyModifiers::CONTROL),
            alt: m.contains(KeyModifiers::ALT),
            shift: m.contains(KeyModifiers::SHIFT),
            sup: m.contains(KeyModifiers::SUPER),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn shifted_symbols_keep_their_character() {
        assert_eq!(
            key_input(&ev(KeyCode::Char('$'), KeyModifiers::SHIFT)),
            Some(KeyInput::new(
                Key::Char('$'),
                Mods {
                    shift: true,
                    ..Mods::NONE
                }
            ))
        );
        assert_eq!(
            key_input(&ev(KeyCode::Char(':'), KeyModifiers::NONE)),
            Some(KeyInput::plain(Key::Char(':')))
        );
    }

    #[test]
    fn non_ascii_characters_survive() {
        for c in ['é', '理', 'ß'] {
            assert_eq!(
                key_input(&ev(KeyCode::Char(c), KeyModifiers::NONE)),
                Some(KeyInput::plain(Key::Char(c)))
            );
        }
    }

    #[test]
    fn back_tab_stays_back_tab() {
        assert_eq!(
            key_input(&ev(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(KeyInput::new(
                Key::BackTab,
                Mods {
                    shift: true,
                    ..Mods::NONE
                }
            ))
        );
    }

    #[test]
    fn raw_control_bytes_are_not_normalised() {
        // `key_event_to_combo` rewrites '\x11' to Ctrl-Q for shortcut lookup. The
        // editing engines never saw that rewrite and must not start now.
        assert_eq!(
            key_input(&ev(KeyCode::Char('\x11'), KeyModifiers::NONE)),
            Some(KeyInput::plain(Key::Char('\x11')))
        );
    }

    #[test]
    fn ctrl_letter_carries_the_modifier() {
        assert_eq!(
            key_input(&ev(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(KeyInput::ctrl('r'))
        );
    }

    #[test]
    fn keys_without_an_editing_meaning_are_none() {
        assert_eq!(key_input(&ev(KeyCode::Null, KeyModifiers::NONE)), None);
        assert_eq!(key_input(&ev(KeyCode::CapsLock, KeyModifiers::NONE)), None);
    }

    #[test]
    fn meta_and_hyper_chords_are_none() {
        // No engine claims these, and mapping them without the modifier would make
        // Meta+d act like `d`.
        assert_eq!(key_input(&ev(KeyCode::Char('x'), KeyModifiers::META)), None);
        assert_eq!(
            key_input(&ev(KeyCode::Char('x'), KeyModifiers::HYPER)),
            None
        );
        assert_eq!(key_input(&ev(KeyCode::Left, KeyModifiers::META)), None);
        assert_eq!(
            key_input(&ev(
                KeyCode::Char('d'),
                KeyModifiers::META | KeyModifiers::SHIFT
            )),
            None
        );
    }

    #[test]
    fn super_is_still_mapped() {
        assert_eq!(
            key_input(&ev(KeyCode::Left, KeyModifiers::SUPER)),
            Some(KeyInput::new(
                Key::Left,
                Mods {
                    sup: true,
                    ..Mods::NONE
                }
            ))
        );
    }
}
