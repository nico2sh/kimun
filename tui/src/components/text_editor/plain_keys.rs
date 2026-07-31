//! What a key means to the **plain** backend.
//!
//! The same shape ADR-0011 gives the vim engine — keys are *reified* into an
//! [`Operation`] first and applied second — for the same reason: the mapping can
//! then be asserted without a buffer. "Ctrl+Left means a word back, without
//! extending" is a fact about the table, and until this existed the only way to
//! ask was to press the key at a buffer and look at where the cursor went.
//!
//! This is also where adr/0038's fourth surprise ends. The widget this replaced
//! bound Ctrl+U and Ctrl+R to undo and redo inside its own input handler, so a key
//! could step history behind the caller's back. Nothing is bound here that is not
//! written here.
//!
//! What is deliberately *not* here: anything needing more than the buffer. The
//! clipboard chords reach the OS, `Tab` indents whole rows, `Enter` may continue a
//! markdown list, and an opening bracket typed over a selection wraps it — all of
//! which the component owns and handles before a key reaches this table.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::rope_buffer::{CursorMove, RopeBuffer};

/// One editing action, named independently of the key that asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Move the cursor. `extend` is Shift: it grows the selection rather than
    /// dropping it.
    Move {
        to: CursorMove,
        extend: bool,
    },
    SelectAll,
    /// Ctrl/Alt+Backspace.
    DeleteWordBack,
    /// Ctrl/Alt+Delete.
    DeleteWordForward,
    Insert(char),
    InsertNewline,
    InsertTab,
    /// Backspace.
    DeleteBack,
    /// Delete.
    DeleteForward,
}

/// What `key` means, or `None` when it means nothing to this backend.
///
/// `None` is not "do nothing loudly": function keys, modifier-only releases and
/// IME composition events all land here, and the caller leaves the buffer alone
/// rather than marking the note dirty for a key that did not edit it.
pub fn operation(key: KeyEvent) -> Option<Operation> {
    let extend = key.modifiers.contains(KeyModifiers::SHIFT);
    let chord = key.modifiers & !KeyModifiers::SHIFT;

    let motion = |to| Some(Operation::Move { to, extend });
    match (chord, key.code) {
        (KeyModifiers::NONE, KeyCode::Left) => motion(CursorMove::Back),
        (KeyModifiers::NONE, KeyCode::Right) => motion(CursorMove::Forward),
        (KeyModifiers::NONE, KeyCode::Up) => motion(CursorMove::Up),
        (KeyModifiers::NONE, KeyCode::Down) => motion(CursorMove::Down),
        (KeyModifiers::NONE, KeyCode::Home) => motion(CursorMove::Head),
        (KeyModifiers::NONE, KeyCode::End) => motion(CursorMove::End),
        (KeyModifiers::NONE, KeyCode::PageUp) => motion(CursorMove::ParagraphBack),
        (KeyModifiers::NONE, KeyCode::PageDown) => motion(CursorMove::ParagraphForward),

        (KeyModifiers::CONTROL, KeyCode::Left) => motion(CursorMove::WordBack),
        (KeyModifiers::CONTROL, KeyCode::Right) => motion(CursorMove::WordForward),
        (KeyModifiers::CONTROL, KeyCode::Home) => motion(CursorMove::Top),
        (KeyModifiers::CONTROL, KeyCode::End) => motion(CursorMove::Bottom),

        // macOS conventions. Terminals translate Option+arrow into `Esc b`/`Esc f`
        // by default, which crossterm reports as Alt+b / Alt+f; the shifted
        // variants arrive as the uppercase character with SHIFT still set, which
        // is why `extend` reads the modifier rather than the letter's case.
        (KeyModifiers::ALT, KeyCode::Left) => motion(CursorMove::WordBack),
        (KeyModifiers::ALT, KeyCode::Right) => motion(CursorMove::WordForward),
        (KeyModifiers::ALT, KeyCode::Char('b' | 'B')) => motion(CursorMove::WordBack),
        (KeyModifiers::ALT, KeyCode::Char('f' | 'F')) => motion(CursorMove::WordForward),
        (KeyModifiers::SUPER, KeyCode::Left) => motion(CursorMove::Head),
        (KeyModifiers::SUPER, KeyCode::Right) => motion(CursorMove::End),
        (KeyModifiers::SUPER, KeyCode::Up) => motion(CursorMove::Top),
        (KeyModifiers::SUPER, KeyCode::Down) => motion(CursorMove::Bottom),

        (KeyModifiers::CONTROL, KeyCode::Char('a')) => Some(Operation::SelectAll),
        (KeyModifiers::CONTROL, KeyCode::Backspace) | (KeyModifiers::ALT, KeyCode::Backspace) => {
            Some(Operation::DeleteWordBack)
        }
        (KeyModifiers::CONTROL, KeyCode::Delete) | (KeyModifiers::ALT, KeyCode::Delete) => {
            Some(Operation::DeleteWordForward)
        }

        // Text entry. Only an unmodified key types: a chord that reached here is
        // one nothing above claimed, and inserting its character would be worse
        // than ignoring it.
        (KeyModifiers::NONE, KeyCode::Char(c)) => Some(Operation::Insert(c)),
        (KeyModifiers::NONE, KeyCode::Enter) => Some(Operation::InsertNewline),
        (KeyModifiers::NONE, KeyCode::Tab) => Some(Operation::InsertTab),
        (KeyModifiers::NONE, KeyCode::Backspace) => Some(Operation::DeleteBack),
        (KeyModifiers::NONE, KeyCode::Delete) => Some(Operation::DeleteForward),

        _ => None,
    }
}

/// Carry `op` out. Reports whether the buffer's text changed — a cursor move
/// never does, and a delete with nothing to delete does not either.
pub fn apply(op: Operation, buf: &mut RopeBuffer) -> bool {
    match op {
        Operation::Move { to, extend } => {
            // Extending starts an anchor if there is not one already; not
            // extending drops it. Saying which, per key, is what keeps a
            // selection from outliving the gesture that made it (adr/0038).
            if extend {
                if buf.selection_range().is_none() {
                    buf.start_selection();
                }
            } else {
                buf.cancel_selection();
            }
            buf.move_cursor(to);
            false
        }
        Operation::SelectAll => {
            buf.select_all();
            false
        }
        Operation::DeleteWordBack => buf.delete_word(),
        Operation::DeleteWordForward => buf.delete_next_word(),
        Operation::Insert(c) => {
            buf.insert_char(c);
            true
        }
        Operation::InsertNewline => {
            buf.insert_newline();
            true
        }
        Operation::InsertTab => buf.insert_tab(),
        Operation::DeleteBack => buf.delete_char(),
        Operation::DeleteForward => buf.delete_next_char(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropetext::Text;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn plain(code: KeyCode) -> Option<Operation> {
        operation(key(code, KeyModifiers::NONE))
    }

    fn with(code: KeyCode, modifiers: KeyModifiers) -> Option<Operation> {
        operation(key(code, modifiers))
    }

    // -- the mapping, asserted without a buffer -----------------------------

    #[test]
    fn arrows_move_without_extending() {
        assert_eq!(
            plain(KeyCode::Left),
            Some(Operation::Move {
                to: CursorMove::Back,
                extend: false
            })
        );
        assert_eq!(
            plain(KeyCode::Down),
            Some(Operation::Move {
                to: CursorMove::Down,
                extend: false
            })
        );
    }

    #[test]
    fn shift_extends_whatever_it_is_held_with() {
        for (code, to) in [
            (KeyCode::Left, CursorMove::Back),
            (KeyCode::Home, CursorMove::Head),
            (KeyCode::PageDown, CursorMove::ParagraphForward),
        ] {
            assert_eq!(
                with(code, KeyModifiers::SHIFT),
                Some(Operation::Move { to, extend: true }),
                "{code:?}"
            );
        }
        assert_eq!(
            with(KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            Some(Operation::Move {
                to: CursorMove::WordBack,
                extend: true
            }),
            "shift composes with a chord rather than replacing it"
        );
    }

    #[test]
    fn control_moves_by_word_and_to_the_ends() {
        assert_eq!(
            with(KeyCode::Right, KeyModifiers::CONTROL),
            Some(Operation::Move {
                to: CursorMove::WordForward,
                extend: false
            })
        );
        assert_eq!(
            with(KeyCode::End, KeyModifiers::CONTROL),
            Some(Operation::Move {
                to: CursorMove::Bottom,
                extend: false
            })
        );
    }

    #[test]
    fn either_modifier_deletes_a_word() {
        for modifier in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            assert_eq!(
                with(KeyCode::Backspace, modifier),
                Some(Operation::DeleteWordBack)
            );
            assert_eq!(
                with(KeyCode::Delete, modifier),
                Some(Operation::DeleteWordForward)
            );
        }
    }

    #[test]
    fn unmodified_keys_type() {
        assert_eq!(plain(KeyCode::Char('x')), Some(Operation::Insert('x')));
        assert_eq!(plain(KeyCode::Enter), Some(Operation::InsertNewline));
        assert_eq!(plain(KeyCode::Tab), Some(Operation::InsertTab));
        assert_eq!(plain(KeyCode::Backspace), Some(Operation::DeleteBack));
        assert_eq!(plain(KeyCode::Delete), Some(Operation::DeleteForward));
        assert_eq!(
            with(KeyCode::Char('X'), KeyModifiers::SHIFT),
            Some(Operation::Insert('X')),
            "a shifted character is still a character"
        );
    }

    #[test]
    fn the_macos_conventions_are_here_too() {
        // These used to be a second table further up the handler. One table means
        // one place to look for what a key does — and one place for a collision to
        // be visible rather than decided by which match ran first.
        assert_eq!(
            with(KeyCode::Left, KeyModifiers::ALT),
            Some(Operation::Move {
                to: CursorMove::WordBack,
                extend: false
            })
        );
        assert_eq!(
            with(KeyCode::Char('f'), KeyModifiers::ALT),
            Some(Operation::Move {
                to: CursorMove::WordForward,
                extend: false
            })
        );
        assert_eq!(
            with(KeyCode::Char('B'), KeyModifiers::ALT | KeyModifiers::SHIFT),
            Some(Operation::Move {
                to: CursorMove::WordBack,
                extend: true
            }),
            "the uppercase letter arrives with SHIFT set; the modifier decides"
        );
        assert_eq!(
            with(KeyCode::Up, KeyModifiers::SUPER),
            Some(Operation::Move {
                to: CursorMove::Top,
                extend: false
            })
        );
    }

    #[test]
    fn undo_and_redo_are_not_bound_here() {
        // adr/0038's fourth surprise: the widget this replaced bound these inside
        // its own input handler, so a key stepped history behind the caller's
        // back. They belong to the component's shortcut layer, which sees the key
        // first — and if it ever stopped, this table must not quietly pick it up.
        assert_eq!(with(KeyCode::Char('u'), KeyModifiers::CONTROL), None);
        assert_eq!(with(KeyCode::Char('r'), KeyModifiers::CONTROL), None);
    }

    #[test]
    fn keys_that_mean_nothing_here_mean_nothing() {
        assert_eq!(plain(KeyCode::F(1)), None);
        assert_eq!(plain(KeyCode::Null), None);
        assert_eq!(plain(KeyCode::Esc), None);
        assert_eq!(with(KeyCode::Char('v'), KeyModifiers::CONTROL), None);
        assert_eq!(
            with(KeyCode::Char('q'), KeyModifiers::ALT),
            None,
            "an unclaimed chord must not type its character"
        );
    }

    // -- application ---------------------------------------------------------

    fn buffer(text: &str) -> RopeBuffer {
        RopeBuffer::new(Text::from(text))
    }

    #[test]
    fn a_move_reports_no_text_change() {
        let mut buf = buffer("hello");
        let changed = apply(
            Operation::Move {
                to: CursorMove::Forward,
                extend: false,
            },
            &mut buf,
        );
        assert!(!changed);
        assert_eq!(buf.cursor(), (0, 1));
    }

    #[test]
    fn extending_grows_a_selection_and_moving_drops_it() {
        let mut buf = buffer("hello");
        for _ in 0..2 {
            apply(
                Operation::Move {
                    to: CursorMove::Forward,
                    extend: true,
                },
                &mut buf,
            );
        }
        assert_eq!(buf.selection_range(), Some(((0, 0), (0, 2))));
        apply(
            Operation::Move {
                to: CursorMove::Forward,
                extend: false,
            },
            &mut buf,
        );
        assert!(buf.selection_range().is_none());
    }

    #[test]
    fn a_delete_with_nothing_to_delete_reports_no_change() {
        let mut buf = buffer("");
        assert!(!apply(Operation::DeleteBack, &mut buf));
        assert!(!apply(Operation::DeleteForward, &mut buf));
        assert!(!apply(Operation::DeleteWordBack, &mut buf));
    }

    #[test]
    fn typing_reports_a_change() {
        let mut buf = buffer("");
        assert!(apply(Operation::Insert('a'), &mut buf));
        assert!(apply(Operation::InsertNewline, &mut buf));
        assert_eq!(buf.text().to_string(), "a\n");
    }
}
