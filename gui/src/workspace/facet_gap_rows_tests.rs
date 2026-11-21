//! UI tests for the split gap rows of the **Facet view**. The row above the
//! crumbs holds the down button, the count and `Show all`. The up button has
//! its own row under the crumbs, right above the text. A gap with nothing
//! above it draws only the up row, with the count in it.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, Pixels, SharedString, TestAppContext, VisualTestContext};

use super::facet::rows;
use super::facet::{
    CLEAR_ID, COLUMN_ID, chunk_id, crumb_id, gap_count_id, gap_down_id, gap_up_id, open_id,
    show_all_id,
};
use super::facet_tests::{bounds, focus_view, has_focus, key, open, shows, tab_to, tag};
use super::facet_ui::{label_inset, ring_id};

/// The bounds of chunk `i`'s editor element.
fn editor(cx: &mut VisualTestContext, i: usize) -> Bounds<Pixels> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.within(chunk_id(i)).find("editor").bounds()
    })
}

/// Whether `a` and `b` share a row: their spans overlap top to bottom.
fn one_row(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    a.top() < b.bottom() && b.top() < a.bottom()
}

#[gpui_kit::test]
fn the_up_button_sits_between_the_crumbs_and_the_text(cx: &mut TestAppContext) {
    // Gap 1 lies between the two chunks and hides two blocks.
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    let (down, up) = (bounds(cx, gap_down_id(1)), bounds(cx, gap_up_id(1)));
    let (count, crumbs) = (bounds(cx, gap_count_id(1)), bounds(cx, open_id(1)));
    let (piece, text) = (bounds(cx, chunk_id(1)), editor(cx, 1));
    assert!(
        down.bottom() <= piece.top(),
        "down: above the piece's crumbs"
    );
    assert!(one_row(down, count), "the count sits with the down button");
    assert!(up.top() >= crumbs.bottom(), "up: under the crumbs");
    assert!(up.bottom() <= text.top(), "up: above the text");
    assert!(up.top() >= piece.top(), "the up row belongs to the piece");
}

#[gpui_kit::test]
fn a_gap_above_the_first_chunk_draws_one_row_under_the_crumbs(cx: &mut TestAppContext) {
    // Gap 0 hides p0 and has nothing above it.
    let text = "p0\n\n#x p1\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert!(!shows(cx, gap_down_id(0)), "nothing above to extend");
    let (up, count) = (bounds(cx, gap_up_id(0)), bounds(cx, gap_count_id(0)));
    let (all, crumbs) = (bounds(cx, show_all_id(0)), bounds(cx, open_id(0)));
    let text = editor(cx, 0);
    assert!(one_row(up, count) && one_row(up, all), "one row");
    assert!(up.top() >= crumbs.bottom(), "under the crumbs");
    assert!(up.bottom() <= text.top(), "above the text");
    // No row above the crumbs: the piece is the column's first item.
    let pad = cx.update(|window, _| rows::column_pad().to_pixels(window.rem_size()));
    let gap = bounds(cx, chunk_id(0)).top() - bounds(cx, COLUMN_ID).top();
    assert!(
        (gap - pad).abs() <= gpui_kit::px(0.5),
        "only the pad: {gap:?}"
    );
}

/// Tab walks the stops top to bottom, left to right: the order they are
/// painted in, across the row above the crumbs, the crumbs and the up row.
#[gpui_kit::test]
fn tab_follows_the_paint_order_across_the_split_rows(cx: &mut TestAppContext) {
    let text = "p0\n\n#x p1\n\np2\n\n#x p3\n\np4\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    focus_view(&view, cx);
    tab_to(cx, CLEAR_ID, 10);
    let want: Vec<SharedString> = vec![
        open_id(0),
        gap_up_id(0),
        show_all_id(0),
        gap_down_id(1),
        show_all_id(1),
        open_id(1),
        gap_up_id(1),
        gap_down_id(2),
        show_all_id(2),
    ];
    let mut last: Option<Bounds<Pixels>> = None;
    for id in &want {
        key(cx, "tab");
        assert!(has_focus(cx, id.clone()), "Tab reached {id}");
        let at = bounds(cx, id.clone());
        if let Some(before) = last {
            let later =
                at.top() >= before.bottom() || (one_row(at, before) && at.left() >= before.right());
            assert!(later, "{id} is painted after the stop before it");
        }
        last = Some(at);
    }
    // The stops the view lists for Tab are the painted ones, in order.
    let drawn = view.read_with(cx, |v, _| v.drawn_stop_ids());
    let at = drawn.iter().position(|id| id == CLEAR_ID).unwrap();
    assert_eq!(drawn[at + 1..], want[..]);
}

/// The up button's focus ring is whole: the piece clips, and the up row
/// keeps room for the ring inside it.
#[gpui_kit::test]
fn the_ring_of_the_up_button_is_not_clipped_by_the_piece(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\n#x p2\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    focus_view(&view, cx);
    tab_to(cx, gap_up_id(1), 20);
    let ring = bounds(cx, ring_id(&gap_up_id(1)));
    let piece = bounds(cx, chunk_id(1));
    assert!(
        ring.top() >= piece.top() && ring.bottom() <= piece.bottom(),
        "the ring {ring:?} inside the piece {piece:?}"
    );
}

/// `j` brings the crumbs of the next piece to the top: the row above the
/// crumbs scrolls out, the up row stays under the crumbs.
#[gpui_kit::test]
fn j_and_k_still_put_the_crumbs_at_the_top(cx: &mut TestAppContext) {
    let text: String = (0..40)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("t")]);
    focus_view(&view, cx);
    let near = |a: Pixels, b: Pixels| (a - b).abs() < gpui_kit::px(1.);
    for (keys, i) in [("j", 1), ("j", 2), ("k", 1)] {
        key(cx, keys);
        let column = bounds(cx, COLUMN_ID).top();
        assert!(
            near(bounds(cx, chunk_id(i)).top(), column),
            "{keys}: piece {i}"
        );
        assert!(
            bounds(cx, gap_down_id(i)).bottom() <= column,
            "{keys}: the row above the crumbs is out of view"
        );
        assert!(
            bounds(cx, gap_up_id(i)).top() >= bounds(cx, open_id(i)).bottom(),
            "{keys}: the up row under the crumbs"
        );
    }
}

/// The down row adds text to the piece above it, so it sits closer to that
/// piece than to the piece under it. The space above it is the column gap
/// less the pull of [`rows::gap_pull`], drawn as the height table counts it.
#[gpui_kit::test]
fn the_down_row_sits_closer_to_the_piece_it_extends(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    let down = bounds(cx, gap_down_id(1));
    let (above, below) = (bounds(cx, chunk_id(0)), bounds(cx, chunk_id(1)));
    let (over, under) = (down.top() - above.bottom(), below.top() - down.bottom());
    assert!(
        over < under,
        "closer to the piece above: {over:?} < {under:?}"
    );
    let want = cx.update(|window, _| {
        let rem = window.rem_size();
        rows::column_gap().to_pixels(rem) - rows::gap_pull().to_pixels(rem)
    });
    assert!(
        (over - want).abs() <= gpui_kit::px(0.5),
        "{over:?} is {want:?}"
    );
}

/// The tail row under the last piece hugs that piece as the lead row does.
#[gpui_kit::test]
fn the_tail_row_sits_closer_to_the_piece_it_extends(cx: &mut TestAppContext) {
    // Gap 1 after p0 hides p1 and p2 and ends the note.
    let text = "#x p0\n\np1\n\np2\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    let down = bounds(cx, gap_down_id(1));
    let piece = bounds(cx, chunk_id(0));
    let want = cx.update(|window, _| {
        let rem = window.rem_size();
        rows::column_gap().to_pixels(rem) - rows::gap_pull().to_pixels(rem)
    });
    let over = down.top() - piece.bottom();
    assert!(
        (over - want).abs() <= gpui_kit::px(0.5),
        "{over:?} is {want:?}"
    );
}

/// One left edge: the piece text starts at the spine ([`rows::spine`]). A
/// crumb's label, the edge of the up button and the edge of the down button
/// start there too, so the eye finds one column.
#[gpui_kit::test]
fn the_crumbs_and_the_gap_buttons_start_at_the_text_edge(cx: &mut TestAppContext) {
    let text = "# H\n\n#x p0\n\np1\n\np2\n\n#x p3\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    let (spine, label_in) = cx.update(|window, _| {
        let rem = window.rem_size();
        (rows::spine().to_pixels(rem), label_inset(rem))
    });
    let piece = bounds(cx, chunk_id(1));
    let edge = piece.left() + spine;
    let near = |a: Pixels, what: &str| {
        assert!(
            (a - edge).abs() <= gpui_kit::px(0.5),
            "{what}: {a:?} not {edge:?}"
        )
    };
    // The editor's own padding puts the text at the spine.
    let pad = cx.update(|window, _| {
        gpui_kit::rems(crate::editor_view::PANE_PAD_X_REMS).to_pixels(window.rem_size())
    });
    near(editor(cx, 1).left() + pad, "text");
    near(bounds(cx, crumb_id(1, 0)).left() + label_in, "crumb label");
    near(bounds(cx, gap_up_id(1)).left(), "up button");
    near(bounds(cx, gap_down_id(1)).left(), "down button");
}
