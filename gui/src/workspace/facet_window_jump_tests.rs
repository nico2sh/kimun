//! UI tests for a jump in the facet view's width or font while the column
//! is scrolled far down. The drawn range is picked from the table of the
//! frame before. When the jump moves the pieces, the view asks for one
//! more frame, so the viewport never stays blank.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, px, size};

use super::facet::{COLUMN_ID, FacetView, chunk_id};
use super::facet_tests::next_frame;
use super::facet_window_view_tests::{draw, drawn, hundred, scroll_to};
use crate::editor_font::EditorFontSetting;

/// Run the frames the last frame asked for, and draw one if any did.
fn asked_frames(cx: &mut VisualTestContext) -> usize {
    let asked = next_frame(cx);
    if asked > 0 {
        cx.update(|window, cx| window.render_frame(cx));
    }
    asked
}

/// The drawn pieces that show in the column's bounds.
fn shown(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<usize> {
    let drawn = drawn(view, cx);
    cx.update(|window, _| {
        let column = window.find(COLUMN_ID).bounds();
        drawn
            .into_iter()
            .filter(|&i| {
                let b = window.find(chunk_id(i)).bounds();
                b.bottom() > column.top() && b.top() < column.bottom()
            })
            .collect()
    })
}

#[gpui_kit::test]
fn a_font_size_jump_far_down_leaves_no_blank_viewport(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    scroll_to(&view, 6000., cx);
    assert!(!shown(&view, cx).is_empty(), "pieces show before the jump");
    cx.update(|_, cx| {
        cx.set_global(EditorFontSetting {
            family: None,
            size: Some(px(30.)),
        })
    });
    draw(cx);
    asked_frames(cx);
    let shown = shown(&view, cx);
    let drawn = drawn(&view, cx);
    assert!(
        !shown.is_empty(),
        "a piece shows after a font jump: {drawn:?}"
    );
}

#[gpui_kit::test]
fn a_width_jump_far_down_leaves_no_blank_viewport(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    scroll_to(&view, 6000., cx);
    cx.simulate_resize(size(px(350.), px(600.)));
    asked_frames(cx);
    let shown = shown(&view, cx);
    let drawn = drawn(&view, cx);
    assert!(!shown.is_empty(), "a piece shows after a resize: {drawn:?}");
}

#[gpui_kit::test]
fn a_steady_frame_asks_for_no_next_frame(cx: &mut TestAppContext) {
    let (_view, cx) = hundred(cx);
    for _ in 0..3 {
        draw(cx);
        assert_eq!(next_frame(cx), 0, "a steady frame");
    }
}
