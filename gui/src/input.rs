//! The GPUI edge of the toolkit-neutral [`KeyInput`] / [`PointerInput`]
//! (mirror of `tui/src/keys/crossterm_input.rs`).
//!
//! A key is what the user *typed*: `Key::Char('$')`, `Key::Char('é')`. Control
//! is literal on every OS and Cmd/Win/Super is passed through as `sup`, never
//! remapped to Ctrl.

use gpui_kit::{
    KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, ScrollEvent, TouchPhase, px,
};
use kimun_notes::logic::input::{Button, Key, KeyInput, Mods, PointerInput, PointerKind};

use crate::grid::CellMetrics;

/// GPUI modifiers -> [`Mods`]. GPUI's `function` (Fn) has no slot and is ignored.
pub fn mods(m: &Modifiers) -> Mods {
    Mods {
        ctrl: m.control,
        alt: m.alt,
        shift: m.shift,
        sup: m.platform,
    }
}

/// A chord as a GPUI [`Keystroke`], for showing it the platform's way (the
/// kit's `Kbd`). The inverse of [`key_input`] for every chord a keymap can
/// hold: `BackTab` is Shift+Tab, a space is `space`.
pub fn keystroke(key: KeyInput) -> Keystroke {
    let name = match key.key {
        Key::Char(' ') => "space".to_string(),
        Key::Char(c) => c.to_string(),
        Key::Enter => "enter".into(),
        Key::Esc => "escape".into(),
        Key::Tab | Key::BackTab => "tab".into(),
        Key::Backspace => "backspace".into(),
        Key::Delete => "delete".into(),
        Key::Insert => "insert".into(),
        Key::Left => "left".into(),
        Key::Right => "right".into(),
        Key::Up => "up".into(),
        Key::Down => "down".into(),
        Key::Home => "home".into(),
        Key::End => "end".into(),
        Key::PageUp => "pageup".into(),
        Key::PageDown => "pagedown".into(),
        Key::F(n) => format!("f{n}"),
    };
    Keystroke {
        modifiers: Modifiers {
            control: key.mods.ctrl,
            alt: key.mods.alt,
            shift: key.mods.shift || key.key == Key::BackTab,
            platform: key.mods.sup,
            function: false,
        },
        key: name,
        key_char: None,
    }
}

/// Convert a key-down event. See [`key_input`], except for character-input
/// keys: when the platform says the modifiers *produced* the text
/// (`prefer_character_input`: AltGr on Windows arrives as Ctrl+Alt with `@` in
/// `key_char`, macOS Option composes `å`), the text is what was typed and the
/// Ctrl/Alt/Super it took are not a chord. Shift is kept, as for any typed
/// symbol.
pub fn key_down_input(event: &KeyDownEvent) -> Option<KeyInput> {
    if event.prefer_character_input
        && let Some(text) = typed_text(&event.keystroke)
    {
        // Multi-char text goes to the input handler (`carries_text`).
        let c = single_char(Some(text))?;
        if !c.is_control() {
            let shift = event.keystroke.modifiers.shift;
            return Some(KeyInput::new(
                Key::Char(c),
                Mods {
                    shift,
                    ..Mods::NONE
                },
            ));
        }
    }
    key_input(&event.keystroke)
}

/// Convert a GPUI keystroke, or `None` for modifier-only presses, keys with no
/// editing meaning, and typed text that is not exactly one char.
///
/// Text selection: with no Ctrl/Alt/Super held, the typed text (`key_char`)
/// wins, so shifted symbols and non-ASCII input survive (`Shift+4` -> `$`).
/// An empty `key_char` counts as no text. With Ctrl/Alt/Super held `key_char`
/// is layout-dependent (`Alt+s` is `ß` on macOS), so the physical `key` is used
/// instead, giving `Ctrl+a` -> `Char('a')` + CTRL as the engines expect.
/// Chorded Shift uses the physical key too: `Ctrl+Shift+a` -> `Char('a')` with
/// CTRL|SHIFT, not `'A'`. `Shift+Tab` becomes `BackTab`, as in crossterm.
///
/// Typed text that is not exactly one char (grapheme clusters, ZWJ emoji) gives
/// `None` and [`carries_text`] says so: it reaches the buffer through the
/// window's input handler instead.
pub fn key_input(ks: &Keystroke) -> Option<KeyInput> {
    let m = mods(&ks.modifiers);
    let named = match ks.key.as_str() {
        "enter" => Some(Key::Enter),
        "escape" => Some(Key::Esc),
        "tab" if m.shift => Some(Key::BackTab),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        "delete" => Some(Key::Delete),
        "insert" => Some(Key::Insert),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" => Some(Key::PageUp),
        "pagedown" => Some(Key::PageDown),
        "space" => Some(Key::Char(' ')),
        // Ctrl-[ *is* Esc in a terminal (both send 0x1b), so the TUI's engines
        // only ever see Esc for it. Same key, same meaning here.
        "[" if m == Mods::CTRL => return Some(KeyInput::plain(Key::Esc)),
        "shift" | "control" | "alt" | "platform" | "function" | "capslock" => return None,
        k => function_key(k),
    };
    if let Some(key) = named {
        return Some(KeyInput::new(key, m));
    }
    let chorded = m.ctrl || m.alt || m.sup;
    let c = match (chorded, typed_text(ks)) {
        // Typed text that is not exactly one char (a grapheme cluster, ZWJ emoji)
        // cannot be a `Key::Char`; it must arrive via the input handler/IME.
        // Falling back to the physical key would insert the wrong character.
        (false, Some(text)) => single_char(Some(text))?,
        _ => single_char(Some(&ks.key))?,
    };
    Some(KeyInput::new(Key::Char(c), m))
}

/// The keystroke's typed text, with an empty `key_char` read as none.
fn typed_text(ks: &Keystroke) -> Option<&str> {
    ks.key_char.as_deref().filter(|t| !t.is_empty())
}

/// Whether a key-down types text that [`key_down_input`] cannot carry (more
/// than one char), unchorded or as character input. The host must let such a
/// key propagate so the platform hands the text to the input handler.
pub fn carries_text(event: &KeyDownEvent) -> bool {
    let ks = &event.keystroke;
    let m = mods(&ks.modifiers);
    (event.prefer_character_input || !(m.ctrl || m.alt || m.sup))
        && typed_text(ks).is_some_and(|t| single_char(Some(t)).is_none())
}

fn function_key(name: &str) -> Option<Key> {
    let n: u8 = name.strip_prefix('f')?.parse().ok()?;
    (1..=35).contains(&n).then_some(Key::F(n))
}

fn single_char(s: Option<&str>) -> Option<char> {
    let mut it = s?.chars();
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

fn button(b: MouseButton) -> Option<Button> {
    match b {
        MouseButton::Left => Some(Button::Left),
        MouseButton::Right => Some(Button::Right),
        MouseButton::Middle => Some(Button::Middle),
        // Back/forward navigation buttons have no editing meaning.
        MouseButton::Navigate(_) => None,
    }
}

// Mouse `click_count` is intentionally ignored by every pointer conversion
// below: `PointerKind` has no double/triple click, and multi-click selection is
// out of M1 scope.

/// Window position -> cell relative to `origin` (the widget's top-left in
/// window space). Positions left of / above the origin read as 0.
fn cell_at(pos: Point<Pixels>, origin: Point<Pixels>, cells: &CellMetrics) -> (u16, u16) {
    (
        cells.col_at(f32::from(pos.x - origin.x)),
        cells.row_at(f32::from(pos.y - origin.y)),
    )
}

/// A button press. `click_count` is ignored (no multi-click in `PointerKind`).
pub fn pointer_down(
    e: &MouseDownEvent,
    origin: Point<Pixels>,
    cells: &CellMetrics,
) -> Option<PointerInput> {
    let (col, row) = cell_at(e.position, origin, cells);
    Some(PointerInput {
        kind: PointerKind::Down(button(e.button)?),
        col,
        row,
        mods: mods(&e.modifiers),
    })
}

/// A button release. `click_count` is ignored (no multi-click in `PointerKind`).
pub fn pointer_up(
    e: &MouseUpEvent,
    origin: Point<Pixels>,
    cells: &CellMetrics,
) -> Option<PointerInput> {
    let (col, row) = cell_at(e.position, origin, cells);
    Some(PointerInput {
        kind: PointerKind::Up(button(e.button)?),
        col,
        row,
        mods: mods(&e.modifiers),
    })
}

/// A move with a button held is a `Drag` of that button, otherwise `Moved`.
pub fn pointer_move(
    e: &MouseMoveEvent,
    origin: Point<Pixels>,
    cells: &CellMetrics,
) -> Option<PointerInput> {
    let (col, row) = cell_at(e.position, origin, cells);
    let kind = match e.pressed_button {
        Some(b) => PointerKind::Drag(button(b)?),
        None => PointerKind::Moved,
    };
    Some(PointerInput {
        kind,
        col,
        row,
        mods: mods(&e.modifiers),
    })
}

/// Turns wheel and trackpad deltas into whole-row scroll steps.
///
/// A trackpad sends many small pixel deltas; quantising each to one step would
/// scroll far too fast, and dropping the fractions would make slow swipes do
/// nothing. The remainder is carried between events instead, so one row moves
/// per row-height of travel. A mouse wheel's line delta becomes that many rows.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ScrollAccumulator {
    /// Vertical travel not yet turned into a row, in pixels (positive = up).
    pending: f32,
}

impl ScrollAccumulator {
    /// Add `dy` pixels of vertical travel (GPUI sign: positive moves content
    /// down, i.e. scrolls up) and return the whole rows it completes, positive
    /// for up. A new gesture or a change of direction drops the old remainder.
    pub fn feed(&mut self, dy: f32, row_height: f32, new_gesture: bool) -> i32 {
        if new_gesture {
            self.pending = 0.0;
        }
        if !dy.is_finite() || dy == 0.0 || !row_height.is_finite() || row_height <= 0.0 {
            return 0;
        }
        // Only a real move can reverse (`0.0.signum()` is 1.0, so a zero tick
        // would otherwise drop a downward remainder).
        if self.pending != 0.0 && dy.signum() != self.pending.signum() {
            self.pending = 0.0;
        }
        self.pending += dy;
        // The nudge keeps an exact multiple (a 3-line wheel notch) from losing
        // a row to float rounding (2.9999 -> 2).
        let rows = (self.pending / row_height + self.pending.signum() * 1e-4).trunc();
        self.pending -= rows * row_height;
        rows as i32
    }
}

/// One wheel/trackpad event -> the `ScrollUp`/`ScrollDown` steps it completes,
/// one per row (GPUI: positive y moves content down, i.e. scrolls *up*). Empty
/// for horizontal, zero, or not-yet-a-row travel. Line deltas count as
/// `cells.height()` pixels per line.
pub fn pointer_scroll(
    e: &ScrollEvent,
    origin: Point<Pixels>,
    cells: &CellMetrics,
    acc: &mut ScrollAccumulator,
) -> Vec<PointerInput> {
    let dy = f32::from(e.delta.pixel_delta(px(cells.height())).y);
    let new_gesture = e.touch_phase == TouchPhase::Started;
    let rows = acc.feed(dy, cells.height(), new_gesture);
    let kind = if rows > 0 {
        PointerKind::ScrollUp
    } else {
        PointerKind::ScrollDown
    };
    let (col, row) = cell_at(e.position, origin, cells);
    let step = PointerInput {
        kind,
        col,
        row,
        mods: mods(&e.modifiers),
    };
    vec![step; rows.unsigned_abs() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{ScrollDelta, point};

    fn ks(key: &str, key_char: Option<&str>, m: Modifiers) -> Keystroke {
        Keystroke {
            modifiers: m,
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }
    const NONE: Modifiers = Modifiers {
        control: false,
        alt: false,
        shift: false,
        platform: false,
        function: false,
    };
    fn with(f: impl FnOnce(&mut Modifiers)) -> Modifiers {
        let mut m = NONE;
        f(&mut m);
        m
    }
    fn input(k: Keystroke) -> Option<KeyInput> {
        key_input(&k)
    }

    /// Every chord shape a keymap holds survives the trip to a GPUI
    /// keystroke and back.
    #[test]
    fn a_chord_round_trips_through_its_keystroke() {
        for chord in [
            KeyInput::new(
                Key::Char('p'),
                Mods {
                    ctrl: true,
                    shift: true,
                    ..Mods::NONE
                },
            ),
            KeyInput::new(Key::Char('q'), Mods::SUPER),
            KeyInput::new(
                Key::Left,
                Mods {
                    alt: true,
                    shift: true,
                    ..Mods::NONE
                },
            ),
            KeyInput::new(
                Key::BackTab,
                Mods {
                    ctrl: true,
                    shift: true,
                    ..Mods::NONE
                },
            ),
            KeyInput::new(Key::Enter, Mods::CTRL),
            KeyInput::plain(Key::F(2)),
        ] {
            assert_eq!(key_input(&keystroke(chord)), Some(chord), "{chord:?}");
        }
    }

    #[test]
    fn plain_letter() {
        assert_eq!(
            input(ks("a", Some("a"), NONE)),
            Some(KeyInput::plain(Key::Char('a')))
        );
    }

    #[test]
    fn shifted_symbol_uses_typed_text() {
        assert_eq!(
            input(ks("4", Some("$"), with(|m| m.shift = true))),
            Some(KeyInput::new(Key::Char('$'), Mods::SHIFT))
        );
        assert_eq!(
            input(ks("a", Some("A"), with(|m| m.shift = true))),
            Some(KeyInput::new(Key::Char('A'), Mods::SHIFT))
        );
    }

    #[test]
    fn non_ascii_text_survives() {
        for c in ["é", "理", "ß"] {
            assert_eq!(
                input(ks("x", Some(c), NONE)),
                Some(KeyInput::plain(Key::Char(c.chars().next().unwrap())))
            );
        }
    }

    #[test]
    fn non_ascii_layout_falls_back_to_key_when_no_text() {
        assert_eq!(
            input(ks("q", None, NONE)),
            Some(KeyInput::plain(Key::Char('q')))
        );
    }

    #[test]
    fn ctrl_letter_is_literal_ctrl() {
        assert_eq!(
            input(ks("a", None, with(|m| m.control = true))),
            Some(KeyInput::ctrl('a'))
        );
        // Even if the platform reports a control byte as text.
        assert_eq!(
            input(ks("a", Some("\u{1}"), with(|m| m.control = true))),
            Some(KeyInput::ctrl('a'))
        );
    }

    #[test]
    fn ctrl_bracket_is_esc_as_in_a_terminal() {
        let ctrl = with(|m| m.control = true);
        assert_eq!(input(ks("[", None, ctrl)), Some(KeyInput::plain(Key::Esc)));
        assert_eq!(
            input(ks("[", Some("\u{1b}"), ctrl)),
            Some(KeyInput::plain(Key::Esc))
        );
        // Only the bare chord: other modifiers and a plain `[` keep their meaning.
        assert_eq!(
            input(ks(
                "[",
                None,
                with(|m| {
                    m.control = true;
                    m.alt = true;
                })
            )),
            Some(KeyInput::new(
                Key::Char('['),
                Mods {
                    alt: true,
                    ..Mods::CTRL
                }
            ))
        );
        assert_eq!(
            input(ks("[", Some("["), NONE)),
            Some(KeyInput::plain(Key::Char('[')))
        );
    }

    #[test]
    fn alt_uses_physical_key_not_composed_text() {
        assert_eq!(
            input(ks("s", Some("ß"), with(|m| m.alt = true))),
            Some(KeyInput::new(Key::Char('s'), Mods::ALT))
        );
    }

    #[test]
    fn platform_key_maps_to_sup_not_ctrl() {
        assert_eq!(
            input(ks("s", None, with(|m| m.platform = true))),
            Some(KeyInput::new(Key::Char('s'), Mods::SUPER))
        );
    }

    #[test]
    fn named_keys() {
        let cases = [
            ("enter", Key::Enter),
            ("escape", Key::Esc),
            ("tab", Key::Tab),
            ("backspace", Key::Backspace),
            ("delete", Key::Delete),
            ("insert", Key::Insert),
            ("left", Key::Left),
            ("right", Key::Right),
            ("up", Key::Up),
            ("down", Key::Down),
            ("home", Key::Home),
            ("end", Key::End),
            ("pageup", Key::PageUp),
            ("pagedown", Key::PageDown),
            ("f1", Key::F(1)),
            ("f12", Key::F(12)),
        ];
        for (name, key) in cases {
            // Named keys may carry key_char (enter: "\n", tab: "\t"); it is ignored.
            let text = match name {
                "enter" => Some("\n"),
                "tab" => Some("\t"),
                _ => None,
            };
            assert_eq!(
                input(ks(name, text, NONE)),
                Some(KeyInput::plain(key)),
                "{name}"
            );
        }
    }

    #[test]
    fn space_is_a_space_char() {
        assert_eq!(
            input(ks("space", Some(" "), NONE)),
            Some(KeyInput::plain(Key::Char(' ')))
        );
    }

    #[test]
    fn shift_tab_is_back_tab() {
        assert_eq!(
            input(ks("tab", None, with(|m| m.shift = true))),
            Some(KeyInput::new(Key::BackTab, Mods::SHIFT))
        );
    }

    #[test]
    fn arrows_keep_modifiers() {
        let m = with(|m| {
            m.control = true;
            m.shift = true;
        });
        assert_eq!(
            input(ks("left", None, m)),
            Some(KeyInput::new(
                Key::Left,
                Mods {
                    ctrl: true,
                    shift: true,
                    ..Mods::NONE
                }
            ))
        );
    }

    #[test]
    fn modifier_only_and_unknown_keys_are_ignored() {
        for k in [
            "shift", "control", "alt", "platform", "function", "capslock",
        ] {
            assert_eq!(input(ks(k, None, NONE)), None, "{k}");
        }
        assert_eq!(input(ks("f99", None, NONE)), None);
        assert_eq!(input(ks("mediaplay", None, NONE)), None);
    }

    #[test]
    fn multi_char_text_is_none_not_the_physical_key() {
        assert_eq!(input(ks("e", Some("e\u{301}"), NONE)), None);
        assert_eq!(
            input(ks("x", Some("\u{1F468}\u{200D}\u{1F469}"), NONE)),
            None
        );
        // With a chord held the text is ignored and the physical key is used.
        assert_eq!(
            input(ks("e", Some("e\u{301}"), with(|m| m.alt = true))),
            Some(KeyInput::new(Key::Char('e'), Mods::ALT))
        );
    }

    #[test]
    fn ctrl_shift_letter_uses_physical_key() {
        let m = with(|m| {
            m.control = true;
            m.shift = true;
        });
        assert_eq!(
            input(ks("a", Some("A"), m)),
            Some(KeyInput::new(
                Key::Char('a'),
                Mods {
                    ctrl: true,
                    shift: true,
                    ..Mods::NONE
                }
            ))
        );
    }

    #[test]
    fn ctrl_shift_tab_is_back_tab_with_both_mods() {
        let m = with(|m| {
            m.control = true;
            m.shift = true;
        });
        assert_eq!(
            input(ks("tab", None, m)),
            Some(KeyInput::new(
                Key::BackTab,
                Mods {
                    ctrl: true,
                    shift: true,
                    ..Mods::NONE
                }
            ))
        );
    }

    #[test]
    fn drag_with_navigate_button_is_ignored() {
        let e = MouseMoveEvent {
            position: point(px(120.0), px(70.0)),
            pressed_button: Some(MouseButton::Navigate(gpui_kit::NavigationDirection::Back)),
            modifiers: NONE,
        };
        assert_eq!(pointer_move(&e, origin(), &cells()), None);
    }

    // ---- pointer ----

    fn cells() -> CellMetrics {
        CellMetrics::new(10.0, 20.0)
    }
    fn origin() -> Point<Pixels> {
        point(px(100.0), px(50.0))
    }

    #[test]
    fn mouse_down_is_in_cells_relative_to_origin() {
        let e = MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(135.0), px(111.0)),
            modifiers: with(|m| m.shift = true),
            click_count: 1,
            first_mouse: false,
        };
        assert_eq!(
            pointer_down(&e, origin(), &cells()),
            Some(PointerInput {
                kind: PointerKind::Down(Button::Left),
                col: 3,
                row: 3,
                mods: Mods::SHIFT,
            })
        );
    }

    #[test]
    fn position_before_origin_clamps_to_zero() {
        let e = MouseUpEvent {
            button: MouseButton::Right,
            position: point(px(10.0), px(5.0)),
            modifiers: NONE,
            click_count: 1,
        };
        let p = pointer_up(&e, origin(), &cells()).unwrap();
        assert_eq!(
            (p.kind, p.col, p.row),
            (PointerKind::Up(Button::Right), 0, 0)
        );
    }

    #[test]
    fn navigate_buttons_are_ignored() {
        let e = MouseDownEvent {
            button: MouseButton::Navigate(gpui_kit::NavigationDirection::Back),
            position: point(px(100.0), px(50.0)),
            ..Default::default()
        };
        assert_eq!(pointer_down(&e, origin(), &cells()), None);
    }

    #[test]
    fn move_with_button_is_drag_else_moved() {
        let mut e = MouseMoveEvent {
            position: point(px(120.0), px(70.0)),
            pressed_button: Some(MouseButton::Left),
            modifiers: NONE,
        };
        let p = pointer_move(&e, origin(), &cells()).unwrap();
        assert_eq!(
            (p.kind, p.col, p.row),
            (PointerKind::Drag(Button::Left), 2, 1)
        );
        e.pressed_button = None;
        assert_eq!(
            pointer_move(&e, origin(), &cells()).unwrap().kind,
            PointerKind::Moved
        );
    }

    fn scroll(delta: ScrollDelta) -> ScrollEvent {
        ScrollEvent {
            position: point(px(100.0), px(50.0)),
            delta,
            ..Default::default()
        }
    }

    fn steps(e: &ScrollEvent, acc: &mut ScrollAccumulator) -> Vec<PointerKind> {
        pointer_scroll(e, origin(), &cells(), acc)
            .into_iter()
            .map(|p| p.kind)
            .collect()
    }

    #[test]
    fn wheel_lines_scroll_that_many_rows_by_vertical_sign() {
        // Positive y = content moves down = scroll up.
        let mut acc = ScrollAccumulator::default();
        let down = scroll(ScrollDelta::Lines(point(0.0, -3.0)));
        assert_eq!(steps(&down, &mut acc), vec![PointerKind::ScrollDown; 3]);
        let up = scroll(ScrollDelta::Lines(point(0.0, 1.0)));
        assert_eq!(steps(&up, &mut acc), vec![PointerKind::ScrollUp]);
    }

    #[test]
    fn trackpad_pixels_accumulate_into_whole_rows() {
        // Rows are 20px: 3 x 8px = 24px is one row with 4px carried over.
        let mut acc = ScrollAccumulator::default();
        let nudge = scroll(ScrollDelta::Pixels(point(px(0.0), px(8.0))));
        assert!(steps(&nudge, &mut acc).is_empty());
        assert!(steps(&nudge, &mut acc).is_empty());
        assert_eq!(steps(&nudge, &mut acc), vec![PointerKind::ScrollUp]);
        // 4 + 8 = 12 < 20: still nothing; 12 + 8 = 20: one more row.
        assert!(steps(&nudge, &mut acc).is_empty());
        assert_eq!(steps(&nudge, &mut acc), vec![PointerKind::ScrollUp]);
    }

    #[test]
    fn reversing_or_a_new_gesture_drops_the_remainder() {
        let mut acc = ScrollAccumulator::default();
        assert_eq!(acc.feed(15.0, 20.0, false), 0);
        // Reversal: the 15px up is forgotten, so -15px is not yet a row down.
        assert_eq!(acc.feed(-15.0, 20.0, false), 0);
        assert_eq!(acc.feed(-5.0, 20.0, false), -1);
        assert_eq!(acc.feed(-15.0, 20.0, false), 0);
        // New gesture: the -15px is forgotten.
        assert_eq!(acc.feed(-5.0, 20.0, true), 0);
    }

    #[test]
    fn horizontal_or_zero_scroll_is_ignored() {
        let mut acc = ScrollAccumulator::default();
        let h = scroll(ScrollDelta::Pixels(point(px(9.0), px(0.0))));
        assert!(steps(&h, &mut acc).is_empty());
        assert_eq!(acc.feed(f32::NAN, 20.0, false), 0);
    }

    #[test]
    fn empty_key_char_reads_as_absent() {
        assert_eq!(
            input(ks("q", Some(""), NONE)),
            Some(KeyInput::plain(Key::Char('q')))
        );
        assert!(!carries_text(&down(ks("q", Some(""), NONE), false)));
    }

    #[test]
    fn carries_text_only_for_unchorded_multi_char_text() {
        assert!(carries_text(&down(ks("e", Some("e\u{301}"), NONE), false)));
        assert!(!carries_text(&down(ks("e", Some("e"), NONE), false)));
        assert!(!carries_text(&down(ks("e", None, NONE), false)));
        let ctrl = with(|m| m.control = true);
        assert!(!carries_text(&down(ks("e", Some("e\u{301}"), ctrl), false)));
        // Character input: the modifiers made the text, so it still carries.
        assert!(carries_text(&down(ks("e", Some("e\u{301}"), ctrl), true)));
    }

    fn down(keystroke: Keystroke, prefer_character_input: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke,
            is_held: false,
            prefer_character_input,
        }
    }

    #[test]
    fn altgr_character_input_types_the_character_not_a_ctrl_alt_chord() {
        let altgr = with(|m| {
            m.control = true;
            m.alt = true;
        });
        // Windows AltGr+q on a German layout: '@'.
        assert_eq!(
            key_down_input(&down(ks("q", Some("@"), altgr), true)),
            Some(KeyInput::plain(Key::Char('@')))
        );
        // Polish AltGr+Shift+a: 'Ą', Shift kept.
        let shifted = with(|m| {
            m.control = true;
            m.alt = true;
            m.shift = true;
        });
        assert_eq!(
            key_down_input(&down(ks("a", Some("Ą"), shifted), true)),
            Some(KeyInput::new(Key::Char('Ą'), Mods::SHIFT))
        );
        // Without the platform's flag the same keystroke is a literal chord.
        assert_eq!(
            key_down_input(&down(ks("q", Some("@"), altgr), false)),
            Some(KeyInput::new(
                Key::Char('q'),
                Mods {
                    ctrl: true,
                    alt: true,
                    ..Mods::NONE
                }
            ))
        );
        // A control character is not text: the chord stands.
        assert_eq!(
            key_down_input(&down(
                ks("a", Some("\u{1}"), with(|m| m.control = true)),
                true
            )),
            Some(KeyInput::ctrl('a'))
        );
    }

    #[test]
    fn a_zero_tick_keeps_a_downward_remainder() {
        let mut acc = ScrollAccumulator::default();
        assert_eq!(acc.feed(-15.0, 20.0, false), 0);
        assert_eq!(acc.feed(0.0, 20.0, false), 0);
        assert_eq!(acc.feed(-5.0, 20.0, false), -1);
    }
}
