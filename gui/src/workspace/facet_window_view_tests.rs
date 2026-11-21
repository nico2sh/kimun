//! UI tests for the drawn window of the **Facet view**: only the pieces
//! near the viewport are drawn, two spacers stand in for the rest, and the
//! piece with the keyboard or the mouse stays drawn. The column is as tall
//! as when every piece is drawn.

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{
    Entity, Focusable as _, MouseButton, TestAppContext, VisualTestContext, point, px, size,
};
use kimun_core::nfs::VaultPath;

use super::facet::{EMPTY_ID, FacetView, MemberInit, SHOW_MORE_ID, chunk_id, gap_down_id, open_id};
use super::facet_tests::{cell, chunk, focus_view, has_focus, open_with, settle, tab_to, tag};
use crate::editor_font::EditorFontSetting;
use crate::editor_view::measure_probe;

/// Pieces a member gives under the pick: three tagged paragraphs.
const PER_MEMBER: usize = 3;
const MEMBERS: usize = 50;

fn words(n: usize, stem: &str) -> String {
    (0..n).map(|i| format!("{stem}{i} ")).collect()
}

pub(super) fn clock() -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt(10, 42, 0).unwrap()
}

/// A member's text: three tagged paragraphs of mixed length, an untagged
/// block between them and one after, so the gaps hide blocks.
pub(super) fn member_text(m: usize) -> String {
    let long = |k: usize| words(4 + (m * 7 + k * 11) % 40, &format!("m{m}k{k}w"));
    format!(
        "#a {}\n\nskip {m}\n\n#a {}\n\nskip\n\n#a {}\n\ntail {m}\n",
        long(0),
        long(1),
        long(2)
    )
}

/// A collection of 50 notes picked to `#a`: 150 pieces, 100 drawn under
/// the cap, in a window 600 by 600, settled.
pub(super) fn hundred(cx: &mut TestAppContext) -> (Entity<FacetView>, &mut VisualTestContext) {
    let members: Vec<MemberInit> = (0..MEMBERS)
        .map(|m| MemberInit {
            path: VaultPath::new(format!("/notes/n{m}.md")),
            title: format!("Note {m}"),
            text: member_text(m),
        })
        .collect();
    let (view, cx, _events, _sub) = open_with(cx, move |cx| {
        FacetView::new_collection_at("#q".into(), members, MEMBERS, vec![tag("a")], clock, cx)
    });
    cx.simulate_resize(size(px(600.), px(600.)));
    settle(cx);
    let n = view.read_with(cx, |v, _| v.chunks().len());
    assert_eq!(
        n,
        100,
        "the cap draws 100 of {} pieces",
        MEMBERS * PER_MEMBER
    );
    (view, cx)
}

/// The pieces the last frame drew.
pub(super) fn drawn(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<usize> {
    let n = view.read_with(cx, |v, _| v.chunks().len());
    cx.update(|window, _| {
        (0..n)
            .filter(|&i| window.try_find(chunk_id(i)).is_some())
            .collect()
    })
}

/// Scroll the column to `y` (from the top, positive) and draw one frame.
pub(super) fn scroll_to(view: &Entity<FacetView>, y: f32, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.read(cx).scroll.set_offset(point(px(0.), px(-y)));
        window.refresh();
        window.render_frame(cx);
    });
}

pub(super) fn offset(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> f32 {
    view.read_with(cx, |v, _| -f32::from(v.scroll.offset().y))
}

pub(super) fn max_offset(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> f32 {
    view.read_with(cx, |v, _| f32::from(v.scroll.max_offset().y))
}

pub(super) fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.render_frame(cx);
    });
}

/// What `read` reads in a frame that draws every piece. The view draws
/// only the near pieces again after.
pub(super) fn with_all<R>(
    view: &Entity<FacetView>,
    cx: &mut VisualTestContext,
    read: impl FnOnce(&mut VisualTestContext) -> R,
) -> R {
    view.update(cx, |v, _| v.set_draw_all(true));
    draw(cx);
    let value = read(cx);
    view.update(cx, |v, _| v.set_draw_all(false));
    draw(cx);
    value
}

/// The column's height in this frame is the height it has with every
/// piece drawn, within a pixel.
pub(super) fn assert_full_height(view: &Entity<FacetView>, cx: &mut VisualTestContext, what: &str) {
    let max = max_offset(view, cx);
    let full = with_all(view, cx, |cx| max_offset(view, cx));
    assert!((max - full).abs() <= 1.0, "{what}: {max} against {full}");
}

#[gpui_kit::test]
fn a_collection_of_100_pieces_draws_only_the_pieces_near_the_viewport(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let drawn = drawn(&view, cx);
    assert!(drawn.contains(&0), "the first piece: {drawn:?}");
    assert!(drawn.len() < 40, "near pieces only: {drawn:?}");
    let range = view.read_with(cx, |v, _| v.drawn_range());
    assert_eq!(range.start, 0);
    assert_eq!(
        drawn,
        range.collect::<Vec<_>>(),
        "the range is what is drawn"
    );
}

#[gpui_kit::test]
fn the_column_is_as_tall_as_the_sum_of_its_pieces_in_every_frame(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    assert_full_height(&view, cx, "the first frame");
    for width in [480., 731., 600.] {
        cx.simulate_resize(size(px(width), px(600.)));
        assert_full_height(&view, cx, &format!("a resize to {width}"));
    }
    for y in [900., 4000., 12000.] {
        scroll_to(&view, y, cx);
        assert_full_height(&view, cx, &format!("scrolled to {y}"));
    }
}

#[gpui_kit::test]
fn scrolling_moves_the_drawn_range_and_keeps_the_total_height(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let max = max_offset(&view, cx);
    assert!(max > 6000., "a long column: {max}");
    let mut last = 0;
    for step in 1..=8 {
        scroll_to(&view, max * step as f32 / 8., cx);
        let range = view.read_with(cx, |v, _| v.drawn_range());
        // Scrolled down, the later pieces are drawn, not the first ones.
        assert!(range.start >= last, "step {step}: {range:?}");
        assert!(range.start > 0, "step {step}: {range:?}");
        last = range.start;
        assert_eq!(max_offset(&view, cx), max, "step {step}: the height");
        assert!(!drawn(&view, cx).contains(&0), "step {step}");
    }
    assert!(drawn(&view, cx).contains(&99), "the last piece at the end");
}

#[gpui_kit::test]
fn a_pick_a_reveal_and_a_reload_keep_the_scroll_offset(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    scroll_to(&view, 5000., cx);
    // A pick of the tag again: the same pieces, planned again.
    view.update(cx, |v, cx| v.toggle(tag("a"), cx));
    view.update(cx, |v, cx| v.toggle(tag("a"), cx));
    draw(cx);
    assert_eq!(offset(&view, cx), 5000., "a pick");
    assert_full_height(&view, cx, "a pick");
    // A reveal in a drawn gap.
    let first = view.read_with(cx, |v, _| v.drawn_range().start);
    let g = (first..first + 200)
        .find(|&g| {
            cx.update(|window, _| window.try_find(gap_down_id(g)).is_some_and(|e| e.visible()))
        })
        .expect("a drawn gap");
    cx.update(|window, cx| window.click(gap_down_id(g), cx));
    cx.run_until_parked();
    draw(cx);
    assert_eq!(offset(&view, cx), 5000., "a reveal");
    assert_full_height(&view, cx, "a reveal");
    // A reload of a member's text.
    view.update(cx, |v, cx| {
        v.set_member_text(&VaultPath::new("/notes/n3.md"), member_text(4), cx)
    });
    draw(cx);
    assert_eq!(offset(&view, cx), 5000., "a reload");
    assert_full_height(&view, cx, "a reload");
}

#[gpui_kit::test]
fn the_focused_piece_is_drawn_even_when_it_is_far_from_the_viewport(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let piece = chunk(&view, 0, cx);
    cx.update(|window, cx| {
        let handle = piece.read(cx).focus_handle.clone();
        window.focus(&handle, cx);
    });
    let max = max_offset(&view, cx);
    scroll_to(&view, max, cx);
    let now = drawn(&view, cx);
    assert!(now.contains(&0), "the focused piece: {now:?}");
    assert!(now.contains(&99) && !now.contains(&30), "{now:?}");
    let focused = cx.update(|window, cx| piece.read(cx).focus_handle.is_focused(window));
    assert!(focused, "the piece keeps the keyboard");
    assert_full_height(&view, cx, "with a pinned piece");
    // A stop of the piece with the keyboard pins it too.
    scroll_to(&view, 0., cx);
    focus_view(&view, cx);
    tab_to(cx, open_id(0), 30);
    scroll_to(&view, max, cx);
    assert!(has_focus(cx, open_id(0)), "the stop keeps the keyboard");
    assert!(!drawn(&view, cx).contains(&30));
}

#[gpui_kit::test]
fn a_piece_in_a_drag_stays_drawn_until_release(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let from = cell(&view, 0, 1, 0, cx);
    let to = cell(&view, 0, 8, 0, cx);
    cx.update(|window, cx| {
        window.pointer_down(from, MouseButton::Left, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
    });
    cx.run_until_parked();
    let piece = chunk(&view, 0, cx);
    assert!(piece.read_with(cx, |p, _| p.dragging()), "a drag");
    // The keyboard leaves the piece: only the drag keeps it drawn.
    focus_view(&view, cx);
    let max = max_offset(&view, cx);
    scroll_to(&view, max, cx);
    assert!(drawn(&view, cx).contains(&0), "drawn while it drags");
    cx.update(|window, cx| window.pointer_up(to, MouseButton::Left, cx));
    cx.run_until_parked();
    assert!(
        !piece.read_with(cx, |p, _| p.dragging()),
        "the release ends it"
    );
    draw(cx);
    assert!(!drawn(&view, cx).contains(&0), "undrawn after the release");
}

#[gpui_kit::test]
fn ctrl_c_copies_the_selection_of_a_far_pinned_piece(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let piece = chunk(&view, 0, cx);
    cx.update(|window, cx| {
        let handle = piece.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        piece.update(cx, |p, _| {
            p.editor.buffer_mut().set_selection((0, 3), (0, 10));
            p.editor.sync_highlight();
        });
    });
    cx.run_until_parked();
    let max = max_offset(&view, cx);
    scroll_to(&view, max, cx);
    cx.simulate_keystrokes("ctrl-c");
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    let want: String = member_text(0).chars().skip(3).take(7).collect();
    assert_eq!(clip, Some(want), "the far piece copies");
}

#[gpui_kit::test]
fn show_more_and_the_empty_message_are_drawn_after_the_trailing_spacer(cx: &mut TestAppContext) {
    // A collection whose notes are empty draws the message, and no spacer.
    let empty = MemberInit {
        path: VaultPath::new("/notes/e.md"),
        title: "E".into(),
        text: String::new(),
    };
    let (_empty, ecx, _events, _sub) = open_with(cx, move |cx| {
        FacetView::new_collection_at("#q".into(), vec![empty], 1, vec![], clock, cx)
    });
    let shown = ecx.update(|window, _| window.try_find(EMPTY_ID).is_some());
    assert!(shown, "the empty message");
    // `Show more` stands where it stands with every piece drawn.
    let (view, cx) = hundred(cx);
    let top = |cx: &mut VisualTestContext| {
        cx.update(|window, _| f32::from(window.find(SHOW_MORE_ID).bounds().top()))
    };
    let at = top(cx);
    let full = with_all(&view, cx, |cx| top(cx));
    assert!(
        (at - full).abs() <= 1.0,
        "Show more at {at}, {full} drawn whole"
    );
    assert!(!drawn(&view, cx).contains(&99));
    let max = max_offset(&view, cx);
    scroll_to(&view, max, cx);
    let at = top(cx);
    let full = with_all(&view, cx, |cx| top(cx));
    assert!(
        (at - full).abs() <= 1.0,
        "at the end: {at}, {full} drawn whole"
    );
}

#[gpui_kit::test]
fn a_font_size_change_clears_the_height_table_and_keeps_the_layout_right(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    scroll_to(&view, 3000., cx);
    let before = max_offset(&view, cx);
    cx.update(|_, cx| {
        cx.set_global(EditorFontSetting {
            family: None,
            size: Some(px(19.)),
        })
    });
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.checked_items()) > 0);
    let after = max_offset(&view, cx);
    assert!(after > before + 100., "taller rows: {before} to {after}");
    assert_full_height(&view, cx, "a font size change");
}

#[gpui_kit::test]
fn the_height_table_matches_every_drawn_item_within_one_pixel(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    let max = max_offset(&view, cx);
    for step in 0..=20 {
        // Each frame checks its drawn pieces; a mismatch panics.
        scroll_to(&view, max * step as f32 / 20., cx);
        let checked = view.read_with(cx, |v, _| v.checked_items());
        assert!(checked > 3, "step {step}: {checked} checked");
    }
    for width in [333., 900.] {
        cx.simulate_resize(size(px(width), px(500.)));
        assert!(view.read_with(cx, |v, _| v.checked_items()) > 3);
    }
}

#[gpui_kit::test]
#[should_panic(expected = "height table")]
fn a_forged_height_trips_the_table_check(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    view.update(cx, |v, _| v.forge_crumbs(px(3.)));
    scroll_to(&view, 2000., cx);
}

/// How many layouts the pieces not drawn in the last frame asked for
/// since the last call.
fn undrawn_layouts(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> usize {
    let log = measure_probe::take();
    let drawn = drawn(view, cx);
    let chunks = view.read_with(cx, |v, _| v.chunks().to_vec());
    chunks
        .iter()
        .enumerate()
        .filter(|(i, _)| !drawn.contains(i))
        .map(|(_, c)| log.get(&c.entity_id()).map_or(0, |m| m.calls))
        .sum()
}

#[gpui_kit::test]
fn a_steady_or_scroll_frame_lays_out_no_undrawn_piece(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    measure_probe::take();
    for _ in 0..3 {
        draw(cx);
    }
    assert_eq!(undrawn_layouts(&view, cx), 0, "steady frames");
    for step in 1..=10 {
        scroll_to(&view, 400. * step as f32, cx);
        assert_eq!(undrawn_layouts(&view, cx), 0, "scroll frame {step}");
    }
    cx.simulate_resize(size(px(640.), px(600.)));
    assert!(
        undrawn_layouts(&view, cx) > 0,
        "a resize lays them out again"
    );
}

/// Piece `i`'s rows in the height table, and its real wrap at the width
/// the table measured at.
fn table_and_real(
    view: &Entity<FacetView>,
    i: usize,
    cx: &mut VisualTestContext,
) -> (Option<usize>, usize) {
    cx.update(|window, cx| {
        let (rows, width) = view.read(cx).table_rows(i, cx);
        let pane = window.rem_size() * 2.;
        let piece = view.read(cx).chunks()[i].clone();
        let real = piece.update(cx, |p, cx| p.rows_at(width - pane, window, cx));
        (rows, real)
    })
}

#[gpui_kit::test]
fn a_font_family_change_clears_the_rows_of_undrawn_pieces(cx: &mut TestAppContext) {
    let (view, cx) = hundred(cx);
    // The far piece that wraps most: a new face changes its wrap.
    let far = (60..100)
        .max_by_key(|&i| table_and_real(&view, i, cx).1)
        .unwrap();
    assert!(!drawn(&view, cx).contains(&far));
    let (rows, real) = table_and_real(&view, far, cx);
    assert_eq!(rows, Some(real), "measured before the change");
    // The headless text system has no fonts, so every family wraps the
    // same here. What shows the cache cleared: the new family lays the
    // undrawn pieces out again, which a steady frame never does.
    measure_probe::take();
    cx.update(|_, cx| {
        cx.set_global(EditorFontSetting {
            family: Some("DejaVu Sans".into()),
            size: None,
        })
    });
    draw(cx);
    assert!(!drawn(&view, cx).contains(&far));
    let laid_out = undrawn_layouts(&view, cx);
    assert!(laid_out > 50, "the new family lays them out: {laid_out}");
    let (rows, real) = table_and_real(&view, far, cx);
    assert_eq!(rows, Some(real), "the far piece wraps in the new face");
    assert_full_height(&view, cx, "a font family change");
}
