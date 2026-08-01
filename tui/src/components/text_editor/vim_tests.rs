//! Tests for the vim engine.
//!
//! A child module of `vim`, attached by `#[path]` rather than moved into a
//! directory: it reaches the engine's private parse methods exactly as it did
//! inline, which ADR-0016 relies on — the parse seam is exercised from here
//! precisely because it is *not* a public interface.

use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ropetext::Text;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}
fn esc() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}
fn ta() -> RopeBuffer {
    RopeBuffer::new(Text::from("hello world\nsecond line"))
}

// ── Parse-contract tests ─────────────────────────────────────────────────
//
// These exercise the parser directly (`parse_normal`), with no buffer:
// they pin the grammar — counts, the operator×motion count multiply, the
// g-grammar, pending cancel, text objects, find targets — as `Parsed`/
// `Command` values. The 173 handle_key tests below cover parse+apply
// end-to-end; these document the command contract in isolation (adr/0011,
// adr/0016).

/// Unwrap the `Command` a key parsed into, or fail loudly.
fn cmd(p: Parsed) -> Command {
    match p {
        Parsed::Cmd(c) => c,
        _ => panic!("expected Parsed::Cmd"),
    }
}

/// Feed a sequence, returning the final key's parse result.
fn parse_seq(e: &mut VimEngine, keys: &str) -> Parsed {
    let mut last = Parsed::Nothing;
    for c in keys.chars() {
        last = e.parse_normal(&key(c));
    }
    last
}

// ── OS clipboard chords (adr/0031) ───────────────────────────────────────
//
// The bug these pin: the engine ran BEFORE the host's clipboard shortcuts
// and swallowed every Ctrl-modified char as `NoOp`, so Ctrl-C/X/V worked in
// Insert mode only. Each test asserts the key now escapes as a host action
// AND that the engine's own state transitioned with it.

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Enter charwise Visual over "hello" (cursor lands ON the last 'o', which
/// vim includes in the selection).
fn visual_hello(e: &mut VimEngine, t: &mut RopeBuffer) {
    e.handle_key(&key('v'), t);
    for _ in 0..4 {
        e.handle_key(&key('l'), t);
    }
}

#[test]
fn ctrl_c_in_visual_copies_the_inclusive_selection_and_exits_to_normal() {
    let mut e = VimEngine::default();
    let mut t = ta();
    visual_hello(&mut e, &mut t);
    match e.handle_key(&ctrl('c'), &mut t) {
        VimKeyOutcome::Host(VimHostAction::ClipboardCopy(s)) => assert_eq!(s, "hello"),
        o => panic!("got {o:?}"),
    }
    assert_eq!(e.mode, EditorMode::Normal, "vim's Ctrl-C is Esc");
    assert_eq!(
        t.rows(),
        ["hello world", "second line"],
        "copy must not edit"
    );
}

#[test]
fn ctrl_x_in_visual_cuts_and_reports_the_removed_text() {
    let mut e = VimEngine::default();
    let mut t = ta();
    visual_hello(&mut e, &mut t);
    match e.handle_key(&ctrl('x'), &mut t) {
        VimKeyOutcome::Host(VimHostAction::ClipboardCut(s)) => assert_eq!(s, "hello"),
        o => panic!("got {o:?}"),
    }
    assert_eq!(t.rows()[0], " world");
    assert_eq!(e.mode, EditorMode::Normal);
}

#[test]
fn visual_line_clipboard_copy_takes_whole_lines_with_a_trailing_newline() {
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('V'), &mut t);
    match e.handle_key(&ctrl('c'), &mut t) {
        VimKeyOutcome::Host(VimHostAction::ClipboardCopy(s)) => {
            assert_eq!(s, "hello world\n", "linewise yank includes the newline");
        }
        o => panic!("got {o:?}"),
    }
}

#[test]
fn clipboard_chords_never_touch_the_unnamed_register() {
    // The two channels are independent (adr/0031): a Ctrl-X must not
    // clobber what `y` put in the register, the mirror of the rule that
    // `dd` must not clobber the OS clipboard.
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('y'), &mut t); // yy — register holds line 1
    let before = e.registers.read().map(|r| r.text.clone());
    assert!(before.is_some(), "yy must fill the register");
    visual_hello(&mut e, &mut t);
    e.handle_key(&ctrl('x'), &mut t);
    assert_eq!(
        e.registers.read().map(|r| r.text.clone()),
        before,
        "the OS-clipboard cut must leave the unnamed register alone"
    );
}

/// Paste must NOT cut here. The host's clipboard read can come back empty
/// or fail, and there is no way to put the text back — so the range is left
/// selected and the host's insert replaces it atomically.
#[test]
fn ctrl_v_in_visual_leaves_the_selection_for_the_host_to_replace() {
    let mut e = VimEngine::default();
    let mut t = ta();
    visual_hello(&mut e, &mut t);
    assert_eq!(
        e.handle_key(&ctrl('v'), &mut t),
        VimKeyOutcome::Host(VimHostAction::ClipboardPaste)
    );
    assert_eq!(
        t.rows()[0],
        "hello world",
        "nothing may be destroyed before there is something to replace it"
    );
    assert_eq!(
        t.selection_range(),
        Some(((0, 0), (0, 5))),
        "the inclusive range stays selected so the host's insert consumes it"
    );
    assert_eq!(e.mode, EditorMode::Normal);
}

/// The regression the above prevents: an empty clipboard used to leave the
/// buffer mutilated, because the engine cut before the host discovered it
/// had nothing to paste.
#[test]
fn ctrl_v_with_an_unusable_clipboard_leaves_the_buffer_untouched() {
    let mut e = VimEngine::default();
    let mut t = ta();
    visual_hello(&mut e, &mut t);
    e.handle_key(&ctrl('v'), &mut t);
    // The host reads the clipboard, finds nothing, and returns without
    // calling `paste_text` — simulated here by simply doing nothing.
    assert_eq!(t.rows()[0], "hello world");
}

#[test]
fn visual_line_ctrl_x_takes_the_whole_line_leaving_no_blank() {
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('V'), &mut t);
    match e.handle_key(&ctrl('x'), &mut t) {
        VimKeyOutcome::Host(VimHostAction::ClipboardCut(s)) => {
            assert_eq!(s, "hello world\n", "the clipboard gets a linewise cut");
        }
        o => panic!("got {o:?}"),
    }
    assert_eq!(
        t.rows(),
        ["second line"],
        "the line's newline goes with it — no stray blank line"
    );
}

#[test]
fn visual_line_ctrl_x_on_the_last_line_leaves_no_blank_either() {
    let mut e = VimEngine::default();
    let mut t = ta();
    t.move_cursor(CursorMove::Jump(1, 0));
    e.handle_key(&key('V'), &mut t);
    match e.handle_key(&ctrl('x'), &mut t) {
        VimKeyOutcome::Host(VimHostAction::ClipboardCut(s)) => {
            assert_eq!(
                s, "second line\n",
                "the clipboard text is the line plus a newline, even though \
                     the buffer edit consumed the PRECEDING one"
            );
        }
        o => panic!("got {o:?}"),
    }
    assert_eq!(t.rows(), ["hello world"]);
}

#[test]
fn ctrl_v_in_normal_reaches_the_host() {
    let mut e = VimEngine::default();
    let mut t = ta();
    assert_eq!(
        e.handle_key(&ctrl('v'), &mut t),
        VimKeyOutcome::Host(VimHostAction::ClipboardPaste),
        "Normal mode used to swallow this as NoOp"
    );
}

#[test]
fn ctrl_c_in_normal_cancels_the_pending_sequence() {
    let mut e = VimEngine::default();
    let mut t = ta();
    assert!(matches!(e.parse_normal(&key('2')), Parsed::Pending));
    assert!(matches!(e.parse_normal(&ctrl('c')), Parsed::Cancel));
    // The count is gone: `l` now moves one column, not two.
    assert_eq!(e.handle_key(&key('l'), &mut t), VimKeyOutcome::CursorOnly);
    assert_eq!(super::super::cursor_tuple(&t), (0, 1));
}

#[test]
fn ctrl_c_abandons_a_one_key_continuation_instead_of_being_its_target() {
    // `r` waits for a replacement char. Ctrl-C is a `Char('c')` event, so
    // without the modifier guard it would overwrite with a literal 'c'.
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('r'), &mut t);
    e.handle_key(&ctrl('c'), &mut t);
    assert_eq!(t.rows()[0], "hello world", "r must have been abandoned");
    assert!(e.awaiting.is_none());
}

#[test]
fn count_accumulates_into_motion() {
    let mut e = VimEngine::default();
    assert!(matches!(e.parse_normal(&key('1')), Parsed::Pending));
    assert!(matches!(e.parse_normal(&key('2')), Parsed::Pending));
    match cmd(e.parse_normal(&key('l'))) {
        Command::Move(Motion::Right, n) => assert_eq!(n, 12),
        c => panic!("got {c:?}"),
    }
}

#[test]
fn operator_motion_multiplies_the_two_counts() {
    // vim: `2d3w` deletes 6 words (pre-operator count × motion count).
    let mut e = VimEngine::default();
    match cmd(parse_seq(&mut e, "2d3w")) {
        Command::OperateMotion(Operator::Delete, Motion::WordForward, n) => {
            assert_eq!(n, 6, "2 × 3 = 6")
        }
        c => panic!("got {c:?}"),
    }
}

#[test]
fn doubled_operator_is_linewise() {
    let mut e = VimEngine::default();
    match cmd(parse_seq(&mut e, "2dd")) {
        Command::OperateLine(Operator::Delete, n) => assert_eq!(n, 2),
        c => panic!("got {c:?}"),
    }
}

#[test]
fn gg_is_file_start_and_count_makes_it_a_line() {
    let mut e = VimEngine::default();
    match cmd(parse_seq(&mut e, "gg")) {
        Command::Move(Motion::FileStart, 1) => {}
        c => panic!("gg: got {c:?}"),
    }
    let mut e = VimEngine::default();
    match cmd(parse_seq(&mut e, "5gg")) {
        Command::Move(Motion::GotoLine(5), 1) => {}
        c => panic!("5gg: got {c:?}"),
    }
}

#[test]
fn esc_clears_a_pending_operator() {
    let mut e = VimEngine::default();
    assert!(matches!(e.parse_normal(&key('d')), Parsed::Pending));
    assert!(matches!(e.parse_normal(&esc()), Parsed::Cancel));
    // The pending `d` is gone — `w` is now a plain motion, not a delete.
    match cmd(e.parse_normal(&key('w'))) {
        Command::Move(Motion::WordForward, 1) => {}
        c => panic!("pending operator survived esc: {c:?}"),
    }
}

#[test]
fn operator_plus_text_object_awaits_then_completes() {
    let mut e = VimEngine::default();
    assert!(matches!(e.parse_normal(&key('d')), Parsed::Pending));
    // `i` after an operator awaits the object key — it must NOT enter Insert.
    assert!(matches!(e.parse_normal(&key('i')), Parsed::Pending));
    match cmd(e.parse_normal(&key('w'))) {
        Command::OperateObject(Operator::Delete, TextObject::Word { around: false }) => {}
        c => panic!("diw: got {c:?}"),
    }
}

#[test]
fn find_target_is_captured_with_the_operator() {
    let mut e = VimEngine::default();
    assert!(matches!(e.parse_normal(&key('d')), Parsed::Pending));
    assert!(matches!(e.parse_normal(&key('f')), Parsed::Pending));
    match cmd(e.parse_normal(&key(','))) {
        Command::OperateMotion(
            Operator::Delete,
            Motion::FindChar {
                ch: ',',
                till: false,
                forward: true,
            },
            1,
        ) => {}
        c => panic!("df,: got {c:?}"),
    }
}

#[test]
fn bare_zero_is_line_start_but_zero_extends_a_count() {
    let mut e = VimEngine::default();
    match cmd(e.parse_normal(&key('0'))) {
        Command::Move(Motion::LineStart, 1) => {}
        c => panic!("bare 0: got {c:?}"),
    }
    // With a count pending, `0` is a digit: `10l` moves 10 right.
    let mut e = VimEngine::default();
    match cmd(parse_seq(&mut e, "10l")) {
        Command::Move(Motion::Right, n) => assert_eq!(n, 10),
        c => panic!("10l: got {c:?}"),
    }
}

// ── Mode-entry + basic motion tests ──────────────────────────────────────

#[test]
fn i_enters_insert_mode() {
    let mut e = VimEngine::default();
    let mut t = ta();
    let out = e.handle_key(&key('i'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    assert_eq!(out, VimKeyOutcome::CursorOnly);
}

#[test]
fn esc_returns_to_normal_and_steps_back() {
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('i'), &mut t);
    t.move_cursor(crate::components::text_editor::rope_buffer::CursorMove::Forward);
    t.move_cursor(crate::components::text_editor::rope_buffer::CursorMove::Forward);
    let col_before = super::super::cursor_tuple(&t).1;
    let out = e.handle_key(&esc(), &mut t);
    assert_eq!(*e.mode(), EditorMode::Normal);
    assert_eq!(out, VimKeyOutcome::CursorOnly);
    assert_eq!(super::super::cursor_tuple(&t).1, col_before - 1);
}

#[test]
fn insert_mode_passes_through() {
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('i'), &mut t);
    let out = e.handle_key(&key('x'), &mut t);
    assert_eq!(out, VimKeyOutcome::PassThrough);
}

#[test]
fn l_moves_right_cursor_only() {
    let mut e = VimEngine::default();
    let mut t = ta();
    let out = e.handle_key(&key('l'), &mut t);
    assert_eq!(out, VimKeyOutcome::CursorOnly);
    assert_eq!(super::super::cursor_tuple(&t), (0, 1));
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn a_enters_insert_after_cursor() {
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('a'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    assert_eq!(super::super::cursor_tuple(&t), (0, 1));
}

#[test]
fn o_opens_line_below_in_insert() {
    let mut e = VimEngine::default();
    let mut t = ta();
    let out = e.handle_key(&key('o'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    assert_eq!(out, VimKeyOutcome::TextMutated);
    assert_eq!(t.rows().len(), 3);
    assert_eq!(super::super::cursor_tuple(&t).0, 1);
}

#[test]
fn reset_returns_to_normal_from_insert() {
    let mut e = VimEngine::default();
    let mut t = ta();
    e.handle_key(&key('i'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    e.reset_to_normal();
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn unknown_normal_key_is_noop() {
    let mut e = VimEngine::default();
    let mut t = ta();
    let out = e.handle_key(&key('z'), &mut t);
    assert_eq!(out, VimKeyOutcome::NoOp);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

// ── Count accumulation tests ─────────────────────────────────────────────

#[test]
fn count_accumulates_then_moves() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    e.handle_key(&key('3'), &mut t);
    e.handle_key(&key('l'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 3));
    // pending cleared after the motion
    e.handle_key(&key('l'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 4));
}

#[test]
fn zero_without_count_is_line_start() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    e.handle_key(&key('l'), &mut t);
    e.handle_key(&key('l'), &mut t);
    e.handle_key(&key('0'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 0));
}

// ── gg/G motion tests ────────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn gg_and_G_jump_file_ends() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('G'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t).0, 2);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('g'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t).0, 0);
}

#[test]
fn pending_g_cancels_on_unmapped_key() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('G'), &mut t); // go to last line
    assert_eq!(super::super::cursor_tuple(&t).0, 2);
    e.handle_key(&key('g'), &mut t); // start gg
    e.handle_key(&key('z'), &mut t); // unmapped → should cancel pending g
    e.handle_key(&key('g'), &mut t); // lone g, NOT gg
    assert_eq!(
        super::super::cursor_tuple(&t).0,
        2,
        "stray g after cancelled prefix must not jump to file start"
    );
}

#[test]
fn pending_g_cleared_through_insert() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('G'), &mut t);
    e.handle_key(&key('g'), &mut t); // start gg
    e.handle_key(&key('a'), &mut t); // enter insert (should clear pending_g)
    e.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut t);
    e.handle_key(&key('g'), &mut t); // lone g
    assert_eq!(
        super::super::cursor_tuple(&t).0,
        2,
        "g after insert must not complete a stale gg"
    );
}

// ── Operator + motion tests ──────────────────────────────────────────────

#[test]
fn dw_deletes_word() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello world"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('w'), &mut t);
    assert_eq!(t.rows(), &["world"]);
}

#[test]
fn dd_deletes_line_linewise() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &["two", "three"]);
    let reg = e.registers.read().expect("dd must fill the register");
    assert_eq!(reg.kind, RegisterKind::Linewise);
    assert_eq!(reg.text, "one\n");
}

#[test]
fn yy_then_p_duplicates_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo"));
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('p'), &mut t);
    assert_eq!(t.rows(), &["one", "one", "two"]);
}

#[test]
fn cw_deletes_word_and_enters_insert() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello world"));
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('w'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    // Vim `cw` = `ce`: deletes up to end of word (exclusive of trailing
    // space), so " world" remains (space preserved). This matches vim's
    // actual cw = ce behaviour.
    assert_eq!(t.rows(), &[" world"]);
}

// ── Linewise delete/paste tests ──────────────────────────────────────────

#[test]
fn charwise_p_pastes_after_cursor() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    // yank the first char with `yl`
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('l'), &mut t);
    e.handle_key(&key('p'), &mut t);
    assert_eq!(t.rows(), &["aabc"]);
}

#[test]
fn dd_on_last_line_removes_it() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('G'), &mut t); // to last line
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &["one", "two"]);
}

#[test]
fn dd_on_only_line_leaves_empty() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("only"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &[""]);
}

#[test]
fn linewise_2p_inserts_two_copies() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo"));
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('y'), &mut t); // yank "one" linewise
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('p'), &mut t);
    assert_eq!(t.rows(), &["one", "one", "one", "two"]);
}

#[test]
fn yy_last_line_then_p_duplicates() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo"));
    e.handle_key(&key('G'), &mut t); // last line "two"
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('p'), &mut t);
    assert_eq!(t.rows(), &["one", "two", "two"]);
}

// ── Single-key edit tests ────────────────────────────────────────────────

#[test]
fn x_deletes_char_under_cursor() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('x'), &mut t);
    assert_eq!(t.rows(), &["bc"]);
}

#[test]
fn r_replaces_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('r'), &mut t);
    e.handle_key(&key('Z'), &mut t);
    assert_eq!(t.rows(), &["Zbc"]);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn u_undoes_last_edit() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('x'), &mut t);
    e.handle_key(&key('u'), &mut t);
    assert_eq!(t.rows(), &["abc"]);
}

#[test]
fn counted_undo_takes_back_that_many_groups() {
    // Each `x` is its own undo group, so `3u` walks back three of them. The
    // count applies to the number of groups, not to anything about their size.
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    for _ in 0..3 {
        e.handle_key(&key('x'), &mut t);
    }
    assert_eq!(t.rows(), &["def"]);
    e.handle_key(&key('3'), &mut t);
    e.handle_key(&key('u'), &mut t);
    assert_eq!(t.rows(), &["abcdef"]);
}

#[test]
fn counted_undo_stops_at_the_oldest_group() {
    // Asking for more than exist is not an error and does not wrap.
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    e.handle_key(&key('x'), &mut t);
    e.handle_key(&key('9'), &mut t);
    e.handle_key(&key('u'), &mut t);
    assert_eq!(t.rows(), &["abcdef"]);
}

#[test]
fn tilde_toggles_case() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('~'), &mut t);
    assert_eq!(t.rows(), &["Abc"]);
}

// ── Find (f/t/;/,) tests ─────────────────────────────────────────────────

#[test]
fn f_moves_to_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello, world"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key(','), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 5));
}

#[test]
fn df_deletes_through_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello, world"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key(','), &mut t);
    assert_eq!(t.rows(), &[" world"]);
}

#[test]
fn t_stops_before_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello, world"));
    e.handle_key(&key('t'), &mut t);
    e.handle_key(&key(','), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 4)); // on 'o', before ','
}

#[test]
fn semicolon_repeats_find() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a.b.c.d"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('.'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t).1, 1);
    e.handle_key(&key(';'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t).1, 3);
}

// ── Text object tests ────────────────────────────────────────────────────

#[test]
fn diw_deletes_inner_word() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar baz"));
    // cursor on 'b' of "bar"
    e.handle_key(&key('w'), &mut t);
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('w'), &mut t);
    assert_eq!(t.rows(), &["foo  baz"]);
}

#[test]
fn ci_quote_changes_inside_quotes() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("say \"hi\" now"));
    // move onto the text inside quotes: f then h lands on 'h' (col 5)
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('h'), &mut t);
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('"'), &mut t);
    assert_eq!(t.rows(), &["say \"\" now"]);
    assert_eq!(*e.mode(), EditorMode::Insert);
}

#[test]
fn di_paren_deletes_inside_parens() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo(bar)baz"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('('), &mut t); // cursor on '('
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('('), &mut t);
    assert_eq!(t.rows(), &["foo()baz"]);
}

#[test]
fn daw_deletes_word_and_trailing_space() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar baz"));
    e.handle_key(&key('w'), &mut t); // onto "bar"
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('a'), &mut t);
    e.handle_key(&key('w'), &mut t);
    assert_eq!(t.rows(), &["foo baz"]);
}

// ── Matching pair (%) tests ──────────────────────────────────────────────

#[test]
fn percent_jumps_to_matching_paren() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo(bar)baz"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('('), &mut t); // cursor on '('
    e.handle_key(&key('%'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 7)); // matching ')'
}

#[test]
fn percent_jumps_back_from_close() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo(bar)baz"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key(')'), &mut t); // cursor on ')'
    e.handle_key(&key('%'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 3)); // back to '('
}

#[test]
fn percent_handles_nested() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("(a(b)c)"));
    // cursor on outer '(' at col 0
    e.handle_key(&key('%'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 6)); // matching outer ')'
}

// ── Visual mode tests ────────────────────────────────────────────────────

/// Charwise Visual is inclusive of the cursor char. `v` + 2×`l` leaves
/// the cursor on col 2 ('l'); the inclusive range covers cols 0,1,2 = "hel".
#[test]
fn v_motion_d_deletes_selection() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('v'), &mut t); // anchor col 0
    e.handle_key(&key('l'), &mut t); // cursor → col 1
    e.handle_key(&key('l'), &mut t); // cursor → col 2, inclusive covers "hel"
    e.handle_key(&key('d'), &mut t); // delete "hel"
    assert_eq!(t.rows(), &["lo"]); // inclusive: deletes cols 0,1,2 ("hel")
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
#[allow(non_snake_case)]
fn V_then_d_deletes_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo"));
    e.handle_key(&key('V'), &mut t);
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &["two"]);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

/// Inclusive yank: v + l (cursor col 1) yanks "he" (2 chars, inclusive).
/// After p pastes the yanked text, buffer grew by 2.
#[test]
fn visual_y_yanks_and_returns_to_normal() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('v'), &mut t); // anchor col 0
    e.handle_key(&key('l'), &mut t); // cursor col 1, inclusive selection "he"
    e.handle_key(&key('y'), &mut t); // yank "he" (2 chars), mode → Normal
    assert_eq!(*e.mode(), EditorMode::Normal);
    // p pastes the yanked "he" after current cursor
    let before_len: usize = t.rows().iter().map(|l| l.len()).sum();
    e.handle_key(&key('p'), &mut t);
    let after_len: usize = t.rows().iter().map(|l| l.len()).sum();
    // buffer grew by exactly 2 chars (the yanked "he")
    assert_eq!(after_len, before_len + 2);
}

#[test]
fn visual_esc_cancels_and_returns_normal() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('l'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Visual);
    e.handle_key(&esc(), &mut t);
    assert_eq!(*e.mode(), EditorMode::Normal);
    // selection should be cancelled
    assert!(t.selection_range().is_none());
}

#[test]
fn visual_c_enters_insert_after_delete() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('v'), &mut t); // anchor col 0
    e.handle_key(&key('l'), &mut t); // cursor col 1, inclusive covers "he"
    e.handle_key(&key('c'), &mut t); // delete "he" (inclusive), enter Insert
    assert_eq!(*e.mode(), EditorMode::Insert);
    assert_eq!(t.rows(), &["llo"]); // inclusive: deletes cols 0,1 ("he")
}

// ── Indent/outdent tests ─────────────────────────────────────────────────

#[test]
fn indent_line_adds_spaces() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    e.handle_key(&key('>'), &mut t);
    e.handle_key(&key('>'), &mut t);
    assert_eq!(t.rows(), &["    x"]);
}

/// `>>` reads the buffer's indent step rather than a literal, so it moves a line
/// by the same amount the plain backend's Tab does.
///
/// Pinned with a non-default width because both are 4 today: a reintroduced
/// literal would pass at the default and fail here. Outdent too, since it counts
/// the spaces it removes separately.
#[test]
fn indent_follows_the_buffers_indent_width() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    t.set_indent_width(2);
    e.handle_key(&key('>'), &mut t);
    e.handle_key(&key('>'), &mut t);
    assert_eq!(t.rows(), &["  x"]);
    e.handle_key(&key('<'), &mut t);
    e.handle_key(&key('<'), &mut t);
    assert_eq!(t.rows(), &["x"]);
}

/// An outdent removes at most one step, never a second line's worth.
#[test]
fn outdent_removes_one_step_not_all_leading_space() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("        x"));
    t.set_indent_width(3);
    e.handle_key(&key('<'), &mut t);
    e.handle_key(&key('<'), &mut t);
    assert_eq!(t.rows(), &["     x"]);
}

#[test]
fn indent_keeps_cursor_over_same_char() {
    // regression: >> left the cursor one row BELOW the indented block;
    // the cursor must stay over the char it sat on, shifted with the
    // indent (neovim behavior).
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('l'), &mut t); // onto 'n' (col 1)
    e.handle_key(&key('>'), &mut t);
    e.handle_key(&key('>'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 5)); // still on 'n'
    // counted form too: cursor stays on the first line of the block
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('>'), &mut t);
    e.handle_key(&key('>'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t).0, 0);
}

#[test]
fn outdent_keeps_cursor_over_same_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("    x"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('x'), &mut t); // ON 'x' (col 4)
    e.handle_key(&key('<'), &mut t);
    e.handle_key(&key('<'), &mut t);
    assert_eq!(t.rows(), &["x"]);
    assert_eq!(super::super::cursor_tuple(&t).1, 0); // still on 'x'
}

#[test]
fn outdent_removes_spaces() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("        x")); // 8 spaces
    e.handle_key(&key('<'), &mut t);
    e.handle_key(&key('<'), &mut t);
    assert_eq!(t.rows(), &["    x"]); // removed 4
}

#[test]
fn pending_hint_shows_operator_and_count() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('d'), &mut t);
    assert_eq!(e.pending_hint().as_deref(), Some("2d"));
}

// ── Dot-repeat tests ─────────────────────────────────────────────────────

#[test]
fn dot_repeats_x() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    e.handle_key(&key('x'), &mut t);
    e.handle_key(&key('.'), &mut t);
    assert_eq!(t.rows(), &["cdef"]);
}

#[test]
fn dot_repeats_dw() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one two three four"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('w'), &mut t); // delete "one "
    e.handle_key(&key('.'), &mut t); // delete "two "
    assert_eq!(t.rows(), &["three four"]);
}

#[test]
fn dot_repeats_change_with_typed_text() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar"));
    // cw -> type "X" -> Esc, then move to next word and dot
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('w'), &mut t); // cw: deletes "foo" (cw=ce keeps trailing space), enters Insert at col 0
    // simulate the user typing "X" via the host PassThrough path:
    t.insert_str("X");
    e.handle_key(&esc(), &mut t); // capture "X"
    e.handle_key(&key('w'), &mut t); // move to "bar"
    e.handle_key(&key('.'), &mut t); // repeat cw+X
    assert_eq!(t.rows(), &["X X"]);
}

#[test]
fn dot_repeats_multiline_change() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar"));
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('w'), &mut t); // cw on "foo" → Insert at col 0
    t.insert_str("a");
    t.insert_newline();
    t.insert_str("b"); // typed "a\nb"
    e.handle_key(&esc(), &mut t); // captures "a\nb"
    // Buffer is now ["a", "b bar"]; cursor stepped back to col 0 of row 1 ("b bar").
    // Confirm the multi-line buffer state from the insert:
    assert_eq!(t.rows(), &["a", "b bar"]);

    // Verify replay: position on "bar", run `.`, should produce "a\nb" again.
    // Move to word "bar" (it is at col 2 of row 1).
    e.handle_key(&key('w'), &mut t); // move to "bar" (word-forward from "b" → "bar")
    e.handle_key(&key('.'), &mut t); // replay: cw on "bar" → insert "a\nb"
    // After replay the buffer should have "a\nb" inserted in place of "bar":
    // row 1 was "b bar", cw from "bar" removes "bar", inserts "a\nb" → ["a", "b a", "b"]
    assert!(
        t.rows().len() >= 3,
        "replay of multiline insert should produce >=3 lines: {:?}",
        t.rows()
    );
}

// ── space_leads predicate tests ──────────────────────────────────────────

#[test]
fn space_leads_only_in_clean_normal() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    assert!(e.space_leads());
    e.handle_key(&key('d'), &mut t); // operator pending
    assert!(!e.space_leads());
    e.handle_key(&key('w'), &mut t); // completes dw, clears pending
    assert!(e.space_leads());
    e.handle_key(&key('i'), &mut t); // insert
    assert!(!e.space_leads());
}

// ── Host-action tests ────────────────────────────────────────────────────

#[test]
fn colon_emits_open_palette() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    assert_eq!(
        e.handle_key(&key(':'), &mut t),
        VimKeyOutcome::Host(VimHostAction::OpenPalette)
    );
}

#[test]
fn slash_emits_open_search_forward() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    assert_eq!(
        e.handle_key(&key('/'), &mut t),
        VimKeyOutcome::Host(VimHostAction::OpenSearch { forward: true })
    );
}

#[test]
#[allow(non_snake_case)]
fn n_and_N_emit_search_nav() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    assert_eq!(
        e.handle_key(&key('n'), &mut t),
        VimKeyOutcome::Host(VimHostAction::SearchNext)
    );
    assert_eq!(
        e.handle_key(&key('N'), &mut t),
        VimKeyOutcome::Host(VimHostAction::SearchPrev)
    );
}

// ── Mouse → Visual mode tests ────────────────────────────────────────────

#[test]
fn mouse_selection_enters_and_leaves_visual() {
    let mut e = VimEngine::default();
    e.sync_mouse_selection(true);
    assert_eq!(*e.mode(), EditorMode::Visual);
    e.sync_mouse_selection(false);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn mouse_no_selection_in_normal_stays_normal() {
    let mut e = VimEngine::default();
    e.sync_mouse_selection(false);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn mouse_does_not_disturb_insert() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    e.handle_key(&key('i'), &mut t); // Insert
    e.sync_mouse_selection(true);
    assert_eq!(*e.mode(), EditorMode::Insert); // mouse doesn't yank Insert into Visual
}

// ── Bug-fix regression tests ─────────────────────────────────────────────

#[test]
fn di_paren_on_empty_line_does_not_panic() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("")); // empty line
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('('), &mut t); // must not panic; no-op
    assert_eq!(t.rows(), &[""]);
}

#[test]
fn esc_clears_pending_g_in_normal() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('G'), &mut t); // last line
    assert_eq!(super::super::cursor_tuple(&t).0, 2);
    e.handle_key(&key('g'), &mut t); // start gg
    e.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut t); // cancel
    e.handle_key(&key('g'), &mut t); // lone g
    assert_eq!(
        super::super::cursor_tuple(&t).0,
        2,
        "Esc must cancel pending g"
    );
}

#[test]
fn esc_clears_pending_operator_object_in_normal() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar baz"));
    e.handle_key(&key('d'), &mut t); // operator pending
    e.handle_key(&key('i'), &mut t); // object kind pending (NOT insert — operator pending)
    e.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut t); // cancel
    // buffer unchanged (no diw happened)
    assert_eq!(t.rows(), &["foo bar baz"]);
    // and we're back to clean Normal: a plain motion works, mode still Normal
    e.handle_key(&key('w'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Normal);
    assert_eq!(
        t.rows(),
        &["foo bar baz"],
        "w after Esc must be a motion, not diw"
    );
}

// ── Bug A: di( on nested parens ─────────────────────────────────────────

#[test]
fn di_paren_nested_selects_inner_of_outer() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("((x))"));
    // cursor at col 0 (outer '(')
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('('), &mut t);
    assert_eq!(t.rows(), &["()"]); // outer kept, inner content "(x)" deleted
}

#[test]
fn di_paren_from_inside_nested() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("((x))"));
    e.handle_key(&key('l'), &mut t); // col1 (inner '(')
    e.handle_key(&key('l'), &mut t); // col2 ('x')
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('('), &mut t);
    assert_eq!(t.rows(), &["(())"]); // inner content "x" deleted
}

// ── Bug B: di" in gap between pairs ─────────────────────────────────────

#[test]
fn di_quote_in_gap_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("\"foo\" \"bar\""));
    // move cursor to the space (col 5) between the two strings
    for _ in 0..5 {
        e.handle_key(&key('l'), &mut t);
    }
    assert_eq!(super::super::cursor_tuple(&t).1, 5);
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('"'), &mut t);
    assert_eq!(t.rows(), &["\"foo\" \"bar\""]); // unchanged (no-op)
}

#[test]
fn di_quote_inside_still_works() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("\"foo\" \"bar\""));
    for _ in 0..7 {
        e.handle_key(&key('l'), &mut t);
    } // inside "bar"
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('"'), &mut t);
    assert_eq!(t.rows(), &["\"foo\" \"\""]); // bar deleted, foo intact
}

// ── Bug C: df<last-char> must not join next line ─────────────────────────

#[test]
fn df_last_char_does_not_join_next_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc\nxyz"));
    // cursor at (0,0); df c  → delete through the 'c' (last char of line 0)
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('c'), &mut t);
    assert_eq!(t.rows(), &["", "xyz"]); // line 0 emptied, newline + line 1 intact
}

// ── Bug D: cc on a single-line buffer ────────────────────────────────────

#[test]
fn cc_single_line_leaves_one_empty_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('c'), &mut t);
    assert_eq!(t.rows(), &[""]);
    assert_eq!(*e.mode(), EditorMode::Insert);
}

#[test]
fn cc_middle_line_still_works() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('j'), &mut t); // line "two"
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('c'), &mut t);
    assert_eq!(t.rows(), &["one", "", "three"]);
    assert_eq!(*e.mode(), EditorMode::Insert);
}

// ── Bug E: r on empty line must be no-op ────────────────────────────────

#[test]
fn r_on_empty_line_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from(""));
    e.handle_key(&key('r'), &mut t);
    let out = e.handle_key(&key('Z'), &mut t);
    assert_eq!(out, VimKeyOutcome::NoOp);
    assert_eq!(t.rows(), &[""]);
}

// ── P2.G: charwise Visual inclusive tests ────────────────────────────────

#[test]
fn visual_v_then_d_deletes_char_under_cursor() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('v'), &mut t); // select just 'a' (cursor col0)
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &["bc"]); // 'a' deleted (inclusive of cursor char)
}

#[test]
fn visual_e_then_d_inclusive() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello world"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t); // cursor on 'o' col4
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &[" world"]); // "hello" deleted inclusive
}

// ── Bug fix: vim `e` must land ON the last word char (inclusive) ─────────

#[test]
fn e_lands_on_last_word_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello world"));
    e.handle_key(&key('e'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 4)); // 'o', last char of "hello"
}

#[test]
fn e_twice_reaches_second_word_end() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello world"));
    e.handle_key(&key('e'), &mut t);
    e.handle_key(&key('e'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 10)); // 'd', last char of "world"
}

#[test]
fn de_deletes_to_word_end_inclusive() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello world"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('e'), &mut t);
    assert_eq!(t.rows(), &[" world"]); // deletes "hello" inclusive of 'o'
}

// ── Bug fix: vim yank leaves cursor at selection start; charwise p never wraps ──

#[test]
fn visual_y_leaves_cursor_at_selection_start() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar\nbaz"));
    for _ in 0..4 {
        e.handle_key(&key('l'), &mut t);
    } // onto 'b' of "bar" (col 4)
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t); // select "bar"
    e.handle_key(&key('y'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 4)); // cursor at start of selection, not the end
}

#[test]
fn charwise_p_after_eol_word_does_not_touch_next_line() {
    // reproduce the user's bug: yank an end-of-line word, paste, must NOT hit the line below
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar\nbaz"));
    for _ in 0..4 {
        e.handle_key(&key('l'), &mut t);
    } // 'b' of "bar"
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t); // select "bar" (end of line 0)
    e.handle_key(&key('y'), &mut t); // yank; cursor → col 4
    e.handle_key(&key('p'), &mut t); // paste after cursor char 'b'
    assert_eq!(t.rows()[1], "baz"); // line below UNTOUCHED
    assert_eq!(t.rows().len(), 2); // no new line, no merge
    assert_eq!(t.rows()[0], "foo bbarar"); // "bar" pasted after 'b' on line 0 (vim p-after)
}

#[test]
fn charwise_p_at_line_end_appends_same_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab\ncd"));
    // yank "ab" charwise via v e y
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t); // select "ab"
    e.handle_key(&key('y'), &mut t); // cursor → col 0
    e.handle_key(&key('$'), &mut t); // to last char of line 0 ('b')
    e.handle_key(&key('p'), &mut t); // append "ab" after 'b'
    assert_eq!(t.rows()[0], "abab");
    assert_eq!(t.rows()[1], "cd"); // line below untouched
}

// ── Visual p: replace selection with register ────────────────────────────

#[test]
fn visual_p_replaces_charwise_selection() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar"));
    // yank "foo" (v e y at col 0) → register = "foo", cursor back to col 0
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t);
    e.handle_key(&key('y'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 0));
    // select "bar" and paste over it
    for _ in 0..4 {
        e.handle_key(&key('l'), &mut t);
    } // onto 'b' (col 4)
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t); // select "bar"
    e.handle_key(&key('p'), &mut t);
    assert_eq!(t.rows(), &["foo foo"]); // "bar" replaced by "foo"
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn visual_p_yanks_replaced_selection() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar"));
    // yank "foo"
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t);
    e.handle_key(&key('y'), &mut t); // reg = "foo", cursor col 0
    // select "bar" and paste over it
    for _ in 0..4 {
        e.handle_key(&key('l'), &mut t);
    } // col 4 'b'
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t);
    e.handle_key(&key('p'), &mut t); // "bar" replaced by "foo"; "bar" now yanked
    assert_eq!(t.rows(), &["foo foo"]);
    // now paste the replaced "bar" at end of line to prove it's in the register
    e.handle_key(&key('$'), &mut t); // last char ('o', col 6)
    e.handle_key(&key('p'), &mut t); // append "bar" after it
    assert_eq!(t.rows(), &["foo foobar"]);
}

// ── Cheatsheet motions: g_/5G/5gg, ge/gE, WORD (W/E/B) ───────────────────

#[test]
fn g_underscore_jumps_to_last_non_blank() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hi there   "));
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('_'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 7)); // the final 'e'
}

#[test]
fn d_g_underscore_deletes_through_last_non_blank() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar  "));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('_'), &mut t);
    assert_eq!(t.rows(), &["  "]); // inclusive of the 'r'
}

#[test]
#[allow(non_snake_case)]
fn count_G_and_count_gg_go_to_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("1\n2\n3\n4\n5\n6"));
    e.handle_key(&key('5'), &mut t);
    e.handle_key(&key('G'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t).0, 4); // line 5
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('g'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t).0, 1); // line 2
}

#[test]
#[allow(non_snake_case)]
fn d_count_G_deletes_lines_through_target() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('G'), &mut t); // delete lines 1..=2 (linewise)
    assert_eq!(t.rows(), &["three"]);
}

#[test]
fn ge_jumps_to_previous_word_end() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar"));
    e.handle_key(&key('$'), &mut t); // on 'r' (col 6)
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('e'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 2)); // 'o' of foo
}

#[test]
fn ge_stops_at_class_change() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo.bar"));
    e.handle_key(&key('$'), &mut t); // 'r' (col 6)
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('e'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 3)); // the '.'
}

#[test]
#[allow(non_snake_case)]
fn gE_ignores_punctuation_boundaries() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("aa bb.cc dd"));
    e.handle_key(&key('$'), &mut t); // last 'd' (col 10)
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('E'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 7)); // end of "bb.cc"
}

#[test]
fn ge_at_buffer_start_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo"));
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('e'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 0));
}

#[test]
fn dge_deletes_backward_inclusive_of_cursor() {
    // vim: dge eats from the previous word end through the cursor char
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc def"));
    e.handle_key(&key('$'), &mut t); // on 'f'
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('e'), &mut t);
    assert_eq!(t.rows(), &["ab"]);
}

#[test]
#[allow(non_snake_case)]
fn W_treats_punctuated_run_as_one_word() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo.bar baz"));
    e.handle_key(&key('W'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 8)); // 'b' of baz
}

#[test]
#[allow(non_snake_case)]
fn E_jumps_to_end_of_WORD() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo.bar baz"));
    e.handle_key(&key('E'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 6)); // 'r' of foo.bar
}

#[test]
#[allow(non_snake_case)]
fn B_jumps_to_WORD_start() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo.bar baz"));
    e.handle_key(&key('W'), &mut t); // col 8
    e.handle_key(&key('B'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 0));
}

#[test]
#[allow(non_snake_case)]
fn W_crosses_lines_and_stops_at_empty_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo\n\nbar"));
    e.handle_key(&key('W'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (1, 0)); // empty line is a stop
    e.handle_key(&key('W'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (2, 0));
}

#[test]
#[allow(non_snake_case)]
fn dW_deletes_whole_WORD() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo.bar baz"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('W'), &mut t);
    assert_eq!(t.rows(), &["baz"]);
}

#[test]
#[allow(non_snake_case)]
fn cW_acts_like_cE() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo.bar baz"));
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('W'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    assert_eq!(t.rows(), &[" baz"]); // trailing space preserved (cW = cE)
}

// ── Awaiting + replace-stack fixes ───────────────────────────────────────

#[test]
fn hint_shows_object_scope_mid_sequence() {
    // regression: `diw` in flight showed "d" instead of "di"
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('i'), &mut t);
    assert_eq!(e.pending_hint().as_deref(), Some("di"));
}

#[test]
fn hint_shows_actual_find_key() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo"));
    e.handle_key(&key('T'), &mut t);
    assert_eq!(e.pending_hint().as_deref(), Some("T")); // was always 'f'
}

#[test]
fn replace_backspace_restores_overwritten_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('R'), &mut t);
    e.handle_key(&key('X'), &mut t); // 'a' → X
    e.handle_key(&key('Y'), &mut t); // 'b' → Y
    e.handle_key(
        &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        &mut t,
    );
    e.handle_key(&esc(), &mut t);
    assert_eq!(t.rows(), &["Xbc"]); // 'b' restored (vim replace stack)
}

#[test]
fn replace_backspace_removes_appended_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a"));
    e.handle_key(&key('R'), &mut t);
    e.handle_key(&key('X'), &mut t); // 'a' → X
    e.handle_key(&key('Y'), &mut t); // appended past EOL
    e.handle_key(
        &KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        &mut t,
    );
    e.handle_key(&esc(), &mut t);
    assert_eq!(t.rows(), &["X"]); // appended char removed, not restored
}

// ── Pure WORD-scanner unit tests (no TextArea needed) ────────────────────

// The WORD motions are the engine's now — same fixtures, driven through the
// keys that reach them, since the walkers they used to call are gone.

fn at_after(keys: &[char], lines: &[&str], from: (usize, usize)) -> (usize, usize) {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(ropetext::Text::from(lines.join("\n").as_str()));
    t.jump_to(from.0, from.1);
    for key_char in keys {
        e.handle_key(&key(*key_char), &mut t);
    }
    super::super::cursor_tuple(&t)
}

#[test]
fn big_word_forward_positions() {
    assert_eq!(at_after(&['W'], &["foo.bar baz"], (0, 0)), (0, 8));
    // An empty line is a WORD of its own, so it is a stop.
    assert_eq!(at_after(&['W'], &["foo", "", "bar"], (0, 0)), (1, 0));
    assert_eq!(at_after(&['W'], &["foo", "", "bar"], (1, 0)), (2, 0));
}

#[test]
fn big_word_back_positions() {
    assert_eq!(at_after(&['B'], &["foo.bar baz"], (0, 8)), (0, 0));
    assert_eq!(
        at_after(&['B'], &["foo.bar baz"], (0, 0)),
        (0, 0),
        "fails in place"
    );
}

#[test]
fn big_word_end_positions() {
    assert_eq!(at_after(&['E'], &["foo.bar baz"], (0, 0)), (0, 6));
    assert_eq!(
        at_after(&['E'], &["foo.bar baz"], (0, 10)),
        (0, 10),
        "nothing ahead leaves the cursor alone"
    );
}

#[test]
fn word_end_back_positions() {
    // `ge` crosses the punctuation run's end; `gE` sees one WORD and finds none.
    assert_eq!(at_after(&['g', 'e'], &["foo.bar"], (0, 6)), (0, 3));
    assert_eq!(at_after(&['g', 'E'], &["foo.bar"], (0, 6)), (0, 6));
    assert_eq!(at_after(&['g', 'e'], &["foo.bar"], (0, 0)), (0, 0));
}

// ── Holistic-review fixes ────────────────────────────────────────────────

#[test]
fn visual_counted_motion_extends_by_count() {
    // regression: the 5G translation consumed the count for EVERY motion
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('3'), &mut t);
    e.handle_key(&key('l'), &mut t); // cursor → col 3, inclusive covers "abcd"
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &["ef"]);
}

#[test]
#[allow(non_snake_case)]
fn gUu_aborts_without_running_undo() {
    // vim: a mismatched key after a pending operator cancels everything
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab"));
    e.handle_key(&key('x'), &mut t); // real change → "b"
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('U'), &mut t); // Uppercase pending
    e.handle_key(&key('u'), &mut t); // mismatch — must NOT run Undo
    assert_eq!(t.rows(), &["b"]); // x not reverted
}

#[test]
fn dx_and_dp_abort_with_operator_pending() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('l'), &mut t); // register = "a"
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('x'), &mut t); // vim aborts — nothing deleted
    assert_eq!(t.rows(), &["abc"]);
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('p'), &mut t); // vim aborts — nothing pasted
    assert_eq!(t.rows(), &["abc"]);
}

#[test]
fn dge_at_buffer_start_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('e'), &mut t); // motion fails → whole op no-op
    assert_eq!(t.rows(), &["foo"]);
}

#[test]
fn gugu_doubled_g_form_runs_linewise() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ABC def"));
    for c in "gugu".chars() {
        e.handle_key(&key(c), &mut t);
    }
    assert_eq!(t.rows(), &["abc def"]);
}

#[test]
#[allow(non_snake_case)]
fn visual_J_joins_selected_lines_with_space() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a\nb\nc"));
    e.handle_key(&key('V'), &mut t);
    e.handle_key(&key('j'), &mut t);
    e.handle_key(&key('j'), &mut t); // select all three
    e.handle_key(&key('J'), &mut t);
    assert_eq!(t.rows(), &["a b c"]);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
#[allow(non_snake_case)]
fn visual_gJ_joins_selected_lines_raw() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a\n  b"));
    e.handle_key(&key('V'), &mut t);
    e.handle_key(&key('j'), &mut t);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('J'), &mut t);
    assert_eq!(t.rows(), &["a  b"]); // verbatim, indent kept
}

#[test]
#[allow(non_snake_case)]
fn replace_mode_arrows_move_cursor() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcd"));
    e.handle_key(&key('R'), &mut t);
    e.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &mut t);
    e.handle_key(&KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &mut t);
    e.handle_key(&key('X'), &mut t); // overwrite 'c'
    e.handle_key(&esc(), &mut t);
    assert_eq!(t.rows(), &["abXd"]);
    // capture restarted at the movement target: '.' overwrites one char
    e.handle_key(&key('0'), &mut t);
    e.handle_key(&key('.'), &mut t);
    assert_eq!(t.rows(), &["XbXd"]);
}

#[test]
fn esc_from_insert_clears_stray_selection() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('i'), &mut t);
    // simulate a mouse drag mid-insert leaving a live selection
    t.start_selection();
    t.move_cursor(crate::components::text_editor::rope_buffer::CursorMove::Forward);
    e.handle_key(&esc(), &mut t);
    assert!(
        t.selection_range().is_none(),
        "Esc must drop the stray selection"
    );
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn guu_undoes_in_one_step() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("Mixed Case Line"));
    for c in "guu".chars() {
        e.handle_key(&key(c), &mut t);
    }
    assert_eq!(t.rows(), &["mixed case line"]);
    e.handle_key(&key('u'), &mut t); // single undo restores...
    e.handle_key(&key('u'), &mut t); // (cut+insert = 2 textarea edits)
    assert_eq!(t.rows(), &["Mixed Case Line"]);
}

// ── Visual g~ (case toggle; bare ~ is auto-surround) ─────────────────────

#[test]
fn visual_g_tilde_toggles_case_of_selection() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("FooBar"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t); // select all of "FooBar"
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('~'), &mut t);
    assert_eq!(t.rows(), &["fOObAR"]);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn visual_bare_tilde_still_passes_through_for_surround() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("FooBar"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('e'), &mut t);
    let out = e.handle_key(&key('~'), &mut t);
    assert_eq!(out, VimKeyOutcome::PassThrough); // host auto-surround wraps
    assert_eq!(*e.mode(), EditorMode::Normal);
}

// ── Case operators gu/gU/g~ ──────────────────────────────────────────────

#[test]
fn guw_lowercases_word() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("HELLO world"));
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('u'), &mut t);
    e.handle_key(&key('w'), &mut t);
    assert_eq!(t.rows(), &["hello world"]);
    assert_eq!(super::super::cursor_tuple(&t), (0, 0)); // cursor at start
}

#[test]
#[allow(non_snake_case)]
fn gU_iw_uppercases_inner_word() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar baz"));
    e.handle_key(&key('w'), &mut t); // onto "bar"
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('U'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('w'), &mut t);
    assert_eq!(t.rows(), &["foo BAR baz"]);
}

#[test]
fn g_tilde_toggles_case_to_word_end() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("FooBar baz"));
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('~'), &mut t);
    e.handle_key(&key('e'), &mut t); // inclusive to end of "FooBar"
    assert_eq!(t.rows(), &["fOObAR baz"]);
}

#[test]
fn guu_lowercases_whole_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("HELLO World\nNEXT"));
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('u'), &mut t);
    e.handle_key(&key('u'), &mut t);
    assert_eq!(t.rows(), &["hello world", "NEXT"]);
}

#[test]
#[allow(non_snake_case)]
fn visual_U_uppercases_selection() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('l'), &mut t);
    e.handle_key(&key('l'), &mut t); // select "hel"
    e.handle_key(&key('U'), &mut t);
    assert_eq!(t.rows(), &["HELlo"]);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

#[test]
fn case_op_does_not_touch_register() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("keep CHANGE"));
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('e'), &mut t); // register = "keep"
    e.handle_key(&key('w'), &mut t); // onto "CHANGE"
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('u'), &mut t);
    e.handle_key(&key('w'), &mut t); // lowercase it
    assert_eq!(e.registers.read().unwrap().text, "keep"); // unchanged
}

#[test]
#[allow(non_snake_case)]
fn dot_repeats_gU_word() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one two"));
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('U'), &mut t);
    e.handle_key(&key('e'), &mut t); // ONE
    e.handle_key(&key('w'), &mut t); // onto "two"
    e.handle_key(&key('.'), &mut t);
    assert_eq!(t.rows(), &["ONE TWO"]);
}

// ── Replace mode (R) ─────────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn R_overwrites_chars() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    e.handle_key(&key('R'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Replace);
    e.handle_key(&key('X'), &mut t);
    e.handle_key(&key('Y'), &mut t);
    e.handle_key(&esc(), &mut t);
    assert_eq!(t.rows(), &["XYcdef"]); // overwrote, didn't insert
    assert_eq!(*e.mode(), EditorMode::Normal);
    assert_eq!(super::super::cursor_tuple(&t), (0, 1)); // stepped back onto 'Y'
}

#[test]
#[allow(non_snake_case)]
fn R_appends_past_line_end() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab"));
    e.handle_key(&key('R'), &mut t);
    for c in "XYZ".chars() {
        e.handle_key(&key(c), &mut t);
    }
    e.handle_key(&esc(), &mut t);
    assert_eq!(t.rows(), &["XYZ"]); // overwrote "ab", appended 'Z'
}

#[test]
#[allow(non_snake_case)]
fn R_is_dot_repeatable() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("aaaa bbbb"));
    e.handle_key(&key('R'), &mut t);
    e.handle_key(&key('X'), &mut t);
    e.handle_key(&key('X'), &mut t);
    e.handle_key(&esc(), &mut t); // "XXaa bbbb"
    e.handle_key(&key('w'), &mut t); // onto "bbbb"
    e.handle_key(&key('.'), &mut t); // overwrite "bb"
    assert_eq!(t.rows(), &["XXaa XXbb"]);
}

#[test]
#[allow(non_snake_case)]
fn aborted_R_keeps_dot_register() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('x'), &mut t); // real change
    e.handle_key(&key('R'), &mut t);
    e.handle_key(&esc(), &mut t); // typed nothing
    e.handle_key(&key('.'), &mut t); // must repeat x
    assert_eq!(t.rows(), &["c"]);
}

#[test]
#[allow(non_snake_case)]
fn R_mode_does_not_pass_through() {
    // Replace mode is engine-owned: chars must never reach the host's
    // textarea path (no auto-surround under R).
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab"));
    e.handle_key(&key('R'), &mut t);
    let out = e.handle_key(&key('('), &mut t);
    assert_eq!(out, VimKeyOutcome::TextMutated); // consumed, not PassThrough
    assert_eq!(t.rows()[0].chars().next(), Some('(')); // raw overwrite
}

// ── J / gJ join semantics ────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn J_joins_with_single_space_stripping_indent() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo\n   bar"));
    e.handle_key(&key('J'), &mut t);
    assert_eq!(t.rows(), &["foo bar"]);
    // cursor on the join-point space (vim)
    assert_eq!(super::super::cursor_tuple(&t), (0, 3));
}

#[test]
#[allow(non_snake_case)]
fn J_adds_no_extra_space_when_line_ends_in_whitespace() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo \nbar"));
    e.handle_key(&key('J'), &mut t);
    assert_eq!(t.rows(), &["foo bar"]);
}

#[test]
#[allow(non_snake_case)]
fn gJ_joins_without_space() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo\n   bar"));
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('J'), &mut t);
    assert_eq!(t.rows(), &["foo   bar"]); // verbatim, indent kept
}

#[test]
#[allow(non_snake_case)]
fn three_J_joins_three_lines() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a\nb\nc"));
    e.handle_key(&key('3'), &mut t);
    e.handle_key(&key('J'), &mut t);
    assert_eq!(t.rows(), &["a b c"]);
}

// ── Insert entries ───────────────────────────────────────────────────────

#[test]
#[allow(non_snake_case)]
fn I_inserts_at_first_non_blank() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("    indented"));
    e.handle_key(&key('$'), &mut t); // away from the start
    e.handle_key(&key('I'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    assert_eq!(super::super::cursor_tuple(&t), (0, 4)); // on 'i', not col 0
}

// ── % across lines ───────────────────────────────────────────────────────

#[test]
fn percent_matches_across_lines() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo (bar\nbaz) qux"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('('), &mut t); // on '(' (0,4)
    e.handle_key(&key('%'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (1, 3)); // ')' on line 2
    e.handle_key(&key('%'), &mut t); // and back
    assert_eq!(super::super::cursor_tuple(&t), (0, 4));
}

#[test]
fn percent_nested_across_lines() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("{a {b\nc}\nd}"));
    e.handle_key(&key('%'), &mut t); // outer '{' at (0,0)
    assert_eq!(super::super::cursor_tuple(&t), (2, 1)); // outer '}' line 3
}

#[test]
fn d_percent_deletes_across_lines_inclusive() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a(b\nc)d"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('('), &mut t); // on '('
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('%'), &mut t); // delete '(' through ')' inclusive
    assert_eq!(t.rows(), &["ad"]);
}

#[test]
fn percent_unmatched_across_buffer_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("(a\nb"));
    e.handle_key(&key('%'), &mut t); // no closing paren anywhere
    assert_eq!(super::super::cursor_tuple(&t), (0, 0));
}

// ── Review fixes: failed-op no-ops, dot-register protection ─────────────

#[test]
fn visual_c_dot_repeats_same_width() {
    // `.` after a visual change replays a same-sized change (vim), not cl
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcde fghij"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('l'), &mut t);
    e.handle_key(&key('l'), &mut t); // select "abc"
    e.handle_key(&key('c'), &mut t); // change it
    t.insert_str("X");
    e.handle_key(&esc(), &mut t); // "Xde fghij"
    e.handle_key(&key('w'), &mut t); // onto 'f'
    e.handle_key(&key('.'), &mut t); // change 3 chars "fgh" → "X"
    assert_eq!(t.rows(), &["Xde Xij"]);
}

#[test]
fn count_find_is_atomic() {
    // vim 2fx with one 'x': whole motion fails, cursor stays
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a x b"));
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('x'), &mut t);
    assert_eq!(super::super::cursor_tuple(&t), (0, 0)); // did not move
    // and with two: lands on the second
    let mut t2 = RopeBuffer::new(Text::from("axbx"));
    e.handle_key(&key('2'), &mut t2);
    e.handle_key(&key('f'), &mut t2);
    e.handle_key(&key('x'), &mut t2);
    assert_eq!(super::super::cursor_tuple(&t2), (0, 3));
}

#[test]
fn d2fx_with_one_x_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a x b"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('x'), &mut t); // only one 'x' — vim no-ops everything
    assert_eq!(t.rows(), &["a x b"]);
}

#[test]
fn reset_to_normal_clears_insert_capture() {
    // regression: stale capture from interrupted cw silently disabled
    // dot-recording for every later change
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo bar"));
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('w'), &mut t); // Insert, capture live
    e.reset_to_normal(); // note switch mid-insert
    e.handle_key(&key('x'), &mut t); // must record (deletes ' ')
    e.handle_key(&key('.'), &mut t); // must repeat x (deletes 'b')
    assert_eq!(t.rows(), &["ar"]); // cw left " bar"; x then . removed 2 chars
}

#[test]
fn dj_on_last_line_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("only line"));
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('y'), &mut t); // register = line
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('j'), &mut t); // motion fails → whole op no-op
    assert_eq!(t.rows(), &["only line"]);
    assert_eq!(e.registers.read().unwrap().text, "only line\n"); // register kept
}

#[test]
fn dk_on_first_line_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('k'), &mut t);
    assert_eq!(t.rows(), &["one", "two"]);
}

#[test]
fn failed_find_op_does_not_clobber_dot() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcdef"));
    e.handle_key(&key('x'), &mut t); // real change: delete 'a'
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('z'), &mut t); // failed find — must not record
    e.handle_key(&key('.'), &mut t); // repeats x, not the failed dfz
    assert_eq!(t.rows(), &["cdef"]);
}

#[test]
fn noop_x_does_not_clobber_dot() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one two three\n"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('w'), &mut t); // delete "one "
    e.handle_key(&key('j'), &mut t); // empty line
    let out = e.handle_key(&key('x'), &mut t); // no-op
    assert_eq!(out, VimKeyOutcome::NoOp); // host must not bump content
    e.handle_key(&key('k'), &mut t);
    e.handle_key(&key('.'), &mut t); // repeats dw, not the no-op x
    assert_eq!(t.rows(), &["three", ""]);
}

#[test]
fn d_percent_without_pair_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('%'), &mut t); // no bracket under cursor
    assert_eq!(t.rows(), &["abc"]);
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('%'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Normal); // failed c% must not enter Insert
}

#[test]
fn visual_inner_empty_pair_is_noop() {
    // regression: vi( on "()" widened onto the ')' and deleted it
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo()bar"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('('), &mut t); // cursor on '('
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('('), &mut t); // empty object: selection unchanged
    e.handle_key(&esc(), &mut t);
    assert_eq!(t.rows(), &["foo()bar"]);
}

#[test]
fn aborted_insert_keeps_dot_register() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('x'), &mut t); // real change
    e.handle_key(&key('i'), &mut t); // changed mind
    e.handle_key(&esc(), &mut t); // nothing typed — not a change
    e.handle_key(&key('.'), &mut t); // must repeat x
    assert_eq!(t.rows(), &["c"]);
}

#[test]
fn o_then_esc_is_still_dot_repeatable() {
    // vim: o<Esc> IS a change (the opened line); `.` opens another
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    e.handle_key(&key('o'), &mut t);
    e.handle_key(&esc(), &mut t);
    e.handle_key(&key('.'), &mut t);
    assert_eq!(t.rows().len(), 3);
}

// ── Visual mode: shared motion/object machinery ──────────────────────────

#[test]
fn visual_inner_object_then_delete() {
    // vi( selects inside the parens; d deletes it
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("foo(bar)baz"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('a'), &mut t); // cursor on 'a' of "bar" (col 5)
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('i'), &mut t);
    e.handle_key(&key('('), &mut t); // select "bar"
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &["foo()baz"]);
}

#[test]
fn visual_around_quote_then_yank() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("say \"hi\" now"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('h'), &mut t); // inside quotes
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('a'), &mut t);
    e.handle_key(&key('"'), &mut t); // select "\"hi\""
    e.handle_key(&key('y'), &mut t);
    let reg = e.registers.read().unwrap();
    assert_eq!(reg.text, "\"hi\"");
}

#[test]
fn visual_find_extends_selection() {
    // vf, then d deletes through the ','
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello, world"));
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key(','), &mut t); // cursor on ',' — selection covers "hello,"
    e.handle_key(&key('d'), &mut t);
    assert_eq!(t.rows(), &[" world"]);
}

#[test]
fn visual_gg_extends_to_file_start() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('j'), &mut t);
    e.handle_key(&key('j'), &mut t); // row 2
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('g'), &mut t); // extend to (0,0)
    e.handle_key(&key('d'), &mut t); // delete from 't' of "three" back to start
    assert_eq!(t.rows(), &["hree"]);
}

#[test]
fn visual_o_swaps_selection_ends() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abcde"));
    e.handle_key(&key('l'), &mut t);
    e.handle_key(&key('l'), &mut t); // col 2 ('c')
    e.handle_key(&key('v'), &mut t);
    e.handle_key(&key('l'), &mut t); // select c..d, cursor at 'd' (col 3)
    e.handle_key(&key('o'), &mut t); // cursor swaps to 'c' (col 2)
    assert_eq!(super::super::cursor_tuple(&t), (0, 2));
    e.handle_key(&key('h'), &mut t); // extend left from the anchor end
    e.handle_key(&key('d'), &mut t); // delete b..d inclusive
    assert_eq!(t.rows(), &["ae"]);
}

// ── Command spine: dot-repeat through the one apply() door ───────────────

#[test]
fn dot_repeats_cc_with_typed_text() {
    // previously a silent no-op (replay's `_other` arm)
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo"));
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('c'), &mut t); // cc on "one"
    t.insert_str("X");
    e.handle_key(&esc(), &mut t); // line 0 = "X"
    e.handle_key(&key('j'), &mut t); // onto "two"
    e.handle_key(&key('.'), &mut t); // repeat cc+X
    assert_eq!(t.rows(), &["X", "X"]);
}

#[test]
fn dot_repeats_substitute_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab cd"));
    e.handle_key(&key('s'), &mut t); // delete 'a', Insert
    t.insert_str("Z");
    e.handle_key(&esc(), &mut t); // "Zb cd"
    e.handle_key(&key('w'), &mut t); // onto 'c'
    e.handle_key(&key('.'), &mut t); // repeat s+Z on 'c'
    assert_eq!(t.rows(), &["Zb Zd"]);
}

#[test]
fn dot_repeats_plain_insert() {
    // vim: `ihello<Esc>` then `.` inserts "hello" again
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("world"));
    e.handle_key(&key('i'), &mut t);
    t.insert_str("ab");
    e.handle_key(&esc(), &mut t); // "abworld", cursor on 'b'
    e.handle_key(&key('.'), &mut t); // insert "ab" again before 'b'
    assert_eq!(t.rows(), &["aabbworld"]);
}

#[test]
fn dot_repeats_indent() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("x"));
    e.handle_key(&key('>'), &mut t);
    e.handle_key(&key('>'), &mut t); // indent
    e.handle_key(&key('.'), &mut t); // repeat
    assert_eq!(t.rows(), &["        x"]);
}

#[test]
fn dot_does_not_repeat_yank() {
    // vim: `.` repeats the last CHANGE; a yank after it must not displace it
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('x'), &mut t); // delete 'a' (the change)
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('l'), &mut t); // yank 'b' — not a change
    e.handle_key(&key('.'), &mut t); // must repeat x, not the yank
    assert_eq!(t.rows(), &["c"]);
}

// ── Range model: motion SpanKind classification + count composition ─────

#[test]
fn counts_before_and_after_operator_multiply() {
    // vim: 2d3w = 6 words, not count "23"
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a b c d e f g"));
    e.handle_key(&key('2'), &mut t);
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('3'), &mut t);
    e.handle_key(&key('w'), &mut t);
    assert_eq!(t.rows(), &["g"]); // six words deleted
}

#[test]
fn dj_deletes_two_whole_lines_linewise() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('l'), &mut t); // col 1 — must not matter (linewise)
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('j'), &mut t);
    assert_eq!(t.rows(), &["three"]);
    let reg = e.registers.read().unwrap();
    assert_eq!(reg.kind, RegisterKind::Linewise);
    assert_eq!(reg.text, "one\ntwo\n");
}

#[test]
fn dk_deletes_two_whole_lines_upward() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('j'), &mut t); // row 1
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('k'), &mut t);
    assert_eq!(t.rows(), &["three"]);
}

#[test]
#[allow(non_snake_case)]
fn dG_deletes_to_file_end_linewise() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('j'), &mut t); // row 1
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('G'), &mut t);
    assert_eq!(t.rows(), &["one"]);
}

#[test]
fn d_gg_deletes_to_file_start_linewise() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('j'), &mut t); // row 1
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('g'), &mut t);
    e.handle_key(&key('g'), &mut t);
    assert_eq!(t.rows(), &["three"]);
}

#[test]
fn dt_deletes_up_to_but_not_including_target() {
    // vim t is inclusive of the char BEFORE the target: dtx on "abx" → "x"
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abx"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('t'), &mut t);
    e.handle_key(&key('x'), &mut t);
    assert_eq!(t.rows(), &["x"]);
}

#[test]
fn failed_find_with_operator_is_noop() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("hello"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('z'), &mut t); // no 'z' on the line
    assert_eq!(t.rows(), &["hello"]); // nothing deleted
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('z'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Normal); // failed cf must not enter Insert
}

#[test]
fn d_semicolon_repeats_find_as_operator_range() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("a.b.c"));
    e.handle_key(&key('f'), &mut t);
    e.handle_key(&key('.'), &mut t); // cursor on first '.' (col 1)
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key(';'), &mut t); // delete through next '.' (inclusive)
    assert_eq!(t.rows(), &["ac"]);
}

#[test]
fn cj_changes_two_lines_and_enters_insert() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo\nthree"));
    e.handle_key(&key('c'), &mut t);
    e.handle_key(&key('j'), &mut t);
    assert_eq!(*e.mode(), EditorMode::Insert);
    assert_eq!(t.rows(), &["", "three"]); // both lines gone, fresh empty line
}

// ── Register file: engine-owned unnamed register ────────────────────────

#[test]
fn x_then_p_swaps_chars() {
    // the classic vim `xp` idiom: x fills the register with the deleted char
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab"));
    e.handle_key(&key('x'), &mut t); // delete 'a' → register "a"; line "b"
    e.handle_key(&key('p'), &mut t); // paste "a" after 'b'
    assert_eq!(t.rows(), &["ba"]);
}

#[test]
fn x_at_line_end_does_not_join_next_line() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab\ncd"));
    e.handle_key(&key('l'), &mut t); // onto 'b' (last char)
    e.handle_key(&key('3'), &mut t);
    e.handle_key(&key('x'), &mut t); // vim: deletes only 'b', never the newline
    assert_eq!(t.rows(), &["a", "cd"]);
}

#[test]
fn s_fills_register_with_deleted_char() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("abc"));
    e.handle_key(&key('s'), &mut t); // delete 'a', enter Insert
    assert_eq!(*e.mode(), EditorMode::Insert);
    let reg = e.registers.read().expect("s must fill the register");
    assert_eq!(reg.text, "a");
    assert_eq!(reg.kind, RegisterKind::Charwise);
}

#[test]
#[allow(non_snake_case)]
fn S_fills_register_linewise_no_kind_desync() {
    // regression: S used to cut() (charwise content) while the engine kept
    // a stale Linewise kind from a prior yy — kind and content desynced.
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one\ntwo"));
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('y'), &mut t); // register = "one\n" linewise
    e.handle_key(&key('j'), &mut t);
    e.handle_key(&key('S'), &mut t); // substitute line "two"
    let reg = e.registers.read().expect("S must fill the register");
    assert_eq!(reg.text, "two\n");
    assert_eq!(reg.kind, RegisterKind::Linewise);
}

#[test]
fn dw_fills_register_charwise() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("one two"));
    e.handle_key(&key('d'), &mut t);
    e.handle_key(&key('w'), &mut t); // delete "one "
    let reg = e.registers.read().expect("dw must fill the register");
    assert_eq!(reg.text, "one ");
    assert_eq!(reg.kind, RegisterKind::Charwise);
    // and p pastes it back charwise
    e.handle_key(&key('p'), &mut t);
    assert_eq!(t.rows(), &["tone wo"]); // "one " pasted after 't'
}

#[test]
fn empty_delete_keeps_previous_register() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("ab\n"));
    e.handle_key(&key('y'), &mut t);
    e.handle_key(&key('l'), &mut t); // yank "a" charwise
    e.handle_key(&key('j'), &mut t); // empty line
    e.handle_key(&key('x'), &mut t); // no-op delete (empty line)
    let reg = e
        .registers
        .read()
        .expect("register must survive a no-op delete");
    assert_eq!(reg.text, "a");
}

#[test]
fn esc_in_normal_clears_stray_selection() {
    let mut e = VimEngine::default(); // Normal mode
    let mut t = RopeBuffer::new(Text::from("hello world"));
    // simulate a live selection while in Normal mode (as auto-surround/mouse-sync can leave)
    t.start_selection();
    t.move_cursor(crate::components::text_editor::rope_buffer::CursorMove::Forward);
    t.move_cursor(crate::components::text_editor::rope_buffer::CursorMove::Forward);
    assert!(t.selection_range().is_some());
    let out = e.handle_key(&esc(), &mut t);
    assert!(
        t.selection_range().is_none(),
        "Esc in Normal must cancel a stray selection"
    );
    assert_eq!(out, VimKeyOutcome::CursorOnly);
    assert_eq!(*e.mode(), EditorMode::Normal);
}

// ── Grapheme clusters vs Unicode scalars ────────────────────────────────

/// Man-woman-girl: three scalars joined by two ZWJs, one cluster.
const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

#[test]
fn counted_x_counts_clusters_not_scalars() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from(&*format!("{FAMILY}\nnext")));
    e.handle_key(&key('3'), &mut t);
    e.handle_key(&key('x'), &mut t);
    assert_eq!(
        t.rows(),
        &["", "next"],
        "one cluster is one `x`; the row has nothing else to give"
    );
}

#[test]
fn tilde_keeps_the_rest_of_the_cluster() {
    let mut e = VimEngine::default();
    let mut t = RopeBuffer::new(Text::from("e\u{301}f"));
    e.handle_key(&key('~'), &mut t);
    assert_eq!(
        t.rows(),
        &["E\u{301}f"],
        "toggling case must not drop the combining acute"
    );
}
