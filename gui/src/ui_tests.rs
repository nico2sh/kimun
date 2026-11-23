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
        crate::workspace::init_facet_keys(cx);
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
                let view = cx.new(|cx| EditorView::new(Some(text), vim, cx));
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
    open_spy_with(cx, text, true)
}

/// [`open_spy`] over the vim editor (`vim`) or the plain one.
fn open_spy_with(
    cx: &mut TestAppContext,
    text: &str,
    vim: bool,
) -> (AnyWindowHandle, Entity<KeySpy>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        crate::workspace::init_facet_keys(cx);
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
                let editor = cx.new(|cx| EditorView::new(Some(text), vim, cx));
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
        // H2 keeps the heading colour at normal weight (H1 is bold; the unit
        // tests in `theme.rs` cover it, as the caret reveals row 0 here).
        let title = piece(&h2, "Heading two").style;
        assert!(!title.bold);
        assert_eq!(title.fg, fg(t.heading_color(), &t));
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
        assert_eq!(bullet.text(), "• bullet item");
        assert_eq!(piece(&bullet, "• ").style.fg, fg(&t.gray, &t));
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

/// Regression lock (final-review ruling): in vim Insert mode Ctrl-W is the
/// editor's, never the tab close (it does not bubble), with or without a
/// pending sequence in Normal mode before; back in Normal mode with nothing
/// pending it bubbles (the tab close) again. The plain editor lets it
/// bubble.
#[gpui_kit::test]
fn ctrl_w_in_vim_insert_mode_stays_in_the_editor(cx: &mut TestAppContext) {
    let (handle, spy) = open_spy(cx, "one two");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = spy.read(cx).editor.clone();
        window.press("A", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("INSERT"));
        window.press("ctrl-w", cx);
        assert_eq!(spy.read(cx).bubbled, 0, "Insert: Ctrl-W is the editor's");
        assert_eq!(mode(&view, cx).as_deref(), Some("INSERT"));
        window.press("escape", cx);
        assert_eq!(mode(&view, cx).as_deref(), Some("NORMAL"));
        window.press("d", cx);
        window.press("ctrl-w", cx);
        assert_eq!(spy.read(cx).bubbled, 0, "Normal, `d` pending: vim's");
        window.press("escape", cx);
        window.press("ctrl-w", cx);
        assert_eq!(
            spy.read(cx).bubbled,
            1,
            "Normal, nothing pending: the close"
        );
    })
    .unwrap();

    let (handle, spy) = open_spy_with(cx, "one two", false);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-w", cx);
        assert_eq!(spy.read(cx).bubbled, 1, "plain editor: the close");
    })
    .unwrap();
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
        // Ctrl-Shift-T is bound to nothing (the new tab has no twin): the
        // editor keeps it, nothing bubbles.
        window.press("ctrl-shift-t", cx);
        assert_eq!(spy.read(cx).bubbled, 0, "Ctrl-Shift-T is no app chord");
        // Vim reads no Ctrl-T: the new tab's chord passes the editor by.
        window.press("ctrl-t", cx);
        assert_eq!(
            spy.read(cx).bubbled,
            1,
            "the new tab's chord passes the editor by"
        );
        // Vim's Ctrl-W window prefix: with an operator pending the engine
        // takes Ctrl-W (no tab closes); with nothing pending it bubbles.
        window.press("d", cx);
        window.press("ctrl-w", cx);
        assert_eq!(spy.read(cx).bubbled, 1, "vim holds `d`: Ctrl-W is vim's");
        window.press("escape", cx);
        window.press("ctrl-w", cx);
        assert_eq!(spy.read(cx).bubbled, 2, "nothing pending: the tab close");
        assert_eq!(text(&view, cx), all, "the `d` deleted nothing");
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
        window.press("ctrl-h", cx);
        assert!(replacing(&view, cx));
        assert_eq!(text(&view, cx), all);
    })
    .unwrap();
}

/// The plain editor keeps M4's Ctrl-F / Ctrl-H, and Ctrl-Shift-H works there too.
/// (Ctrl-Shift-F is the global search, so Find has no twin: vim reads no Ctrl-F.)
#[gpui_kit::test]
fn the_plain_editor_keeps_ctrl_f_and_ctrl_h(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "foo");
    cx.update_window(handle, |_, window, cx| {
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
/// slot, the tabs) is not the editor's to act on: Alt+Left no longer moves
/// by word (ADR-0050 §4), Ctrl-Tab and Ctrl-T do not reach the editor.
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

// The configured editor font (`[gui] editor_font_*`, the
// `EditorFontSetting` global).

use crate::editor_element::LINE_HEIGHT;
use crate::editor_font::EditorFontSetting;

fn set_font(cx: &mut gpui_kit::App, family: Option<&str>, size: Option<f32>) {
    cx.set_global(EditorFontSetting {
        family: family.map(|f| gpui_kit::SharedString::from(f.to_string())),
        size: size.map(px),
    });
}

#[gpui_kit::test]
fn the_edit_surface_uses_the_configured_size(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "some text\nmore text");
    cx.update_window(handle, |_, window, cx| {
        set_font(cx, None, Some(20.));
        window.refresh();
        window.render_frame(cx);
        assert_eq!(
            painted(&view, cx).cells.height(),
            (20. * LINE_HEIGHT).round()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_the_size_repaints_rows_at_the_new_width(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "one\ntwo\nthree");
    cx.update_window(handle, |_, window, cx| {
        set_font(cx, None, Some(13.));
        window.refresh();
        window.render_frame(cx);
        let small = painted(&view, cx).cells.width();
        let built = view.read(cx).row_cache.built();
        // An unchanged frame reuses every row.
        window.refresh();
        window.render_frame(cx);
        assert_eq!(view.read(cx).row_cache.built(), built);

        set_font(cx, None, Some(20.));
        window.refresh();
        window.render_frame(cx);
        let wide = painted(&view, cx).cells.width();
        assert!(wide > small, "{wide} > {small}");
        // Every painted row was shaped again: none kept the old width.
        let rebuilt = view.read(cx).row_cache.built() - built;
        assert!(rebuilt >= 3, "{rebuilt} rows rebuilt after the size change");
    })
    .unwrap();
}

/// Degraded: the headless text system lists no font at all, and adding the
/// embedded Inter faces does not change that (`ui_font::register` reports
/// `Missing`). So no second family can be painted here, and this test does
/// not show a preferred family being painted. It pins what can be seen: the
/// preferred family is handed to the face choice
/// (`editor_font::face_inputs`), and one that is not installed falls back to
/// the theme's face. That an installed preferred family wins is pinned by
/// `fonts::tests::a_preferred_family_*` on the pure choice rule.
#[gpui_kit::test]
fn a_preferred_family_that_is_not_installed_paints_the_theme_face(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "text");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.text_system().all_font_names().is_empty());
        window.render_frame(cx);
        let before = painted(&view, cx).fonts;
        set_font(cx, Some("Not Installed Mono"), None);
        let preferred = crate::editor_font::face_inputs(cx).preferred;
        assert_eq!(preferred.as_deref(), Some("Not Installed Mono"));
        window.refresh();
        window.render_frame(cx);
        assert_eq!(painted(&view, cx).fonts, before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_unset_global_changes_nothing(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, "text");
    cx.update_window(handle, |_, window, cx| {
        set_font(cx, None, None);
        window.refresh();
        window.render_frame(cx);
        let p = painted(&view, cx);
        let t = gpui_kit::component::ActiveTheme::theme(cx);
        let size = t.mono_font_size;
        let family = t.mono_font_family.clone();
        assert_eq!(p.cells.height(), (f32::from(size) * LINE_HEIGHT).round());
        assert_eq!(p.fonts.family, family);
    })
    .unwrap();
}

// --- read-only editor (facet view's chunks) ---

type Events = std::rc::Rc<std::cell::RefCell<Vec<editor_view::EditorEvent>>>;

/// A read-only editor over `text`, its app channel's receiver and the events
/// it emits.
fn open_read_only(
    cx: &mut TestAppContext,
    text: &str,
) -> (
    AnyWindowHandle,
    Entity<EditorView>,
    tokio::sync::mpsc::UnboundedReceiver<kimun_notes::logic::message::AppMessage>,
    Events,
    gpui_kit::Subscription,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        crate::workspace::init_facet_keys(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let text = text.to_string();
    let events = Events::default();
    let sink = events.clone();
    let (handle, view, sub) = cx.update(|cx| {
        let mut sub = None;
        let (handle, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(640.), px(480.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| EditorView::new_read_only_on(&text, tx, cx));
                window.focus(&view.read(cx).focus_handle(cx), cx);
                sub = Some(cx.subscribe(&view, move |_, event, _| {
                    sink.borrow_mut().push(event.clone());
                }));
                view
            },
        )
        .expect("open test window");
        (handle, view, sub.unwrap())
    });
    (handle, view, rx, events, sub)
}

#[gpui_kit::test]
fn read_only_editor_ignores_typing(cx: &mut TestAppContext) {
    let (handle, view, _rx, _events, _sub) = open_read_only(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("xyz", cx);
        window.press("backspace", cx);
        window.press("enter", cx);
        assert_eq!(text(&view, cx), "abc");
    })
    .unwrap();
}

/// No message on the editor's channel asks to follow.
fn assert_no_follow(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<kimun_notes::logic::message::AppMessage>,
) {
    while let Ok(message) = rx.try_recv() {
        assert!(
            !matches!(
                message,
                kimun_notes::logic::message::AppMessage::FollowLabel(_)
                    | kimun_notes::logic::message::AppMessage::FollowLink { .. }
            ),
            "a read-only press must not follow on the channel"
        );
    }
}

/// A plain press off any target is a text press: it places the caret and
/// tells no one.
#[gpui_kit::test]
fn a_plain_read_only_press_emits_no_event(cx: &mut TestAppContext) {
    let (handle, view, mut rx, events, _sub) = open_read_only(cx, "see #idea here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 1, 0, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 1), "the caret is placed");
    })
    .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    assert_no_follow(&mut rx);
}

/// A plain click on a tag asks the owner to follow it at the release,
/// through the event (never the channel).
#[gpui_kit::test]
fn a_plain_read_only_click_on_a_tag_emits_a_follow(cx: &mut TestAppContext) {
    let (handle, view, mut rx, events, _sub) = open_read_only(cx, "see #idea here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 6, 0, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        assert!(events.borrow().is_empty(), "not at the press");
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 6), "the caret is placed");
    })
    .unwrap();
    assert_eq!(
        *events.borrow(),
        vec![editor_view::EditorEvent::ReadOnlyFollow {
            target: kimun_notes::logic::editor::claim::FollowTarget::Label("idea".to_string()),
        }]
    );
    assert_no_follow(&mut rx);
}

/// An editor that edits keeps its press: a single click on a tag places the
/// caret and follows nothing, on the channel or as an event.
#[gpui_kit::test]
fn a_normal_editor_click_on_a_tag_only_places_the_caret(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        crate::workspace::init_facet_keys(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let events = Events::default();
    let sink = events.clone();
    let (handle, view, _sub) = cx.update(|cx| {
        let mut sub = None;
        let (handle, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(640.), px(480.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| {
                    let mut view = EditorView::on_channel(false, tx, cx);
                    view.editor.set_text("see #idea here".to_string());
                    view
                });
                window.focus(&view.read(cx).focus_handle(cx), cx);
                sub = Some(cx.subscribe(&view, move |_, event, _| {
                    sink.borrow_mut().push(event.clone());
                }));
                view
            },
        )
        .expect("open test window");
        (handle, view, sub.unwrap())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 6, 0, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 6), "the caret is placed");
    })
    .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    assert_no_follow(&mut rx);
}

/// A double click on a tag follows once, on the first click. The second
/// press is a plain press: no word selected, no second follow.
#[gpui_kit::test]
fn a_read_only_double_click_on_a_tag_follows_once(cx: &mut TestAppContext) {
    let (handle, view, mut rx, events, _sub) = open_read_only(cx, "see #idea here");
    cx.update(|cx| view.update(cx, |view, _| view.set_clock(editor_view::frozen_clock)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 6, 0, cx);
        for _ in 0..2 {
            window.pointer_down(at, MouseButton::Left, cx);
            window.pointer_up(at, MouseButton::Left, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(events.borrow().len(), 1, "{:?}", events.borrow());
    assert_no_follow(&mut rx);
    let selected = cx.update(|cx| view.read(cx).editor.buffer().selection_text());
    assert_eq!(selected, None);
}

/// New text between the press and the release (the owner re-plans the
/// piece): the release follows nothing.
#[gpui_kit::test]
fn new_text_between_press_and_release_follows_nothing(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_read_only(cx, "see #idea here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 6, 0, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        view.update(cx, |view, cx| view.set_text_read_only("saw #idea here", cx));
        window.render_frame(cx);
        window.pointer_move(at, Some(MouseButton::Left), cx);
        window.pointer_up(at, MouseButton::Left, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
}

/// A wobble of a few pixels between press and release on a tag is still a
/// click: it follows.
#[gpui_kit::test]
fn a_small_wobble_on_a_tag_still_follows(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_read_only(cx, "see #longideatag here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 8, 0, cx);
        let wobble = at + gpui_kit::point(px(2.), px(0.));
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_move(wobble, Some(MouseButton::Left), cx);
        window.pointer_up(wobble, MouseButton::Left, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(events.borrow().len(), 1, "{:?}", events.borrow());
}

/// A drag of more than a few pixels that stays inside one tag selects the
/// text it covers and follows nothing: a drag across part of a long title
/// copies it.
#[gpui_kit::test]
fn a_drag_inside_a_tag_selects_and_does_not_follow(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_read_only(cx, "see #longideatag here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 6, 0, cx);
        let to = at + gpui_kit::point(px(10.), px(0.));
        let end = cell_center(&view, 14, 0, cx);
        assert!(to.x < end.x, "the drag stays inside the tag");
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
        window.pointer_up(to, MouseButton::Left, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    let selected = cx.update(|cx| view.read(cx).editor.buffer().selection_text());
    assert!(selected.is_some(), "the drag selected text");
}

/// A primary press on a tag asks the owner to follow it, through the event
/// (never the channel).
#[gpui_kit::test]
fn a_primary_read_only_press_emits_a_primary_event(cx: &mut TestAppContext) {
    let (handle, view, mut rx, events, _sub) = open_read_only(cx, "see #idea here\nplain");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.change_modifiers(gpui_kit::Modifiers::control(), cx);
        let at = cell_center(&view, 6, 0, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        // Off any follow target, a primary press emits nothing.
        let plain = cell_center(&view, 2, 1, cx);
        window.pointer_down(plain, MouseButton::Left, cx);
        window.pointer_up(plain, MouseButton::Left, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![editor_view::EditorEvent::ReadOnlyFollow {
            target: kimun_notes::logic::editor::claim::FollowTarget::Label("idea".to_string()),
        }]
    );
    assert_no_follow(&mut rx);
}

/// With the macOS table the primary press is Cmd+click, whatever OS runs the
/// test: a Ctrl+click on a tag follows nothing there.
#[gpui_kit::test]
fn the_macos_table_follows_on_cmd_click(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_read_only(cx, "see #idea here");
    cx.update(|cx| keymap::init(cx, Keymap::defaults(Platform::MacOs)));
    let press = |modifiers: gpui_kit::Modifiers, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.change_modifiers(modifiers, cx);
            let at = cell_center(&view, 6, 0, cx);
            window.pointer_down(at, MouseButton::Left, cx);
            window.pointer_up(at, MouseButton::Left, cx);
            window.change_modifiers(gpui_kit::Modifiers::none(), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    press(gpui_kit::Modifiers::control(), cx);
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    press(gpui_kit::Modifiers::command(), cx);
    assert_eq!(
        *events.borrow(),
        vec![editor_view::EditorEvent::ReadOnlyFollow {
            target: kimun_notes::logic::editor::claim::FollowTarget::Label("idea".to_string()),
        }]
    );
}

/// A read-only editor draws its follow targets with the pointing hand with
/// no modifier held, and plain text with the text cursor.
#[gpui_kit::test]
fn a_read_only_target_shows_the_hand_without_a_modifier(cx: &mut TestAppContext) {
    let (handle, view, _rx, _events, _sub) = open_read_only(cx, "see #idea here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let tag = cell_center(&view, 6, 0, cx);
        let text = cell_center(&view, 1, 0, cx);
        assert!(painted(&view, cx).hand_at(tag), "the hand over the tag");
        assert!(!painted(&view, cx).hand_at(text), "the I-beam over text");
    })
    .unwrap();
}

// --- right-click on a link: the Link menu ---

/// An editor that edits over `text`, on a channel the test holds, and the
/// events it emits.
fn open_editing(
    cx: &mut TestAppContext,
    text: &str,
) -> (
    AnyWindowHandle,
    Entity<EditorView>,
    tokio::sync::mpsc::UnboundedReceiver<kimun_notes::logic::message::AppMessage>,
    Events,
    gpui_kit::Subscription,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        crate::workspace::init_facet_keys(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let text = text.to_string();
    let events = Events::default();
    let sink = events.clone();
    let (handle, view, sub) = cx.update(|cx| {
        let mut sub = None;
        let (handle, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(640.), px(480.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| {
                    let mut view = EditorView::on_channel(false, tx, cx);
                    view.editor.set_text(text);
                    view
                });
                window.focus(&view.read(cx).focus_handle(cx), cx);
                sub = Some(cx.subscribe(&view, move |_, event, _| {
                    sink.borrow_mut().push(event.clone());
                }));
                view
            },
        )
        .expect("open test window");
        (handle, view, sub.unwrap())
    });
    (handle, view, rx, events, sub)
}

/// A right press and its release at `at`.
fn right_click(window: &mut gpui_kit::Window, at: Point<Pixels>, cx: &mut gpui_kit::App) {
    window.pointer_down(at, MouseButton::Right, cx);
    window.pointer_up(at, MouseButton::Right, cx);
}

/// The cell column where `needle` starts on painted row `row`.
fn col_of(view: &Entity<EditorView>, row: usize, needle: &str, cx: &gpui_kit::App) -> u16 {
    let text = painted_row(view, row, cx).text();
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("no {needle:?} on row {row}: {text:?}"));
    text[..at].chars().count() as u16
}

fn wikilink(target: &str) -> kimun_notes::logic::editor::claim::FollowTarget {
    kimun_notes::logic::editor::claim::FollowTarget::Link {
        target: target.to_string(),
        kind: kimun_core::note::scan::LinkSpanKind::WikiLink,
    }
}

/// A right press on a link, with no selection, asks for the Link menu at
/// the pointer, with the link's target. No note menu.
#[gpui_kit::test]
fn a_right_click_on_a_link_emits_link_menu_with_its_target(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_editing(cx, "see [[note]] here\nplain");
    let at = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let at = cell_center(&view, 7, 0, cx);
            right_click(window, at, cx);
            at
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![editor_view::EditorEvent::LinkMenu {
            at,
            target: wikilink("note"),
        }]
    );
}

/// A right press on a label asks for the Link menu with the label.
#[gpui_kit::test]
fn a_right_click_on_a_label_emits_link_menu_with_the_label(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_editing(cx, "see #idea here");
    let at = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let at = cell_center(&view, 6, 0, cx);
            right_click(window, at, cx);
            at
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![editor_view::EditorEvent::LinkMenu {
            at,
            target: kimun_notes::logic::editor::claim::FollowTarget::Label("idea".to_string()),
        }]
    );
}

/// With the caret off the link its brackets are hidden, so the link's
/// first and last drawn cells are `n` and `e` of `note`: both are on the
/// link, and the cells just outside it are not.
#[gpui_kit::test]
fn a_right_click_on_the_first_and_last_cell_of_a_link_emits_link_menu(cx: &mut TestAppContext) {
    // The link draws as `note`, its brackets hidden off the caret.
    let (handle, view, _rx, events, _sub) = open_editing(cx, "see [[note]] here");
    let first = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let first = col_of(&view, 0, "note", cx);
            for col in [first, first + 3] {
                let at = cell_center(&view, col, 0, cx);
                right_click(window, at, cx);
            }
            first
        })
        .unwrap();
    cx.run_until_parked();
    let menus: Vec<_> = events
        .borrow()
        .iter()
        .map(|e| match e {
            editor_view::EditorEvent::LinkMenu { target, .. } => Some(target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(menus, vec![Some(wikilink("note")), Some(wikilink("note"))]);
    events.borrow_mut().clear();
    cx.update_window(handle, |_, window, cx| {
        for col in [first - 1, first + 4] {
            let at = cell_center(&view, col, 0, cx);
            right_click(window, at, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        events
            .borrow()
            .iter()
            .all(|e| matches!(e, editor_view::EditorEvent::ContextMenu { .. })),
        "next to the link is plain text: {:?}",
        events.borrow()
    );
    assert_eq!(events.borrow().len(), 2);
}

/// With the caret inside the link its brackets show: a right press on the
/// first `[` and on the last `]` still finds the link.
#[gpui_kit::test]
fn a_right_click_on_the_revealed_brackets_of_a_link_emits_link_menu(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_editing(cx, "see [[note]] here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // One plain press puts the caret in the link (a second would follow).
        let inside = cell_center(&view, col_of(&view, 0, "note", cx) + 1, 0, cx);
        window.pointer_down(inside, MouseButton::Left, cx);
        window.pointer_up(inside, MouseButton::Left, cx);
        window.render_frame(cx);
        assert_eq!(
            painted_row(&view, 0, cx).text().trim_end(),
            "see [[note]] here",
            "the brackets show"
        );
        let first = col_of(&view, 0, "[[", cx);
        let last = col_of(&view, 0, "]]", cx) + 1;
        for col in [first, last] {
            let at = cell_center(&view, col, 0, cx);
            right_click(window, at, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    let menus: Vec<_> = events
        .borrow()
        .iter()
        .map(|e| match e {
            editor_view::EditorEvent::LinkMenu { target, .. } => Some(target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(menus, vec![Some(wikilink("note")), Some(wikilink("note"))]);
}

/// A link that wraps onto a second row: a right press on that second row
/// still finds it.
#[gpui_kit::test]
fn a_right_click_on_a_wrapped_second_row_of_a_link_emits_link_menu(cx: &mut TestAppContext) {
    let title = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda omega";
    let line = format!("{}[[{title}]]", "word ".repeat(10));
    let (handle, view, _rx, events, _sub) = open_editing(cx, &line);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = painted_row(&view, 0, cx).text();
        let second = painted_row(&view, 1, cx).text();
        assert!(
            first.contains("alpha"),
            "the link starts on row 0: {first:?} / {second:?}"
        );
        let col = second
            .find("omega")
            .unwrap_or_else(|| panic!("the link wraps onto row 1: {second:?}"));
        let at = cell_center(&view, col as u16, 1, cx);
        right_click(window, at, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(
            events.borrow().as_slice(),
            [editor_view::EditorEvent::LinkMenu { target, .. }] if *target == wikilink(title)
        ),
        "{:?}",
        events.borrow()
    );
}

/// A right press on plain text, or below the text under a link's columns,
/// asks for the note menu, as before.
#[gpui_kit::test]
fn a_right_click_on_plain_text_still_requests_the_note_menu(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_editing(cx, "see [[note]] here");
    let (text_at, below_at) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let text_at = cell_center(&view, 1, 0, cx);
            right_click(window, text_at, cx);
            let below_at = cell_center(&view, 7, 3, cx);
            right_click(window, below_at, cx);
            (text_at, below_at)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![
            editor_view::EditorEvent::ContextMenu { at: text_at },
            editor_view::EditorEvent::ContextMenu { at: below_at },
        ]
    );
}

/// With a selection, a right press on a link copies the selection and asks
/// for no menu, as before.
#[gpui_kit::test]
fn a_right_click_on_a_link_with_a_selection_copies_and_emits_no_menu(cx: &mut TestAppContext) {
    let (handle, view, mut rx, events, _sub) = open_editing(cx, "see [[note]] here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let from = cell_center(&view, 0, 0, cx);
        let to = cell_center(&view, 3, 0, cx);
        window.pointer_down(from, MouseButton::Left, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
        window.pointer_up(to, MouseButton::Left, cx);
        assert_eq!(
            view.read(cx).editor.buffer().selection_text().as_deref(),
            Some("see")
        );
        let at = cell_center(&view, 7, 0, cx);
        right_click(window, at, cx);
        assert_eq!(
            view.read(cx).editor.buffer().selection_text().as_deref(),
            Some("see"),
            "the selection stays"
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    let mut copied = false;
    while let Ok(message) = rx.try_recv() {
        // The copy reports on the channel, whether or not the host has a
        // clipboard to write to.
        copied |= matches!(
            message,
            kimun_notes::logic::message::AppMessage::FlashMessage(_)
        );
    }
    assert!(copied, "the selection was copied");
}

/// With the find bar open, a press never acts on a link: a right press on
/// one asks for the note menu, as before.
#[gpui_kit::test]
fn a_right_click_on_a_link_with_the_find_bar_open_still_requests_the_note_menu(
    cx: &mut TestAppContext,
) {
    let (handle, view, _rx, events, _sub) = open_editing(cx, "see [[note]] here");
    let at = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-f", cx);
            window.render_frame(cx);
            assert!(view.read(cx).editor.find_open(), "the find bar is open");
            let at = cell_center(&view, col_of(&view, 0, "note", cx) + 1, 0, cx);
            right_click(window, at, cx);
            at
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![editor_view::EditorEvent::ContextMenu { at }]
    );
}

/// A right press that finds no link moves neither the caret nor the
/// selection.
#[gpui_kit::test]
fn a_right_click_that_finds_no_link_leaves_the_caret_where_it_was(cx: &mut TestAppContext) {
    let (handle, view, _rx, events, _sub) = open_editing(cx, "see [[note]] here\nplain words");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let caret_at = cell_center(&view, 2, 0, cx);
        window.pointer_down(caret_at, MouseButton::Left, cx);
        window.pointer_up(caret_at, MouseButton::Left, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 2));
        let at = cell_center(&view, 5, 1, cx);
        right_click(window, at, cx);
        assert_eq!(view.read(cx).editor.cursor(), (0, 2), "the caret stays");
        assert_eq!(view.read(cx).editor.selection(), None, "no selection");
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(
            events.borrow().as_slice(),
            [editor_view::EditorEvent::ContextMenu { .. }]
        ),
        "{:?}",
        events.borrow()
    );
}

/// In read-only text, a right press on a follow target asks for the Link
/// menu. It never follows on the channel.
#[gpui_kit::test]
fn a_read_only_right_click_on_a_link_emits_link_menu(cx: &mut TestAppContext) {
    let (handle, view, mut rx, events, _sub) = open_read_only(cx, "see [[note]] and #idea");
    let (link_at, tag_at) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let link_at = cell_center(&view, 7, 0, cx);
            right_click(window, link_at, cx);
            window.render_frame(cx);
            let tag_at = cell_center(&view, col_of(&view, 0, "#idea", cx) + 1, 0, cx);
            right_click(window, tag_at, cx);
            (link_at, tag_at)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![
            editor_view::EditorEvent::LinkMenu {
                at: link_at,
                target: wikilink("note"),
            },
            editor_view::EditorEvent::LinkMenu {
                at: tag_at,
                target: kimun_notes::logic::editor::claim::FollowTarget::Label("idea".to_string()),
            },
        ]
    );
    assert_no_follow(&mut rx);
}

/// In read-only text, a right press off any follow target does nothing.
#[gpui_kit::test]
fn a_read_only_right_click_on_plain_text_emits_nothing(cx: &mut TestAppContext) {
    let (handle, view, mut rx, events, _sub) = open_read_only(cx, "see #idea here");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 1, 0, cx);
        right_click(window, at, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    assert_no_follow(&mut rx);
}

#[gpui_kit::test]
fn read_only_editor_ignores_the_find_chord(cx: &mut TestAppContext) {
    let (handle, view, _rx, _events, _sub) = open_read_only(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-f", cx);
        assert!(!view.read(cx).editor.find_open());
    })
    .unwrap();
}

#[gpui_kit::test]
fn read_only_editor_ignores_the_replace_chord(cx: &mut TestAppContext) {
    let (handle, view, _rx, _events, _sub) = open_read_only(cx, "abc");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-h", cx);
        assert!(!view.read(cx).editor.find_open());
    })
    .unwrap();
}

// ---- Frontmatter block ---------------------------------------------------------

const FRONTMATTER_NOTE: &str = "---\ntags: [a]\ndue: x\n---\nBody";

#[gpui_kit::test]
fn a_frontmatter_block_paints_folded_to_one_ruled_row(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, FRONTMATTER_NOTE);
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let summary = painted_row(&view, 0, cx);
        assert!(!summary.rules.is_empty(), "the rule is a stroke");
        let title = piece(&summary, "properties").style;
        assert_eq!(title.fg, fg(&t.fg_secondary, &t));
        assert!(summary.text().contains("tags, due"), "{:?}", summary.text());
        assert_eq!(painted_row(&view, 1, cx).text(), "Body");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_click_on_the_summary_pins_the_block_open_on_a_box(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, FRONTMATTER_NOTE);
    let t = Theme::default();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let at = cell_center(&view, 4, 0, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        assert_eq!(view.read(cx).editor.cursor(), (1, 0));
        window.render_frame(cx);
        let tags = painted_row(&view, 1, cx);
        assert_eq!(tags.text(), "tags: [a]");
        let box_bg = theme::to_gpui(&t.code_bg, Token::Background, &t).into();
        assert_eq!(tags.fill_at(0), box_bg, "on the same box a code block uses");
        assert_eq!(painted_row(&view, 2, cx).text(), "due: x");
        assert_eq!(painted_row(&view, 4, cx).text(), "Body");
    })
    .unwrap();
}

#[gpui_kit::test]
fn down_from_the_first_line_enters_the_block_and_opens_it(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, FRONTMATTER_NOTE);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(painted_row(&view, 1, cx).text(), "Body", "folded");
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).editor.cursor().0, 1);
        assert_eq!(painted_row(&view, 1, cx).text(), "tags: [a]", "opened");
    })
    .unwrap();
}

#[gpui_kit::test]
fn up_from_the_body_enters_the_block_at_its_closing_fence(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, FRONTMATTER_NOTE);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let body = cell_center(&view, 1, 1, cx);
        window.pointer_down(body, MouseButton::Left, cx);
        window.pointer_up(body, MouseButton::Left, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).editor.cursor().0, 4, "on Body, block folded");
        window.press("up", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).editor.cursor().0, 3, "the closing fence");
        assert_eq!(painted_row(&view, 1, cx).text(), "tags: [a]", "opened");
        // Leaving the block folds it again.
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).editor.cursor().0, 4);
        assert_eq!(painted_row(&view, 1, cx).text(), "Body", "folded again");
    })
    .unwrap();
}

#[gpui_kit::test]
fn typing_in_an_open_block_keeps_the_caret_on_its_row(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, FRONTMATTER_NOTE);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("end", cx);
        window.input("z", cx);
        window.render_frame(cx);
        assert_eq!(text(&view, cx), "---\ntags: [a]z\ndue: x\n---\nBody");
        assert_eq!(painted_row(&view, 1, cx).text(), "tags: [a]z");
        let caret = painted(&view, cx).caret.expect("the caret is drawn");
        let p = painted(&view, cx);
        assert_eq!(p.cells.row_at(f32::from(caret.origin.y - p.origin.y)), 1);
    })
    .unwrap();
}

// ---- Inline images: the room the display reserves (CONTEXT.md, "Inline
// image"). Painting them is another step's.

mod inline_images {
    use super::*;
    use crate::inline_image::{fit, rows_for};
    use crate::test_vault::{TestVault, test_vault};
    use kimun_core::nfs::VaultPath;

    /// A PNG of `w`x`h` pixels.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// An editor showing `text` as the note `n.md` of a vault holding the
    /// attachments `files` (path, bytes), drawing its images. The runtime and
    /// the vault must outlive the test body.
    fn open_note(
        cx: &mut TestAppContext,
        text: &str,
        files: &[(&str, Vec<u8>)],
    ) -> (
        AnyWindowHandle,
        Entity<EditorView>,
        TestVault,
        tokio::runtime::Runtime,
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let vault = rt.block_on(async {
            let v = test_vault(&[("n.md", text)], &[]).await;
            for (path, bytes) in files {
                v.vault
                    .save_attachment(&VaultPath::new(*path), bytes)
                    .await
                    .expect("the attachment is saved");
            }
            v
        });
        let (handle, view) = open(cx, "");
        let (note, shared) = (text.to_string(), vault.vault.clone());
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.editor.open_note(VaultPath::new("n.md"), note);
                view.set_image_vault(Some(shared), cx);
            })
        });
        settle(cx, handle);
        (handle, view, vault, rt)
    }

    /// Draw until the attachment listing and every decode have landed.
    fn settle(cx: &mut TestAppContext, handle: AnyWindowHandle) {
        for _ in 0..3 {
            cx.update_window(handle, |_, window, cx| {
                window.refresh();
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
    }

    fn pads_under(view: &Entity<EditorView>, row: usize, cx: &gpui_kit::App) -> usize {
        view.read(cx)
            .editor
            .session()
            .display
            .layout
            .visual_lines()
            .iter()
            .filter(|l| l.pad && l.logical_row == row)
            .count()
    }

    /// The lines an image of `natural` size takes in this window, when
    /// neither the column nor the pane caps it (a narrow picture in a tall
    /// pane).
    fn image_rows(
        view: &Entity<EditorView>,
        natural: (u32, u32),
        window: &gpui_kit::Window,
        cx: &gpui_kit::App,
    ) -> usize {
        let scale = window.scale_factor();
        let (_, height) = fit(natural, 10_000.0, 10_000.0);
        rows_for(height, painted(view, cx).cells.height() * scale)
    }

    /// The screen rows painting `text`.
    fn rows_painting(view: &Entity<EditorView>, text: &str, cx: &gpui_kit::App) -> Vec<usize> {
        painted(view, cx)
            .row_paint
            .iter()
            .enumerate()
            .filter(|(_, row)| row.text() == text)
            .map(|(r, _)| r)
            .collect()
    }

    const NOTE: &str = "intro\n![a](pic.png)\nafter";

    #[gpui_kit::test]
    fn an_embed_line_off_the_cursor_reserves_rows_minus_one(cx: &mut TestAppContext) {
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 60))]);
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(view.read(cx).editor.cursor().0, 0);
            let rows = image_rows(&view, (10, 60), window, cx);
            assert!(rows >= 2, "a picture taller than a line: {rows}");
            assert_eq!(pads_under(&view, 1, cx), rows - 1);
            // The embed line draws nothing: the image takes it and its pads.
            assert_eq!(painted_row(&view, 1, cx).text(), "");
            assert_eq!(rows_painting(&view, "after", cx), [1 + rows]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn an_embed_line_on_the_cursor_reserves_all_rows(cx: &mut TestAppContext) {
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 60))]);
        cx.update_window(handle, |_, window, cx| {
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.cursor().0, 1);
            let rows = image_rows(&view, (10, 60), window, cx);
            assert_eq!(pads_under(&view, 1, cx), rows);
            // The raw markdown shows, the image below it.
            assert_eq!(painted_row(&view, 1, cx).text(), "![a](pic.png)");
            assert_eq!(rows_painting(&view, "after", cx), [2 + rows]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn moving_the_cursor_onto_an_embed_keeps_the_same_line_in_view(cx: &mut TestAppContext) {
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 60))]);
        cx.update_window(handle, |_, window, cx| {
            let rows = image_rows(&view, (10, 60), window, cx);
            assert_eq!(pads_under(&view, 1, cx), rows - 1, "off the cursor");
            // From above: the lines above the image stay put.
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(pads_under(&view, 1, cx), rows, "one line more on it");
            assert_eq!(rows_painting(&view, "intro", cx), [0]);
            assert_eq!(rows_painting(&view, "![a](pic.png)", cx), [1]);
            // From below: the embed line comes back where it was drawn.
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.cursor().0, 2);
            window.press("up", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.cursor().0, 1);
            assert_eq!(rows_painting(&view, "intro", cx), [0]);
            assert_eq!(rows_painting(&view, "![a](pic.png)", cx), [1]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn moving_onto_an_embed_in_a_scrolled_view_keeps_the_top_line(cx: &mut TestAppContext) {
        let mut lines: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
        lines.insert(50, "![a](pic.png)".to_string());
        let note = lines.join("\n");
        let (handle, view, _vault, _rt) = open_note(cx, &note, &[("pic.png", png(10, 60))]);
        cx.update_window(handle, |_, window, cx| {
            let rows = image_rows(&view, (10, 60), window, cx);
            window.press("ctrl-end", cx);
            window.render_frame(cx);
            // Up to the line below the embed, with the embed in view.
            while view.read(cx).editor.cursor().0 > 51 {
                window.press("up", cx);
            }
            window.render_frame(cx);
            let top = painted_row(&view, 0, cx).text();
            assert_ne!(top, "line 0", "the view is scrolled");
            let above = rows_painting(&view, "line 49", cx);
            assert_eq!(above.len(), 1, "the line above the embed is in view");
            assert_eq!(pads_under(&view, 50, cx), rows - 1, "off the cursor");
            window.press("up", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.cursor().0, 50);
            assert_eq!(pads_under(&view, 50, cx), rows, "on the cursor");
            assert_eq!(painted_row(&view, 0, cx).text(), top, "the same top line");
            assert_eq!(rows_painting(&view, "line 49", cx), above);
            assert_eq!(rows_painting(&view, "![a](pic.png)", cx), [above[0] + 1]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_failed_image_reserves_no_pad_lines(cx: &mut TestAppContext) {
        let note = "intro\n![x](missing.png)\n![y](text.png)\nafter";
        let files = [("text.png", b"not an image".to_vec())];
        let (handle, view, _vault, _rt) = open_note(cx, note, &files);
        cx.update_window(handle, |_, window, cx| {
            for row in [1, 2] {
                assert_eq!(pads_under(&view, row, cx), 0, "off the cursor, row {row}");
                // Off the cursor its note takes the line: no text is drawn.
                assert_eq!(painted_row(&view, row, cx).text(), "");
            }
            assert_eq!(rows_painting(&view, "after", cx), [3]);
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(pads_under(&view, 1, cx), 0, "on the cursor");
            assert_eq!(painted_row(&view, 1, cx).text(), "![x](missing.png)");
            assert_eq!(rows_painting(&view, "after", cx), [3]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn resizing_the_pane_recomputes_the_reserve(cx: &mut TestAppContext) {
        // Taller than 60% of a short pane, shorter than 60% of a tall one.
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 400))]);
        let tall = cx
            .update_window(handle, |_, window, cx| {
                let rows = image_rows(&view, (10, 400), window, cx);
                assert_eq!(pads_under(&view, 1, cx), rows - 1, "natural size");
                rows
            })
            .unwrap();
        cx.simulate_window_resize(handle, size(px(640.), px(160.)));
        settle(cx, handle);
        cx.update_window(handle, |_, window, cx| {
            let p = painted(&view, cx);
            let line = p.cells.height() * window.scale_factor();
            // Capped at 60% of a pane under 160 px tall.
            let cap = rows_for(160.0 * window.scale_factor() * 0.6, line);
            let short = pads_under(&view, 1, cx) + 1;
            assert!(short < tall, "{short} rows, was {tall}");
            assert!(short <= cap, "{short} rows, cap {cap}");
        })
        .unwrap();
    }

    /// Two images on screen whose decoded pixels do not fit the cache
    /// together: an image on screen is never evicted, so both are drawn,
    /// each decoded once.
    #[gpui_kit::test]
    fn images_over_the_cache_budget_are_decoded_once(cx: &mut TestAppContext) {
        use crate::inline_image::ImageStore;
        // One 10x60 image decodes to 2400 bytes: one fits, two do not.
        let store = ImageStore::with_test_budget(3000);
        cx.update(|cx| cx.set_global(store.clone()));
        let note = "intro\n![a](a.png)\n![b](b.png)\nafter";
        let files = [("a.png", png(10, 60)), ("b.png", png(10, 60))];
        let (handle, view, _vault, _rt) = open_note(cx, note, &files);
        assert_eq!(store.decodes_started(), 2, "one decode per image");
        cx.update(|cx| {
            assert!(image_of(&view, 1, cx).is_some_and(|i| i.ready), "a drawn");
            assert!(image_of(&view, 2, cx).is_some_and(|i| i.ready), "b drawn");
        });
        // More frames, cursor moves and an edit start nothing new.
        settle(cx, handle);
        cx.update_window(handle, |_, window, cx| {
            window.press("down", cx);
            window.press("down", cx);
            window.render_frame(cx);
        })
        .unwrap();
        settle(cx, handle);
        cx.update_window(handle, |_, window, cx| {
            window.press("ctrl-end", cx);
            window.input("x", cx);
        })
        .unwrap();
        settle(cx, handle);
        assert_eq!(
            store.decodes_started(),
            2,
            "an eviction decodes nothing again"
        );
        cx.update(|cx| {
            assert!(pads_under(&view, 1, cx) > 0, "the images keep their room");
            assert!(pads_under(&view, 2, cx) > 0);
            assert!(image_of(&view, 1, cx).is_some_and(|i| i.ready), "a drawn");
            assert!(image_of(&view, 2, cx).is_some_and(|i| i.ready), "b drawn");
        });
    }

    /// The painted image of logical row `row`, when one is painted.
    fn image_of(
        view: &Entity<EditorView>,
        row: usize,
        cx: &gpui_kit::App,
    ) -> Option<crate::editor_view::PaintedImage> {
        painted(view, cx).images.into_iter().find(|i| i.row == row)
    }

    /// Window-space top of screen row `r`.
    fn row_top(p: &Painted, r: u16) -> Pixels {
        p.origin.y + px(p.cells.y_of(r))
    }

    #[gpui_kit::test]
    fn a_ready_image_paints_over_its_reserved_rows(cx: &mut TestAppContext) {
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 60))]);
        cx.update_window(handle, |_, window, cx| {
            let scale = window.scale_factor();
            let rows = image_rows(&view, (10, 60), window, cx);
            let p = painted(&view, cx);
            let lh = px(p.cells.height());
            // Off the cursor: over the embed row and its pads.
            let image = image_of(&view, 1, cx).expect("the image is painted");
            assert!(image.ready, "its pixels, not the empty box");
            assert_eq!(image.rect.origin, point(p.origin.x, row_top(&p, 1)));
            // One image pixel to one device pixel.
            assert_eq!(image.rect.size, size(px(10.0 / scale), px(60.0 / scale)));
            assert_eq!(image.clip.origin.y, row_top(&p, 1));
            assert_eq!(image.clip.size.height, lh * rows as f32);
            assert!(image.clip.contains(&image.rect.origin));
            // On the cursor: under the raw markdown, over the pads.
            window.press("down", cx);
            window.render_frame(cx);
            let p = painted(&view, cx);
            let image = image_of(&view, 1, cx).expect("the image is painted");
            assert_eq!(image.rect.origin.y, row_top(&p, 2));
            assert_eq!(image.clip.origin.y, row_top(&p, 2));
            assert_eq!(image.clip.size.height, lh * rows as f32);
            assert_eq!(rows_painting(&view, "![a](pic.png)", cx), [1]);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_pending_image_paints_an_empty_box_of_the_reserved_size(cx: &mut TestAppContext) {
        use crate::inline_image::ImageStore;
        // The decode never lands: the image stays at its header size.
        let store = ImageStore::default();
        store.hold_decodes();
        cx.update(|cx| cx.set_global(store.clone()));
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 60))]);
        assert_eq!(store.decodes_started(), 1);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let scale = window.scale_factor();
            let pending = image_of(&view, 1, cx).unwrap();
            assert!(!pending.ready, "an empty box");
            assert_eq!(pending.rect.size, size(px(10.0 / scale), px(60.0 / scale)));
            let rows = image_rows(&view, (10, 60), window, cx);
            assert_eq!(
                pads_under(&view, pending.row, cx),
                rows - 1,
                "its room kept"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_failed_image_paints_the_note_only_off_the_cursor_row(cx: &mut TestAppContext) {
        let note = "intro\n![x](missing.png)\n![[text.png]]\nafter";
        let files = [("text.png", b"not an image".to_vec())];
        let (handle, view, _vault, _rt) = open_note(cx, note, &files);
        cx.update_window(handle, |_, window, cx| {
            let notes = painted(&view, cx).image_notes;
            assert_eq!(
                notes,
                [
                    (1, "image not found: missing.png (x)".to_string()),
                    (2, "cannot draw image: text.png".to_string()),
                ]
            );
            assert!(painted(&view, cx).images.is_empty(), "no box for a failure");
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(
                painted(&view, cx).image_notes,
                [(2, "cannot draw image: text.png".to_string())],
                "the cursor row shows its markdown, no note"
            );
            assert_eq!(painted_row(&view, 1, cx).text(), "![x](missing.png)");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn hovering_an_image_shows_the_alt_text_and_a_wikilink_shows_the_file_name(
        cx: &mut TestAppContext,
    ) {
        use crate::editor_element::image_hover_id;
        let note = "intro\n![a red square](a.png)\n![[b.png]]\n![](c.png)\nafter";
        let files = [
            ("a.png", png(10, 20)),
            ("b.png", png(10, 20)),
            ("c.png", png(10, 20)),
        ];
        let (handle, view, _vault, _rt) = open_note(cx, note, &files);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for (row, tip) in [(1, "a red square"), (2, "b.png"), (3, "c.png")] {
                let image = image_of(&view, row, cx).expect("painted");
                assert_eq!(image.tip.as_ref(), tip);
                let target = window.find(image_hover_id(row));
                assert_eq!(target.label(), Some(tip), "row {row}");
                assert_eq!(target.bounds(), image.rect.intersect(&image.clip));
            }
            crate::editor_element::BUILT_TIPS.with(|b| b.borrow_mut().clear());
            window.hover(image_hover_id(1), cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let built = crate::editor_element::BUILT_TIPS.with(|b| b.borrow().clone());
            assert_eq!(
                built.last().map(|t| t.as_ref()),
                Some("a red square"),
                "the tooltip shows"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_image_never_paints_outside_the_pane(cx: &mut TestAppContext) {
        let mut lines: Vec<String> = (0..80).map(|i| format!("line {i}")).collect();
        lines.insert(40, "![a](pic.png)".to_string());
        let note = lines.join("\n");
        let (handle, view, _vault, _rt) = open_note(cx, &note, &[("pic.png", png(10, 400))]);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let p = painted(&view, cx);
            let pane_bottom = row_top(&p, p.rows);
            let rows = pads_under(&view, 40, cx) + 1;
            assert!(rows >= 5, "an image of a few lines: {rows}");
            assert!(image_of(&view, 40, cx).is_none(), "below the pane");
            // Scrolled so the image starts four lines above the bottom: it
            // runs past the pane, and is clipped at its bottom.
            let down = 40 - (p.rows as usize - 4);
            window.scroll("editor", ScrollDelta::Lines(point(0., -(down as f32))), cx);
            window.render_frame(cx);
            let image = image_of(&view, 40, cx).expect("partly in view");
            assert_eq!(image.rect.origin.y, row_top(&p, p.rows - 4));
            assert!(image.rect.origin.y + image.rect.size.height > pane_bottom);
            assert_eq!(image.clip.origin.y + image.clip.size.height, pane_bottom);
            // The find bar takes rows at the bottom: the image stops above it.
            // The caret on the top line first, so the view stays put.
            window.pointer_down(cell_center(&view, 0, 0, cx), MouseButton::Left, cx);
            window.pointer_up(cell_center(&view, 0, 0, cx), MouseButton::Left, cx);
            window.press("ctrl-f", cx);
            window.render_frame(cx);
            let p = painted(&view, cx);
            let image = image_of(&view, 40, cx).expect("partly in view");
            assert!(image.clip.origin.y + image.clip.size.height <= row_top(&p, p.rows));
            window.press("escape", cx);
            // Scrolled so the image starts a line above the pane: clipped at
            // the pane's top.
            window.scroll(
                "editor",
                ScrollDelta::Lines(point(0., -(p.rows as f32 - 1.0))),
                cx,
            );
            window.render_frame(cx);
            let p = painted(&view, cx);
            let image = image_of(&view, 40, cx).expect("partly in view");
            assert!(image.rect.origin.y < p.origin.y, "starts above the pane");
            assert_eq!(image.clip.origin.y, p.origin.y, "clipped at the pane top");
            assert!(image.clip.size.height > px(0.));
        })
        .unwrap();
    }

    /// Images are decoded only once on screen, again each time the set on
    /// screen changes, and a decode that lands asks for nothing: with two
    /// images over the cache budget the count of decodes stays bounded.
    #[gpui_kit::test]
    fn only_images_on_screen_are_decoded_and_the_count_settles(cx: &mut TestAppContext) {
        use crate::inline_image::ImageStore;
        let store = ImageStore::with_test_budget(3000);
        cx.update(|cx| cx.set_global(store.clone()));
        let mut lines: Vec<String> = (0..120).map(|i| format!("line {i}")).collect();
        lines.insert(1, "![a](a.png)".to_string());
        lines.push("![b](b.png)".to_string());
        lines.push("end".to_string());
        let note = lines.join("\n");
        let files = [("a.png", png(10, 60)), ("b.png", png(10, 60))];
        let (handle, view, _vault, _rt) = open_note(cx, &note, &files);
        assert_eq!(store.decodes_started(), 1, "only the image on screen");
        cx.update(|cx| {
            assert!(pads_under(&view, 121, cx) > 0, "the one below has its room");
        });
        // To the end: `b` comes into view, `a` leaves.
        cx.update_window(handle, |_, window, cx| window.press("ctrl-end", cx))
            .unwrap();
        settle(cx, handle);
        assert_eq!(store.decodes_started(), 2);
        settle(cx, handle);
        assert_eq!(store.decodes_started(), 2, "settled");
        // Back to the top: `a` was evicted by `b`, so it decodes again.
        cx.update_window(handle, |_, window, cx| window.press("ctrl-home", cx))
            .unwrap();
        settle(cx, handle);
        assert_eq!(store.decodes_started(), 3);
        settle(cx, handle);
        assert_eq!(store.decodes_started(), 3, "settled");
        cx.update(|cx| {
            assert!(
                image_of(&view, 1, cx).is_some_and(|i| i.ready),
                "drawn again"
            );
        });
    }

    /// A point inside the painted image of `row`, `from_bottom` above its
    /// visible bottom edge.
    fn inside_image(
        view: &Entity<EditorView>,
        row: usize,
        from_bottom: Pixels,
        cx: &gpui_kit::App,
    ) -> Point<Pixels> {
        let image = image_of(view, row, cx).expect("the image is painted");
        let shown = image.rect.intersect(&image.clip);
        point(
            shown.origin.x + px(1.),
            shown.origin.y + shown.size.height - from_bottom,
        )
    }

    /// A click on the image (here its last line, a pad line) puts the
    /// cursor at the start of its embed line: the markdown shows where the
    /// image's top was, the image right under it, and the lines above stay.
    #[gpui_kit::test]
    fn a_click_on_an_image_puts_the_cursor_on_its_line(cx: &mut TestAppContext) {
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 60))]);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.cursor(), (0, 0));
            assert!(pads_under(&view, 1, cx) >= 1, "the image has pad lines");
            let at = inside_image(&view, 1, px(1.), cx);
            window.pointer_down(at, MouseButton::Left, cx);
            window.pointer_up(at, MouseButton::Left, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.cursor(), (1, 0));
            assert!(view.read(cx).editor.selection().is_none_or(|(a, b)| a == b));
            assert_eq!(rows_painting(&view, "intro", cx), [0]);
            assert_eq!(rows_painting(&view, "![a](pic.png)", cx), [1]);
            let p = painted(&view, cx);
            let image = image_of(&view, 1, cx).expect("still drawn");
            assert_eq!(image.rect.origin.y, row_top(&p, 2), "under the markdown");
        })
        .unwrap();
    }

    /// An image that starts above the pane: a click on its visible part
    /// brings its embed line into view, the image under it.
    #[gpui_kit::test]
    fn a_click_on_an_image_cut_at_the_pane_top_shows_its_line(cx: &mut TestAppContext) {
        let mut lines: Vec<String> = (0..80).map(|i| format!("line {i}")).collect();
        lines.insert(40, "![a](pic.png)".to_string());
        let note = lines.join("\n");
        let (handle, view, _vault, _rt) = open_note(cx, &note, &[("pic.png", png(10, 400))]);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Scrolled so the image starts a line above the pane.
            window.scroll("editor", ScrollDelta::Lines(point(0., -41.)), cx);
            window.render_frame(cx);
            let p = painted(&view, cx);
            let image = image_of(&view, 40, cx).expect("partly in view");
            assert!(image.rect.origin.y < p.origin.y, "starts above the pane");
            let at = inside_image(&view, 40, px(1.), cx);
            window.pointer_down(at, MouseButton::Left, cx);
            window.pointer_up(at, MouseButton::Left, cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.cursor(), (40, 0));
            let raw = rows_painting(&view, "![a](pic.png)", cx);
            assert_eq!(raw.len(), 1, "the embed line is in view");
            let p = painted(&view, cx);
            let image = image_of(&view, 40, cx).expect("still drawn");
            assert_eq!(image.rect.origin.y, row_top(&p, raw[0] as u16 + 1));
        })
        .unwrap();
    }

    /// A drag across an image selects the raw markdown of its line: the
    /// copy is text, never the pixels.
    #[gpui_kit::test]
    fn selection_across_an_image_line_copies_the_raw_markdown(cx: &mut TestAppContext) {
        let (handle, view, _vault, _rt) = open_note(cx, NOTE, &[("pic.png", png(10, 60))]);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let from = cell_center(&view, 0, 0, cx);
            window.pointer_down(from, MouseButton::Left, cx);
            // Through the image, then onto "after", wherever it is drawn now.
            let through = inside_image(&view, 1, px(1.), cx);
            window.pointer_move(through, Some(MouseButton::Left), cx);
            window.render_frame(cx);
            let after = rows_painting(&view, "after", cx)[0] as u16;
            let to = cell_center(&view, 2, after, cx);
            window.pointer_move(to, Some(MouseButton::Left), cx);
            window.pointer_up(to, MouseButton::Left, cx);
            window.render_frame(cx);
            let editor = &view.read(cx).editor;
            let (start, end) = editor.selection().expect("a selection");
            let (start, end) = (start.min(end), start.max(end));
            let text = editor.get_text();
            // Rows and columns to byte offsets (the note is ASCII).
            let offset = |(row, col): (usize, usize)| {
                text.split('\n')
                    .take(row)
                    .map(|l| l.len() + 1)
                    .sum::<usize>()
                    + col
            };
            let selected = &text[offset(start)..offset(end)];
            assert_eq!(selected, "intro\n![a](pic.png)\naf");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn without_a_vault_an_embed_line_is_text(cx: &mut TestAppContext) {
        let (handle, view) = open(cx, NOTE);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(pads_under(&view, 1, cx), 0);
            assert_eq!(rows_painting(&view, "after", cx), [2]);
        })
        .unwrap();
    }
}
