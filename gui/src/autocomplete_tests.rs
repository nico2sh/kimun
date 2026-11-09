//! UI integration tests for the Edit surface's autocomplete: a real
//! `EditorView` over a real vault in a headless window, driven by GPUI keys,
//! text input and pointer events.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementInputHandler, Entity, Focusable, InputHandler,
    SharedString, TestAppContext, WindowBounds, WindowOptions, point, px, size,
};
use kimun_notes::logic::autocomplete::VaultSuggestions;
use tokio::runtime::Runtime;

use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::test_vault::test_vault;

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// An editor over a vault holding two notes starting `al` and a `#projects`
/// tag, in a focused window. Needs the runtime entered: the popup's queries
/// spawn on it.
fn open(cx: &mut TestAppContext, rt: &Runtime, vim: bool) -> (AnyWindowHandle, Entity<EditorView>) {
    let vault = rt
        .block_on(test_vault(
            &[
                ("alpha.md", "# Alpha\n\n#projects\n"),
                ("alpine.md", "# Alpine\n"),
                ("beta.md", "# Beta\n"),
            ],
            &[],
        ))
        .vault
        .clone();
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let (handle, view) = cx.update(|cx| {
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
                let view = cx.new(|cx| EditorView::new(None, vim, cx));
                window.focus(&view.read(cx).focus_handle(cx), cx);
                view
            },
        )
        .expect("open test window")
    });
    cx.update(|cx| {
        view.update(cx, |view, _| {
            view.set_suggestions(Some(Arc::new(VaultSuggestions { vault })));
        });
    });
    (handle, view)
}

/// Run tokio and GPUI in turn until `done`, or fail after ~10 s.
fn drive(
    rt: &Runtime,
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    for _ in 0..2000 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(5)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        if done(cx) {
            return;
        }
    }
    panic!("timed out waiting");
}

fn names(view: &Entity<EditorView>, cx: &mut TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        view.read(cx)
            .autocomplete_state()
            .map(|s| s.items.iter().map(|i| i.display.clone()).collect())
            .unwrap_or_default()
    })
}

fn is_open(view: &Entity<EditorView>, cx: &mut TestAppContext) -> bool {
    cx.read(|cx| view.read(cx).autocomplete_state().is_some())
}

fn text(view: &Entity<EditorView>, cx: &mut TestAppContext) -> String {
    cx.read(|cx| view.read(cx).editor.get_text())
}

/// Type `s` and wait for the popup to list something.
fn type_and_open(
    rt: &Runtime,
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<EditorView>,
    s: &str,
) {
    cx.update_window(handle, |_, window, cx| window.input(s, cx))
        .unwrap();
    drive(rt, cx, handle, |cx| is_open(view, cx));
}

fn press(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &str) {
    cx.update_window(handle, |_, window, cx| window.press(key, cx))
        .unwrap();
}

#[gpui_kit::test]
fn typing_a_wikilink_opens_the_popup_with_matching_notes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "see [[al");
    let found = names(&view, cx);
    assert!(found.contains(&"alpha".to_string()), "{found:?}");
    assert!(found.contains(&"alpine".to_string()), "{found:?}");
    assert!(!found.contains(&"beta".to_string()), "{found:?}");
    // The popup was drawn, below the caret.
    let painted = cx.read(|cx| view.read(cx).painted.clone().expect("painted"));
    let popup = painted.popup.expect("the popup is placed");
    let caret = painted.caret.expect("a caret");
    assert!(popup.top() >= caret.bottom(), "{popup:?} vs {caret:?}");
    // What was drawn is where `place` put it: the pane's own origin must not
    // be added on top.
    cx.update_window(handle, |_, window, _| {
        let drawn = window.find("autocomplete-popup");
        assert!(drawn.visible());
        let (a, b) = (drawn.bounds(), popup);
        let near = |x: gpui_kit::Pixels, y: gpui_kit::Pixels| f32::from(x - y).abs() < 0.5;
        assert!(
            near(a.origin.x, b.origin.x) && near(a.origin.y, b.origin.y),
            "drawn {a:?}, placed {b:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_accepts_the_highlighted_note_and_closes_the_popup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "see [[al");
    let first = names(&view, cx).remove(0);
    press(cx, handle, "tab");
    assert_eq!(text(&view, cx), format!("see [[{first}]]"));
    assert!(!is_open(&view, cx), "closed, and not reopened on the word");
    cx.read(|cx| assert!(view.read(cx).editor.is_dirty()));
    // Typing on carries on after the link.
    cx.update_window(handle, |_, window, cx| window.input("!", cx))
        .unwrap();
    assert_eq!(text(&view, cx), format!("see [[{first}]]!"));
}

#[gpui_kit::test]
fn enter_accepts_and_down_moves_the_highlight(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    let second = names(&view, cx)[1].clone();
    press(cx, handle, "down");
    assert!(is_open(&view, cx), "Down is the popup's, not the editor's");
    press(cx, handle, "enter");
    assert_eq!(text(&view, cx), format!("[[{second}]]"));
}

#[gpui_kit::test]
fn escape_closes_the_popup_and_nothing_else(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    press(cx, handle, "escape");
    assert!(!is_open(&view, cx));
    assert_eq!(text(&view, cx), "[[al", "the buffer is untouched");
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("editor").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hashtag_completes_to_the_label_without_a_trailing_space(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "todo #pro");
    assert!(names(&view, cx).contains(&"projects".to_string()));
    press(cx, handle, "tab");
    assert_eq!(text(&view, cx), "todo #projects");
    assert!(!is_open(&view, cx));
}

#[gpui_kit::test]
fn backspacing_past_the_sigil_closes_the_popup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    for _ in 0..3 {
        press(cx, handle, "backspace");
    }
    assert_eq!(text(&view, cx), "[");
    assert!(!is_open(&view, cx));
}

#[gpui_kit::test]
fn typing_narrows_the_popup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    assert!(names(&view, cx).len() >= 2);
    cx.update_window(handle, |_, window, cx| window.input("ph", cx))
        .unwrap();
    drive(&rt, cx, handle, |cx| names(&view, cx) == ["alpha"]);
}

#[gpui_kit::test]
fn no_popup_opens_in_a_code_block_or_frontmatter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    cx.update(|cx| {
        view.update(cx, |view, _| {
            view.editor.set_text("```\n\n```\n".into());
        });
    });
    press(cx, handle, "down");
    cx.update_window(handle, |_, window, cx| window.input("[[al", cx))
        .unwrap();
    // A popup would show within a few queries' time; give it that.
    for _ in 0..40 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(5)));
        cx.run_until_parked();
    }
    assert!(!is_open(&view, cx), "inside a fence");

    cx.update(|cx| {
        view.update(cx, |view, _| {
            view.editor.set_text("---\ntitle: \n---\n".into());
        });
    });
    press(cx, handle, "ctrl-home");
    press(cx, handle, "down");
    press(cx, handle, "end");
    cx.update_window(handle, |_, window, cx| window.input("#pro", cx))
        .unwrap();
    for _ in 0..40 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(5)));
        cx.run_until_parked();
    }
    assert!(!is_open(&view, cx), "inside frontmatter");
}

#[gpui_kit::test]
fn vim_escape_closes_the_popup_first_and_leaves_insert_second(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, true);
    cx.update_window(handle, |_, window, cx| window.input("i", cx))
        .unwrap();
    type_and_open(&rt, cx, handle, &view, "[[al");
    press(cx, handle, "escape");
    assert!(!is_open(&view, cx));
    assert_eq!(
        cx.read(|cx| view.read(cx).editor.modal_is_insert()),
        Some(true),
        "the first Esc only closed the popup"
    );
    press(cx, handle, "escape");
    assert_eq!(
        cx.read(|cx| view.read(cx).editor.modal_is_insert()),
        Some(false),
        "the second Esc left Insert"
    );
}

#[gpui_kit::test]
fn vim_normal_mode_never_opens_a_popup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, true);
    cx.update(|cx| {
        view.update(cx, |view, _| {
            view.editor.set_text("see [[alp and #pro".into())
        });
    });
    // Normal mode: `x` edits the text with the caret inside the open link.
    press(cx, handle, "$");
    press(cx, handle, "x");
    for _ in 0..40 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(5)));
        cx.run_until_parked();
    }
    assert!(!is_open(&view, cx));
}

#[gpui_kit::test]
fn accepting_is_one_undo_step_back_to_what_was_typed(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "see [[al");
    press(cx, handle, "tab");
    assert_ne!(text(&view, cx), "see [[al");
    press(cx, handle, "ctrl-z");
    assert_eq!(
        text(&view, cx),
        "see [[al",
        "one step undoes the accept only"
    );
}

#[gpui_kit::test]
fn a_press_on_a_row_accepts_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    let second = names(&view, cx)[1].clone();
    cx.update_window(handle, |_, window, cx| {
        window.click((SharedString::from(second.clone()), 1usize), cx);
    })
    .unwrap();
    assert_eq!(text(&view, cx), format!("[[{second}]]"));
    assert!(!is_open(&view, cx));
}

#[gpui_kit::test]
fn hovering_a_row_highlights_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    let second = names(&view, cx)[1].clone();
    cx.update_window(handle, |_, window, cx| {
        window.hover((SharedString::from(second), 1usize), cx);
    })
    .unwrap();
    cx.read(|cx| {
        let highlighted = view.read(cx).autocomplete_state().map(|s| s.highlighted);
        assert_eq!(highlighted, Some(1));
    });
}

#[gpui_kit::test]
fn a_press_on_the_text_closes_the_popup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    // Row 0, col 0: well clear of the popup, which sits below the caret.
    let at = cx.read(|cx| {
        let p = view.read(cx).painted.clone().expect("painted");
        point(
            p.origin.x + px(p.cells.width() / 2.0),
            p.origin.y + px(p.cells.height() / 2.0),
        )
    });
    cx.update_window(handle, |_, window, cx| window.click_at("editor", at, cx))
        .unwrap();
    assert!(!is_open(&view, cx));
}

#[gpui_kit::test]
fn a_note_loaded_under_the_popup_closes_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    type_and_open(&rt, cx, handle, &view, "[[al");
    cx.update(|cx| {
        view.update(cx, |view, _| view.editor.set_text("another note".into()));
    });
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!is_open(&view, cx));
}

#[gpui_kit::test]
fn a_composition_in_progress_opens_no_popup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find("editor").bounds();
        let mut handler = ElementInputHandler::new(bounds, view.clone());
        // A preedit that looks like a trigger: it is not in the buffer, so
        // there is nothing to complete until it is committed.
        handler.replace_and_mark_text_in_range(None, "[[al", None, window, cx);
    })
    .unwrap();
    for _ in 0..40 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(5)));
        cx.run_until_parked();
    }
    assert!(!is_open(&view, cx));
    assert_eq!(text(&view, cx), "", "a preedit is not an edit");
}

#[gpui_kit::test]
fn the_popup_follows_the_theme(cx: &mut TestAppContext) {
    // Drawn in the light and the dark theme without panicking, from theme
    // roles only.
    use kimun_notes::settings::themes::Theme;
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    for theme in [Theme::gruvbox_dark(), Theme::gruvbox_light()] {
        cx.update(|cx| {
            crate::theme::set_current(cx, theme.clone());
        });
        type_and_open(&rt, cx, handle, &view, "[[al");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("autocomplete-popup").visible());
        })
        .unwrap();
        press(cx, handle, "escape");
        cx.update(|cx| {
            view.update(cx, |view, _| view.editor.set_text(String::new()));
        });
    }
}

#[gpui_kit::test]
fn a_popup_opened_at_the_end_of_a_long_line_stays_inside_the_window(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (handle, view) = open(cx, &rt, false);
    // Most of a 640px-wide line: the caret is near the right edge.
    let line = "x".repeat(60);
    type_and_open(&rt, cx, handle, &view, &format!("{line} [[al"));
    let popup = cx
        .read(|cx| view.read(cx).painted.clone().and_then(|p| p.popup))
        .expect("placed");
    cx.update_window(handle, |_, window, _| {
        let drawn = window.find("autocomplete-popup").bounds();
        let win = window.viewport_size();
        assert!(drawn.left() >= px(0.) && drawn.top() >= px(0.), "{drawn:?}");
        assert!(
            drawn.right() <= win.width && drawn.bottom() <= win.height,
            "{drawn:?} outside {win:?}"
        );
        assert!(f32::from(drawn.origin.x - popup.origin.x).abs() < 0.5);
    })
    .unwrap();
}
