//! What a key means to the **plain** backend.
//!
//! The same shape the vim engine uses — keys are *reified* into an
//! [`Operation`] first and applied second — for the same reason: the mapping can
//! then be asserted without a buffer. "Ctrl+Left means a word back, without
//! extending" is a fact about the table, and until this existed the only way to
//! ask was to press the key at a buffer and look at where the cursor went.
//!
//! This is also where the last of the old widget's surprises ends. It
//! bound Ctrl+U and Ctrl+R to undo and redo inside its own input handler, so a key
//! could step history behind the caller's back. Nothing is bound here that is not
//! written here.
//!
//! What is deliberately *not* here: anything needing more than the buffer. The
//! clipboard chords reach the OS; `Tab` indents whole rows, `Enter` may continue
//! a markdown list, and an opening bracket typed over a selection wraps it — the
//! component routes those, ahead of this table, to the buffer's **indent step**
//! and to `markdown_edits`.

use crate::logic::editor::rope_buffer::{CursorMove, RopeBuffer};
use crate::logic::input::{Key, KeyInput, Mods};

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
pub fn operation(key: KeyInput) -> Option<Operation> {
    let extend = key.mods.shift;
    let chord = Mods {
        shift: false,
        ..key.mods
    };

    let motion = |to| Some(Operation::Move { to, extend });
    match (chord, key.key) {
        (Mods::NONE, Key::Left) => motion(CursorMove::Back),
        (Mods::NONE, Key::Right) => motion(CursorMove::Forward),
        (Mods::NONE, Key::Up) => motion(CursorMove::Up),
        (Mods::NONE, Key::Down) => motion(CursorMove::Down),
        (Mods::NONE, Key::Home) => motion(CursorMove::Head),
        (Mods::NONE, Key::End) => motion(CursorMove::End),
        (Mods::NONE, Key::PageUp) => motion(CursorMove::ParagraphBack),
        (Mods::NONE, Key::PageDown) => motion(CursorMove::ParagraphForward),

        (Mods::CTRL, Key::Left) => motion(CursorMove::WordBack),
        (Mods::CTRL, Key::Right) => motion(CursorMove::WordForward),
        (Mods::CTRL, Key::Home) => motion(CursorMove::Top),
        (Mods::CTRL, Key::End) => motion(CursorMove::Bottom),

        // macOS conventions. Terminals translate Option+arrow into `Esc b`/`Esc f`
        // by default, which crossterm reports as Alt+b / Alt+f; the shifted
        // variants arrive as the uppercase character with SHIFT still set, which
        // is why `extend` reads the modifier rather than the letter's case.
        (Mods::ALT, Key::Left) => motion(CursorMove::WordBack),
        (Mods::ALT, Key::Right) => motion(CursorMove::WordForward),
        (Mods::ALT, Key::Char('b' | 'B')) => motion(CursorMove::WordBack),
        (Mods::ALT, Key::Char('f' | 'F')) => motion(CursorMove::WordForward),
        (Mods::SUPER, Key::Left) => motion(CursorMove::Head),
        (Mods::SUPER, Key::Right) => motion(CursorMove::End),
        (Mods::SUPER, Key::Up) => motion(CursorMove::Top),
        (Mods::SUPER, Key::Down) => motion(CursorMove::Bottom),

        (Mods::CTRL, Key::Char('a')) => Some(Operation::SelectAll),
        (Mods::CTRL, Key::Backspace) | (Mods::ALT, Key::Backspace) => {
            Some(Operation::DeleteWordBack)
        }
        (Mods::CTRL, Key::Delete) | (Mods::ALT, Key::Delete) => Some(Operation::DeleteWordForward),

        // Text entry. Only an unmodified key types: a chord that reached here is
        // one nothing above claimed, and inserting its character would be worse
        // than ignoring it.
        (Mods::NONE, Key::Char(c)) => Some(Operation::Insert(c)),
        (Mods::NONE, Key::Enter) => Some(Operation::InsertNewline),
        // No Tab: the component intercepts it ahead of this table and routes it
        // to `indent_lines`, which indents whole lines by the **indent step**.
        // The operation that used to sit here inserted spaces to the next tab
        // stop computed from the cursor's *char* column — a third statement of
        // the tab-stop rule, on a basis neither the wrap nor the renderer uses.
        (Mods::NONE, Key::Backspace) => Some(Operation::DeleteBack),
        (Mods::NONE, Key::Delete) => Some(Operation::DeleteForward),

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
            // selection from outliving the gesture that made it.
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
        Operation::DeleteBack => buf.delete_char(),
        Operation::DeleteForward => buf.delete_next_char(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ropetext::Text;

    fn plain(key: Key) -> Option<Operation> {
        operation(KeyInput::plain(key))
    }

    fn with(key: Key, mods: Mods) -> Option<Operation> {
        operation(KeyInput::new(key, mods))
    }

    // -- the mapping, asserted without a buffer -----------------------------

    #[test]
    fn arrows_move_without_extending() {
        assert_eq!(
            plain(Key::Left),
            Some(Operation::Move {
                to: CursorMove::Back,
                extend: false
            })
        );
        assert_eq!(
            plain(Key::Down),
            Some(Operation::Move {
                to: CursorMove::Down,
                extend: false
            })
        );
    }

    #[test]
    fn shift_extends_whatever_it_is_held_with() {
        for (code, to) in [
            (Key::Left, CursorMove::Back),
            (Key::Home, CursorMove::Head),
            (Key::PageDown, CursorMove::ParagraphForward),
        ] {
            assert_eq!(
                with(code, Mods::SHIFT),
                Some(Operation::Move { to, extend: true }),
                "{code:?}"
            );
        }
        assert_eq!(
            with(
                Key::Left,
                Mods {
                    ctrl: true,
                    shift: true,
                    ..Mods::NONE
                }
            ),
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
            with(Key::Right, Mods::CTRL),
            Some(Operation::Move {
                to: CursorMove::WordForward,
                extend: false
            })
        );
        assert_eq!(
            with(Key::End, Mods::CTRL),
            Some(Operation::Move {
                to: CursorMove::Bottom,
                extend: false
            })
        );
    }

    #[test]
    fn either_modifier_deletes_a_word() {
        for modifier in [Mods::CTRL, Mods::ALT] {
            assert_eq!(
                with(Key::Backspace, modifier),
                Some(Operation::DeleteWordBack)
            );
            assert_eq!(
                with(Key::Delete, modifier),
                Some(Operation::DeleteWordForward)
            );
        }
    }

    #[test]
    fn unmodified_keys_type() {
        assert_eq!(plain(Key::Char('x')), Some(Operation::Insert('x')));
        assert_eq!(plain(Key::Enter), Some(Operation::InsertNewline));
        // Tab is claimed by the component ahead of this table, so it is not a
        // typing key here — see the note beside `InsertNewline` in `operation`.
        assert_eq!(plain(Key::Tab), None);
        assert_eq!(plain(Key::Backspace), Some(Operation::DeleteBack));
        assert_eq!(plain(Key::Delete), Some(Operation::DeleteForward));
        assert_eq!(
            with(Key::Char('X'), Mods::SHIFT),
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
            with(Key::Left, Mods::ALT),
            Some(Operation::Move {
                to: CursorMove::WordBack,
                extend: false
            })
        );
        assert_eq!(
            with(Key::Char('f'), Mods::ALT),
            Some(Operation::Move {
                to: CursorMove::WordForward,
                extend: false
            })
        );
        assert_eq!(
            with(
                Key::Char('B'),
                Mods {
                    alt: true,
                    shift: true,
                    ..Mods::NONE
                }
            ),
            Some(Operation::Move {
                to: CursorMove::WordBack,
                extend: true
            }),
            "the uppercase letter arrives with SHIFT set; the modifier decides"
        );
        assert_eq!(
            with(Key::Up, Mods::SUPER),
            Some(Operation::Move {
                to: CursorMove::Top,
                extend: false
            })
        );
    }

    #[test]
    fn undo_and_redo_are_not_bound_here() {
        // The widget this replaced bound these inside
        // its own input handler, so a key stepped history behind the caller's
        // back. They belong to the component's shortcut layer, which sees the key
        // first — and if it ever stopped, this table must not quietly pick it up.
        assert_eq!(with(Key::Char('u'), Mods::CTRL), None);
        assert_eq!(with(Key::Char('r'), Mods::CTRL), None);
    }

    #[test]
    fn keys_that_mean_nothing_here_mean_nothing() {
        assert_eq!(plain(Key::F(1)), None);
        assert_eq!(plain(Key::Esc), None);
        assert_eq!(with(Key::Char('v'), Mods::CTRL), None);
        assert_eq!(
            with(Key::Char('q'), Mods::ALT),
            None,
            "an unclaimed chord must not type its character"
        );
    }

    #[test]
    fn ctrl_a_selects_all_via_key_input() {
        assert!(matches!(
            operation(KeyInput::ctrl('a')),
            Some(Operation::SelectAll)
        ));
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
