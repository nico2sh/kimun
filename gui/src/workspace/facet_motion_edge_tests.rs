//! UI tests for the edges of the facet bar's motion (ADR-0060): a change on
//! the bar's second frame, a bar that comes back after it was hidden, a
//! growth past the height cap, growth cases of the column tests, and an
//! open list that stays open and in place through a pick while the bar
//! moves. A `FacetView` alone in a headless window with motion on.
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, Entity, MouseButton, TestAppContext, VisualTestContext, px};
use kimun_core::nfs::VaultPath;
use kimun_core::note::Facet;

use super::facet::{
    CLEAR_ID, FACET_BAR_ID, FacetView, HINT_FRAME_ID, ListKind, MORE_LIST_ID, UNDO_ID, menu_id,
    more_item_id, pill_id, row_tick_id,
};
use super::facet_motion_tests::{
    TWO_TAGS, animated, bar_frame, column_top, drawn_bar, found, frame, long, long_note, moving,
    natural, rest, toggle, wait,
};
use super::facet_tests::{
    click, field_focused, focus_view, has_focus, key, next_frame, open_animated, open_with, picks,
    settle, tag,
};

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
        let bar = bar_frame(cx).or_else(|| found(cx, FACET_BAR_ID));
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

// ── a list stays open through picks while the bar moves ─────────────────

/// How a pick in the open list is made.
#[derive(Clone, Copy, Debug)]
enum PickBy {
    /// A press and a release on the row, with a frame between them.
    Click,
    /// Down to the row, then Enter.
    Enter,
}

/// What one frame showed of the open list: whether the view holds it open,
/// whether it was drawn, whether its field held the keyboard, and the
/// element paths of the list and of the chip it hangs from. The kit keys
/// the list's popover state by its path: a new path drops that state and
/// builds it again.
#[derive(Debug)]
struct ListFrame {
    moving: bool,
    open: bool,
    drawn: bool,
    field: bool,
    list_path: Option<Vec<gpui_kit::ElementId>>,
    chip_path: Option<Vec<gpui_kit::ElementId>>,
}

/// The element path of `id` in the last frame.
fn path_of(
    cx: &mut VisualTestContext,
    id: impl Into<gpui_kit::SharedString>,
) -> Option<Vec<gpui_kit::ElementId>> {
    let id: gpui_kit::SharedString = id.into();
    cx.update(|window, _| window.try_find(id).map(|e| e.path().to_vec()))
}

/// The open list as the last frame drew it.
fn list_frame(view: &Entity<FacetView>, cx: &mut VisualTestContext, kind: ListKind) -> ListFrame {
    let open = view.read_with(cx, |view, _| view.list_is_open(kind));
    ListFrame {
        moving: moving(cx),
        open,
        drawn: found(cx, MORE_LIST_ID).is_some(),
        field: open && field_focused(view, cx),
        list_path: path_of(cx, MORE_LIST_ID),
        chip_path: path_of(cx, menu_id(kind)),
    }
}

const LIST_NOTE: &str = "#a #b #c #d\n\nsome text\n";

/// A view of [`LIST_NOTE`] with `picks`, motion on (`motion`) or reduced,
/// at rest, with `kind`'s list open (by `/` for the list of every kind).
fn with_list_open(
    cx: &mut TestAppContext,
    motion: bool,
    kind: ListKind,
    picks: Vec<Facet>,
) -> (Entity<FacetView>, &mut VisualTestContext) {
    let build = view_of(LIST_NOTE, picks);
    let (view, cx, _events, sub) = if motion {
        open_animated(cx, build)
    } else {
        open_with(cx, build)
    };
    drop(sub);
    settle(cx);
    if kind == ListKind::All {
        focus_view(&view, cx);
        key(cx, "/");
    } else {
        click(cx, menu_id(kind));
    }
    rest(cx, 20);
    assert!(view.read_with(cx, |view, _| view.list_is_open(kind)));
    (view, cx)
}

/// Pick the first unpicked row of the open list `by` a click or Enter,
/// then draw a frame every 20 ms over the whole motion and past it. Each
/// frame from the press on, and the picked facet.
fn pick_and_watch(
    view: &Entity<FacetView>,
    cx: &mut VisualTestContext,
    kind: ListKind,
    by: PickBy,
) -> (Vec<ListFrame>, Facet) {
    let picked = picks(view, cx);
    let rows = view.read_with(cx, |view, cx| view.list_results(cx));
    let at = rows.iter().position(|f| !picked.contains(f)).unwrap();
    let facet = rows[at].clone();
    let mut seen = Vec::new();
    match by {
        PickBy::Click => {
            // The pointer goes to the row, presses, and a frame is drawn
            // before it lets go, as on screen.
            let point = found(cx, more_item_id(&facet)).expect("the row").center();
            cx.update(|window, cx| {
                window.pointer_move(point, None, cx);
                window.render_frame(cx);
                window.pointer_down(point, MouseButton::Left, cx);
            });
            cx.run_until_parked();
            frame(cx);
            seen.push(list_frame(view, cx, kind));
            cx.update(|window, cx| window.pointer_up(point, MouseButton::Left, cx));
            cx.run_until_parked();
        }
        PickBy::Enter => {
            for _ in 0..at {
                key(cx, "down");
            }
            key(cx, "enter");
        }
    }
    for _ in 0..12 {
        frame(cx);
        seen.push(list_frame(view, cx, kind));
        wait(cx, 20);
    }
    (seen, facet)
}

/// A pick in `kind`'s list (`by` a click or Enter, after `picks_before`)
/// leaves the list open, drawn and holding the keyboard in every frame,
/// in one place in the element tree, its rows in their order, the picked
/// row ticked and lit, and the picks row holding the new pill.
fn stays_open_through_a_pick(
    cx: &mut TestAppContext,
    motion: bool,
    kind: ListKind,
    by: PickBy,
    picks_before: Vec<Facet>,
) {
    let (view, cx) = with_list_open(cx, motion, kind, picks_before.clone());
    let before = view.read_with(cx, |view, cx| view.list_results(cx));
    frame(cx);
    let at_rest = list_frame(&view, cx, kind);
    let (seen, facet) = pick_and_watch(&view, cx, kind, by);
    let case = format!("motion {motion}, {kind:?}, {by:?}, picks {picks_before:?}");
    // The first pick adds the picks row and the summary: the bar grows
    // over several frames. A second pick on the same line may only
    // measure, in a frame the test cannot catch.
    if motion && picks_before.is_empty() {
        assert!(
            seen.iter().any(|f| f.moving),
            "{case}: the bar moved: {seen:?}"
        );
    }
    for (i, f) in seen.iter().enumerate() {
        assert!(
            f.open && f.drawn && f.field,
            "{case}: frame {i} lost the list: {seen:#?}"
        );
        assert_eq!(
            (&f.list_path, &f.chip_path),
            (&at_rest.list_path, &at_rest.chip_path),
            "{case}: frame {i} (moving {}) moved the list in the element tree",
            f.moving
        );
    }
    assert!(picks(&view, cx).contains(&facet), "{case}: picked");
    assert!(found(cx, row_tick_id(&facet)).is_some(), "{case}: ticked");
    assert!(found(cx, pill_id(&facet)).is_some(), "{case}: the pill");
    let after = view.read_with(cx, |view, cx| view.list_results(cx));
    assert_eq!(before, after, "{case}: the order is kept");
    let lit = cx.update(|window, _| {
        window
            .try_find(more_item_id(&facet))
            .and_then(|e| e.selected())
    });
    assert_eq!(lit, Some(true), "{case}: the picked row is lit");
}

#[gpui_kit::test]
fn a_first_click_pick_keeps_the_tags_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::Tags, PickBy::Click, vec![]);
}

#[gpui_kit::test]
fn a_second_click_pick_keeps_the_tags_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::Tags, PickBy::Click, vec![tag("a")]);
}

#[gpui_kit::test]
fn a_first_enter_pick_keeps_the_tags_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::Tags, PickBy::Enter, vec![]);
}

#[gpui_kit::test]
fn a_second_enter_pick_keeps_the_tags_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::Tags, PickBy::Enter, vec![tag("a")]);
}

#[gpui_kit::test]
fn a_first_click_pick_keeps_the_find_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::All, PickBy::Click, vec![]);
}

#[gpui_kit::test]
fn a_second_click_pick_keeps_the_find_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::All, PickBy::Click, vec![tag("a")]);
}

#[gpui_kit::test]
fn a_first_enter_pick_keeps_the_find_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::All, PickBy::Enter, vec![]);
}

#[gpui_kit::test]
fn a_second_enter_pick_keeps_the_find_list_open_with_motion(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, true, ListKind::All, PickBy::Enter, vec![tag("a")]);
}

#[gpui_kit::test]
fn a_first_click_pick_keeps_the_tags_list_open_reduced(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, false, ListKind::Tags, PickBy::Click, vec![]);
}

#[gpui_kit::test]
fn a_second_enter_pick_keeps_the_tags_list_open_reduced(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, false, ListKind::Tags, PickBy::Enter, vec![tag("a")]);
}

#[gpui_kit::test]
fn a_first_enter_pick_keeps_the_find_list_open_reduced(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, false, ListKind::All, PickBy::Enter, vec![]);
}

#[gpui_kit::test]
fn a_second_click_pick_keeps_the_find_list_open_reduced(cx: &mut TestAppContext) {
    stays_open_through_a_pick(cx, false, ListKind::All, PickBy::Click, vec![tag("a")]);
}

#[gpui_kit::test]
fn escape_while_the_bar_moves_closes_the_list_and_hands_the_keyboard_to_its_chip(
    cx: &mut TestAppContext,
) {
    let (view, cx) = with_list_open(cx, true, ListKind::Tags, vec![]);
    click(cx, more_item_id(&tag("a")));
    frame(cx);
    assert!(moving(cx), "the bar moves");
    key(cx, "escape");
    assert!(!view.read_with(cx, |view, _| view.list_is_open(ListKind::Tags)));
    rest(cx, 20);
    assert!(found(cx, MORE_LIST_ID).is_none(), "the list is gone");
    assert!(
        has_focus(cx, menu_id(ListKind::Tags)),
        "the chip has the keyboard"
    );
    assert_eq!(picks(&view, cx), [tag("a")]);
}
