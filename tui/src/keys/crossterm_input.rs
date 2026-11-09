//! The crossterm edge of the terminal-neutral [`KeyInput`].
//!
//! Distinct from [`super::key_event_to_combo`]: a `KeyCombo` names the physical
//! key (and lowercases it, and has no entry for `$`, `:` or `é`), which is right
//! for shortcut lookup and wrong for editing, where the typed character *is* the
//! input. This adapter keeps it.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::logic::input::{Button, Key, KeyInput, Mods, PointerInput, PointerKind};

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

/// Convert a crossterm key event to a [`KeyInput`] for a one-line text field
/// (the find bar, the dialogs' inputs). Unlike [`key_input`], META and HYPER
/// are dropped rather than refusing the chord: a field has always matched only
/// Ctrl and Alt (a char is typed unless one of those is held), so Meta+d types
/// `d` there as it always did. `None` only for keys with no field meaning.
pub fn field_key_input(event: &KeyEvent) -> Option<KeyInput> {
    let stripped = KeyEvent {
        modifiers: event
            .modifiers
            .difference(KeyModifiers::META | KeyModifiers::HYPER),
        ..*event
    };
    key_input(&stripped)
}

/// Convert a crossterm mouse event to a [`PointerInput`] positioned relative to
/// `origin` (the `(x, y)` of the widget's area; saturating, so an event left of
/// or above it reads as 0). `None` for horizontal scrolling, which no widget
/// uses. Unlike [`key_input`], META and HYPER do not make the event `None`: the
/// mouse never looked at modifiers, so those bits are simply not copied into
/// [`Mods`] (which has no field for them) and the click acts as it always did.
pub fn pointer_input(event: &MouseEvent, origin: (u16, u16)) -> Option<PointerInput> {
    let button = |b: MouseButton| match b {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    };
    let kind = match event.kind {
        MouseEventKind::Down(b) => PointerKind::Down(button(b)),
        MouseEventKind::Up(b) => PointerKind::Up(button(b)),
        MouseEventKind::Drag(b) => PointerKind::Drag(button(b)),
        MouseEventKind::ScrollUp => PointerKind::ScrollUp,
        MouseEventKind::ScrollDown => PointerKind::ScrollDown,
        MouseEventKind::Moved => PointerKind::Moved,
        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => return None,
    };
    let m = event.modifiers;
    Some(PointerInput {
        kind,
        col: event.column.saturating_sub(origin.0),
        row: event.row.saturating_sub(origin.1),
        mods: Mods {
            ctrl: m.contains(KeyModifiers::CONTROL),
            alt: m.contains(KeyModifiers::ALT),
            shift: m.contains(KeyModifiers::SHIFT),
            sup: m.contains(KeyModifiers::SUPER),
        },
    })
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

    fn mev(kind: MouseEventKind, column: u16, row: u16, mods: KeyModifiers) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: mods,
        }
    }

    #[test]
    fn pointer_position_is_relative_to_the_origin() {
        let e = mev(
            MouseEventKind::Down(MouseButton::Left),
            12,
            7,
            KeyModifiers::NONE,
        );
        let p = pointer_input(&e, (10, 5)).unwrap();
        assert_eq!((p.col, p.row), (2, 2));
    }

    #[test]
    fn pointer_position_saturates_instead_of_underflowing() {
        let e = mev(MouseEventKind::Moved, 3, 3, KeyModifiers::NONE);
        let p = pointer_input(&e, (10, 5)).unwrap();
        assert_eq!((p.col, p.row), (0, 0));
    }

    #[test]
    fn pointer_kinds_map_one_to_one() {
        for (cb, b) in [
            (MouseButton::Left, Button::Left),
            (MouseButton::Right, Button::Right),
            (MouseButton::Middle, Button::Middle),
        ] {
            let k = |kind| {
                pointer_input(&mev(kind, 0, 0, KeyModifiers::NONE), (0, 0))
                    .unwrap()
                    .kind
            };
            assert_eq!(k(MouseEventKind::Down(cb)), PointerKind::Down(b));
            assert_eq!(k(MouseEventKind::Up(cb)), PointerKind::Up(b));
            assert_eq!(k(MouseEventKind::Drag(cb)), PointerKind::Drag(b));
        }
        let k = |kind| {
            pointer_input(&mev(kind, 0, 0, KeyModifiers::NONE), (0, 0))
                .unwrap()
                .kind
        };
        assert_eq!(k(MouseEventKind::ScrollUp), PointerKind::ScrollUp);
        assert_eq!(k(MouseEventKind::ScrollDown), PointerKind::ScrollDown);
        assert_eq!(k(MouseEventKind::Moved), PointerKind::Moved);
    }

    #[test]
    fn horizontal_scroll_has_no_pointer_input() {
        for kind in [MouseEventKind::ScrollLeft, MouseEventKind::ScrollRight] {
            assert_eq!(
                pointer_input(&mev(kind, 1, 1, KeyModifiers::NONE), (0, 0)),
                None
            );
        }
    }

    #[test]
    fn pointer_ignores_meta_and_hyper_bits() {
        for (m, want) in [
            (KeyModifiers::META, Mods::NONE),
            (KeyModifiers::HYPER, Mods::NONE),
            (KeyModifiers::META | KeyModifiers::CONTROL, Mods::CTRL),
        ] {
            let e = mev(MouseEventKind::Down(MouseButton::Left), 1, 1, m);
            let p = pointer_input(&e, (0, 0)).unwrap();
            assert_eq!(p.kind, PointerKind::Down(Button::Left));
            assert_eq!(p.mods, want);
        }
    }

    #[test]
    fn pointer_carries_ctrl_alt_shift_super() {
        let m =
            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT | KeyModifiers::SUPER;
        let e = mev(MouseEventKind::ScrollUp, 1, 1, m);
        assert_eq!(
            pointer_input(&e, (0, 0)).unwrap().mods,
            Mods {
                ctrl: true,
                alt: true,
                shift: true,
                sup: true
            }
        );
        let e = mev(MouseEventKind::ScrollUp, 1, 1, KeyModifiers::NONE);
        assert_eq!(pointer_input(&e, (0, 0)).unwrap().mods, Mods::NONE);
    }
}
