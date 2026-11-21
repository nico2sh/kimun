//! UI tests for the edges of the column reveal and of a stop that asks
//! for the keyboard: a stale stop past the end, and a stop drawn a frame
//! late.

use gpui_kit::{Entity, TestAppContext, VisualTestContext, point, px, size};

use super::facet::{FacetView, open_id};
use super::facet_tests::{has_focus, open, settle, tag};
use super::facet_window_view_tests::{draw, drawn, hundred, offset, scroll_to, table_and_real};

/// A note with `n` tagged paragraphs of a few rows each.
fn note(n: usize) -> String {
    (0..n)
        .map(|k| format!("#a p{k} {}\n\nskip {k}\n\n", "word ".repeat(80 + k % 9)))
        .collect()
}

#[gpui_kit::test]
fn a_stale_stop_past_the_end_leaves_the_column_where_it_is(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &note(40), vec![tag("a")]);
    cx.simulate_resize(size(px(600.), px(600.)));
    settle(cx);
    scroll_to(&view, 3000., cx);
    let k = *drawn(&view, cx).last().expect("drawn pieces");
    // Fewer pieces than `k`, still more column than the offset needs.
    let m = k - 1;
    assert!(m >= 12, "a far piece: {k}");
    // The stop takes the keyboard in the frame whose plan drops its piece.
    cx.update(|window, cx| {
        let handle = view.read(cx).open_handle(k).expect("Open k is drawn");
        window.focus(&handle, cx);
        view.update(cx, |v, cx| v.set_text(note(m), cx));
    });
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.chunks().len()), m);
    assert_eq!(offset(&view, cx), 3000., "the column stays");
}

fn far_open_asked(cx: &mut TestAppContext) -> (Entity<FacetView>, &mut VisualTestContext) {
    let (view, cx) = hundred(cx);
    scroll_to(&view, 6000., cx);
    assert!(!drawn(&view, cx).contains(&0));
    view.update(cx, |v, _| v.ask_focus_open(0));
    // This frame does not draw piece 0.
    draw(cx);
    (view, cx)
}

#[gpui_kit::test]
fn a_stop_drawn_one_frame_late_still_takes_the_keyboard(cx: &mut TestAppContext) {
    let (view, cx) = far_open_asked(cx);
    scroll_to(&view, 0., cx);
    assert!(
        has_focus(cx, open_id(0)),
        "the late stop takes the keyboard"
    );
}

#[gpui_kit::test]
fn a_stop_that_never_arrives_is_dropped_after_one_more_frame(cx: &mut TestAppContext) {
    let (view, cx) = far_open_asked(cx);
    // The retry frame does not draw it either.
    draw(cx);
    scroll_to(&view, 0., cx);
    assert!(!has_focus(cx, open_id(0)), "the request went");
}

/// A frame at an editor width of 0 or less stores no rows. Back at the old
/// width, a piece that the narrow frame drew has its real rows in the
/// table, not the rows of one column.
#[gpui_kit::test]
fn a_frame_with_no_editor_width_leaves_no_inflated_rows(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let piece = 0;
    assert!(drawn(&view, cx).contains(&piece));
    // 60 px of window: the column is narrower than the editor's insets.
    cx.simulate_resize(size(px(60.), px(600.)));
    draw(cx);
    assert!(
        drawn(&view, cx).contains(&piece),
        "the narrow frame draws it"
    );
    // Far down at the old width: the piece stands in the leading spacer,
    // and no frame draws it at that width.
    view.read_with(cx, |v, _| v.scroll.set_offset(point(px(0.), px(-6000.))));
    cx.simulate_resize(size(px(600.), px(600.)));
    draw(cx);
    assert!(!drawn(&view, cx).contains(&piece));
    let (rows, real) = table_and_real(&view, piece, cx);
    assert_eq!(rows, Some(real), "the piece has its real rows");
}

/// A short note of `n` one-line tagged paragraphs: every piece is drawn.
fn short(n: usize) -> String {
    (0..n).map(|k| format!("#a p{k}\n\nskip {k}\n\n")).collect()
}

/// A stop asked for the keyboard arrives a frame late, but the keyboard
/// went to another stop in between: the late stop does not take it.
#[gpui_kit::test]
fn a_late_stop_does_not_take_the_keyboard_after_it_moved(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &short(3), vec![tag("a")]);
    settle(cx);
    // Piece 5 does not exist yet: the request waits a frame.
    view.update(cx, |v, _| v.ask_focus_open(5));
    draw(cx);
    // The keyboard moves to piece 1's stop, and piece 5 comes.
    cx.update(|window, cx| {
        let handle = view.read(cx).open_handle(1).expect("Open 1 is drawn");
        window.focus(&handle, cx);
        view.update(cx, |v, cx| v.set_text(short(6), cx));
    });
    draw(cx);
    assert!(!has_focus(cx, open_id(5)), "the late stop takes nothing");
    assert!(
        has_focus(cx, open_id(1)),
        "the keyboard stays where it went"
    );
}
