//! UI integration tests for the Edit surface: a real `EditorView` in a
//! headless window, driven by GPUI keyboard, pointer and input-handler events.

use gpui_kit::test::{TestEventExt, TestInput, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementInputHandler, Entity, Focusable, InputHandler,
    MouseButton, Pixels, Point, ScrollDelta, TestAppContext, WindowBounds, WindowOptions, point,
    px, size,
};
use kimun_notes::settings::themes::Theme;

use crate::editor_view::{self, EditorView};

fn open(cx: &mut TestAppContext, text: &str) -> (AnyWindowHandle, Entity<EditorView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
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
                let view = cx.new(|cx| EditorView::new(Some(text), Theme::default(), cx));
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
    let painted = view.read(cx).painted.expect("a frame was painted");
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
        let row = view.read(cx).painted.unwrap().cells.height();
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
