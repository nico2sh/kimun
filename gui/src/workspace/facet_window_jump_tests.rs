//! UI tests for a jump in the facet view's width or font while the column
//! is scrolled far down. The drawn range is picked from the table of the
//! frame before. When the jump moves the pieces, the view asks for one
//! more frame, so the viewport never stays blank.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, point, px, size};

use kimun_core::nfs::VaultPath;

use super::facet::{COLUMN_ID, FacetView, MemberInit, chunk_id};
use super::facet_tests::{focus_view, next_frame, open_with, settle, tag};
use super::facet_window_view_tests::{clock, draw, drawn, hundred, scroll_to};
use crate::editor_font::EditorFontSetting;
use crate::editor_view::measure_probe;

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

/// The text of a note with `n` tagged paragraphs.
fn paragraphs(n: usize) -> String {
    (0..n)
        .map(|k| format!("#a p{k} {}\n\nskip {k}\n\n", "word ".repeat(10 + k % 7)))
        .collect()
}

/// A collection drawn whole from its first frame, with no spacer yet,
/// grows and becomes windowed. Its first pieces keep their rows: the
/// first spacer, which now stands in for them, lays none of them out.
#[gpui_kit::test]
fn a_collection_that_becomes_windowed_keeps_the_rows_of_its_pieces(cx: &mut TestAppContext) {
    let path = VaultPath::new("/notes/n0.md");
    let member = MemberInit {
        path: path.clone(),
        title: "Note 0".into(),
        text: paragraphs(3),
    };
    let (view, cx, _events, _sub) = open_with(cx, move |cx| {
        FacetView::new_collection_at("#q".into(), vec![member], 1, vec![tag("a")], clock, cx)
    });
    settle(cx);
    assert_eq!(drawn(&view, cx), [0, 1, 2], "drawn whole");
    let first: Vec<_> = view.read_with(cx, |v, _| {
        v.chunks()[..3].iter().map(|c| c.entity_id()).collect()
    });
    // The column far down, then more pieces: the update draws the first
    // windowed frame, and the first three stand in the leading spacer.
    view.read_with(cx, |v, _| v.scroll.set_offset(point(px(0.), px(-4000.))));
    measure_probe::take();
    view.update(cx, |v, cx| v.set_member_text(&path, paragraphs(60), cx));
    draw(cx);
    let drawn = drawn(&view, cx);
    assert!(!drawn.contains(&0), "windowed: {drawn:?}");
    let log = measure_probe::take();
    let laid: usize = first
        .iter()
        .map(|id| log.get(id).map_or(0, |m| m.calls))
        .sum();
    assert_eq!(laid, 0, "the first pieces keep their rows");
}

/// The scan of the pieces for the keyboard runs only while the keyboard
/// is in the view.
#[gpui_kit::test]
fn no_piece_is_asked_for_the_keyboard_while_it_is_outside_the_view(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    cx.update(|window, cx| window.blur(cx));
    draw(cx);
    let before = view.read_with(cx, |v, _| v.focus_scans());
    for _ in 0..3 {
        draw(cx);
    }
    let after = view.read_with(cx, |v, _| v.focus_scans());
    assert_eq!(after, before, "no scan while the keyboard is outside");
    focus_view(&view, cx);
    draw(cx);
    let inside = view.read_with(cx, |v, _| v.focus_scans());
    assert!(inside > after, "a scan while the keyboard is in the view");
}
