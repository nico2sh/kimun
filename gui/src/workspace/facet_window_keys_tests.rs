//! UI tests for the keys of the drawn window of the **Facet view**: `j`
//! and `k` reach far pieces, Tab and Shift+Tab walk every stop of the
//! column in the order of a frame that draws every piece, and a column stop
//! that takes the keyboard is scrolled into view with its focus ring whole.
//! Tab from the bar enters at piece 0. The bar keeps its own reveal.

use gpui_kit::{Bounds, Entity, Pixels, SharedString, TestAppContext, VisualTestContext, px, size};
use kimun_core::nfs::VaultPath;
use kimun_core::note::Facet;

use super::facet::{
    CLEAR_ID, COLUMN_ID, FACET_BAR_ID, FacetView, MemberInit, SHOW_MORE_ID, chunk_id, gap_down_id,
    open_id, pill_id,
};
use super::facet_tests::{
    bounds, focus_view, has_focus, key, open, open_with, settle, tab_to, tag, view_focused,
};
use super::facet_ui::ring_id;
use super::facet_window_view_tests::{
    clock, draw, drawn, hundred, max_offset, member_text, offset, scroll_to, with_all,
};

/// Five notes picked to `#a`, 15 pieces of which 14 are drawn under a
/// cap, so `Show more` follows them, in a window 480 by 360: a column more
/// than three viewports tall.
fn small(cx: &mut TestAppContext) -> (Entity<FacetView>, &mut VisualTestContext) {
    let members: Vec<MemberInit> = (0..5)
        .map(|m| MemberInit {
            path: VaultPath::new(format!("/notes/n{m}.md")),
            title: format!("Note {m}"),
            text: member_text(m),
        })
        .collect();
    let (view, cx, _events, _sub) = open_with(cx, move |cx| {
        FacetView::new_collection_at("#q".into(), members, 5, vec![tag("a")], clock, cx)
    });
    view.update(cx, |v, cx| v.cap_pieces(14, cx));
    cx.simulate_resize(size(px(480.), px(360.)));
    settle(cx);
    let viewport = view.read_with(cx, |v, _| f32::from(v.scroll.bounds().size.height));
    let max = max_offset(&view, cx);
    assert!(
        max > viewport * 2.,
        "over three viewports: {max} + {viewport}"
    );
    assert!(drawn(&view, cx).len() < 14, "not every piece drawn");
    (view, cx)
}

/// The stop drawn with the focus ring in a fresh frame.
fn ringed(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Option<SharedString> {
    draw(cx);
    view.read_with(cx, |v, _| v.ringed())
}

/// Whether `inner` lies inside `outer`, top to bottom.
fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    let slack = px(0.5);
    inner.top() >= outer.top() - slack && inner.bottom() <= outer.bottom() + slack
}

/// The stop with the keyboard has its whole ring inside the column; that
/// stop.
fn assert_ring_in_view(
    view: &Entity<FacetView>,
    cx: &mut VisualTestContext,
    what: &str,
) -> SharedString {
    let stop = ringed(view, cx).unwrap_or_else(|| panic!("{what}: no ring"));
    let ring = bounds(cx, ring_id(&stop));
    let column = bounds(cx, COLUMN_ID);
    assert!(
        inside(ring, column),
        "{what}: the ring of {stop} at {ring:?} is outside the column {column:?}"
    );
    stop
}

/// `ids` (the order the stops were tracked) in Tab order: a gap row
/// tracks its `Show all` first but paints it after its chevrons.
fn tab_order(ids: &[SharedString]) -> Vec<SharedString> {
    let mut out = Vec::with_capacity(ids.len());
    let mut held: Option<(SharedString, String)> = None;
    for id in ids {
        if let Some((show_all, g)) = &held {
            let chevron = id
                .strip_prefix("facet-gap-")
                .and_then(|r| r.split_once(':'));
            if chevron.is_none_or(|(_, n)| n != g) {
                out.push(show_all.clone());
                held = None;
            }
        }
        match id.strip_prefix("facet-show-all:") {
            Some(g) => held = Some((id.clone(), g.to_string())),
            None => out.push(id.clone()),
        }
    }
    out.extend(held.map(|(id, _)| id));
    out
}

/// The stops the last frame drew after `Clear`, in Tab order.
fn drawn_column_stops(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<SharedString> {
    let all = tab_order(&view.read_with(cx, |v, _| v.drawn_stop_ids()));
    let at = all.iter().position(|id| id == CLEAR_ID).expect("Clear");
    all[at + 1..].to_vec()
}

/// The column's stops in Tab order, from a frame that draws every piece.
fn column_stops(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<SharedString> {
    with_all(view, cx, |cx| drawn_column_stops(view, cx))
}

/// Bring the keyboard to `Clear`, the bar's last stop.
fn to_clear(view: &Entity<FacetView>, cx: &mut VisualTestContext) {
    focus_view(view, cx);
    tab_to(cx, CLEAR_ID, 10);
}

#[gpui_kit::test]
fn j_and_k_land_on_the_right_piece_for_a_far_piece(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    focus_view(&view, cx);
    // A member's second and third pieces come to the top themselves.
    let top = |cx: &mut VisualTestContext, i: usize| {
        f32::from(bounds(cx, chunk_id(i)).top() - bounds(cx, COLUMN_ID).top())
    };
    for _ in 0..61 {
        key(cx, "j");
    }
    assert!(!drawn(&view, cx).contains(&0), "far from the start");
    let at = top(cx, 61);
    assert!(at.abs() < 1.0, "j 61 times: piece 61 at the top, {at}");
    for _ in 0..11 {
        key(cx, "k");
    }
    let at = top(cx, 50);
    assert!(at.abs() < 1.0, "k 11 times: piece 50 at the top, {at}");
}

#[gpui_kit::test]
fn tab_walks_every_stop_down_the_column_and_shift_tab_walks_back(cx: &mut TestAppContext) {
    let (view, cx) = small(cx);
    let want = column_stops(&view, cx);
    assert_eq!(want.last().map(|s| s.as_ref()), Some(SHOW_MORE_ID));
    to_clear(&view, cx);
    let mut walked = Vec::new();
    for step in 1..=want.len() {
        key(cx, "tab");
        walked.push(assert_ring_in_view(&view, cx, &format!("Tab {step}")));
    }
    assert_eq!(walked, want, "Tab walks every stop in order");
    let mut back = Vec::new();
    for step in 1..=want.len() {
        key(cx, "shift-tab");
        let stop = ringed(&view, cx).expect("a stop has the keyboard");
        if stop != CLEAR_ID {
            assert_ring_in_view(&view, cx, &format!("Shift+Tab {step}"));
        }
        back.push(stop);
    }
    let mut want_back: Vec<SharedString> = want.iter().rev().skip(1).cloned().collect();
    want_back.push(CLEAR_ID.into());
    assert_eq!(back, want_back, "Shift+Tab walks back to Clear");
}

#[gpui_kit::test]
fn a_stop_that_takes_the_keyboard_is_scrolled_into_view(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    // Shift+Tab from the first chip wraps to `Show more`, far below.
    focus_view(&view, cx);
    let chip = view.read_with(cx, |v, _| v.drawn_stop_ids()[0].clone());
    tab_to(cx, chip, 5);
    key(cx, "shift-tab");
    assert!(has_focus(cx, SHOW_MORE_ID));
    let max = max_offset(&view, cx);
    let at = offset(&view, cx);
    assert!(
        at > max - 20.,
        "Show more: the column is at its end, {at} of {max}"
    );
    assert_ring_in_view(&view, cx, "Show more");
    // Shift+Tab again: the last stop of the last piece.
    key(cx, "shift-tab");
    assert_ring_in_view(&view, cx, "the last piece");
    // Back at the top, the first stop below the viewport is drawn in the
    // margin. Tab to it: the column scrolls to show it.
    scroll_to(&view, 0., cx);
    let column = bounds(cx, COLUMN_ID);
    let below = drawn_column_stops(&view, cx)
        .into_iter()
        .find(|id| bounds(cx, id.clone()).top() > column.bottom())
        .expect("a stop below the viewport");
    to_clear(&view, cx);
    tab_to(cx, below.clone(), 60);
    assert!(offset(&view, cx) > 0., "the column scrolled");
    assert!(inside(bounds(cx, below), bounds(cx, COLUMN_ID)));
    // A stop pinned in the leading spacer's place: Tab goes on to the
    // first drawn stop after it, which scrolls into view.
    scroll_to(&view, 0., cx);
    to_clear(&view, cx);
    tab_to(cx, open_id(0), 5);
    scroll_to(&view, max, cx);
    assert!(has_focus(cx, open_id(0)), "pinned far above");
    let stops = drawn_column_stops(&view, cx);
    let at = stops.iter().position(|id| *id == open_id(0)).unwrap();
    let next = stops[at + 1].clone();
    key(cx, "tab");
    assert_eq!(ringed(&view, cx), Some(next));
    assert_ring_in_view(&view, cx, "after a pinned stop");
}

#[gpui_kit::test]
fn the_focus_ring_of_a_scrolled_in_stop_is_not_clipped(cx: &mut TestAppContext) {
    let (view, cx) = small(cx);
    to_clear(&view, cx);
    let mut steps = 0;
    while offset(&view, cx) < max_offset(&view, cx) / 2. {
        key(cx, "tab");
        steps += 1;
        assert_ring_in_view(&view, cx, &format!("Tab {steps}"));
        assert!(steps < 100, "Tab never scrolled the column far");
    }
    for back in 1..=steps {
        key(cx, "shift-tab");
        if has_focus(cx, CLEAR_ID) {
            break;
        }
        assert_ring_in_view(&view, cx, &format!("Shift+Tab {back}"));
    }
}

#[gpui_kit::test]
fn tab_from_the_bar_enters_at_piece_zero_even_when_scrolled_down(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let first = column_stops(&view, cx)[0].clone();
    scroll_to(&view, 5000., cx);
    to_clear(&view, cx);
    assert_eq!(offset(&view, cx), 5000., "a bar stop leaves the column");
    assert!(view.read_with(cx, |v, _| v.drawn_range().start) > 0);
    key(cx, "tab");
    assert_eq!(ringed(&view, cx), Some(first), "piece 0's first stop");
    assert_eq!(offset(&view, cx), 0., "the column is at its top");
    assert_ring_in_view(&view, cx, "piece 0's first stop");
}

#[gpui_kit::test]
fn shift_tab_from_piece_zero_gives_the_keyboard_back_to_the_bar(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    scroll_to(&view, 5000., cx);
    to_clear(&view, cx);
    key(cx, "tab");
    key(cx, "shift-tab");
    assert!(has_focus(cx, CLEAR_ID), "back to Clear");
}

#[gpui_kit::test]
fn shift_tab_from_show_more_enters_at_the_last_piece(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let all = column_stops(&view, cx);
    let last = all[all.len() - 2].clone();
    assert!(last.ends_with(":133") || last.ends_with(":99"), "{last}");
    // Shift+Tab from the first chip wraps to `Show more`, then on into
    // the column with the column at its top.
    focus_view(&view, cx);
    let chip = view.read_with(cx, |v, _| v.drawn_stop_ids()[0].clone());
    tab_to(cx, chip, 5);
    scroll_to(&view, 0., cx);
    key(cx, "shift-tab");
    assert!(has_focus(cx, SHOW_MORE_ID));
    scroll_to(&view, 0., cx);
    key(cx, "shift-tab");
    assert_eq!(ringed(&view, cx), Some(last), "the last piece's last stop");
    assert_ring_in_view(&view, cx, "the last piece's last stop");
}

#[gpui_kit::test]
fn reveal_arrived_still_scrolls_the_bar_for_a_bar_stop(cx: &mut TestAppContext) {
    let long: Vec<Facet> = (0..20)
        .map(|i| tag(&format!("longtagname{i:02}")))
        .collect();
    let mut text: String = (0..20)
        .map(|i| format!("#longtagname{i:02} piece {i}\n\n"))
        .collect();
    text.push_str("#other\n");
    let (view, cx, _events, _sub) = open(cx, &text, long.clone());
    cx.simulate_resize(gpui_kit::size(px(320.), px(260.)));
    settle(cx);
    scroll_to(&view, 120., cx);
    let before = offset(&view, cx);
    assert!(before > 0., "the column scrolls");
    focus_view(&view, cx);
    let last = pill_id(&long[19]);
    tab_to(cx, last.clone(), 40);
    settle(cx);
    assert!(
        inside(bounds(cx, last), bounds(cx, FACET_BAR_ID)),
        "the bar scrolled to the last pick"
    );
    assert_eq!(offset(&view, cx), before, "the column did not move");
    // On into the column: a column stop scrolls the column, not the bar.
    tab_to(cx, CLEAR_ID, 5);
    key(cx, "tab");
    assert_ring_in_view(&view, cx, "the first column stop");
}

#[gpui_kit::test]
fn settle_focus_keeps_the_keyboard_when_the_column_scrolls(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    // Gap 1 is under piece 1, a stop the gap's piece pins.
    to_clear(&view, cx);
    tab_to(cx, gap_down_id(1), 10);
    let max = max_offset(&view, cx);
    for y in [max, max / 2., max, 0.] {
        scroll_to(&view, y, cx);
        draw(cx);
        assert!(has_focus(cx, gap_down_id(1)), "at {y}: the stop keeps it");
        assert!(!view_focused(&view, cx), "at {y}: not the view");
    }
    // The same for a piece's own stop.
    tab_to(cx, open_id(4), 20);
    scroll_to(&view, max, cx);
    draw(cx);
    assert!(has_focus(cx, open_id(4)));
    assert!(!view_focused(&view, cx));
}
