//! UI tests for the motion of the **Facet view**'s bar (ADR-0060): its
//! height eases to a new height over the Quick speed, the column follows,
//! and the first-use hint collapses with it. A `FacetView` alone in a
//! headless window with motion on, time stepped by the executor clock.
#![cfg(test)]

use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, Entity, Pixels, SharedString, TestAppContext, VisualTestContext, px};
use kimun_core::nfs::VaultPath;
use kimun_core::note::Facet;

use super::facet::{
    BAR_FRAME_ID, CLEAR_ID, COLUMN_ID, FACET_BAR_ID, FacetView, HINT_FRAME_ID, HINT_ID, UNDO_ID,
    pill_id,
};
use super::facet_size_tests::assert_no_correction;
use super::facet_tests::{
    click, focus_view, has_focus, key, open, open_animated, settle, tab_to, tag,
};

/// A view of `text` with `picks`, motion on, settled at rest.
pub(super) fn animated<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    picks: Vec<Facet>,
) -> (Entity<FacetView>, &'a mut VisualTestContext) {
    let text = text.to_string();
    let (view, cx, _events, sub) = open_animated(cx, move |cx| {
        FacetView::new(VaultPath::new("/notes/n.md"), text, picks, cx)
    });
    drop(sub);
    settle(cx);
    (view, cx)
}

pub(super) fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.render_frame(cx));
}

pub(super) fn wait(cx: &mut VisualTestContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
}

/// The bounds of `id` in the last frame, `None` when it was not drawn. Draws
/// no frame.
pub(super) fn found(
    cx: &mut VisualTestContext,
    id: impl Into<SharedString>,
) -> Option<Bounds<Pixels>> {
    let id: SharedString = id.into();
    cx.update(|window, _| window.try_find(id).map(|e| e.bounds()))
}

/// The bounds of the frame that clips the bar in the last frame, `None` at
/// rest. The frame has no element id (it must not change the bar's element
/// path), so it is found by its debug selector.
pub(super) fn bar_frame(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.debug_bounds(BAR_FRAME_ID)
}

/// The bounds of the frame that clips the hint in the last frame, `None`
/// when the hint does not collapse. Found by its debug selector, like the
/// bar's frame.
pub(super) fn hint_frame(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.debug_bounds(HINT_FRAME_ID)
}

/// The height the bar takes in the view: its frame while it moves, else
/// the bar itself.
pub(super) fn drawn_bar(cx: &mut VisualTestContext) -> f32 {
    let bounds = bar_frame(cx)
        .or_else(|| found(cx, FACET_BAR_ID))
        .expect("the bar is drawn");
    f32::from(bounds.size.height)
}

/// The height the bar's content lays out at in the last frame.
pub(super) fn natural(cx: &mut VisualTestContext) -> f32 {
    f32::from(found(cx, FACET_BAR_ID).expect("the bar").size.height)
}

pub(super) fn column_top(cx: &mut VisualTestContext) -> f32 {
    f32::from(found(cx, COLUMN_ID).expect("the column").top())
}

pub(super) fn toggle(view: &Entity<FacetView>, cx: &mut VisualTestContext, facet: Facet) {
    view.update(cx, |view, cx| view.toggle(facet, cx));
}

pub(super) fn moving(cx: &mut VisualTestContext) -> bool {
    bar_frame(cx).is_some()
}

/// Draw frames until the bar rests, at most `limit` steps of 20 ms.
pub(super) fn rest(cx: &mut VisualTestContext, limit: usize) {
    for _ in 0..limit {
        frame(cx);
        if !moving(cx) && hint_frame(cx).is_none() {
            return;
        }
        wait(cx, 20);
    }
    panic!("the bar never rested");
}

pub(super) const TWO_TAGS: &str = "#a #b\n\nsome text\n";

/// Long tags that wrap a pick row at a narrow width.
pub(super) fn long(i: usize) -> Facet {
    tag(&format!("longtagname{i:02}"))
}

pub(super) fn long_note(n: usize) -> String {
    (0..n)
        .map(|i| format!("#longtagname{i:02} piece {i}\n\n"))
        .collect()
}

#[gpui_kit::test]
fn the_bar_grows_over_time_when_the_first_pick_adds_rows(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    let start = drawn_bar(cx);
    toggle(&view, cx, tag("a"));
    frame(cx);
    // The pick's frame measures: the bar keeps its height and clips.
    assert!(moving(cx), "the bar is framed while it moves");
    assert!((drawn_bar(cx) - start).abs() < 0.5, "starts where it was");
    frame(cx);
    let end = natural(cx);
    assert!(end > start + 10.0, "new rows: {start} to {end}");
    wait(cx, 60);
    frame(cx);
    let middle = drawn_bar(cx);
    assert!(
        middle > start + 0.5 && middle < end - 0.5,
        "after 60 ms: {start} < {middle} < {end}"
    );
    wait(cx, 70);
    frame(cx);
    assert!(!moving(cx), "at rest after the Quick speed");
    assert!((drawn_bar(cx) - end).abs() < 0.5);
}

#[gpui_kit::test]
fn the_column_follows_the_bar_in_one_motion(cx: &mut TestAppContext) {
    let (_view, cx) = animated(cx, TWO_TAGS, vec![tag("a")]);
    click(cx, CLEAR_ID);
    frame(cx);
    frame(cx);
    let mut last = column_top(cx);
    let mut steps = 0;
    for _ in 0..8 {
        wait(cx, 20);
        frame(cx);
        let bar = bar_frame(cx).or_else(|| found(cx, FACET_BAR_ID));
        let top = column_top(cx);
        assert!(
            (f32::from(bar.unwrap().bottom()) - top).abs() < 0.5,
            "the column starts under the bar"
        );
        assert!(top <= last + 0.01, "one way only: {last} then {top}");
        if top < last - 0.01 {
            steps += 1;
        }
        last = top;
    }
    assert!(steps >= 3, "the column moves over several frames: {steps}");
}

#[gpui_kit::test]
fn the_bar_shrinks_over_time_on_clear(cx: &mut TestAppContext) {
    let (_view, cx) = animated(cx, TWO_TAGS, vec![tag("a")]);
    let start = drawn_bar(cx);
    click(cx, CLEAR_ID);
    frame(cx);
    frame(cx);
    let end = natural(cx);
    assert!(end < start - 10.0, "fewer rows: {start} to {end}");
    wait(cx, 60);
    frame(cx);
    let middle = drawn_bar(cx);
    assert!(
        middle < start - 0.5 && middle > end + 0.5,
        "after 60 ms: {end} < {middle} < {start}"
    );
    wait(cx, 70);
    frame(cx);
    assert!(!moving(cx));
    assert!((drawn_bar(cx) - end).abs() < 0.5);
}

#[gpui_kit::test]
fn held_picks_retarget_and_end_one_motion_after_the_last(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, &long_note(8), vec![long(0)]);
    cx.simulate_resize(gpui_kit::size(px(320.), px(900.)));
    settle(cx);
    let mut last = drawn_bar(cx);
    // A pick every 30 ms, as a held Enter does: each one adds a pill.
    for i in 1..8 {
        toggle(&view, cx, long(i));
        frame(cx);
        let now = drawn_bar(cx);
        assert!(
            (now - last).abs() < 0.5,
            "a new pick goes on from the value on screen: {last} then {now}"
        );
        frame(cx);
        wait(cx, 30);
        frame(cx);
        let now = drawn_bar(cx);
        assert!(now >= last - 0.01, "it only grows: {last} then {now}");
        last = now;
    }
    let end = natural(cx);
    assert!(last <= end + 0.01, "never past the end: {last} <= {end}");
    // One motion after the last pick, not one per pick: 7 picks over 210
    // ms, and at rest 130 ms after the last.
    wait(cx, 100);
    frame(cx);
    assert!(!moving(cx), "at rest one motion after the last pick");
    assert!((drawn_bar(cx) - end).abs() < 0.5);
}

#[gpui_kit::test]
fn a_capped_bar_snaps_and_keeps_its_scroll(cx: &mut TestAppContext) {
    let picks: Vec<Facet> = (0..20).map(long).collect();
    let (view, cx) = animated(cx, &long_note(20), picks);
    cx.simulate_resize(gpui_kit::size(px(320.), px(260.)));
    settle(cx);
    let scrolled = gpui_kit::point(px(0.), px(-30.));
    cx.update(|window, cx| {
        view.read(cx).bar_scroll.set_offset(scrolled);
        window.refresh();
        window.render_frame(cx);
    });
    let max = cx.update(|_, cx| view.read(cx).bar_scroll.max_offset().y);
    assert!(max > px(30.), "capped: {max:?}");
    // Unpick the last pick: the bar stays capped.
    toggle(&view, cx, long(19));
    for _ in 0..3 {
        frame(cx);
        assert!(!moving(cx), "a capped bar is never framed");
        wait(cx, 30);
    }
    let offset = cx.update(|_, cx| view.read(cx).bar_scroll.offset());
    assert_eq!(offset, scrolled, "the bar keeps its scroll");
}

#[gpui_kit::test]
fn a_width_change_snaps_the_bar(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    toggle(&view, cx, tag("a"));
    frame(cx);
    frame(cx);
    wait(cx, 30);
    frame(cx);
    assert!(moving(cx), "on its way");
    cx.simulate_resize(gpui_kit::size(px(500.), px(600.)));
    frame(cx);
    frame(cx);
    assert!(!moving(cx), "a new width snaps the bar");
    assert!((drawn_bar(cx) - natural(cx)).abs() < 0.5);
}

#[gpui_kit::test]
fn the_first_frame_has_no_motion(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open_animated(cx, |cx| {
        FacetView::new(
            VaultPath::new("/notes/n.md"),
            TWO_TAGS.into(),
            vec![tag("a")],
            cx,
        )
    });
    // The view a session restores with picks: drawn once, at its height.
    assert!(!moving(cx), "the first frame draws no frame");
    let first = drawn_bar(cx);
    frame(cx);
    assert!(!moving(cx));
    assert!((drawn_bar(cx) - first).abs() < 0.5, "at its natural height");
    assert!((first - natural(cx)).abs() < 0.5);
}

#[gpui_kit::test]
fn a_stop_that_takes_the_keyboard_snaps_the_bar(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, &long_note(8), (0..3).map(long).collect());
    cx.simulate_resize(gpui_kit::size(px(320.), px(900.)));
    settle(cx);
    focus_view(&view, cx);
    tab_to(cx, pill_id(&long(2)), 20);
    let start = drawn_bar(cx);
    // A pick adds a pill in a new last row, and the bar grows.
    toggle(&view, cx, long(3));
    frame(cx);
    frame(cx);
    wait(cx, 30);
    frame(cx);
    assert!(moving(cx), "the bar grows");
    assert!(natural(cx) > drawn_bar(cx) + 5.0, "the new row is clipped");
    assert!(drawn_bar(cx) >= start);
    // Tab onto the new pill: it takes the keyboard, and the bar snaps so it
    // is seen whole.
    key(cx, "tab");
    frame(cx);
    assert!(has_focus(cx, pill_id(&long(3))));
    assert!(!moving(cx), "a stop that took the keyboard snaps the bar");
    assert!((drawn_bar(cx) - natural(cx)).abs() < 0.5);
    let pill = found(cx, pill_id(&long(3))).expect("the new pill");
    let bar = found(cx, FACET_BAR_ID).expect("the bar");
    assert!(pill.bottom() <= bar.bottom(), "the pill is seen whole");
}

#[gpui_kit::test]
fn a_clear_moves_the_keyboard_to_undo_and_still_eases(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![tag("a")]);
    focus_view(&view, cx);
    tab_to(cx, CLEAR_ID, 10);
    // Enter on `Clear`: `Undo` takes the keyboard once drawn. The bar
    // shrinks, so nothing is clipped, and it eases on.
    key(cx, "enter");
    frame(cx);
    frame(cx);
    assert!(has_focus(cx, UNDO_ID));
    wait(cx, 30);
    frame(cx);
    assert!(moving(cx), "a shrinking bar clips nothing and moves on");
}

#[gpui_kit::test]
fn reduced_motion_draws_the_end_height_at_once(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, TWO_TAGS, vec![]);
    settle(cx);
    let start = drawn_bar(cx);
    toggle(&view, cx, tag("a"));
    assert!(!moving(cx), "no frame under reduced motion");
    frame(cx);
    assert!(!moving(cx), "no frame under reduced motion");
    assert!(hint_frame(cx).is_none());
    assert!(found(cx, HINT_ID).is_none(), "the hint goes at once");
    let end = drawn_bar(cx);
    assert!(
        end > start + 10.0,
        "the end height at once: {start} to {end}"
    );
    assert!((end - natural(cx)).abs() < 0.5);
}

#[gpui_kit::test]
fn at_rest_the_bar_has_no_frame_and_its_natural_height(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    assert!(!moving(cx));
    toggle(&view, cx, tag("a"));
    frame(cx);
    frame(cx);
    let end = natural(cx);
    rest(cx, 20);
    assert!(!moving(cx), "no frame at rest");
    let bar = found(cx, FACET_BAR_ID).expect("the bar");
    assert!((f32::from(bar.size.height) - end).abs() < 0.5);
    let max = cx.update(|_, cx| view.read(cx).bar_scroll.max_offset().y);
    assert_eq!(max, px(0.), "nothing to scroll under the cap");
    assert!(
        (f32::from(bar.bottom()) - column_top(cx)).abs() < 0.5,
        "the column starts under the bar"
    );
}

#[gpui_kit::test]
fn no_correcting_frame_is_asked_while_the_bar_moves(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![tag("a")]);
    click(cx, CLEAR_ID);
    // The pick's new chunks are sized in their first frame.
    assert_no_correction(&view, cx, "the clear's frame");
    for step in 0..5 {
        wait(cx, 20);
        frame(cx);
        assert!(moving(cx), "the bar moves");
        assert_no_correction(&view, cx, &format!("moving frame {step}"));
    }
}

#[gpui_kit::test]
fn the_hint_collapses_with_the_bar_on_the_first_pick(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    let hint = f32::from(found(cx, HINT_ID).expect("the hint").size.height);
    assert!(hint > 5.0);
    let start = column_top(cx);
    toggle(&view, cx, tag("a"));
    frame(cx);
    frame(cx);
    let held = hint_frame(cx).expect("the hint collapses in a frame");
    assert!((f32::from(held.size.height) - hint).abs() < 0.5);
    assert!((column_top(cx) - start).abs() < 0.5, "nothing moved yet");
    wait(cx, 60);
    frame(cx);
    let middle = f32::from(hint_frame(cx).expect("still collapsing").size.height);
    assert!(middle > 0.5 && middle < hint - 0.5, "after 60 ms: {middle}");
    assert!(moving(cx), "the bar moves with it");
    let frame_bottom = hint_frame(cx).unwrap().bottom();
    assert!((f32::from(frame_bottom) - column_top(cx)).abs() < 0.5);
    wait(cx, 70);
    frame(cx);
    assert!(hint_frame(cx).is_none(), "the hint is gone");
    assert!(found(cx, HINT_ID).is_none());
    assert!(!moving(cx), "and the bar rests with it");
}

/// The element path of `id` in the last frame.
fn path_of(cx: &mut VisualTestContext, id: &'static str) -> Option<Vec<gpui_kit::ElementId>> {
    cx.update(|window, _| window.try_find(id).map(|e| e.path().to_vec()))
}

#[gpui_kit::test]
fn the_hint_frame_has_no_element_id_and_keeps_its_children_path(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    let at_rest = path_of(cx, HINT_ID).expect("the hint at rest");
    toggle(&view, cx, tag("a"));
    frame(cx);
    frame(cx);
    let framed = path_of(cx, HINT_ID).expect("the hint in its frame");
    assert_eq!(at_rest, framed, "the frame keeps the hint's element path");
    assert!(hint_frame(cx).is_some(), "the hint collapses in a frame");
}

#[gpui_kit::test]
fn both_frames_are_found_by_selector(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    toggle(&view, cx, tag("a"));
    frame(cx);
    frame(cx);
    assert!(bar_frame(cx).is_some(), "the bar's frame by its selector");
    assert!(hint_frame(cx).is_some(), "the hint's frame by its selector");
}
