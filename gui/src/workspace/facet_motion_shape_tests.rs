//! UI tests for when the facet bar asks to move (ADR-0060): only a change
//! in what the bar draws marks it. `Show more`, `Show all`, a gap reveal
//! and a text reload that keep the bar's rows ask for no measuring frame.
//! A new summary text marks it. A `FacetView` alone in a headless window
//! with motion on.
#![cfg(test)]

use gpui_kit::{Entity, TestAppContext, VisualTestContext, px, size};
use kimun_core::note::Facet;

use super::facet::{FacetView, SHOW_MORE_ID, gap_down_id, show_all_id};
use super::facet_motion_tests::{
    TWO_TAGS, animated, drawn_bar, frame, moving, natural, rest, toggle, wait,
};
use super::facet_tests::{click, tag, to_bottom};

/// How many measuring frames the bar drew so far.
fn measures(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| view.bar_measure_frames())
}

/// No frame since `before` measured the bar, and the frames that follow
/// draw it at rest. The action's own update may have drawn a frame
/// already: the last frame is checked before a new one is drawn.
fn stays_at_rest(view: &Entity<FacetView>, cx: &mut VisualTestContext, before: usize) {
    assert!(!moving(cx), "the bar is not framed");
    for _ in 0..2 {
        frame(cx);
        assert!(!moving(cx), "the bar is not framed");
    }
    assert_eq!(measures(view, cx), before, "no measuring frame");
}

fn set_text(view: &Entity<FacetView>, cx: &mut VisualTestContext, text: &str) {
    view.update(cx, |view, cx| view.set_text(text.into(), cx));
}

/// A view of `text` with `picks`, motion on, at rest.
fn resting<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    picks: Vec<Facet>,
) -> (Entity<FacetView>, &'a mut VisualTestContext) {
    let (view, cx) = animated(cx, text, picks);
    rest(cx, 20);
    (view, cx)
}

#[gpui_kit::test]
fn show_more_asks_for_no_measuring_frame(cx: &mut TestAppContext) {
    let text: String = (0..150)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (view, cx) = resting(cx, &text, vec![tag("t")]);
    to_bottom(&view, cx);
    let before = view.read_with(cx, |view, _| view.chunks().len());
    let measured = measures(&view, cx);
    click(cx, SHOW_MORE_ID);
    let after = view.read_with(cx, |view, _| view.chunks().len());
    assert!(after > before, "more pieces: {before} to {after}");
    stays_at_rest(&view, cx, measured);
}

#[gpui_kit::test]
fn a_gap_reveal_asks_for_no_measuring_frame(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n";
    let (view, cx) = resting(cx, text, vec![tag("x")]);
    let before = view.read_with(cx, |view, cx| view.chunks()[0].read(cx).editor.get_text());
    let measured = measures(&view, cx);
    click(cx, gap_down_id(1));
    let after = view.read_with(cx, |view, cx| view.chunks()[0].read(cx).editor.get_text());
    assert_ne!(before, after, "the gap revealed a block");
    stays_at_rest(&view, cx, measured);
}

#[gpui_kit::test]
fn a_text_reload_that_keeps_the_bar_shape_asks_for_no_measuring_frame(cx: &mut TestAppContext) {
    let (view, cx) = resting(cx, TWO_TAGS, vec![tag("a")]);
    let measured = measures(&view, cx);
    set_text(&view, cx, "#a #b\n\nother text\n");
    stays_at_rest(&view, cx, measured);
}

#[gpui_kit::test]
fn a_pick_still_marks_the_bar(cx: &mut TestAppContext) {
    let (view, cx) = resting(cx, TWO_TAGS, vec![]);
    let measured = measures(&view, cx);
    toggle(&view, cx, tag("a"));
    frame(cx);
    assert!(moving(cx), "the bar is framed");
    assert_eq!(measures(&view, cx), measured + 1, "one measuring frame");
}

#[gpui_kit::test]
fn a_count_change_without_a_pick_marks_the_bar(cx: &mut TestAppContext) {
    let (view, cx) = resting(cx, TWO_TAGS, vec![]);
    let measured = measures(&view, cx);
    // A third tag: the `Tags` chip counts 3, not 2.
    set_text(&view, cx, "#a #b #c\n\nsome text\n");
    frame(cx);
    assert_eq!(
        measures(&view, cx),
        measured + 1,
        "a new chip count draws a measuring frame"
    );
}

#[gpui_kit::test]
fn show_all_asks_for_no_measuring_frame(cx: &mut TestAppContext) {
    // The gap after the only piece hides two blocks. `Show all` adds them
    // to that piece: still one piece.
    let text = "#x p0\n\np1\n\np2\n";
    let (view, cx) = resting(cx, text, vec![tag("x")]);
    let before = view.read_with(cx, |view, cx| view.chunks()[0].read(cx).editor.get_text());
    let measured = measures(&view, cx);
    click(cx, show_all_id(1));
    let after = view.read_with(cx, |view, cx| view.chunks()[0].read(cx).editor.get_text());
    assert_ne!(before, after, "the gap revealed its blocks");
    stays_at_rest(&view, cx, measured);
}

/// Two pieces with one block between them. Its reveal joins them in one
/// piece, so the summary says `1 piece`, not `2 pieces`.
const JOINS: &str = "#x p0\n\np1\n\n#x p2\n";

/// The view at `width`, with [`JOINS`] loaded again (no block revealed),
/// at rest. The bar's natural height.
fn reload_at(view: &Entity<FacetView>, cx: &mut VisualTestContext, width: f32) -> f32 {
    cx.simulate_resize(size(px(width), px(900.)));
    set_text(view, cx, JOINS);
    rest(cx, 20);
    natural(cx)
}

#[gpui_kit::test]
fn a_new_summary_text_that_wraps_the_row_eases_the_bar(cx: &mut TestAppContext) {
    let (view, cx) = resting(cx, JOINS, vec![tag("x")]);
    // Find a width where the new summary text wraps its row in another
    // number of lines. Pill and chip counts stay the same.
    let mut wraps = None;
    for width in (160..480).step_by(2) {
        let width = width as f32;
        let before = reload_at(&view, cx, width);
        click(cx, gap_down_id(1));
        rest(cx, 20);
        let after = natural(cx);
        if (after - before).abs() > 0.5 {
            wraps = Some((width, before, after));
            break;
        }
    }
    let (width, start, end) = wraps.expect("a width where the summary wraps otherwise");
    let again = reload_at(&view, cx, width);
    assert!((again - start).abs() < 0.5, "the reload undid the reveal");
    let measured = measures(&view, cx);
    click(cx, gap_down_id(1));
    let mut drawn = vec![drawn_bar(cx)];
    for _ in 0..12 {
        frame(cx);
        drawn.push(drawn_bar(cx));
        wait(cx, 20);
    }
    assert!(
        measures(&view, cx) > measured,
        "a measuring frame: {drawn:?}"
    );
    let (low, high) = (start.min(end), start.max(end));
    assert!(
        drawn.iter().any(|d| *d > low + 0.5 && *d < high - 0.5),
        "the bar eases from {start} to {end}: {drawn:?}"
    );
}
