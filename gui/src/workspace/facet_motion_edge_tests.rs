//! UI tests for the edges of the facet bar's motion (ADR-0060): a change on
//! the bar's second frame, a bar that comes back after it was hidden, a
//! growth past the height cap, and growth cases of the column tests. A
//! `FacetView` alone in a headless window with motion on.
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px};
use kimun_core::nfs::VaultPath;
use kimun_core::note::Facet;

use super::facet::{BAR_FRAME_ID, CLEAR_ID, FACET_BAR_ID, FacetView, HINT_FRAME_ID, UNDO_ID};
use super::facet_motion_tests::{
    TWO_TAGS, animated, column_top, drawn_bar, found, frame, long, long_note, moving, natural,
    rest, toggle, wait,
};
use super::facet_tests::{click, next_frame, settle, tag};

/// The drawn bar height of each frame, one frame every 20 ms, from the
/// next frame until the bar and the hint rest.
fn heights(cx: &mut VisualTestContext) -> Vec<f32> {
    let mut drawn = Vec::new();
    for _ in 0..20 {
        frame(cx);
        drawn.push(drawn_bar(cx));
        if !moving(cx) && found(cx, HINT_FRAME_ID).is_none() {
            return drawn;
        }
        wait(cx, 20);
    }
    panic!("the bar never rested: {drawn:?}");
}

/// The bar holds `start` first, then eases to `end`: never outside them,
/// one way only, and over more than one frame.
fn holds_then_eases(drawn: &[f32], start: f32, end: f32) {
    assert!(
        (drawn[0] - start).abs() < 0.5,
        "the first frame holds the old height {start}: {drawn:?}"
    );
    let (low, high) = (start.min(end), start.max(end));
    for pair in drawn.windows(2) {
        assert!(
            pair[1] >= low - 0.5 && pair[1] <= high + 0.5,
            "between {start} and {end}: {drawn:?}"
        );
        let step = (pair[1] - pair[0]) * (end - start).signum();
        assert!(step >= -0.01, "one way only: {drawn:?}");
    }
    let between = drawn
        .iter()
        .filter(|d| **d > low + 0.5 && **d < high - 0.5)
        .count();
    assert!(between >= 2, "it eases: {drawn:?}");
    assert!((drawn[drawn.len() - 1] - end).abs() < 0.5);
}

fn view_of(
    text: &str,
    picks: Vec<Facet>,
) -> impl FnOnce(&mut gpui_kit::Context<FacetView>) -> FacetView + 'static {
    let text = text.to_string();
    move |cx| FacetView::new(VaultPath::new("/notes/n.md"), text, picks, cx)
}

fn set_text(view: &Entity<FacetView>, cx: &mut VisualTestContext, text: &str) {
    view.update(cx, |view, cx| view.set_text(text.into(), cx));
}

/// The facet view `build` makes, in its own window, with motion on. Only
/// the frame the window draws as it opens is drawn.
fn open_undrawn(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut gpui_kit::Context<FacetView>) -> FacetView + 'static,
) -> (Entity<FacetView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::editor_view::init(cx);
        crate::keymap::init(
            cx,
            crate::keymap::Keymap::defaults(crate::keymap::Platform::LinuxWindows),
        );
    });
    let built: Rc<RefCell<Option<Entity<FacetView>>>> = Rc::default();
    let slot = built.clone();
    let (_root, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(build);
        *slot.borrow_mut() = Some(view.clone());
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = built.borrow_mut().take().expect("the view was built");
    (view, cx)
}

#[gpui_kit::test]
fn a_change_on_the_bars_second_frame_holds_then_eases(cx: &mut TestAppContext) {
    // The window draws the bar's first frame as it opens.
    let (view, cx) = open_undrawn(cx, view_of(TWO_TAGS, vec![]));
    let start = drawn_bar(cx);
    // The pick lands between the bar's first frame and its second.
    toggle(&view, cx, tag("a"));
    let drawn = heights(cx);
    holds_then_eases(&drawn, start, natural(cx));
}

#[gpui_kit::test]
fn a_bar_drawn_again_after_it_was_hidden_holds_then_eases(cx: &mut TestAppContext) {
    let picks: Vec<Facet> = (0..6).map(long).collect();
    let (view, cx) = animated(cx, &long_note(6), picks);
    cx.simulate_resize(gpui_kit::size(px(320.), px(900.)));
    settle(cx);
    let tall = drawn_bar(cx);
    // No facet: no bar. Then facets again: the bar comes back, shorter.
    set_text(&view, cx, "plain\n");
    frame(cx);
    assert!(found(cx, FACET_BAR_ID).is_none(), "the bar is hidden");
    set_text(&view, cx, TWO_TAGS);
    // The update draws the bar's first frame back. The pick lands before
    // its second frame.
    let start = drawn_bar(cx);
    toggle(&view, cx, tag("a"));
    assert!(start < tall - 10.0, "shorter than before: {start} {tall}");
    let drawn = heights(cx);
    holds_then_eases(&drawn, start, natural(cx));
}

#[gpui_kit::test]
fn a_growth_past_the_cap_snaps_to_the_cap(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    // A short view: the first pick takes the bar past its 40 percent cap.
    cx.simulate_resize(gpui_kit::size(px(800.), px(200.)));
    settle(cx);
    let start = drawn_bar(cx);
    assert!(start < 80.0, "under the cap: {start}");
    toggle(&view, cx, tag("a"));
    frame(cx);
    frame(cx);
    assert!(!moving(cx), "no ease frames past the cap");
    let bar = drawn_bar(cx);
    assert!((bar - 80.0).abs() < 0.5, "at the cap: {bar}");
    let (offset, max) = cx.update(|_, cx| {
        let scroll = &view.read(cx).bar_scroll;
        (scroll.offset().y, scroll.max_offset().y)
    });
    assert_eq!(offset, px(0.), "scrolled to the top");
    assert!(max > px(0.), "capped: {max:?}");
}

/// A view with picks, cleared and at rest: `Undo` grows the bar again.
fn cleared(cx: &mut TestAppContext) -> &mut VisualTestContext {
    let (_view, cx) = animated(cx, TWO_TAGS, vec![tag("a")]);
    click(cx, CLEAR_ID);
    rest(cx, 20);
    cx
}

#[gpui_kit::test]
fn the_column_follows_a_growing_bar_in_one_motion(cx: &mut TestAppContext) {
    let cx = cleared(cx);
    let start = column_top(cx);
    click(cx, UNDO_ID);
    frame(cx);
    frame(cx);
    let mut last = column_top(cx);
    let mut steps = 0;
    for _ in 0..8 {
        wait(cx, 20);
        frame(cx);
        let bar = found(cx, BAR_FRAME_ID).or_else(|| found(cx, FACET_BAR_ID));
        let top = column_top(cx);
        assert!(
            (f32::from(bar.unwrap().bottom()) - top).abs() < 0.5,
            "the column starts under the bar"
        );
        assert!(top >= last - 0.01, "one way only: {last} then {top}");
        if top > last + 0.01 {
            steps += 1;
        }
        last = top;
    }
    assert!(last > start + 10.0, "the bar grew: {start} to {last}");
    assert!(steps >= 3, "the column moves over several frames: {steps}");
}

#[gpui_kit::test]
fn no_correcting_frame_is_asked_while_the_bar_grows(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![tag("a")]);
    click(cx, CLEAR_ID);
    rest(cx, 20);
    click(cx, UNDO_ID);
    // The undo's own new chunks may ask for one correcting frame.
    frame(cx);
    next_frame(cx);
    frame(cx);
    next_frame(cx);
    for _ in 0..5 {
        wait(cx, 20);
        frame(cx);
        assert!(moving(cx), "the bar grows");
        let asked = view.read_with(cx, |view, _| view.resize_asked());
        assert!(!asked, "a growing bar asks for no correcting frame");
    }
}

#[gpui_kit::test]
fn the_bar_and_the_hint_move_the_column_one_way_on_the_first_pick(cx: &mut TestAppContext) {
    let (view, cx) = animated(cx, TWO_TAGS, vec![]);
    toggle(&view, cx, tag("a"));
    let mut tops = Vec::new();
    for _ in 0..20 {
        frame(cx);
        tops.push(column_top(cx));
        if !moving(cx) && found(cx, HINT_FRAME_ID).is_none() {
            break;
        }
        wait(cx, 20);
    }
    let up = tops.windows(2).any(|pair| pair[1] < pair[0] - 0.01);
    let down = tops.windows(2).any(|pair| pair[1] > pair[0] + 0.01);
    assert!(!(up && down), "the column moves one way only: {tops:?}");
}
