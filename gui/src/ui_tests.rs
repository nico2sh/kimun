//! UI integration tests for the Edit surface: a real `EditorView` in a
//! headless window, driven by GPUI keyboard, pointer and input-handler events.

use gpui_kit::test::{TestEventExt, TestInput, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementInputHandler, Entity, Focusable, InputHandler,
    MouseButton, Pixels, Point, ScrollDelta, TestAppContext, WindowBounds, WindowOptions, point,
    px, size,
};
use kimun_notes::logic::editor::Editor;
use kimun_notes::settings::themes::Theme;

use crate::editor_view::{self, CaretShape, EditorView, ModeLine, Painted};
use crate::keymap::{self, Keymap, Platform};

fn open(cx: &mut TestAppContext, text: &str) -> (AnyWindowHandle, Entity<EditorView>) {
    open_with(cx, text, false)
}

/// A vim editor (starts in Normal).
fn open_vim(cx: &mut TestAppContext, text: &str) -> (AnyWindowHandle, Entity<EditorView>) {
    open_with(cx, text, true)
}

fn open_with(
    cx: &mut TestAppContext,
    text: &str,
    vim: bool,
) -> (AnyWindowHandle, Entity<EditorView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        // A fixed platform, so `ctrl-f` means the same on every CI runner.
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let text = text.to_string();
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(640.), px(480.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| EditorView::new(Some(text), Theme::default(), vim, cx));
                window.focus(&view.read(cx).focus_handle(cx), cx);
                view
            },
        )
        .expect("open test window")
    })
}

fn text(view: &Entity<EditorView>, cx: &mut gpui_kit::App) -> String {
    view.read(cx).editor.get_text()
}

/// Window-local centre of cell (`col`, `row`) in the last painted frame.
fn cell_center(view: &Entity<EditorView>, col: u16, row: u16, cx: &gpui_kit::App) -> Point<Pixels> {
    let painted = view.read(cx).painted.clone().expect("a frame was painted");
    let c = painted.cells;
    point(
        painted.origin.x + px(c.x_of(col) + c.width() / 2.0),
        painted.origin.y + px(c.y_of(row) + c.height() / 2.0),
    )
}

#[gpui_kit::test]
fn typing_inserts_text_including_shifted_and_non_ascii(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("editor").focused(), Some(true));
        window.input("hé$ 理", cx);
        assert_eq!(text(&view, cx), "hé$ 理");
    })
    .unwrap();
}

#[gpui_kit::test]
fn backspace_and_enter_edit_the_buffer(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "");
    cx.update_window(handle, |_, window, cx| {
        window.input("abc", cx);
        window.press("backspace", cx);
        assert_eq!(text(&view, cx), "ab");
        window.press("enter", cx);
        window.input("d", cx);
        assert_eq!(text(&view, cx), "ab\nd");
    })
    .unwrap();
}

#[gpui_kit::test]
fn arrows_move_the_cursor(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "abc\ndef");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 0));
        window.press("right", cx);
        window.press("right", cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 2));
        window.press("down", cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 2));
        window.press("home", cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 0));
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_indents_and_shift_tab_dedents_instead_of_moving_focus(cx: &mut TestAppContext) {
    // The kit's Root binds tab / shift-tab to focus traversal; the editor
    // disables them.
    let (handle, view) = open(cx, "item");
    cx.update_window(handle, |_, window, cx| {
        window.press("tab", cx);
        assert_eq!(text(&view, cx), "    item");
        assert_eq!(window.find("editor").focused(), Some(true));
        window.press("shift-tab", cx);
        assert_eq!(text(&view, cx), "item");
        assert_eq!(window.find("editor").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn kit_bindings_on_editor_keys_are_disabled_in_the_editor_only(cx: &mut TestAppContext) {
    use gpui_kit::Keystroke;
    // Pressing Ctrl-C for real would touch the OS clipboard, so the keymap is
    // asked directly: with the editor focused no binding wins for these keys,
    // while one level up (the kit's Root) they are bound.
    let (handle, _view) = open(cx, "x");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let stack = window.context_stack();
        assert!(
            stack
                .last()
                .is_some_and(|c| c.contains(editor_view::KEY_CONTEXT))
        );
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let bound = |key: &'static str, stack: &[gpui_kit::KeyContext]| {
            let ks = Keystroke::parse(key).unwrap();
            !keymap.bindings_for_input(&[ks], stack).0.is_empty()
        };
        let os_specific = if cfg!(target_os = "macos") {
            ["cmd-c", "cmd-alt-i"]
        } else {
            ["ctrl-c", "ctrl-shift-i"]
        };
        for key in ["tab", "shift-tab"].into_iter().chain(os_specific) {
            assert!(bound(key, &stack[..stack.len() - 1]), "{key} bound outside");
            assert!(!bound(key, &stack), "{key} still bound in the editor");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn click_moves_the_cursor_to_the_cell(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "hello\nworld");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 3, 1, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 3));
        // A click past the end of a row lands at its end.
        let past = cell_center(&view, 40, 0, cx);
        window.pointer_down(past, MouseButton::Left, cx);
        window.pointer_up(past, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 5));
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_selects_and_typing_replaces_the_selection(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "hello world");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let from = cell_center(&view, 0, 0, cx);
        let to = cell_center(&view, 5, 0, cx);
        // The press lands mid-cell 0, which maps to column 0.
        window.pointer_down(from, MouseButton::Left, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
        window.pointer_up(to, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.selection(), Some(((0, 0), (0, 5))));
        window.input("bye", cx);
        assert_eq!(text(&view, cx), "bye world");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_drag_that_began_outside_the_editor_selects_nothing(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "hello world");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let to = cell_center(&view, 5, 0, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
        window.pointer_up(to, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.selection(), None);
        assert_eq!(view.read(cx).editor.cursor(), (0, 0));
    })
    .unwrap();
}

#[gpui_kit::test]
fn wheel_scrolls_the_view_not_the_cursor(cx: &mut TestAppContext) {
    let long = (0..200)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (handle, view) = open(cx, &long);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll("editor", ScrollDelta::Lines(point(0., -3.)), cx);
        window.render_frame(cx);
        let top = view
            .read(cx)
            .editor
            .session()
            .display
            .click_at_screen(0, 0)
            .0;
        assert_eq!(top, 3, "one 3-line wheel notch scrolls three rows");
        assert_eq!(view.read(cx).editor.cursor(), (0, 0));
        // Trackpad pixels below one row accumulate instead of being dropped.
        let row = view.read(cx).painted.as_ref().unwrap().cells.height();
        let nudge = ScrollDelta::Pixels(point(px(0.), px(-row * 0.6)));
        window.scroll("editor", nudge, cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx)
                .editor
                .session()
                .display
                .click_at_screen(0, 0)
                .0,
            3
        );
        window.scroll("editor", nudge, cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx)
                .editor
                .session()
                .display
                .click_at_screen(0, 0)
                .0,
            4
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn multi_codepoint_text_arrives_through_the_input_handler(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find("editor").bounds();
        let mut ime = TestInput::new(ElementInputHandler::new(bounds, view.clone()), window);
        // e + combining acute, then a ZWJ family: neither is one `char`.
        ime.commit_text("e\u{301}", cx);
        ime.commit_text("\u{1F468}\u{200D}\u{1F469}", cx);
        assert_eq!(text(&view, cx), "e\u{301}\u{1F468}\u{200D}\u{1F469}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_preedit_stays_out_of_the_buffer_until_committed(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "ab");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("end", cx);
        let bounds = window.find("editor").bounds();
        let mut ime = TestInput::new(ElementInputHandler::new(bounds, view.clone()), window);
        ime.compose_text(None, "に", None, cx);
        assert_eq!(text(&view, cx), "ab");
        assert!(!view.read(cx).editor.is_dirty());
        assert_eq!(view.read(cx).preedit.as_deref(), Some("に"));
        // The preedit is in the document (after "ab") though not the buffer,
        // and the caret sits after it.
        assert_eq!(ime.marked_text_range(cx), Some(2..3));
        assert_eq!(ime.selected_text_range(cx).map(|s| s.range), Some(3..3));
        ime.commit_text("日", cx);
        assert_eq!(text(&view, cx), "ab日");
        assert_eq!(view.read(cx).preedit, None);
        // Ending a composition keeps what was composed.
        ime.compose_text(None, "x", None, cx);
        ime.unmark_text(cx);
        assert_eq!(text(&view, cx), "ab日x");
    })
    .unwrap();
}

#[gpui_kit::test]
fn input_handler_range_replacement_targets_the_cursor_row(cx: &mut TestAppContext) {
    // macOS press-and-hold replaces the previous char by range.
    let (handle, view) = open(cx, "first\nxe");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("end", cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        handler.replace_text_in_range(Some(1..2), "é", window, cx);
        assert_eq!(text(&view, cx), "first\nxé");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_finished_background_job_redraws_without_input(cx: &mut TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;
    let (_handle, view) = open(cx, "x");
    let notified = Rc::new(Cell::new(0));
    let _observer = cx.update(|cx| {
        let notified = notified.clone();
        cx.observe(&view, move |_, _| notified.set(notified.get() + 1))
    });
    let redraw = cx
        .update(|cx| view.read(cx).editor.session().notifier.clone())
        .expect("the redraw notifier is installed");
    cx.run_until_parked();
    let before = notified.get();
    // What a background parse/layout task does on completion. In production
    // it calls from a tokio worker thread; GPUI's test scheduler rejects
    // foreign-thread wakes as non-deterministic, so the call is made here.
    redraw();
    cx.run_until_parked();
    assert!(notified.get() > before, "the view was notified");
}

#[gpui_kit::test]
fn a_composed_key_down_ends_the_preedit(cx: &mut TestAppContext) {
    // Dead key: the platform shows "´" as a preedit, then delivers the
    // finished "é" as an ordinary key-down rather than through the handler.
    let (handle, view) = open(cx, "");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find("editor").bounds();
        let mut ime = TestInput::new(ElementInputHandler::new(bounds, view.clone()), window);
        ime.compose_text(None, "´", None, cx);
        assert_eq!(view.read(cx).preedit.as_deref(), Some("´"));
        window.input("é", cx);
        assert_eq!(text(&view, cx), "é");
        assert_eq!(view.read(cx).preedit, None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn cancelling_a_composition_deletes_only_the_preedit(cx: &mut TestAppContext) {
    // Linux IME cancel / focus loss: delete the marked range.
    let (handle, view) = open(cx, "ab");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("end", cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        handler.replace_and_mark_text_in_range(None, "にほ", None, window, cx);
        let marked = handler.marked_text_range(window, cx).expect("composing");
        assert_eq!(marked, 2..4);
        handler.replace_text_in_range(Some(marked), "", window, cx);
        assert_eq!(text(&view, cx), "ab");
        assert_eq!(view.read(cx).preedit, None);
        assert_eq!(handler.marked_text_range(window, cx), None);
        assert!(!view.read(cx).editor.is_dirty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn input_handler_ranges_are_honoured(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "abcd");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("end", cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        // An empty range inserts at its start, not at the cursor.
        handler.replace_text_in_range(Some(1..1), "X", window, cx);
        assert_eq!(text(&view, cx), "aXbcd");
        // A marked-text range over buffer text (reconversion) takes it over.
        handler.replace_and_mark_text_in_range(Some(3..5), "シ", None, window, cx);
        assert_eq!(text(&view, cx), "aXb");
        assert_eq!(view.read(cx).preedit.as_deref(), Some("シ"));
        handler.replace_text_in_range(None, "字", window, cx);
        assert_eq!(text(&view, cx), "aXb字");
    })
    .unwrap();
}

/// Select the first two chars of "abcd" with Shift+Right.
fn select_ab(window: &mut gpui_kit::Window, view: &Entity<EditorView>, cx: &mut gpui_kit::App) {
    window.render_frame(cx);
    window.press("shift-right", cx);
    window.press("shift-right", cx);
    assert_eq!(view.read(cx).editor.selection(), Some(((0, 0), (0, 2))));
}

#[gpui_kit::test]
fn cancelling_a_composition_over_a_selection_keeps_the_selection(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "abcd");
    cx.update_window(handle, |_, window, cx| {
        select_ab(window, &view, cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        // Composing leaves the selection and the buffer as they were.
        handler.replace_and_mark_text_in_range(None, "に", None, window, cx);
        assert_eq!(text(&view, cx), "abcd");
        assert_eq!(view.read(cx).editor.selection(), Some(((0, 0), (0, 2))));
        // Linux cancel: delete the marked range.
        let marked = handler.marked_text_range(window, cx).expect("composing");
        handler.replace_text_in_range(Some(marked), "", window, cx);
        assert_eq!(text(&view, cx), "abcd");
        assert_eq!(view.read(cx).editor.selection(), Some(((0, 0), (0, 2))));
        assert_eq!(view.read(cx).preedit, None);
        // Cancel by an empty preedit update, then an empty commit: still intact.
        handler.replace_and_mark_text_in_range(None, "に", None, window, cx);
        handler.replace_and_mark_text_in_range(None, "", None, window, cx);
        assert_eq!(view.read(cx).preedit, None);
        handler.replace_and_mark_text_in_range(None, "に", None, window, cx);
        handler.replace_text_in_range(None, "", window, cx);
        assert_eq!(text(&view, cx), "abcd");
        assert!(!view.read(cx).editor.is_dirty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn committing_a_composition_over_a_selection_replaces_it_like_typing(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "abcd");
    cx.update_window(handle, |_, window, cx| {
        select_ab(window, &view, cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        handler.replace_and_mark_text_in_range(None, "に", None, window, cx);
        let marked = handler.marked_text_range(window, cx).expect("composing");
        handler.replace_text_in_range(Some(marked), "日", window, cx);
        assert_eq!(text(&view, cx), "日cd");
        assert_eq!(view.read(cx).editor.selection(), None);
    })
    .unwrap();
}

// ---- Vim ---------------------------------------------------------------------

fn mode(view: &Entity<EditorView>, cx: &gpui_kit::App) -> Option<String> {
    view.read(cx).editor.mode_label()
}

fn hint(view: &Entity<EditorView>, cx: &gpui_kit::App) -> Option<String> {
    view.read(cx).editor.pending_input_hint()
}

/// What the last frame painted: the caret's shape and the mode line.
fn painted(view: &Entity<EditorView>, cx: &gpui_kit::App) -> Painted {
    view.read(cx).painted.clone().expect("a frame was painted")
}

fn mode_line(label: &str, hint: Option<&str>) -> Option<ModeLine> {
    Some(ModeLine {
        label: label.into(),
        hint: hint.map(Into::into),
    })
}

#[gpui_kit::test]
fn vim_starts_in_normal_with_a_block_caret_and_a_mode_line_row(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let vim = painted(&view, cx);
        assert_eq!(vim.caret_shape, Some(CaretShape::Block));
        assert_eq!(vim.mode_line, mode_line("NORMAL", None));
        // The same pane as a plain editor: one more text row, no mode line,
        // a bar caret.
        view.update(cx, |v, _| v.editor = Editor::new(false));
        window.refresh();
        window.render_frame(cx);
        let plain = painted(&view, cx);
        assert_eq!(plain.rows, vim.rows + 1);
        assert_eq!(plain.mode_line, None);
        assert_eq!(plain.caret_shape, Some(CaretShape::Bar));
    })
    .unwrap();
}

#[gpui_kit::test]
fn insert_typing_and_escape_switch_mode_caret_and_label(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.input("i", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("INSERT"));
        assert_eq!(painted(&view, cx).caret_shape, Some(CaretShape::Bar));
        assert_eq!(painted(&view, cx).mode_line, mode_line("INSERT", None));
        window.input("xy", cx);
        assert_eq!(text(&view, cx), "xyabc");
        window.press("escape", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        assert_eq!(painted(&view, cx).caret_shape, Some(CaretShape::Block));
        assert_eq!(painted(&view, cx).mode_line, mode_line("NORMAL", None));
        // Ctrl-[ is Esc, as in a terminal.
        window.input("a", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("INSERT"));
        window.press("ctrl-[", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        assert_eq!(text(&view, cx), "xyabc");
    })
    .unwrap();
}

#[gpui_kit::test]
fn dd_deletes_the_line_and_x_the_char(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "one\ntwo\nthree");
    cx.update_window(handle, |_, window, cx| {
        window.input("dd", cx);
        assert_eq!(text(&view, cx), "two\nthree");
        window.input("x", cx);
        assert_eq!(text(&view, cx), "wo\nthree");
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_count_moves_that_many_lines(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "a\nb\nc\nd\ne");
    cx.update_window(handle, |_, window, cx| {
        window.input("3j", cx);
        assert_eq!(view.read(cx).editor.cursor(), (3, 0));
        assert_eq!(text(&view, cx), "a\nb\nc\nd\ne");
    })
    .unwrap();
}

#[gpui_kit::test]
fn visual_selection_is_painted_then_yanked_or_deleted(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "hello world");
    cx.update_window(handle, |_, window, cx| {
        window.input("ve", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("VISUAL"));
        assert_eq!(painted(&view, cx).mode_line, mode_line("VISUAL", None));
        // Visual is inclusive: "hello", cursor on the 'o'.
        assert_eq!(view.read(cx).editor.selection(), Some(((0, 0), (0, 5))));
        assert_eq!(painted(&view, cx).caret_shape, Some(CaretShape::Block));
        window.input("y", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        assert_eq!(view.read(cx).editor.selection(), None);
        window.input("$p", cx);
        assert_eq!(text(&view, cx), "hello worldhello");
        window.input("0ved", cx);
        assert_eq!(text(&view, cx), " worldhello");
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        assert_eq!(view.read(cx).editor.selection(), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn undo_redo_and_dot_repeat_reach_the_engine(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "abcd");
    cx.update_window(handle, |_, window, cx| {
        window.input("x", cx);
        assert_eq!(text(&view, cx), "bcd");
        window.input("u", cx);
        assert_eq!(text(&view, cx), "abcd");
        window.press("ctrl-r", cx);
        assert_eq!(text(&view, cx), "bcd");
        window.input(".", cx);
        assert_eq!(text(&view, cx), "cd");
    })
    .unwrap();
}

#[gpui_kit::test]
fn requests_nothing_serves_yet_are_dropped(cx: &mut TestAppContext) {
    // `:` asks for the palette, which the editor's owner opens (none here):
    // the request is served as an event, never left queued.
    let (handle, view) = open_vim(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.input(":", cx);
        assert_eq!(text(&view, cx), "abc");
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        assert!(!view.read(cx).editor.find_open());
        assert!(view.update(cx, |v, _| v.editor.take_requests()).is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_mode_line_shows_the_pending_command(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "one\ntwo\nthree\nfour");
    cx.update_window(handle, |_, window, cx| {
        window.input("2", cx);
        assert_eq!(painted(&view, cx).mode_line, mode_line("NORMAL", Some("2")));
        window.input("d", cx);
        assert_eq!(hint(&view, cx).as_deref(), Some("2d"));
        assert_eq!(
            painted(&view, cx).mode_line,
            mode_line("NORMAL", Some("2d"))
        );
        window.press("escape", cx);
        assert_eq!(painted(&view, cx).mode_line, mode_line("NORMAL", None));
        assert_eq!(text(&view, cx), "one\ntwo\nthree\nfour");
        window.input("2dd", cx);
        assert_eq!(text(&view, cx), "three\nfour");
        assert_eq!(painted(&view, cx).mode_line, mode_line("NORMAL", None));
    })
    .unwrap();
}

#[gpui_kit::test]
fn operator_pending_paints_no_stale_selection(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "hello");
    cx.update_window(handle, |_, window, cx| {
        window.input("vl", cx);
        assert!(view.read(cx).editor.selection().is_some());
        window.press("escape", cx);
        window.input("d", cx);
        assert_eq!(hint(&view, cx).as_deref(), Some("d"));
        assert_eq!(view.read(cx).editor.selection(), None);
        assert_eq!(painted(&view, cx).caret_shape, Some(CaretShape::Block));
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_input_is_refused_outside_insert(cx: &mut TestAppContext) {
    use gpui_kit::InputHandler;
    let (handle, view) = open_vim(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        assert!(!handler.accepts_text_input(window, cx));
        let mut ime = TestInput::new(ElementInputHandler::new(bounds, view.clone()), window);
        // No composition and no multi-char commit in Normal.
        ime.compose_text(None, "に", None, cx);
        assert_eq!(view.read(cx).preedit, None);
        assert_eq!(ime.marked_text_range(cx), None);
        ime.commit_text("日本", cx);
        ime.unmark_text(cx);
        assert_eq!(text(&view, cx), "abc");
        assert!(!view.read(cx).editor.is_dirty());
        // A single committed char is the keystroke it is: vim `x`.
        ime.commit_text("x", cx);
        assert_eq!(text(&view, cx), "bc");
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        // In Insert the IME types as in the plain editor.
        window.input("i", cx);
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        assert!(handler.accepts_text_input(window, cx));
        let mut ime = TestInput::new(ElementInputHandler::new(bounds, view.clone()), window);
        ime.compose_text(None, "に", None, cx);
        assert_eq!(view.read(cx).preedit.as_deref(), Some("に"));
        ime.commit_text("日本", cx);
        assert_eq!(text(&view, cx), "日本bc");
    })
    .unwrap();
}

/// A parent of the editor that counts the key-downs bubbling out of it.
struct KeySpy {
    editor: Entity<EditorView>,
    bubbled: usize,
}

impl gpui_kit::Render for KeySpy {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{InteractiveElement, ParentElement, Styled};
        gpui_kit::div()
            .size_full()
            .on_key_down(cx.listener(|this, _: &gpui_kit::KeyDownEvent, _, _| {
                this.bubbled += 1;
            }))
            .child(self.editor.clone())
    }
}

/// A vim editor showing `text` inside a [`KeySpy`].
fn open_spy(cx: &mut TestAppContext, text: &str) -> (AnyWindowHandle, Entity<KeySpy>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let text = text.to_string();
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(640.), px(480.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let editor = cx.new(|cx| EditorView::new(Some(text), Theme::default(), true, cx));
                window.focus(&editor.read(cx).focus_handle(cx), cx);
                cx.new(|_| KeySpy { editor, bubbled: 0 })
            },
        )
    })
    .expect("open test window")
}

#[gpui_kit::test]
fn a_multi_char_key_reaches_the_platform_only_in_insert(cx: &mut TestAppContext) {
    use gpui_kit::Keystroke;
    let (handle, spy) = open_spy(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Text that is not one `char` (no `KeyInput`) bubbles out to the
        // platform's input handler only where the editor types.
        let cluster = || {
            let mut key = Keystroke::parse("e").unwrap();
            key.key_char = Some("e\u{301}".into());
            key
        };
        let view = spy.read(cx).editor.clone();
        window.dispatch_keystroke(cluster(), cx);
        window.render_frame(cx);
        assert_eq!(spy.read(cx).bubbled, 0, "stopped in Normal");
        assert_eq!(text(&view, cx), "abc");
        // `i` is consumed by the engine and does not bubble either.
        window.input("i", cx);
        assert_eq!(spy.read(cx).bubbled, 0);
        window.dispatch_keystroke(cluster(), cx);
        assert_eq!(spy.read(cx).bubbled, 1, "propagates in Insert");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_click_leaves_visual_and_a_drag_enters_it(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "hello\nworld");
    cx.update_window(handle, |_, window, cx| {
        window.input("vl", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("VISUAL"));
        let at = cell_center(&view, 2, 1, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        // The press leaves an empty drag anchor at the cursor (as in the TUI),
        // which paints nothing.
        let empty = |sel: Option<((usize, usize), (usize, usize))>| sel.is_none_or(|(a, b)| a == b);
        assert!(empty(view.read(cx).editor.selection()));
        assert_eq!(view.read(cx).editor.cursor(), (1, 2));
        // A drag selects what it covers, in Visual.
        window.render_frame(cx);
        let from = cell_center(&view, 0, 0, cx);
        let to = cell_center(&view, 3, 0, cx);
        window.pointer_down(from, MouseButton::Left, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
        window.pointer_up(to, MouseButton::Left, cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("VISUAL"));
        assert!(view.read(cx).editor.selection().is_some());
        // The engine's drag rule: the char at the far end is not covered.
        window.input("d", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        assert_eq!(text(&view, cx), "lo\nworld");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_press_on_the_mode_line_does_not_move_the_cursor(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "hello");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rows = painted(&view, cx).rows;
        let at = cell_center(&view, 3, rows, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 0));
        assert_eq!(window.find("editor").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_press_below_the_last_row_reaches_the_plain_editor(cx: &mut TestAppContext) {
    // Without a mode line the strip under the last full row is still text
    // area (M1): the press goes to the editor, which clamps it.
    let (handle, view) = open(cx, "hello\nworld");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = painted(&view, cx);
        assert_eq!(p.mode_line, None);
        let at = point(
            p.origin.x + px(p.cells.x_of(2) + 1.0),
            p.origin.y + px(p.cells.y_of(p.rows) + 1.0),
        );
        assert!(
            window.find("editor").bounds().contains(&at),
            "the 480px test pane leaves a partial row"
        );
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 2));
    })
    .unwrap();
}

#[gpui_kit::test]
fn following_the_cursor_keeps_it_above_the_mode_line(cx: &mut TestAppContext) {
    let long = (0..200)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (handle, view) = open_vim(cx, &long);
    cx.update_window(handle, |_, window, cx| {
        window.input("G", cx);
        assert_eq!(view.read(cx).editor.cursor().0, 199);
        let p = painted(&view, cx);
        let caret = p.caret.expect("the caret is drawn");
        // The caret is on the last text row, never on the mode-line row.
        let row = p.cells.row_at(f32::from(caret.origin.y - p.origin.y));
        assert_eq!(row, p.rows - 1);
        assert_eq!(p.caret_shape, Some(CaretShape::Block));
        let cursor = view
            .update(cx, |v, _| v.editor.session().display.frame(80, true).cursor)
            .expect("the frame has a cursor");
        assert!(cursor.1 < p.rows);
    })
    .unwrap();
}

// ---- Markdown rendering (M3) -------------------------------------------------

use crate::row_paint::{PaintPiece, RowPaint};
use crate::theme::{self, Token};

/// What row `row` painted in the last frame.
fn painted_row(view: &Entity<EditorView>, row: usize, cx: &gpui_kit::App) -> RowPaint {
    (*painted(view, cx).row_paint[row]).clone()
}

fn piece<'a>(row: &'a RowPaint, text: &str) -> &'a PaintPiece {
    row.pieces
        .iter()
        .find(|p| p.text == text)
        .unwrap_or_else(|| panic!("no piece {text:?} in {:?}", row.text()))
}

fn fg(color: &kimun_notes::settings::themes::ThemeColor, t: &Theme) -> gpui_kit::Rgba {
    theme::to_gpui(color, Token::Foreground, t)
}

/// The fixture with the caret parked on its first row (a heading, nothing to
/// reveal below it).
fn open_fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<EditorView>) {
    open(cx, crate::fixture::RICH)
}

#[gpui_kit::test]
fn headings_and_emphasis_paint_in_their_roles_with_bold_drawn(cx: &mut TestAppContext) {
    let (handle, view) = open_fixture(cx);
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = painted(&view, cx);
        // The headless text system has no fonts: no family with a bold face,
        // so bold is synthetic (painted twice).
        assert!(p.fonts.faux_bold);
        let h2 = painted_row(&view, 1, cx);
        assert_eq!(h2.text(), "## Heading two");
        assert_eq!(piece(&h2, "## ").style.fg, fg(&t.gray, &t));
        let title = piece(&h2, "Heading two").style;
        assert!(title.bold);
        assert_eq!(title.fg, fg(&t.fg_bright, &t));
        // H4-H6 paint as H3 (the logic has three heading kinds).
        let h6 = painted_row(&view, 5, cx);
        assert_eq!(piece(&h6, "Heading six").style.fg, fg(&t.yellow, &t));

        let em = painted_row(&view, 6, cx);
        assert_eq!(em.text(), "Plain bold and italic and gone and both end");
        let bold = piece(&em, "bold").style;
        assert!(bold.bold && !bold.italic && !bold.strike);
        assert_eq!(bold.fg, theme::foreground(&t));
        let italic = piece(&em, "italic").style;
        assert!(italic.italic && !italic.bold);
        assert!(piece(&em, "gone").style.strike);
        let both = piece(&em, "both").style;
        assert!(both.bold && both.italic);
        // Pieces sit on the cells the logic laid the text out on.
        assert_eq!(piece(&em, "bold").col, 6);
        assert_eq!(piece(&em, " end").col, 39);

        let nested = painted_row(&view, 7, cx);
        let inner = piece(&nested, "inner").style;
        assert!(inner.bold && inner.italic);
        let code = piece(&nested, "inline code");
        assert_eq!(code.style.fg, fg(&t.aqua, &t));
        assert_eq!(
            nested.fill_at(code.col),
            theme::to_gpui(&t.bg_soft, Token::Background, &t).into()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fenced_block_is_one_solid_box_per_row_padded_to_its_widest_line(cx: &mut TestAppContext) {
    let (handle, view) = open_fixture(cx);
    let t = Theme::default();
    let code_bg = theme::to_gpui(&t.code_bg, Token::Background, &t);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Fence, two code rows ("let wide = 1;" is the widest: 13), fence.
        for row in 8..=11 {
            let r = painted_row(&view, row, cx);
            assert_eq!(r.fills.len(), 1, "row {row}: {:?}", r.fills);
            assert_eq!((r.fills[0].col, r.fills[0].cells), (0, 13), "row {row}");
            assert_eq!(r.fills[0].color, code_bg);
        }
        let code = painted_row(&view, 9, cx);
        assert_eq!(code.text(), "fn main() {}");
        // Code is plain text colour, never markdown-styled.
        assert_eq!(piece(&code, "fn main() {}").style.fg, theme::foreground(&t));
        // The row after the block has no box.
        assert!(painted_row(&view, 12, cx).fills.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn quote_bars_are_strokes_per_depth_and_the_rule_a_line_across_the_pane(cx: &mut TestAppContext) {
    let (handle, view) = open_fixture(cx);
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bar = fg(&t.blockquote_bar, &t);
        let quote = painted_row(&view, 12, cx);
        let cols: Vec<_> = quote.bars.iter().map(|b| (b.col, b.color)).collect();
        assert_eq!(cols, vec![(0, bar)]);
        assert_eq!(quote.text(), "  quoted line");
        let nested = painted_row(&view, 13, cx);
        assert_eq!(
            nested.bars.iter().map(|b| b.col).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(nested.text(), "   nested quote");
        // No `│` glyph is drawn: the strokes stand in for it.
        assert!(!quote.text().contains('│'));

        let rule = painted_row(&view, 20, cx);
        assert!(rule.pieces.is_empty(), "no ─ glyphs");
        assert_eq!(rule.rules.len(), 1);
        let p = painted(&view, cx);
        let pane = window.find("editor").bounds();
        let (cols, _) = p.cells.fit(
            f32::from(pane.size.width) - 32.0,
            f32::from(pane.size.height) - 16.0,
        );
        assert_eq!((rule.rules[0].col, rule.rules[0].cells), (0, cols));
        assert_eq!(rule.rules[0].color, fg(&t.gray, &t));
    })
    .unwrap();
}

#[gpui_kit::test]
fn lists_and_task_boxes_paint_their_markers_and_done_tasks_dim_and_strike(cx: &mut TestAppContext) {
    let (handle, view) = open_fixture(cx);
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bullet = painted_row(&view, 14, cx);
        assert_eq!(bullet.text(), "- bullet item");
        assert_eq!(piece(&bullet, "- ").style.fg, fg(&t.gray, &t));
        assert_eq!(painted_row(&view, 15, cx).text(), "1. ordered item");

        let open = painted_row(&view, 16, cx);
        assert_eq!(piece(&open, "[ ]").style.fg, fg(&t.accent, &t));
        assert!(!piece(&open, " open task").style.strike);

        let done = painted_row(&view, 17, cx);
        assert_eq!(piece(&done, "[x]").style.fg, fg(&t.accent, &t));
        let text = piece(&done, " done task").style;
        assert!(text.strike);
        // Dimmed toward the page, not the plain foreground.
        assert_ne!(text.fg, theme::foreground(&t));
    })
    .unwrap();
}

#[gpui_kit::test]
fn links_wikilinks_images_and_labels_are_hand_cursor_targets_and_links_underlined(
    cx: &mut TestAppContext,
) {
    let (handle, view) = open_fixture(cx);
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let row = painted_row(&view, 18, cx);
        assert_eq!(row.text(), "A link and Wiki Note and #label here");
        assert_eq!(row.targets, vec![(2, 4), (11, 9), (25, 6)]);
        assert_eq!(piece(&row, "link").style.fg, fg(&t.accent, &t));
        assert_eq!(piece(&row, "Wiki Note").style.fg, fg(&t.blue, &t));
        // Underlined as in the TUI editor; the label is not.
        assert!(piece(&row, "link").style.underline);
        assert!(piece(&row, "Wiki Note").style.underline);
        assert!(!piece(&row, "#label").style.underline);
        let label = piece(&row, "#label").style;
        assert!(label.bold);
        assert_eq!(label.fg, fg(&t.color_tag, &t));
        let image = painted_row(&view, 19, cx);
        assert_eq!(image.text(), "An image [pic.png] inline");
        assert_eq!(image.targets, vec![(9, 9)]);
        assert!(piece(&image, "[pic.png]").style.italic);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_link_under_the_caret_is_raw_source_and_no_target(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "x\nA [link](u) end");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(painted_row(&view, 1, cx).targets, vec![(2, 4)]);
        // A press on the link (under its hand-cursor hitbox) still reaches
        // the editor and only places the caret: on the 'i', buffer col 4.
        let at = cell_center(&view, 3, 1, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 4));
        window.render_frame(cx);
        let row = painted_row(&view, 1, cx);
        assert_eq!(row.text(), "A [link](u) end");
        assert!(row.targets.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn wide_chars_take_two_cells_and_combining_marks_ride_their_base(cx: &mut TestAppContext) {
    let (handle, view) = open_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let row = painted_row(&view, 21, cx);
        let cells: Vec<_> = row
            .pieces
            .iter()
            .map(|p| (p.col, p.cells, p.text.as_str(), p.grid))
            .collect();
        assert_eq!(
            cells,
            vec![
                (0, 5, "Wide ", true),
                (5, 2, "理", false),
                (7, 2, "理", false),
                (9, 7, " emoji ", true),
                (16, 2, "😀", false),
                (18, 16, " combining e\u{301} end", true),
            ]
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_caret_entering_bold_reveals_its_sigils_and_leaving_hides_them(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "x\nsome **bold** text");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(painted_row(&view, 1, cx).text(), "some bold text");
        // On the row but outside the element: still concealed (the TUI
        // reveals the element under the caret, not the whole row).
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(painted_row(&view, 1, cx).text(), "some bold text");
        for _ in 0..6 {
            window.press("right", cx);
        }
        window.render_frame(cx);
        let row = painted_row(&view, 1, cx);
        assert_eq!(row.text(), "some **bold** text");
        // Revealed source is muted raw text: no bold, sigil grey.
        assert!(!piece(&row, "**bold**").style.bold);
        // The caret is drawn on the cell of the char it is on (col 6, '*').
        let p = painted(&view, cx);
        let caret = p.caret.expect("caret");
        assert_eq!(p.cells.col_at(f32::from(caret.origin.x - p.origin.x)), 6);
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(painted_row(&view, 1, cx).text(), "some bold text");
    })
    .unwrap();
}

#[gpui_kit::test]
fn clicks_land_on_the_char_painted_under_them_hidden_or_revealed(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "x\nsome **bold** text");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Concealed: "some bold text", cell 6 shows the 'o' of bold, which
        // is buffer column 8.
        let at = cell_center(&view, 6, 1, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 8));
        window.render_frame(cx);
        // Now revealed: "some **bold** text"; the caret cell is the 'o'.
        let row = painted_row(&view, 1, cx);
        assert_eq!(row.text(), "some **bold** text");
        let p = painted(&view, cx);
        let caret = p.caret.expect("caret");
        assert_eq!(p.cells.col_at(f32::from(caret.origin.x - p.origin.x)), 8);
        // A click on revealed text: cell 14 is the 't' of "text".
        let at = cell_center(&view, 14, 1, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 14));
        window.render_frame(cx);
        // Concealed again; the caret follows the 't' to cell 10.
        assert_eq!(painted_row(&view, 1, cx).text(), "some bold text");
        let p = painted(&view, cx);
        let caret = p.caret.expect("caret");
        assert_eq!(p.cells.col_at(f32::from(caret.origin.x - p.origin.x)), 10);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_selection_over_styled_runs_keeps_their_ink_under_one_selection_fill(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "pre **bold** and `code` end");
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.press("end", cx);
        window.press("shift-home", cx);
        window.render_frame(cx);
        let row = painted_row(&view, 0, cx);
        assert_eq!(row.text(), "pre bold and code end");
        // One fill over every cell, inline code's own background included.
        assert_eq!(row.fills.len(), 1, "{:?}", row.fills);
        assert_eq!((row.fills[0].col, row.fills[0].cells), (0, 21));
        assert_eq!(row.fills[0].color, theme::selection_background(&t));
        let bold = piece(&row, "bold").style;
        assert!(bold.bold);
        assert_eq!(bold.fg, theme::foreground(&t));
        assert_eq!(piece(&row, "code").style.fg, fg(&t.aqua, &t));
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_needles_are_emphasised_where_they_are_drawn(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "x\nsome **bold** text");
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        // What opening a note from a vault search does (M5+).
        view.update(cx, |v, _| v.editor.set_search_needles(vec!["text".into()]));
        window.refresh();
        window.render_frame(cx);
        let row = painted_row(&view, 1, cx);
        let needle = piece(&row, "text").style;
        assert!(needle.bold);
        assert_eq!(needle.fg, fg(&t.color_search_match, &t));
        assert_eq!(piece(&row, "text").col, 10);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_5k_line_note_rebuilds_only_the_rows_that_change(cx: &mut TestAppContext) {
    let note: String = (0..5000)
        .map(|i| match i % 5 {
            0 => format!("## Section {i}"),
            1 => format!("Some **bold {i}** and `code` and [[link {i}]]"),
            2 => format!("- [ ] task {i}"),
            3 => format!("> quote {i}"),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A note this size parses and lays out on tokio tasks (the GUI's main
    // enters a runtime). Here a current-thread runtime runs them on the test
    // thread, when `settle` drives it: GPUI's test scheduler rejects wakes
    // from other threads.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let _guard = rt.enter();
    let settle = || {
        rt.block_on(async {
            for _ in 0..16 {
                tokio::task::yield_now().await;
            }
        })
    };
    let (handle, view) = open(cx, &note);
    cx.update_window(handle, |_, window, cx| {
        let built = |cx: &gpui_kit::App| view.read(cx).row_cache.built();
        window.render_frame(cx);
        settle();
        window.refresh();
        window.render_frame(cx);
        let rows = usize::from(painted(&view, cx).rows);
        // The rich parse has landed: row 1 is styled.
        assert_eq!(painted_row(&view, 1, cx).text(), "Some bold 1 and code and link 1");
        let first = built(cx);
        assert!(first <= 2 * rows, "{first} rows built for {rows} visible");
        // A frame with nothing new builds nothing.
        window.refresh();
        window.render_frame(cx);
        assert_eq!(built(cx), first);
        // Typing rebuilds the edited row only.
        let started = std::time::Instant::now();
        for (n, ch) in "abcdefghij".chars().enumerate() {
            let before = built(cx);
            window.input(&ch.to_string(), cx);
            settle();
            window.refresh();
            window.render_frame(cx);
            assert!(built(cx) - before <= 1, "keystroke {n} rebuilt {}", built(cx) - before);
        }
        let typing = started.elapsed();
        // A three-row wheel notch builds the three rows scrolled in.
        let before = built(cx);
        window.scroll("editor", ScrollDelta::Lines(point(0., -3.)), cx);
        window.render_frame(cx);
        assert!(built(cx) - before <= 3, "scroll rebuilt {}", built(cx) - before);
        println!(
            "5k-line note: {rows} rows; first frame built {first}; 10 keystrokes+frames in {typing:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn emphasis_inside_a_link_label_is_part_of_its_hand_cursor_target(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "x\nA [**bold** text](u) end");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let row = painted_row(&view, 1, cx);
        assert_eq!(row.text(), "A bold text end");
        // The bold run's role is Bold, yet the whole label is one target.
        assert!(piece(&row, "bold").style.bold);
        assert_eq!(row.targets, vec![(2, 9)]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_block_caret_on_a_wide_char_repaints_its_glyph_centred_like_the_row(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "理x");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let p = painted(&view, cx);
        // `caret` records the caret's first cell; the block spans two here.
        let block = p.caret.expect("caret");
        let glyph = p.caret_glyph.expect("glyph under the block");
        // The glyph is narrower than its two cells: centred, not at the
        // block's left edge, and never past its middle.
        assert!(glyph.x > block.origin.x, "{glyph:?} vs {block:?}");
        assert!(glyph.x < block.origin.x + px(p.cells.width()));
        // On a one-cell char the glyph starts on its cell.
        window.input("l", cx);
        let p = painted(&view, cx);
        assert_eq!(p.caret_glyph.map(|g| g.x), p.caret.map(|c| c.origin.x));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_click_after_a_wide_char_lands_on_the_concealed_text_drawn_there(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "x\n理 **bold**");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(painted_row(&view, 1, cx).text(), "理 bold");
        // 理 covers cells 0-1, the space 2, "bold" 3-6: cell 4 is the 'o',
        // buffer column 5 (理 0, space 1, ** 2-3, b 4).
        let at = cell_center(&view, 4, 1, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 5));
        window.render_frame(cx);
        // Revealed: "理 **bold**", the caret on the 'o' at cell 6.
        assert_eq!(painted_row(&view, 1, cx).text(), "理 **bold**");
        let p = painted(&view, cx);
        let caret = p.caret.expect("caret");
        assert_eq!(p.cells.col_at(f32::from(caret.origin.x - p.origin.x)), 6);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_wrapped_quote_keeps_its_bars_on_every_continuation_row(cx: &mut TestAppContext) {
    let long = "word ".repeat(60);
    let (handle, view) = open(cx, &format!("x\n>> {long}\n\nafter"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rows = painted(&view, cx).row_paint;
        // The quote wraps onto at least three rows (a blank line ends it:
        // "after" right below would be a lazy continuation of the quote).
        let quote: Vec<_> = rows[1..]
            .iter()
            .take_while(|r| !r.bars.is_empty())
            .collect();
        assert!(quote.len() >= 3, "{} quote rows", quote.len());
        for (i, row) in quote.iter().enumerate() {
            let cols: Vec<_> = row.bars.iter().map(|b| b.col).collect();
            assert_eq!(cols, vec![0, 1], "row {i}");
            // Text starts after the gutter on every row.
            assert!(row.pieces.iter().all(|p| p.col >= 3), "row {i}");
        }
        assert_eq!(rows[1 + quote.len()].text(), "");
        assert_eq!(rows[2 + quote.len()].text(), "after");
    })
    .unwrap();
}

// ---- Find bar (M4) -----------------------------------------------------------

use kimun_notes::logic::editor::find_bar::{BarFocus, FIND_HINTS};

/// The find bar's rows as the last frame painted them.
fn bar_rows(view: &Entity<EditorView>, cx: &gpui_kit::App) -> Vec<String> {
    painted(view, cx)
        .find_bar
        .map(|b| b.rows.iter().map(RowPaint::text).collect())
        .unwrap_or_default()
}

fn query(view: &Entity<EditorView>, cx: &gpui_kit::App) -> String {
    view.read(cx)
        .editor
        .find_bar()
        .expect("the bar is open")
        .query()
        .to_string()
}

/// The pane row the caret was drawn on in the last frame.
fn caret_row(p: &Painted) -> u16 {
    let caret = p.caret.expect("a caret is drawn");
    p.cells.row_at(f32::from(caret.origin.y - p.origin.y))
}

#[gpui_kit::test]
fn ctrl_f_docks_a_bar_that_searches_instead_of_typing(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "one foo\ntwo foo\nthree");
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let text_rows = painted(&view, cx).rows;
        assert_eq!(painted(&view, cx).find_bar, None);
        window.press("ctrl-f", cx);
        assert!(view.read(cx).editor.find_open());
        // One row docked below the text.
        assert_eq!(painted(&view, cx).rows, text_rows - 1);
        window.input("foo", cx);
        assert_eq!(text(&view, cx), "one foo\ntwo foo\nthree");
        assert!(!view.read(cx).editor.is_dirty());
        // The TUI's words; the hints run off a 640px pane and are cut.
        let rows = bar_rows(&view, cx);
        assert_eq!(rows.len(), 1);
        let full = format!(" Find: foo  2 matches  any case{FIND_HINTS}");
        assert!(full.starts_with(&rows[0]) && rows[0].len() > 50, "{rows:?}");
        // The cursor is on the first match, which is painted as the selection;
        // every match is emphasised.
        assert_eq!(view.read(cx).editor.cursor(), (0, 4));
        let first = painted_row(&view, 0, cx);
        assert_eq!(first.fill_at(4), Some(theme::selection_background(&t)));
        let second = painted_row(&view, 1, cx);
        let m = piece(&second, "foo").style;
        assert!(m.bold);
        assert_eq!(m.fg, fg(&t.color_search_match, &t));
        assert_eq!(second.fill_at(4), None);
        // The bar has the caret, a bar after "foo" (" Find: " is 7 cells).
        let p = painted(&view, cx);
        assert_eq!(caret_row(&p), p.rows);
        assert_eq!(p.caret_shape, Some(CaretShape::Bar));
        let caret = p.caret.unwrap();
        assert_eq!(p.cells.col_at(f32::from(caret.origin.x - p.origin.x)), 10);
        // Ctrl-F on an open bar steps to the next match.
        window.press("ctrl-f", cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 4));
        assert_eq!(
            painted_row(&view, 1, cx).fill_at(4),
            Some(theme::selection_background(&t))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn vim_slash_opens_the_bar_enter_steps_and_esc_keeps_the_pattern_for_n(cx: &mut TestAppContext) {
    let (handle, view) = open_vim(cx, "foo a\nfoo b\nfoo c");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let text_rows = painted(&view, cx).rows;
        window.input("/", cx);
        assert!(view.read(cx).editor.find_open());
        // In Normal, yet the keys type into the bar, not vim commands.
        window.input("foo", cx);
        assert_eq!(query(&view, cx), "foo");
        assert_eq!(text(&view, cx), "foo a\nfoo b\nfoo c");
        // The bar sits between the text and the mode line.
        let p = painted(&view, cx);
        assert_eq!(p.rows, text_rows - 1);
        assert_eq!(p.mode_line, mode_line("NORMAL", None));
        assert_eq!(caret_row(&p), p.rows);
        assert_eq!(view.read(cx).editor.cursor(), (0, 0));
        window.press("enter", cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 0));
        window.press("enter", cx);
        assert_eq!(view.read(cx).editor.cursor(), (2, 0));
        window.press("shift-enter", cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 0));
        window.press("escape", cx);
        assert!(!view.read(cx).editor.find_open());
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        assert_eq!(view.read(cx).editor.selection(), None);
        let p = painted(&view, cx);
        assert_eq!((p.rows, p.find_bar.clone()), (text_rows, None));
        assert_eq!(p.caret_shape, Some(CaretShape::Block));
        // The pattern outlives the bar: `n` / `N` step with it.
        window.input("n", cx);
        assert_eq!(view.read(cx).editor.cursor(), (2, 0));
        window.input("N", cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 0));
        assert_eq!(text(&view, cx), "foo a\nfoo b\nfoo c");
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_reveals_replace_with_a_preview_enter_replaces_and_ctrl_a_replaces_all(
    cx: &mut TestAppContext,
) {
    let (handle, view) = open(cx, "foo foo foo");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let text_rows = painted(&view, cx).rows;
        window.press("ctrl-f", cx);
        window.input("foo", cx);
        window.press("tab", cx);
        assert_eq!(painted(&view, cx).rows, text_rows - 2);
        window.input("x", cx);
        let bar = view.read(cx).editor.find_bar().unwrap();
        assert_eq!((bar.focus(), bar.replacement()), (BarFocus::Replace, "x"));
        // The preview shows the note as it would read; the buffer is intact.
        assert_eq!(painted_row(&view, 0, cx).text(), "x x x");
        assert_eq!(text(&view, cx), "foo foo foo");
        let rows = bar_rows(&view, cx);
        assert_eq!(rows[0], " Find: foo  3 matches  any case");
        assert!(
            rows[1].starts_with(" Replace: x  [Enter] replace"),
            "{rows:?}"
        );
        // The caret is in the replace field (second bar row).
        let p = painted(&view, cx);
        assert_eq!(caret_row(&p), p.rows + 1);
        window.press("enter", cx);
        assert_eq!(text(&view, cx), "x foo foo");
        window.press("ctrl-a", cx);
        assert_eq!(text(&view, cx), "x x x");
        // Ctrl-Z in the bar takes the replace all back as one step.
        window.press("ctrl-z", cx);
        assert_eq!(text(&view, cx), "x foo foo");
        window.press("ctrl-y", cx);
        assert_eq!(text(&view, cx), "x x x");
    })
    .unwrap();
}

#[gpui_kit::test]
fn ctrl_h_opens_replace_and_an_empty_replace_all_arms_first(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "foo bar foo");
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-h", cx);
        let bar = view.read(cx).editor.find_bar().expect("open");
        // Nothing to replace yet: the query field has the keyboard.
        assert!(bar.is_replacing());
        assert_eq!(bar.focus(), BarFocus::Find);
        window.input("foo", cx);
        window.press("tab", cx);
        window.press("ctrl-a", cx);
        assert_eq!(text(&view, cx), "foo bar foo");
        let p = painted(&view, cx).find_bar.unwrap();
        let tail = p.rows[1].pieces.last().unwrap().clone();
        assert!(tail.text.contains("delete every match?"), "{}", tail.text);
        assert_eq!(tail.style.fg, theme::error(&t));
        // Esc disarms first and keeps the bar.
        window.press("escape", cx);
        assert!(view.read(cx).editor.find_open());
        assert!(!view.read(cx).editor.find_bar().unwrap().is_armed());
        window.press("ctrl-a", cx);
        window.press("ctrl-a", cx);
        assert_eq!(text(&view, cx), " bar ");
        window.press("escape", cx);
        assert!(!view.read(cx).editor.find_open());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_invalid_regex_and_no_match_are_reported_in_the_bar(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "abc");
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("(", cx);
        let row = bar_rows(&view, cx)[0].clone();
        assert!(row.starts_with(" Find: (  invalid regex: "), "{row}");
        window.press("backspace", cx);
        window.input("zz", cx);
        assert_eq!(bar_rows(&view, cx)[0], " Find: zz  no match  any case");
        let p = painted(&view, cx).find_bar.unwrap();
        assert_eq!(p.rows[0].pieces.last().unwrap().style.fg, theme::error(&t));
        assert_eq!(text(&view, cx), "abc");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_current_match_is_scrolled_into_view(cx: &mut TestAppContext) {
    let long = (0..200)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (handle, view) = open(cx, &long);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-f", cx);
        window.input("line 150", cx);
        assert_eq!(view.read(cx).editor.cursor(), (150, 0));
        let p = painted(&view, cx);
        let shown: Vec<String> = p.row_paint.iter().map(|r| r.text()).collect();
        assert!(shown.iter().any(|r| r == "line 150"), "{shown:?}");
        // A wheel scroll away, then a step: the view follows the match back.
        window.press("escape", cx);
        window.press("ctrl-f", cx);
        window.input("line 10$", cx);
        window.scroll("editor", ScrollDelta::Lines(point(0., -30.)), cx);
        window.render_frame(cx);
        window.press("enter", cx);
        let p = painted(&view, cx);
        let shown: Vec<String> = p.row_paint.iter().map(|r| r.text()).collect();
        assert!(shown.iter().any(|r| r == "line 10"), "{shown:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_input_lands_in_the_bar_even_in_vim_normal(cx: &mut TestAppContext) {
    use gpui_kit::InputHandler;
    let (handle, view) = open_vim(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.input("/", cx);
        window.render_frame(cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        assert!(handler.accepts_text_input(window, cx));
        let mut ime = TestInput::new(ElementInputHandler::new(bounds, view.clone()), window);
        ime.compose_text(None, "に", None, cx);
        assert_eq!(view.read(cx).preedit.as_deref(), Some("に"));
        // The preedit is drawn in the bar, not in the text.
        assert!(bar_rows(&view, cx)[0].starts_with(" Find: に"));
        assert_eq!(ime.marked_text_range(cx), Some(0..1));
        ime.commit_text("日本", cx);
        assert_eq!(query(&view, cx), "日本");
        assert_eq!(view.read(cx).preedit, None);
        // A range replacement (macOS press-and-hold) edits the field.
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        handler.replace_text_in_range(Some(1..2), "é", window, cx);
        assert_eq!(query(&view, cx), "日é");
        assert_eq!(text(&view, cx), "abc");
        assert!(!view.read(cx).editor.is_dirty());
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        // After the bar closes Normal refuses text again.
        window.press("escape", cx);
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        assert!(!handler.accepts_text_input(window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn super_chords_do_not_type_into_the_bar_and_paste_reads_the_clipboard(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "foo bar");
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-f", cx);
        // macOS Cmd-Z / Cmd-A must not type `z` / `a` into the field.
        window.press("cmd-z", cx);
        window.press("cmd-a", cx);
        assert_eq!(query(&view, cx), "");
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("bar\nbaz".into()));
        window.press("ctrl-v", cx);
        // First line only: a field is one line.
        assert_eq!(query(&view, cx), "bar");
        assert_eq!(view.read(cx).editor.cursor(), (0, 4));
        window.press("backspace", cx);
        window.press("backspace", cx);
        window.press("backspace", cx);
        window.press("cmd-v", cx);
        assert_eq!(query(&view, cx), "bar");
        assert_eq!(text(&view, cx), "foo bar");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_press_on_a_bar_row_gives_its_field_the_keyboard(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "foo");
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("fo", cx);
        window.press("tab", cx);
        assert_eq!(
            view.read(cx).editor.find_bar().unwrap().focus(),
            BarFocus::Replace
        );
        let rows = painted(&view, cx).rows;
        let at = cell_center(&view, 3, rows, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(
            view.read(cx).editor.find_bar().unwrap().focus(),
            BarFocus::Find
        );
        window.input("o", cx);
        assert_eq!(query(&view, cx), "foo");
        // The press did not reach the text.
        assert_eq!(text(&view, cx), "foo");
        let at = cell_center(&view, 3, rows + 1, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(
            view.read(cx).editor.find_bar().unwrap().focus(),
            BarFocus::Replace
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_preview_far_from_the_cursor_paints_and_closes_without_a_stale_row(cx: &mut TestAppContext) {
    // Regression (manual gate): Esc on a bar whose preview had changed a row
    // more than four rows from the cursor panicked in the next frame.
    let note = "a here\n\nfiller 1\n\nfiller 3\n\nfiller 5\n\nfiller 7\nand a there";
    let (handle, view) = open(cx, note);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-h", cx);
        window.input("a ", cx);
        window.press("tab", cx);
        window.input("LONGER ", cx);
        assert_eq!(painted_row(&view, 9, cx).text(), "and LONGER there");
        window.press("escape", cx);
        assert!(!view.read(cx).editor.find_open());
        assert_eq!(painted_row(&view, 0, cx).text(), "a here");
        assert_eq!(painted_row(&view, 9, cx).text(), "and a there");
        assert_eq!(text(&view, cx), note);
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_empty_replacement_marks_the_removal_point_and_nothing_after_it(cx: &mut TestAppContext) {
    // Regression (manual gate): `ee` -> "" on "Three" lit the cells after
    // "Thr" with the selection (the old match's real columns).
    let (handle, view) = open(cx, "One Three apples\nsee eel ee");
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-h", cx);
        window.input("ee", cx);
        window.press("tab", cx);
        let row = painted_row(&view, 0, cx);
        assert_eq!(row.text(), "One Thr apples");
        assert_eq!(painted_row(&view, 1, cx).text(), "s l ");
        // No selection fill anywhere on the previewed row.
        let sel = theme::selection_background(&t);
        assert!(row.fills.iter().all(|f| f.color != sel), "{:?}", row.fills);
        // The removal point is the one cell after "Thr" (here the space),
        // filled in the search-match colour (inverse video); nothing else is filled.
        let preview = theme::fg_of(&t.color_search_match, &t);
        let fills: Vec<_> = row
            .fills
            .iter()
            .map(|f| (f.col, f.cells, f.color))
            .collect();
        assert_eq!(fills, vec![(7, 1, preview)]);
        assert_eq!(piece(&row, "apples").style.fg, theme::foreground(&t));
    })
    .unwrap();
}

// ---- The GUI keymap (M5b-1) --------------------------------------------------

fn bar_open(view: &Entity<EditorView>, cx: &gpui_kit::App) -> bool {
    view.read(cx).editor.find_open()
}

fn replacing(view: &Entity<EditorView>, cx: &gpui_kit::App) -> bool {
    view.read(cx)
        .editor
        .find_bar()
        .is_some_and(|b| b.is_replacing())
}

/// The vim rule (ADR-0050 §6, narrowed): only the Ctrl+letters the vim engine
/// reads (R, C, X, V) are vim's. Ctrl-F and Ctrl-H are the app's under vim too,
/// and the Ctrl+Shift twins work as well.
#[gpui_kit::test]
fn under_vim_the_app_keeps_ctrl_letters_vim_does_not_read(cx: &mut TestAppContext) {
    let all = "foo\nbar\nfoo";
    let (handle, spy) = open_spy(cx, all);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = spy.read(cx).editor.clone();
        // Ctrl-R is vim's redo: consumed by the engine, nothing bubbles.
        window.press("ctrl-r", cx);
        assert!(!bar_open(&view, cx));
        assert_eq!(spy.read(cx).bubbled, 0, "vim consumed Ctrl-R");
        window.press("ctrl-shift-t", cx);
        assert_eq!(
            spy.read(cx).bubbled,
            1,
            "the right drawer's chord passes the editor by"
        );
        // Ctrl-T has no vim twin, but vim reads no Ctrl-T: it passes too.
        window.press("ctrl-t", cx);
        assert_eq!(
            spy.read(cx).bubbled,
            2,
            "the left drawer's chord passes the editor by"
        );
        window.press("ctrl-f", cx);
        assert!(bar_open(&view, cx), "Ctrl-F is find under vim");
        assert!(!replacing(&view, cx));
        window.input("foo", cx);
        // An open bar is a plain field: app chords win, Ctrl-F steps.
        let before = view.read(cx).editor.cursor();
        window.press("ctrl-f", cx);
        assert!(bar_open(&view, cx));
        assert_ne!(
            view.read(cx).editor.cursor(),
            before,
            "stepped to the next match"
        );
        assert_eq!(query(&view, cx), "foo");
        window.press("escape", cx);
        assert!(!bar_open(&view, cx));
        window.press("ctrl-shift-f", cx);
        assert!(bar_open(&view, cx), "the twin finds too");
        window.press("escape", cx);
        window.press("ctrl-h", cx);
        assert!(replacing(&view, cx));
        assert_eq!(text(&view, cx), all);
    })
    .unwrap();
}

/// The plain editor keeps M4's Ctrl-F / Ctrl-H, and the twins work there too.
#[gpui_kit::test]
fn the_plain_editor_keeps_ctrl_f_and_ctrl_h(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "foo");
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-shift-f", cx);
        assert!(bar_open(&view, cx));
        window.press("escape", cx);
        window.press("ctrl-shift-h", cx);
        assert!(replacing(&view, cx));
        window.press("escape", cx);
        window.press("ctrl-h", cx);
        assert!(replacing(&view, cx));
        assert_eq!(text(&view, cx), "foo");
    })
    .unwrap();
}

/// On macOS `primary` is Cmd: Cmd-F finds, Cmd-Option-F replaces, and
/// literal Ctrl-F is the editor's.
#[gpui_kit::test]
fn the_macos_table_finds_on_cmd_f(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "foo");
    cx.update(|cx| keymap::init(cx, Keymap::defaults(Platform::MacOs)));
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-f", cx);
        assert!(!bar_open(&view, cx));
        window.press("cmd-f", cx);
        assert!(bar_open(&view, cx));
        assert!(!replacing(&view, cx));
        window.press("escape", cx);
        window.press("cmd-alt-f", cx);
        assert!(replacing(&view, cx));
    })
    .unwrap();
}

/// A chord bound to an app command the editor does not own (focus, the
/// drawer) is not the editor's to act on: Alt+Left no longer moves by word
/// (ADR-0050 §4), Ctrl-T does not reach the editor.
#[gpui_kit::test]
fn app_chords_pass_the_editor_by(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "foo bar");
    cx.update_window(handle, |_, window, cx| {
        window.press("end", cx);
        let end = view.read(cx).editor.cursor();
        window.press("alt-left", cx);
        window.press("ctrl-tab", cx);
        window.press("ctrl-t", cx);
        assert_eq!(view.read(cx).editor.cursor(), end);
        assert_eq!(text(&view, cx), "foo bar");
        // Ctrl+Left is still word motion.
        window.press("ctrl-left", cx);
        assert_ne!(view.read(cx).editor.cursor(), end);
    })
    .unwrap();
}
