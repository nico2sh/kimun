//! UI tests for the **Facet view** entity (ADR-0061): a `FacetView` alone in
//! a headless window, driven by clicks on its pills, gap buttons and chunks.
//! The workspace side (a group showing one) is tested further down, by the
//! next task.
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, MouseButton, Pixels, Point, SharedString,
    Subscription, TestAppContext, VisualTestContext, point, px,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, FacetKind, MAX_RANGES};
use kimun_notes::logic::editor::claim::FollowTarget;

use super::facet::{
    CLEAR_ID, COLUMN_ID, EMPTY_ID, FACET_BAR_ID, FacetEvent, FacetView, HINT_ID, ListKind,
    MORE_LIST_ID, REMOVE_LAST_ID, SHOW_MORE_ID, SUMMARY_ID, chunk_id, crumb_id, gap_count_id,
    gap_down_id, gap_up_id, menu_id, more_item_id, open_id, pill_id, row_tick_id, show_all_id,
};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};

pub(super) type Events = Rc<RefCell<Vec<FacetEvent>>>;

/// A facet view of `text` (a note at `/notes/n.md`) with `picks`, in its own
/// window, drawn once, and the events it emits.
pub(super) fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    picks: Vec<Facet>,
) -> (
    Entity<FacetView>,
    &'a mut VisualTestContext,
    Events,
    Subscription,
) {
    let text = text.to_string();
    open_with(cx, move |cx| {
        FacetView::new(VaultPath::new("/notes/n.md"), text, picks, cx)
    })
}

/// The facet view `build` makes, in its own window, drawn once, and the
/// events it emits. Motion is reduced, so the bar draws its end height at
/// once ([`open_animated`] leaves it on).
pub(super) fn open_with(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut gpui_kit::Context<FacetView>) -> FacetView + 'static,
) -> (
    Entity<FacetView>,
    &mut VisualTestContext,
    Events,
    Subscription,
) {
    open_motion(cx, true, build)
}

/// [`open_with`] with motion on: the bar moves over time.
pub(super) fn open_animated(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut gpui_kit::Context<FacetView>) -> FacetView + 'static,
) -> (
    Entity<FacetView>,
    &mut VisualTestContext,
    Events,
    Subscription,
) {
    open_motion(cx, false, build)
}

/// The facet view `build` makes, with reduced motion `reduce`.
fn open_motion(
    cx: &mut TestAppContext,
    reduce: bool,
    build: impl FnOnce(&mut gpui_kit::Context<FacetView>) -> FacetView + 'static,
) -> (
    Entity<FacetView>,
    &mut VisualTestContext,
    Events,
    Subscription,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
        cx.set_reduce_motion(reduce);
    });
    // Under the kit's root, as the app's window is: it walks Tab.
    let built: Rc<RefCell<Option<Entity<FacetView>>>> = Rc::default();
    let slot = built.clone();
    let (_root, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(build);
        *slot.borrow_mut() = Some(view.clone());
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = built.borrow_mut().take().expect("the view was built");
    let events = Events::default();
    let sink = events.clone();
    let sub = cx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &FacetEvent, _| {
            sink.borrow_mut().push(event.clone());
        })
    });
    cx.update(|window, cx| window.render_frame(cx));
    (view, cx, events, sub)
}

pub(super) fn click(cx: &mut VisualTestContext, id: impl Into<SharedString>) {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    });
    cx.run_until_parked();
}

pub(super) fn shows(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> bool {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
}

/// Scroll the view's column to its end and draw it there.
pub(super) fn to_bottom(view: &Entity<FacetView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        view.read(cx).scroll.scroll_to_bottom();
        window.refresh();
        window.render_frame(cx);
    });
}

pub(super) fn picks(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<Facet> {
    view.read_with(cx, |view, _| view.picks().to_vec())
}

/// The text each chunk's editor holds, in order.
pub(super) fn chunk_texts(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |view, cx| {
        view.chunks()
            .iter()
            .map(|chunk| chunk.read(cx).editor.get_text())
            .collect()
    })
}

fn chunk(view: &Entity<FacetView>, i: usize, cx: &mut VisualTestContext) -> Entity<EditorView> {
    view.read_with(cx, |view, _| view.chunks()[i].clone())
}

/// Window position of cell (`col`, `row`) in chunk `i`'s last frame.
pub(super) fn cell(
    view: &Entity<FacetView>,
    i: usize,
    col: u16,
    row: u16,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let chunk = chunk(view, i, cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        let painted = chunk
            .read(cx)
            .painted
            .clone()
            .expect("the chunk was painted");
        let c = painted.cells;
        point(
            painted.origin.x + px(c.x_of(col) + c.width() / 2.0),
            painted.origin.y + px(c.y_of(row) + c.height() / 2.0),
        )
    })
}

/// A press and release at `at`, with the primary modifier held when `primary`.
pub(super) fn press(cx: &mut VisualTestContext, at: Point<Pixels>, primary: bool) {
    cx.update(|window, cx| {
        if primary {
            window.change_modifiers(gpui_kit::Modifiers::control(), cx);
        }
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        if primary {
            window.change_modifiers(gpui_kit::Modifiers::none(), cx);
        }
    });
    cx.run_until_parked();
}

pub(super) fn tag(key: &str) -> Facet {
    Facet::new(FacetKind::Tag, key)
}

/// The list a menu chip opens for facets of `kind`.
pub(super) fn menu_of(kind: FacetKind) -> ListKind {
    match kind {
        FacetKind::Tag => ListKind::Tags,
        FacetKind::NoteLink => ListKind::Links,
        FacetKind::Heading => ListKind::Sections,
    }
}

/// Pick `facet` as a user does: open its kind's menu, click it there, and
/// close the menu if the pick left it open.
pub(super) fn pick_from_menu(view: &Entity<FacetView>, cx: &mut VisualTestContext, facet: &Facet) {
    let menu = menu_of(facet.kind);
    if !view.read_with(cx, |view, _| view.list_is_open(menu)) {
        click(cx, menu_id(menu));
    }
    click(cx, more_item_id(facet));
    if list_open(view, cx) {
        key(cx, "escape");
    }
}

pub(super) fn picks_changed(events: &Events) -> usize {
    events
        .borrow()
        .iter()
        .filter(|e| matches!(e, FacetEvent::PicksChanged))
        .count()
}

const NOTE: &str = "# Head\n\nsome #a\n\nmore #b\n";

// ── drawing ──────────────────────────────────────────────────────────────

#[gpui_kit::test]
fn no_picks_draws_one_chunk_with_the_whole_note(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    assert_eq!(chunk_texts(&view, cx), ["# Head\n\nsome #a\n\nmore #b"]);
    assert!(shows(cx, chunk_id(0)));
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 2")
    );
    assert!(
        !shows(cx, pill_id(&tag("a"))),
        "an unpicked facet is not in the bar"
    );
    assert_eq!(
        label(cx, menu_id(ListKind::Sections)).as_deref(),
        Some("Sections 1")
    );
    assert!(!shows(cx, SHOW_MORE_ID));
    // The chunk is tall enough for all of its lines.
    let chunk = chunk(&view, 0, cx);
    let rows = cx.update(|_, cx| chunk.read(cx).painted.clone().unwrap().rows);
    assert!(rows >= 5, "{rows} rows");
}

/// In a group narrower than a paragraph the editor soft-wraps it: the chunk
/// grows to every visual row, none cut off below its bounds.
#[gpui_kit::test]
fn a_wrapped_paragraph_shows_every_visual_row(cx: &mut TestAppContext) {
    let long: String = (0..60).map(|i| format!("word{i} ")).collect();
    let text = format!("{long}\n\nnext #a\n");
    let (view, cx, _events, _sub) = open(cx, &text, vec![]);
    cx.simulate_resize(gpui_kit::size(px(260.), px(1200.)));
    for _ in 0..3 {
        cx.update(|window, cx| window.render_frame(cx));
        cx.run_until_parked();
    }
    let chunk = chunk(&view, 0, cx);
    let (visual, painted) = cx.update(|_, cx| {
        let editor = chunk.read(cx);
        (
            editor.editor.session().display.layout.visual_line_count(),
            editor.painted.clone().expect("painted"),
        )
    });
    assert!(visual > 4, "the paragraph wraps: {visual} visual rows");
    assert!(
        usize::from(painted.rows) >= visual,
        "{} grid rows for {visual} visual rows",
        painted.rows
    );
    let bounds = cx.update(|window, cx| {
        window.render_frame(cx);
        window.find(chunk_id(0)).bounds()
    });
    let last_row_bottom = painted.origin.y + px(painted.cells.y_of(visual as u16));
    assert!(
        last_row_bottom <= bounds.bottom(),
        "row bottom {last_row_bottom:?} past the chunk's {:?}",
        bounds.bottom()
    );
}

/// A large chunk is laid out unwrapped first and wrapped on a background
/// task: the chunk grows again when that layout lands.
#[gpui_kit::test]
fn a_large_chunk_grows_when_its_background_layout_lands(cx: &mut TestAppContext) {
    let rt = super::start_tests::runtime();
    let _guard = rt.enter();
    let long: String = (0..60).map(|i| format!("word{i} ")).collect();
    let rest: String = (0..1200).map(|i| format!("l{i}\n")).collect();
    let text = format!("{long}\n{rest}");
    let (view, cx, _events, _sub) = open(cx, &text, vec![]);
    cx.simulate_resize(gpui_kit::size(px(260.), px(800.)));
    let chunk = chunk(&view, 0, cx);
    let visual = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            chunk
                .read(cx)
                .editor
                .session()
                .display
                .layout
                .visual_line_count()
        })
    };
    let written = text.split('\n').count();
    for _ in 0..100 {
        cx.update(|window, cx| window.render_frame(cx));
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(10)));
        cx.run_until_parked();
        if visual(cx) > written {
            break;
        }
    }
    let visual = visual(cx);
    assert!(
        visual > written,
        "the background layout wrapped the paragraph"
    );
    cx.update(|window, cx| window.render_frame(cx));
    let rows = cx.update(|_, cx| chunk.read(cx).painted.clone().unwrap().rows);
    assert!(usize::from(rows) >= visual, "{rows} grid rows for {visual}");
}

/// Draw until the view settles: every chunk sized to its rows.
pub(super) fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, cx| window.render_frame(cx));
        next_frame(cx);
    }
    cx.update(|window, cx| window.render_frame(cx));
}

/// Runs the next-frame callbacks the last frame asked for; how many ran.
pub(super) fn next_frame(cx: &mut VisualTestContext) -> usize {
    let ran = cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    ran
}

/// A frame that finds a chunk sized for other rows than its editor laid
/// out asks for exactly one more frame (a notify while painting draws
/// nothing); a frame with stable rows asks for none.
#[gpui_kit::test]
fn a_frame_that_changes_the_rows_asks_for_one_more_and_then_settles(cx: &mut TestAppContext) {
    let long: String = (0..60).map(|i| format!("word{i} ")).collect();
    let text = format!("{long}\n\nnext #a\n");
    let (view, cx, _events, _sub) = open(cx, &text, vec![]);
    cx.update(|window, cx| window.render_frame(cx));
    next_frame(cx);
    cx.update(|window, cx| window.render_frame(cx));
    assert_eq!(next_frame(cx), 0, "a stable frame asks for nothing");

    cx.simulate_resize(gpui_kit::size(px(260.), px(1200.)));
    cx.update(|window, cx| window.render_frame(cx));
    assert_eq!(next_frame(cx), 1, "the wrap changed: one more frame");
    cx.update(|window, cx| window.render_frame(cx));
    assert_eq!(next_frame(cx), 0, "settled");
    let chunk = chunk(&view, 0, cx);
    let (visual, rows) = cx.update(|_, cx| {
        let editor = chunk.read(cx);
        (
            editor.editor.session().display.layout.visual_line_count(),
            editor.painted.clone().unwrap().rows,
        )
    });
    assert!(
        visual > 4 && usize::from(rows) >= visual,
        "{rows} rows for {visual}"
    );
}

/// New text in a reused editor with as many rows as written as the old:
/// until the editor lays it out, the chunk is sized by the new text, never
/// by the old text's wrapped rows.
#[gpui_kit::test]
fn a_swapped_text_is_not_sized_by_the_old_layout(cx: &mut TestAppContext) {
    let long: String = (0..60).map(|i| format!("word{i} ")).collect();
    let (view, cx, _events, _sub) = open(cx, &format!("#a {long}\n\nother\n"), vec![tag("a")]);
    cx.simulate_resize(gpui_kit::size(px(260.), px(1200.)));
    for _ in 0..3 {
        cx.update(|window, cx| window.render_frame(cx));
        next_frame(cx);
    }
    assert!(
        view.read_with(cx, |view, _| view.sized_rows()[0]) > 4,
        "the old text wraps"
    );
    // In one update, so no frame lays the new text out in between.
    let rows = cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_text("#a short\n\nother\n".to_string(), cx);
            view.rows_of(0, cx)
        })
    });
    assert_eq!(rows, 1);
}

#[gpui_kit::test]
fn picking_a_tag_pill_narrows_to_its_blocks(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![]);
    let first = chunk(&view, 0, cx);
    pick_from_menu(&view, cx, &tag("a"));
    assert_eq!(picks(&view, cx), [tag("a")]);
    assert_eq!(chunk_texts(&view, cx), ["some #a"]);
    assert_eq!(picks_changed(&events), 1);
    // The editor is reused for the new text.
    assert_eq!(chunk(&view, 0, cx), first);
    // A click on its picked pill unpicks it: the whole note again.
    click(cx, pill_id(&tag("a")));
    assert!(picks(&view, cx).is_empty());
    assert_eq!(chunk_texts(&view, cx), ["# Head\n\nsome #a\n\nmore #b"]);
    assert_eq!(picks_changed(&events), 2);
}

// ── presses in a piece: text, never the filter ──────────────────────────

/// A press and release at `at` with `mods` held.
fn press_with(cx: &mut VisualTestContext, at: Point<Pixels>, mods: gpui_kit::Modifiers) {
    cx.update(|window, cx| {
        window.change_modifiers(mods, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        window.change_modifiers(gpui_kit::Modifiers::none(), cx);
    });
    cx.run_until_parked();
}

/// A left drag from `from` to `to`.
fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    cx.update(|window, cx| {
        window.pointer_down(from, MouseButton::Left, cx);
        window.pointer_move(to, Some(MouseButton::Left), cx);
        window.pointer_up(to, MouseButton::Left, cx);
    });
    cx.run_until_parked();
}

/// The text selected in chunk `i`'s editor, after a frame (the window tells
/// focus listeners of a focus change when it draws).
fn selected_text(view: &Entity<FacetView>, i: usize, cx: &mut VisualTestContext) -> Option<String> {
    let chunk = chunk(view, i, cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        chunk.read(cx).editor.buffer().selection_text()
    })
}

/// The picks, the pieces and the events are as the view opened.
fn filter_unchanged(view: &Entity<FacetView>, events: &Events, cx: &mut VisualTestContext) {
    assert!(picks(view, cx).is_empty(), "{:?}", picks(view, cx));
    assert_eq!(picks_changed(events), 0);
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
}

/// The follows the view asked of its host.
fn follows(events: &Events) -> Vec<FacetEvent> {
    events
        .borrow()
        .iter()
        .filter(|e| matches!(e, FacetEvent::Follow { .. }))
        .cloned()
        .collect()
}

/// A plain click on a tag follows it at the release. The filter does not
/// change: the host decides what a follow does.
#[gpui_kit::test]
fn a_click_on_a_tag_in_a_read_only_piece_follows_it(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![]);
    // "more #b" is row 4; col 6 is the `b`.
    let at = cell(&view, 0, 6, 4, cx);
    press(cx, at, false);
    assert_eq!(
        follows(&events),
        [FacetEvent::Follow {
            from: VaultPath::new("/notes/n.md"),
            target: FollowTarget::Label("b".into())
        }]
    );
    assert!(picks(&view, cx).is_empty(), "the filter is unchanged");
    assert_eq!(picks_changed(&events), 0);
    let editor = chunk(&view, 0, cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).editor.cursor()),
        (4, 6),
        "the press placed the caret, as text"
    );
}

/// A plain click on a note link follows it too.
#[gpui_kit::test]
fn a_click_on_a_note_link_in_a_read_only_piece_follows_it(cx: &mut TestAppContext) {
    let text = "see [[Plan]] now\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    let link = cell(&view, 0, 7, 0, cx);
    press(cx, link, false);
    assert_eq!(follows(&events).len(), 1, "{:?}", events.borrow());
    assert!(picks(&view, cx).is_empty());
}

/// A plain click on a web link follows it too: the host opens the browser.
#[gpui_kit::test]
fn a_click_on_a_url_in_a_read_only_piece_follows_it(cx: &mut TestAppContext) {
    let text = "web [x](https://e.com)\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    // The link draws as `x`, at col 4.
    let url = cell(&view, 0, 4, 0, cx);
    press(cx, url, false);
    let follows = follows(&events);
    assert!(
        matches!(
            follows.as_slice(),
            [FacetEvent::Follow { target: FollowTarget::Link { target, .. }, .. }]
                if target == "https://e.com"
        ),
        "{follows:?}"
    );
    assert!(picks(&view, cx).is_empty());
}

/// A press and its release a few pixels apart on the same tag is still a
/// click: a small wobble. A longer move selects instead (the slop is 4 px).
#[gpui_kit::test]
fn a_wobble_inside_a_tag_is_still_a_click(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "see #idea here\n", vec![]);
    let from = cell(&view, 0, 5, 0, cx);
    let to = from + gpui_kit::point(px(3.), px(0.));
    drag(cx, from, to);
    assert_eq!(follows(&events).len(), 1, "{:?}", events.borrow());
}

/// A move that leaves the tag makes the press a drag, even when it comes
/// back before the release.
#[gpui_kit::test]
fn a_move_off_the_tag_and_back_is_a_drag(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "see #idea here\n", vec![]);
    let from = cell(&view, 0, 5, 0, cx);
    let off = cell(&view, 0, 11, 0, cx);
    cx.update(|window, cx| {
        window.pointer_down(from, MouseButton::Left, cx);
        window.pointer_move(off, Some(MouseButton::Left), cx);
        window.pointer_move(from, Some(MouseButton::Left), cx);
        window.pointer_up(from, MouseButton::Left, cx);
    });
    cx.run_until_parked();
    assert!(follows(&events).is_empty(), "{:?}", events.borrow());
}

/// The piece gets new text between the press and the release (a re-plan):
/// the release follows nothing, old or new.
#[gpui_kit::test]
fn a_replan_between_press_and_release_follows_nothing(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "see [[Plan]] now\n", vec![]);
    let at = cell(&view, 0, 7, 0, cx);
    cx.update(|window, cx| window.pointer_down(at, MouseButton::Left, cx));
    view.update(cx, |view, cx| {
        view.set_text("saw [[Plan]] now\n".into(), cx)
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.pointer_move(at, Some(MouseButton::Left), cx);
        window.pointer_up(at, MouseButton::Left, cx);
    });
    cx.run_until_parked();
    assert!(follows(&events).is_empty(), "{:?}", events.borrow());
}

#[gpui_kit::test]
fn a_click_on_plain_text_in_a_read_only_piece_emits_nothing(cx: &mut TestAppContext) {
    let text = "# Head\n\nsome #a\n\n# Other\n\nmore\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    // A heading line, then plain words.
    let at = cell(&view, 0, 3, 4, cx);
    press(cx, at, false);
    let at = cell(&view, 0, 1, 2, cx);
    press(cx, at, false);
    filter_unchanged(&view, &events, cx);
}

/// A drag that starts on a tag selects text and follows nothing.
#[gpui_kit::test]
fn a_drag_that_starts_on_a_tag_selects_and_does_not_follow(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "see #idea here\n", vec![]);
    let from = cell(&view, 0, 5, 0, cx);
    let to = cell(&view, 0, 12, 0, cx);
    drag(cx, from, to);
    filter_unchanged(&view, &events, cx);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("idea he"));
}

/// There is no Alt+click: an Alt press is a plain press.
#[gpui_kit::test]
fn an_alt_press_does_not_change_the_filter_either(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![]);
    let at = cell(&view, 0, 6, 4, cx);
    press_with(cx, at, gpui_kit::Modifiers::alt());
    filter_unchanged(&view, &events, cx);
}

/// The first click of a double (or triple) click on a tag follows. The
/// later presses are plain presses: they place the caret, select no word
/// and follow nothing more. In the app the follow has moved on by then
/// (a search, another tab), so they rarely reach the piece at all.
#[gpui_kit::test]
fn a_double_click_on_a_tag_follows_on_the_first_click_only(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "a #idea b\n", vec![]);
    let editor = chunk(&view, 0, cx);
    // The presses are one instant apart, however loaded the machine.
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.set_clock(crate::editor_view::frozen_clock)
        })
    });
    let at = cell(&view, 0, 4, 0, cx);
    press(cx, at, false);
    assert_eq!(follows(&events).len(), 1, "the first click follows");
    press(cx, at, false);
    press(cx, at, false);
    assert_eq!(follows(&events).len(), 1, "{:?}", events.borrow());
    assert!(picks(&view, cx).is_empty());
    assert_eq!(selected_text(&view, 0, cx), None, "no word selected");
}

/// A double click on plain text still selects the word.
#[gpui_kit::test]
fn a_double_click_on_plain_text_selects_the_word(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "plain words #idea\n", vec![]);
    let editor = chunk(&view, 0, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.set_clock(crate::editor_view::frozen_clock)
        })
    });
    let at = cell(&view, 0, 8, 0, cx);
    press(cx, at, false);
    press(cx, at, false);
    filter_unchanged(&view, &events, cx);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("words"));
}

#[gpui_kit::test]
fn dragging_across_a_tag_selects_text(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "see #idea here\n", vec![]);
    let from = cell(&view, 0, 0, 0, cx);
    let to = cell(&view, 0, 12, 0, cx);
    drag(cx, from, to);
    filter_unchanged(&view, &events, cx);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("see #idea he"));
}

/// A drag across a link drawn as its text copies the link as written.
#[gpui_kit::test]
fn copy_across_a_concealed_link_copies_its_markdown(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "see [text](note.md) end\n", vec![]);
    let from = cell(&view, 0, 0, 0, cx);
    let to = cell(&view, 0, 30, 0, cx);
    drag(cx, from, to);
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("before".into()));
    key(cx, "ctrl-c");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert!(
        copied
            .as_deref()
            .is_some_and(|c| c.contains("[text](note.md)")),
        "{copied:?}"
    );
}

#[gpui_kit::test]
fn copy_copies_the_selected_text_of_a_piece(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "see #idea here\n", vec![]);
    let from = cell(&view, 0, 4, 0, cx);
    let to = cell(&view, 0, 9, 0, cx);
    drag(cx, from, to);
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("before".into()));
    key(cx, "ctrl-c");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("#idea"));
    filter_unchanged(&view, &events, cx);
    assert_eq!(
        selected_text(&view, 0, cx).as_deref(),
        Some("#idea"),
        "kept"
    );
}

/// Make the test window the active one: the window tells focus listeners
/// of a focus change only while it is active, as the app's window is.
fn activate(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
}

/// The selection chunk `i`'s editor draws.
fn drawn_selection(
    view: &Entity<FacetView>,
    i: usize,
    cx: &mut VisualTestContext,
) -> Option<((usize, usize), (usize, usize))> {
    let chunk = chunk(view, i, cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        chunk.read(cx).editor.selection()
    })
}

/// A piece that loses the keyboard (Escape) drops its selection: the copy
/// chord could no longer reach it, so it is not drawn either. The text and
/// the picks stay.
#[gpui_kit::test]
fn escape_out_of_a_piece_clears_its_selection(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "see #idea here\n", vec![]);
    activate(cx);
    let from = cell(&view, 0, 4, 0, cx);
    let to = cell(&view, 0, 9, 0, cx);
    drag(cx, from, to);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("#idea"));
    key(cx, "escape");
    assert!(view_focused(&view, cx), "the view has the keyboard");
    assert_eq!(selected_text(&view, 0, cx), None);
    assert_eq!(drawn_selection(&view, 0, cx), None, "no selection drawn");
    assert_eq!(chunk_texts(&view, cx), ["see #idea here"]);
    filter_unchanged(&view, &events, cx);
}

/// Tab out of a piece also takes the keyboard away: the selection goes.
#[gpui_kit::test]
fn tab_out_of_a_piece_clears_its_selection(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# Head\n\nsee #idea here\n", vec![]);
    activate(cx);
    let from = cell(&view, 0, 4, 2, cx);
    let to = cell(&view, 0, 9, 2, cx);
    drag(cx, from, to);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("#idea"));
    key(cx, "tab");
    assert_eq!(drawn_selection(&view, 0, cx), None);
}

/// A pick from a menu takes the keyboard out of the piece: its selection
/// goes, though the pick keeps the piece's text (the whole note is the one
/// block that holds the tag).
#[gpui_kit::test]
fn a_click_on_a_pill_clears_a_pieces_selection(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "see #idea here\n", vec![]);
    activate(cx);
    let from = cell(&view, 0, 0, 0, cx);
    let to = cell(&view, 0, 3, 0, cx);
    drag(cx, from, to);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("see"));
    pick_from_menu(&view, cx, &tag("idea"));
    assert_eq!(picks(&view, cx), [tag("idea")]);
    assert_eq!(chunk_texts(&view, cx), ["see #idea here"]);
    assert_eq!(selected_text(&view, 0, cx), None);
    assert_eq!(drawn_selection(&view, 0, cx), None);
}

/// A selection in piece A, then one in piece B: only B's is left, and the
/// copy chord copies it.
#[gpui_kit::test]
fn a_selection_in_another_piece_clears_the_first(cx: &mut TestAppContext) {
    let text = "one #a here\n\nmiddle\n\ntwo #a there\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("a")]);
    activate(cx);
    assert_eq!(chunk_texts(&view, cx).len(), 2, "two pieces");
    let from = cell(&view, 0, 0, 0, cx);
    let to = cell(&view, 0, 3, 0, cx);
    drag(cx, from, to);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("one"));
    let from = cell(&view, 1, 0, 0, cx);
    let to = cell(&view, 1, 3, 0, cx);
    drag(cx, from, to);
    assert_eq!(selected_text(&view, 0, cx), None, "A's selection went");
    assert_eq!(drawn_selection(&view, 0, cx), None);
    assert_eq!(selected_text(&view, 1, cx).as_deref(), Some("two"));
    assert!(drawn_selection(&view, 1, cx).is_some(), "B's is drawn");
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("before".into()));
    key(cx, "ctrl-c");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("two"));
}

/// A second drag inside the same piece keeps the keyboard there: it selects
/// as the first did, and nothing is cleared in between.
#[gpui_kit::test]
fn a_second_drag_in_the_same_piece_still_selects(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "see #idea here\n", vec![]);
    activate(cx);
    let from = cell(&view, 0, 0, 0, cx);
    let to = cell(&view, 0, 3, 0, cx);
    drag(cx, from, to);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("see"));
    let from = cell(&view, 0, 4, 0, cx);
    let to = cell(&view, 0, 9, 0, cx);
    drag(cx, from, to);
    assert_eq!(selected_text(&view, 0, cx).as_deref(), Some("#idea"));
    assert!(drawn_selection(&view, 0, cx).is_some());
}

/// Ctrl+click (Cmd+click on macOS) on a target follows, as a plain click
/// does.
#[gpui_kit::test]
fn ctrl_click_on_a_target_still_follows_in_a_read_only_piece(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![]);
    let at = cell(&view, 0, 6, 4, cx);
    press(cx, at, true);
    assert!(picks(&view, cx).is_empty());
    let want = [FacetEvent::Follow {
        from: VaultPath::new("/notes/n.md"),
        target: FollowTarget::Label("b".into()),
    }];
    assert_eq!(*events.borrow(), want, "it followed once");
}

/// A piece draws its follow targets with the pointing hand at all times,
/// with no modifier held.
#[gpui_kit::test]
fn the_pointer_is_the_hand_over_every_target_without_a_modifier(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    settle(cx);
    let tag_at = cell(&view, 0, 6, 4, cx);
    let text_at = cell(&view, 0, 1, 4, cx);
    let chunk = chunk(&view, 0, cx);
    let (over_tag, over_text) = cx.update(|_, cx| {
        let painted = chunk.read(cx).painted.clone().expect("painted");
        (painted.hand_at(tag_at), painted.hand_at(text_at))
    });
    assert!(over_tag, "the hand over a tag");
    assert!(!over_text, "the text cursor over plain text");
}

// ── revealing ────────────────────────────────────────────────────────────

#[gpui_kit::test]
fn reveal_down_adds_one_block_and_a_second_click_closes_the_gap(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x p0", "#x p3"]);
    // Gap 1 sits between chunk 0 and chunk 1; nothing hides above or below.
    assert!(shows(cx, gap_down_id(1)));
    assert!(!shows(cx, gap_down_id(0)));
    assert!(!shows(cx, gap_down_id(2)));
    click(cx, gap_down_id(1));
    assert_eq!(chunk_texts(&view, cx), ["#x p0\n\np1", "#x p3"]);
    click(cx, gap_down_id(1));
    assert_eq!(chunk_texts(&view, cx), ["#x p0\n\np1\n\np2\n\n#x p3"]);
    assert!(!shows(cx, gap_down_id(1)));
}

#[gpui_kit::test]
fn open_note_emits_the_first_line_of_the_chunk(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\n#x p2\n";
    let (_view, cx, events, _sub) = open(cx, text, vec![tag("x")]);
    click(cx, open_id(1));
    assert_eq!(
        *events.borrow(),
        [FacetEvent::OpenNote {
            note: VaultPath::new("/notes/n.md"),
            line: 4
        }]
    );
}

// ── piece presentation: breadcrumb, no frames, gap rows ──────────────────

/// The bounds of chunk `i`'s editor element.
fn editor_bounds(cx: &mut VisualTestContext, i: usize) -> gpui_kit::Bounds<Pixels> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.within(chunk_id(i)).find("editor").bounds()
    })
}

#[gpui_kit::test]
fn a_piece_shows_its_breadcrumb_above_it(cx: &mut TestAppContext) {
    let text = "# Top\n\n## Sub\n\nalpha #x\n\nbeta\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["alpha #x"]);
    assert_eq!(label(cx, crumb_id(0, 0)).as_deref(), Some("Top"));
    assert_eq!(label(cx, crumb_id(0, 1)).as_deref(), Some("Sub"));
    assert!(!shows(cx, crumb_id(0, 2)));
    let top = bounds(cx, crumb_id(0, 0));
    let sub = bounds(cx, crumb_id(0, 1));
    let editor = editor_bounds(cx, 0);
    let piece = bounds(cx, chunk_id(0));
    assert!(top.right() <= sub.left(), "outermost first, left to right");
    assert!(top.top() >= piece.top(), "the row belongs to the piece");
    assert!(top.bottom() <= editor.top(), "the row sits above the text");
}

#[gpui_kit::test]
fn a_piece_with_no_headings_still_draws_the_source_row(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "#x p0\n\np1\n", vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x p0"]);
    assert!(!shows(cx, crumb_id(0, 0)));
    assert!(shows(cx, open_id(0)));
    let open = bounds(cx, open_id(0));
    let editor = editor_bounds(cx, 0);
    assert!(open.bottom() <= editor.top(), "the row sits above the text");
}

#[gpui_kit::test]
fn crumb_clicks_still_toggle(cx: &mut TestAppContext) {
    let text = "# Top\n\nalpha #x\n\n# Other\n\nbeta #x\n";
    let (view, cx, events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(label(cx, crumb_id(0, 0)).as_deref(), Some("Top"));
    click(cx, crumb_id(0, 0));
    assert_eq!(picks(&view, cx), [tag("x"), heading("Top")]);
    assert_eq!(chunk_texts(&view, cx), ["alpha #x"]);
    assert_eq!(picks_changed(&events), 1);
}

#[gpui_kit::test]
fn a_repeated_heading_crumb_toggles_the_merged_facet(cx: &mut TestAppContext) {
    let text = "# A\n\nx #t\n\nmore a1\n\n# B\n\ny #t\n\n# A\n\nz #t\n\nmore a2\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("t")]);
    assert_eq!(chunk_texts(&view, cx), ["x #t", "y #t", "z #t"]);
    assert_eq!(label(cx, crumb_id(2, 0)).as_deref(), Some("A"));
    click(cx, crumb_id(2, 0));
    assert_eq!(picks(&view, cx), [tag("t"), heading("A")]);
    assert_eq!(
        chunk_texts(&view, cx),
        ["x #t", "z #t"],
        "both sections called A show"
    );
}

#[gpui_kit::test]
fn open_note_is_a_labelled_text_button_in_the_breadcrumb_row(cx: &mut TestAppContext) {
    let text = "# Top\n\nalpha #x\n";
    let (_view, cx, events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(label(cx, open_id(0)).as_deref(), Some("Open note"));
    let open = bounds(cx, open_id(0));
    let crumb = bounds(cx, crumb_id(0, 0));
    let piece = bounds(cx, chunk_id(0));
    assert!(
        f32::from(open.size.height) >= 24.0,
        "a 24 px target: {open:?}"
    );
    assert!(
        f32::from(open.size.width) > 48.0,
        "a text button, wider than an icon: {open:?}"
    );
    assert!(
        open.top() < crumb.bottom() && crumb.top() < open.bottom(),
        "one row: {open:?} {crumb:?}"
    );
    assert!(
        open.left() > piece.left() + (piece.size.width / 2.0),
        "at the row's right end"
    );
    click(cx, open_id(0));
    assert_eq!(
        *events.borrow(),
        [FacetEvent::OpenNote {
            note: VaultPath::new("/notes/n.md"),
            line: 2
        }]
    );
}

/// A breadcrumb wider than its row shortens with an ellipsis, outer crumbs
/// first: the innermost crumb ends before `Open note`, which stays whole.
#[gpui_kit::test]
fn a_long_breadcrumb_shortens_before_open_note(cx: &mut TestAppContext) {
    let text = "# The outermost section of this rather long note\n\n\
                ## A middle section with a long name too\n\n\
                ### The innermost section where the text lives\n\n\
                alpha #x\n\nbeta\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    let natural: Vec<f32> = (0..3)
        .map(|i| f32::from(bounds(cx, crumb_id(0, i)).size.width))
        .collect();
    let open = f32::from(bounds(cx, open_id(0)).size.width);
    cx.simulate_resize(gpui_kit::size(px(420.), px(800.)));
    cx.update(|window, cx| window.render_frame(cx));
    let last = bounds(cx, crumb_id(0, 2));
    let button = bounds(cx, open_id(0));
    assert!(
        last.right() <= button.left(),
        "the last crumb ends before Open note: {last:?} {button:?}"
    );
    assert_eq!(
        f32::from(button.size.width),
        open,
        "Open note keeps its width"
    );
    let now: Vec<f32> = (0..3)
        .map(|i| f32::from(bounds(cx, crumb_id(0, i)).size.width))
        .collect();
    assert!(
        now.iter().all(|&w| w >= 24.0),
        "every crumb stays visible, a tab stop with a ring: {now:?}"
    );
    assert!(
        now[0] < natural[0],
        "the outer crumb shrank: {now:?} {natural:?}"
    );
    assert!(
        now[2] / natural[2] >= now[0] / natural[0],
        "the innermost keeps the most: {now:?} {natural:?}"
    );
}

/// No frame around a piece: its editor spans the piece's full width.
#[gpui_kit::test]
fn pieces_have_no_border(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, "#x p0\n\np1\n\n#x p2\n", vec![tag("x")]);
    for i in 0..2 {
        let piece = bounds(cx, chunk_id(i));
        let editor = editor_bounds(cx, i);
        assert_eq!(editor.left(), piece.left(), "piece {i}: no left border");
        assert_eq!(editor.right(), piece.right(), "piece {i}: no right border");
        assert_eq!(
            editor.bottom(),
            piece.bottom(),
            "piece {i}: no bottom border"
        );
    }
}

#[gpui_kit::test]
fn the_gap_row_says_n_blocks_hidden(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n\np4\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(
        label(cx, gap_count_id(1)).as_deref(),
        Some("2 blocks hidden")
    );
    assert_eq!(
        label(cx, gap_count_id(2)).as_deref(),
        Some("1 block hidden")
    );
    assert!(!shows(cx, gap_count_id(0)), "nothing hides above");
}

#[gpui_kit::test]
fn show_all_appears_for_a_gap_of_three_or_fewer_and_not_for_four(cx: &mut TestAppContext) {
    let text = "#x p0\n\na\n\nb\n\nc\n\n#x p4\n\nd\n\ne\n\nf\n\ng\n\n#x p9\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x p0", "#x p4", "#x p9"]);
    assert_eq!(
        label(cx, gap_count_id(1)).as_deref(),
        Some("3 blocks hidden")
    );
    assert_eq!(label(cx, show_all_id(1)).as_deref(), Some("Show all"));
    assert_eq!(
        label(cx, gap_count_id(2)).as_deref(),
        Some("4 blocks hidden")
    );
    assert!(!shows(cx, show_all_id(2)), "four is too many");
    let target = bounds(cx, show_all_id(1));
    assert!(
        f32::from(target.size.height) >= 24.0 && f32::from(target.size.width) >= 24.0,
        "a 24 px target: {target:?}"
    );
}

#[gpui_kit::test]
fn show_all_fills_the_gap_and_merges_the_pieces(cx: &mut TestAppContext) {
    let text = "#x p0\n\na\n\nb\n\n#x p3\n\nc\n\n#x p5\n";
    let (view, cx, events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x p0", "#x p3", "#x p5"]);
    click(cx, show_all_id(1));
    assert_eq!(
        chunk_texts(&view, cx),
        ["#x p0\n\na\n\nb\n\n#x p3", "#x p5"]
    );
    assert_eq!(
        label(cx, gap_count_id(1)).as_deref(),
        Some("1 block hidden"),
        "gap 1 now lies between the merged piece and p5"
    );
    assert_eq!(picks_changed(&events), 0, "a reveal is no pick");
    // A change of text resets the reveal.
    view.update(cx, |view, cx| view.set_text(text.to_string(), cx));
    assert_eq!(chunk_texts(&view, cx), ["#x p0", "#x p3", "#x p5"]);
}

#[gpui_kit::test]
fn gap_arrows_are_at_least_24_px(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    for (id, name) in [
        (gap_down_id(1), "Show next block"),
        (gap_up_id(1), "Show previous block"),
    ] {
        assert_eq!(label(cx, id.clone()).as_deref(), Some(name));
        let b = bounds(cx, id);
        assert!(
            f32::from(b.size.height) >= 24.0 && f32::from(b.size.width) >= 24.0,
            "{name}: a 24 px target: {b:?}"
        );
    }
}

// ── new content ──────────────────────────────────────────────────────────

#[gpui_kit::test]
fn set_text_prunes_a_vanished_pick_and_shows_the_whole_note(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![tag("a"), tag("gone")]);
    assert_eq!(picks(&view, cx), [tag("a")], "new() prunes");
    view.update(cx, |view, cx| {
        view.set_text("no tags\n\nsecond\n".to_string(), cx)
    });
    assert!(picks(&view, cx).is_empty());
    assert_eq!(chunk_texts(&view, cx), ["no tags\n\nsecond"]);
    assert_eq!(picks_changed(&events), 1);
}

#[gpui_kit::test]
fn set_text_resets_revealed_blocks(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n";
    let (view, cx, events, _sub) = open(cx, text, vec![tag("x")]);
    click(cx, gap_down_id(1));
    view.update(cx, |view, cx| view.set_text(text.to_string(), cx));
    assert_eq!(chunk_texts(&view, cx), ["#x p0", "#x p3"]);
    assert_eq!(picks_changed(&events), 0, "nothing was pruned");
}

#[gpui_kit::test]
fn set_source_retargets_the_view(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    view.update(cx, |view, _| view.set_source(VaultPath::new("/other/m.md")));
    view.read_with(cx, |view, _| {
        assert_eq!(view.source(), Some(&VaultPath::new("/other/m.md")))
    });
}

// ── many matches ─────────────────────────────────────────────────────────

#[gpui_kit::test]
fn three_hundred_matches_draw_one_hundred_chunks_and_show_more(cx: &mut TestAppContext) {
    let text: String = (0..300)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("t")]);
    assert_eq!(chunk_texts(&view, cx).len(), MAX_RANGES);
    // The button ends the one scrolling column: off screen until scrolled to.
    assert!(!shows(cx, SHOW_MORE_ID));
    to_bottom(&view, cx);
    assert!(shows(cx, SHOW_MORE_ID));
    let label = view.read_with(cx, |view, _| view.show_more_label());
    assert_eq!(label.as_deref(), Some("Show 100 more"));
    click(cx, SHOW_MORE_ID);
    assert_eq!(chunk_texts(&view, cx).len(), 2 * MAX_RANGES);
    let label = view.read_with(cx, |view, _| view.show_more_label());
    assert_eq!(label.as_deref(), Some("Show 100 more"));
    to_bottom(&view, cx);
    click(cx, SHOW_MORE_ID);
    assert_eq!(chunk_texts(&view, cx).len(), 300);
    to_bottom(&view, cx);
    assert!(!shows(cx, SHOW_MORE_ID));
}

/// `Show N more` counts what the next click shows: the limit, or what is
/// left when that is less.
#[gpui_kit::test]
fn show_more_label_counts_what_the_next_click_shows(cx: &mut TestAppContext) {
    let text: String = (0..350)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("t")]);
    to_bottom(&view, cx);
    let label = view.read_with(cx, |view, _| view.show_more_label());
    assert_eq!(label.as_deref(), Some("Show 100 more"));
    click(cx, SHOW_MORE_ID);
    to_bottom(&view, cx);
    let label = view.read_with(cx, |view, _| view.show_more_label());
    assert_eq!(label.as_deref(), Some("Show 150 more"));
}

/// The wheel over a chunk scrolls the whole column, not the chunk's editor.
#[gpui_kit::test]
fn the_wheel_over_a_chunk_scrolls_the_column(cx: &mut TestAppContext) {
    let text: String = (0..40)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("t")]);
    let top =
        |cx: &mut VisualTestContext| cx.update(|_, cx| f32::from(view.read(cx).scroll.offset().y));
    assert_eq!(top(cx), 0.0);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.within(chunk_id(1)).scroll(
            "editor",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-200.))),
            cx,
        );
        window.render_frame(cx);
    });
    assert!(top(cx) < 0.0, "the column scrolled: {}", top(cx));
}

// ── the lists (a menu chip, `/`) ─────────────────────────────────────────

#[gpui_kit::test]
fn the_list_is_fuzzy_and_picking_from_it_toggles(cx: &mut TestAppContext) {
    let text: String = (0..14).map(|i| format!("#t{i:02} ")).collect();
    let (view, cx, events, _sub) = open(cx, &text, vec![]);
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 14")
    );
    focus_view(&view, cx);
    key(cx, "/");
    assert!(shows(cx, more_item_id(&tag("t00"))));
    // Fourteen rows scroll inside the list: the highlight brings t13 in.
    for _ in 0..13 {
        key(cx, "down");
    }
    assert!(shows(cx, more_item_id(&tag("t13"))));
    let input = view.read_with(cx, |view, _| view.more_input().expect("the list is open"));
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_value("13", window, cx)));
    cx.run_until_parked();
    assert!(!shows(cx, more_item_id(&tag("t00"))));
    assert!(shows(cx, more_item_id(&tag("t13"))));
    click(cx, more_item_id(&tag("t13")));
    assert_eq!(picks(&view, cx), [tag("t13")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(
        view.read_with(cx, |view, _| view.more_input().is_some()),
        "it stays open for more picks"
    );
    // A pick shows in the picks row.
    assert!(shows(cx, pill_id(&tag("t13"))));
}

/// A note with 80 tags. Its first line is plain text.
pub(super) fn crowded_tags() -> String {
    "plain\n\n".to_string() + &(0..80).map(|i| format!("#t{i:02} ")).collect::<String>()
}

/// Fourteen tags.
fn many_tags() -> String {
    (0..14).map(|i| format!("#t{i:02} ")).collect::<String>() + "\n\nplain\n"
}

pub(super) fn list_open(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.more_input().is_some())
}

/// Whether the view's own focus handle has the keyboard.
pub(super) fn view_focused(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| view.read(cx).focus_handle(cx).is_focused(window))
}

/// Whether the open list's filter field has the keyboard.
pub(super) fn field_focused(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
    let input = view.read_with(cx, |view, _| view.more_input().expect("the list is open"));
    cx.update(|window, cx| input.read(cx).focus_handle(cx).is_focused(window))
}

pub(super) fn key(cx: &mut VisualTestContext, key: &str) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.press(key, cx);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_list_opens_over_the_content_without_moving_it(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded_tags(), vec![]);
    settle(cx);
    let bar = bounds(cx, FACET_BAR_ID);
    let first = bounds(cx, chunk_id(0));
    let rows = view.read_with(cx, |view, _| view.sized_rows().to_vec());
    click(cx, menu_id(ListKind::Tags));
    assert!(list_open(&view, cx));
    assert!(shows(cx, MORE_LIST_ID));
    assert_eq!(bounds(cx, FACET_BAR_ID), bar, "the bar did not grow");
    assert_eq!(bounds(cx, chunk_id(0)), first, "the content did not move");
    assert_eq!(
        view.read_with(cx, |view, _| view.sized_rows().to_vec()),
        rows,
        "no piece was sized again"
    );
    cx.update(|window, cx| window.render_frame(cx));
    assert!(
        !view.read_with(cx, |view, _| view.resize_asked()),
        "no resize frame was asked for"
    );
    let chip = bounds(cx, menu_id(ListKind::Tags));
    let list = bounds(cx, MORE_LIST_ID);
    assert!(
        list.top() >= chip.bottom(),
        "under the chip: {list:?} {chip:?}"
    );
    assert!(
        list.bottom() > bar.bottom(),
        "it floats past the bar, over the content: {list:?} {bar:?}"
    );
}

#[gpui_kit::test]
fn the_filter_field_is_focused_when_the_list_opens(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded_tags(), vec![]);
    settle(cx);
    click(cx, menu_id(ListKind::Tags));
    assert!(field_focused(&view, cx), "the chip's list");
    key(cx, "escape");
    // The all-kinds list, opened by `/` (here through `open_list`).
    let text = "# Head\n\nsome #a and [[plan]]\n";
    view.update(cx, |view, cx| view.set_text(text.to_string(), cx));
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_list(ListKind::All, window, cx)));
    cx.run_until_parked();
    assert!(field_focused(&view, cx), "the list opened by `/`");
    assert!(shows(cx, MORE_LIST_ID));
    assert!(shows(cx, more_item_id(&tag("a"))));
    assert!(shows(
        cx,
        more_item_id(&Facet::new(FacetKind::NoteLink, "plan"))
    ));
    assert!(
        shows(cx, more_item_id(&heading("head"))),
        "the all-kinds list holds the headings too"
    );
}

/// Escape closes the list and nothing else: the picks stay. The keyboard
/// goes back to the chip that opened the list (a tab stop).
#[gpui_kit::test]
fn escape_closes_the_list_and_returns_focus_to_the_chip(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, &crowded_tags(), vec![]);
    settle(cx);
    click(cx, menu_id(ListKind::Tags));
    click(cx, more_item_id(&tag("t00")));
    assert!(list_open(&view, cx));
    key(cx, "escape");
    assert!(!list_open(&view, cx), "closed");
    assert!(!shows(cx, MORE_LIST_ID));
    assert_eq!(picks(&view, cx), [tag("t00")], "the picks stay");
    assert_eq!(picks_changed(&events), 1);
    assert!(
        has_focus(cx, menu_id(ListKind::Tags)),
        "the keyboard is back on the chip"
    );
}

// ── the keyboard ─────────────────────────────────────────────────────────

/// Whether the element `id` holds the keyboard.
pub(super) fn has_focus(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> bool {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.focused())
            .unwrap_or(false)
    })
}

/// Whether the element `id` is marked selected (the list's highlight).
fn selected(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> bool {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.selected())
            .unwrap_or(false)
    })
}

/// Give the keyboard to the view, as the workspace does when its group
/// shows it.
pub(super) fn focus_view(view: &Entity<FacetView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let handle = view.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
}

pub(super) fn query(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, cx| {
        view.more_input().unwrap().read(cx).value().to_string()
    })
}

/// Tab until `id` holds the keyboard; panics after `limit` presses.
pub(super) fn tab_to(cx: &mut VisualTestContext, id: impl Into<SharedString>, limit: usize) {
    let id: SharedString = id.into();
    for _ in 0..limit {
        key(cx, "tab");
        if has_focus(cx, id.clone()) {
            return;
        }
    }
    panic!("Tab never reached {id}");
}

#[gpui_kit::test]
fn enter_in_the_list_toggles_the_highlighted_facet(cx: &mut TestAppContext) {
    let text = "# Head\n\nsome #a and #b and [[plan]]\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    let items = view.read_with(cx, |view, cx| view.list_results(cx));
    assert!(items.len() >= 3, "{items:?}");
    assert!(selected(cx, more_item_id(&items[0])), "the first is lit");
    key(cx, "down");
    assert!(selected(cx, more_item_id(&items[1])));
    assert!(!selected(cx, more_item_id(&items[0])));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [items[1].clone()]);
    assert_eq!(picks_changed(&events), 1);
    assert!(list_open(&view, cx), "it stays open");
    assert!(field_focused(&view, cx), "the field keeps the keyboard");
    // Up stops at the first.
    key(cx, "up");
    key(cx, "up");
    let now = view.read_with(cx, |view, cx| view.list_results(cx));
    assert!(selected(cx, more_item_id(&now[0])));
    // A new filter lights its first result.
    let input = view.read_with(cx, |view, _| view.more_input().unwrap());
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_value("pla", window, cx)));
    cx.run_until_parked();
    let plan = Facet::new(FacetKind::NoteLink, "plan");
    assert!(selected(cx, more_item_id(&plan)));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [items[1].clone(), plan]);
}

#[gpui_kit::test]
fn j_and_k_move_between_pieces(cx: &mut TestAppContext) {
    let text: String = (0..40)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("t")]);
    cx.update(|window, cx| window.render_frame(cx));
    focus_view(&view, cx);
    let at_top = |cx: &mut VisualTestContext, i: usize| {
        let top = bounds(cx, COLUMN_ID).top();
        let piece = bounds(cx, chunk_id(i)).top();
        (f32::from(piece) - f32::from(top)).abs() < 1.0
    };
    key(cx, "j");
    assert!(at_top(cx, 1), "j: the second piece at the top");
    key(cx, "j");
    assert!(at_top(cx, 2), "j again: the third");
    key(cx, "k");
    assert!(at_top(cx, 1), "k: back to the second");
    key(cx, "k");
    assert!(at_top(cx, 0), "k: the first");
}

#[gpui_kit::test]
fn escape_closes_the_list_first_then_clears_the_picks(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![tag("a")]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    key(cx, "escape");
    assert!(!list_open(&view, cx), "the list closes first");
    assert_eq!(picks(&view, cx), [tag("a")], "the picks stay");
    assert_eq!(picks_changed(&events), 0);
    key(cx, "escape");
    assert!(picks(&view, cx).is_empty(), "then the picks clear");
    assert_eq!(picks_changed(&events), 1);
    key(cx, "escape");
    assert_eq!(picks_changed(&events), 1, "no picks: nothing");
}

#[gpui_kit::test]
fn plain_letters_do_nothing_while_a_text_field_has_focus(cx: &mut TestAppContext) {
    let text: String = many_tags()
        + &(0..40)
            .map(|i| format!("#t00 m{i}\n\n"))
            .collect::<String>();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("t00")]);
    focus_view(&view, cx);
    key(cx, "/");
    let input = view.read_with(cx, |view, _| view.more_input().unwrap());
    let offset = view.read_with(cx, |view, _| view.scroll.offset());
    key(cx, "j");
    key(cx, "k");
    key(cx, "/");
    key(cx, "escape");
    assert!(!list_open(&view, cx), "Escape still closes the list");
    assert_eq!(
        input.read_with(cx, |input, _| input.value().to_string()),
        "jk/",
        "typed into the field"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.scroll.offset()),
        offset,
        "nothing scrolled"
    );
    assert_eq!(picks(&view, cx), [tag("t00")], "nothing cleared");
}

#[gpui_kit::test]
fn tab_reaches_every_pill_and_shows_a_focus_ring(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    focus_view(&view, cx);
    // Reading order: the bar (the chips, the search chip, the picks, then
    // the summary line), then the column (gap, piece crumbs, Open note,
    // gap).
    let want: Vec<SharedString> = vec![
        menu_id(ListKind::Tags),
        menu_id(ListKind::Sections),
        menu_id(ListKind::All),
        pill_id(&tag("a")),
        CLEAR_ID.into(),
        gap_up_id(0),
        show_all_id(0),
        crumb_id(0, 0),
        open_id(0),
        gap_down_id(1),
        show_all_id(1),
    ];
    for id in &want {
        key(cx, "tab");
        assert!(has_focus(cx, id.clone()), "Tab reached {id}");
        assert_eq!(
            view.read_with(cx, |view, _| view.ringed()),
            Some(id.clone()),
            "{id} drawn with the focus ring"
        );
    }
    key(cx, "shift-tab");
    assert!(has_focus(cx, gap_down_id(1)), "Shift+Tab goes back");
}

#[gpui_kit::test]
fn enter_on_a_focused_pill_toggles_it(cx: &mut TestAppContext) {
    let text = format!("# Head\n\n{}", many_tags());
    let (view, cx, events, _sub) = open(cx, &text, vec![tag("t01"), tag("t02")]);
    focus_view(&view, cx);
    tab_to(cx, pill_id(&tag("t01")), 20);
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("t02")]);
    assert!(
        has_focus(cx, pill_id(&tag("t02"))),
        "the next pick takes the keyboard"
    );
    key(cx, "space");
    assert!(picks(&view, cx).is_empty(), "Space too");
    assert_eq!(picks_changed(&events), 2);
    // A chip does what a click does: `Sections N` opens its list.
    tab_to(cx, menu_id(ListKind::Sections), 20);
    key(cx, "enter");
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    key(cx, "escape");
    assert!(has_focus(cx, menu_id(ListKind::Sections)));
    // `Clear` too.
    tab_to(cx, menu_id(ListKind::Tags), 20);
    key(cx, "enter");
    key(cx, "enter");
    key(cx, "escape");
    assert_eq!(picks(&view, cx).len(), 1, "a tag picked");
    tab_to(cx, CLEAR_ID, 30);
    key(cx, "enter");
    assert!(picks(&view, cx).is_empty(), "Clear cleared");
    assert!(
        has_focus(cx, super::facet::UNDO_ID),
        "Clear is gone: the keyboard is on Undo"
    );
}

/// A press outside the list closes it; the press still does what it does
/// there (here, unpicks a pill of the picks row). The search list hangs
/// leftward from the row's right end, clear of the piece's first cells.
#[gpui_kit::test]
fn an_outside_click_closes_the_list(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded_tags(), vec![tag("t79")]);
    settle(cx);
    click(cx, menu_id(ListKind::All));
    assert!(list_open(&view, cx));
    let plain = cell(&view, 0, 1, 0, cx);
    press(cx, plain, false);
    assert!(!list_open(&view, cx), "a press on the content closes it");
    assert_eq!(picks(&view, cx), [tag("t79")], "plain text picks nothing");
    assert!(!shows(cx, MORE_LIST_ID));
    click(cx, menu_id(ListKind::All));
    assert!(list_open(&view, cx));
    click(cx, pill_id(&tag("t79")));
    assert!(!list_open(&view, cx), "a press on a picked pill closes it");
    assert!(picks(&view, cx).is_empty(), "and unpicks that pill");
}

#[gpui_kit::test]
fn picking_in_the_list_toggles_and_keeps_it_open(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, &many_tags(), vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    let input = view.read_with(cx, |view, _| view.more_input().expect("the list is open"));
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_value("1", window, cx)));
    cx.run_until_parked();
    click(cx, more_item_id(&tag("t12")));
    click(cx, more_item_id(&tag("t13")));
    assert_eq!(picks(&view, cx), [tag("t12"), tag("t13")]);
    assert_eq!(picks_changed(&events), 2);
    assert!(list_open(&view, cx), "still open");
    let query = view.read_with(cx, |view, cx| {
        view.more_input().unwrap().read(cx).value().to_string()
    });
    assert_eq!(query, "1", "the filter stays");
    // A picked facet stays in the list with a tick, in its place while
    // the list is open (its pill joins the picks row too); the rest stay.
    let listed = view.read_with(cx, |view, cx| view.list_results(cx));
    let at = |f: Facet| listed.iter().position(|l| *l == f).unwrap();
    assert_eq!(at(tag("t13")), at(tag("t12")) + 1, "no row moved");
    assert!(shows(cx, row_tick_id(&tag("t12"))));
    assert!(shows(cx, row_tick_id(&tag("t13"))));
    assert!(shows(cx, more_item_id(&tag("t11"))));
    assert!(shows(cx, pill_id(&tag("t12"))));
}

/// A note with no facet draws no bar, so `/` has no line to hang a list
/// from: it opens nothing, and the keyboard stays on the view.
#[gpui_kit::test]
fn slash_does_nothing_in_a_note_with_no_facets(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "just text\n\nmore text\n", vec![]);
    assert!(!shows(cx, FACET_BAR_ID));
    focus_view(&view, cx);
    key(cx, "/");
    assert!(!list_open(&view, cx), "no list without a bar");
    assert!(view_focused(&view, cx), "the keyboard stays on the view");
}

/// A live edit that takes away everything the open list holds leaves it
/// nowhere to hang: it closes and the view takes the keyboard back.
#[gpui_kit::test]
fn an_edit_that_removes_the_open_lists_kind_closes_it(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# H\n\n#a #b\n", vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    assert!(field_focused(&view, cx));
    view.update(cx, |view, cx| view.set_text("# H\n\nplain\n".into(), cx));
    cx.update(|window, cx| window.render_frame(cx));
    cx.run_until_parked();
    assert!(!list_open(&view, cx), "no tag: the Tags list closed");
    assert!(view_focused(&view, cx), "the keyboard is back on the view");
    // Headings only: `/` lists them. It closes once they go.
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert!(shows(cx, more_item_id(&heading("h"))));
    view.update(cx, |view, cx| view.set_text("plain\n".into(), cx));
    cx.update(|window, cx| window.render_frame(cx));
    cx.run_until_parked();
    assert!(!list_open(&view, cx), "no facet: the list closed");
    assert!(view_focused(&view, cx));
    key(cx, "/");
    assert!(!list_open(&view, cx), "no facet: `/` opens nothing");
}

/// A toggle keeps its row in place and the highlight on it, so Down then
/// Enter walks on to the next facet. The list stays open with every row
/// ticked until Escape.
#[gpui_kit::test]
fn the_highlight_stays_on_its_row_after_a_toggle(cx: &mut TestAppContext) {
    let text = "#a #a #a\n\n#b #b\n\n#c\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    let order = [tag("a"), tag("b"), tag("c")];
    assert_eq!(view.read_with(cx, |view, cx| view.list_results(cx)), order);
    key(cx, "down");
    assert!(selected(cx, more_item_id(&tag("b"))));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("b")]);
    assert_eq!(
        view.read_with(cx, |view, cx| view.list_results(cx)),
        order,
        "b keeps its place"
    );
    assert!(selected(cx, more_item_id(&tag("b"))), "b stays lit");
    key(cx, "down");
    assert!(selected(cx, more_item_id(&tag("c"))));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("b"), tag("c")]);
    assert!(
        selected(cx, more_item_id(&tag("c"))),
        "c, now ticked, is lit"
    );
    key(cx, "up");
    key(cx, "up");
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("b"), tag("c"), tag("a")]);
    assert!(list_open(&view, cx), "every row ticked: it stays open");
    assert!(selected(cx, more_item_id(&tag("a"))), "a, ticked, is lit");
    key(cx, "escape");
    assert!(!list_open(&view, cx), "Escape closes it");
    assert!(
        view_focused(&view, cx),
        "no chip left: the view has the keyboard"
    );
}

/// The keyboard inside a piece (after a press there, to select text): the
/// first Escape only hands the keyboard to the view; the next clears.
#[gpui_kit::test]
fn escape_in_a_piece_first_moves_the_keyboard_to_the_view(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "some #a\n\nmore #b\n", vec![tag("a")]);
    assert_eq!(chunk_texts(&view, cx), ["some #a"]);
    let at = cell(&view, 0, 1, 0, cx);
    press(cx, at, false);
    assert_eq!(picks(&view, cx), [tag("a")], "plain text toggles nothing");
    let editor = chunk(&view, 0, cx);
    let in_piece = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.read(cx).focus_handle(cx).is_focused(window))
    };
    assert!(in_piece(cx), "the press put the keyboard in the piece");
    key(cx, "escape");
    assert_eq!(picks(&view, cx), [tag("a")], "the picks stay");
    assert_eq!(picks_changed(&events), 0);
    assert!(view_focused(&view, cx), "the keyboard is on the view");
    key(cx, "escape");
    assert!(picks(&view, cx).is_empty(), "now Escape clears");
}

/// Tab and Shift+Tab walk on from a piece the keyboard is in.
#[gpui_kit::test]
fn tab_walks_on_from_a_piece(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "some #a\n\nmore #b\n", vec![tag("a")]);
    let at = cell(&view, 0, 1, 0, cx);
    press(cx, at, false);
    key(cx, "tab");
    assert!(has_focus(cx, gap_down_id(1)), "Tab: the next stop");
    press(cx, at, false);
    key(cx, "shift-tab");
    assert!(has_focus(cx, open_id(0)), "Shift+Tab: the previous stop");
}

// ── the pinned filter bar ────────────────────────────────────────────────

pub(super) fn bounds(
    cx: &mut VisualTestContext,
    id: impl Into<SharedString>,
) -> gpui_kit::Bounds<Pixels> {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.find(id).bounds()
    })
}

pub(super) fn label(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> Option<String> {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .filter(|e| e.visible())
            .and_then(|e| e.label().map(str::to_string))
    })
}

pub(super) fn heading(text: &str) -> Facet {
    Facet::new(FacetKind::Heading, text)
}

/// `n` headings, each over a line of its own.
fn many_headings(n: usize) -> String {
    (0..n)
        .map(|i| format!("# H{i:03}\n\nbody {i}\n\n"))
        .collect()
}

#[gpui_kit::test]
fn the_bar_stays_while_the_column_scrolls(cx: &mut TestAppContext) {
    let text: String = (0..40)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("t")]);
    let bar = bounds(cx, FACET_BAR_ID);
    let first = bounds(cx, chunk_id(0));
    assert!(first.top() >= bar.bottom(), "the column sits below the bar");
    to_bottom(&view, cx);
    let offset = cx.update(|_, cx| f32::from(view.read(cx).scroll.offset().y));
    assert!(offset < 0.0, "the column scrolled: {offset}");
    assert_eq!(bounds(cx, FACET_BAR_ID), bar, "the bar did not move");
    assert!(shows(cx, pill_id(&tag("t"))), "the pills stay in view");
}

#[gpui_kit::test]
fn the_bar_never_exceeds_forty_percent_of_the_view(cx: &mut TestAppContext) {
    let tags: String = (0..30).map(|i| format!("#longtagname{i:02} ")).collect();
    let links: String = (0..30)
        .map(|i| format!("[[a long note name {i:02}]] "))
        .collect();
    let text = format!("# Head\n\n{tags}\n\n{links}\n");
    let (_view, cx, _events, _sub) = open(cx, &text, vec![tag("longtagname00")]);
    cx.simulate_resize(gpui_kit::size(px(320.), px(260.)));
    let bar = bounds(cx, FACET_BAR_ID);
    let whole = bounds(cx, super::facet::FACET_VIEW_ID);
    let cap = f32::from(whole.size.height) * 0.4 + 0.5;
    assert!(
        f32::from(bar.size.height) <= cap,
        "bar {:?} over 40% of {:?}",
        bar.size.height,
        whole.size.height
    );
    assert!(f32::from(bar.size.height) > 0.0);
}

#[gpui_kit::test]
fn the_summary_line_names_the_picks_and_the_piece_count(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(
        label(cx, SUMMARY_ID).as_deref(),
        Some("Showing text tagged #x \u{b7} 2 pieces")
    );
    // Revealing the gap joins the two into one piece.
    click(cx, gap_down_id(1));
    click(cx, gap_down_id(1));
    assert_eq!(
        label(cx, SUMMARY_ID).as_deref(),
        Some("Showing text tagged #x \u{b7} 1 piece")
    );
}

/// The count covers the pieces the cap leaves out, not only those drawn.
#[gpui_kit::test]
fn the_summary_counts_pieces_past_the_cap(cx: &mut TestAppContext) {
    let text: String = (0..300)
        .map(|i| format!("#t m{i}\n\nplain {i}\n\n"))
        .collect();
    let (_view, cx, _events, _sub) = open(cx, &text, vec![tag("t")]);
    assert_eq!(
        label(cx, SUMMARY_ID).as_deref(),
        Some("Showing text tagged #t \u{b7} 300 pieces")
    );
}

#[gpui_kit::test]
fn clear_removes_every_pick_and_emits_picks_changed(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3 #y\n";
    let (view, cx, events, _sub) = open(cx, text, vec![tag("x"), tag("y")]);
    click(cx, gap_up_id(1));
    assert_eq!(
        chunk_texts(&view, cx),
        ["#x p0", "p2\n\n#x p3 #y"],
        "p2 revealed"
    );
    let before = picks_changed(&events);
    let target = bounds(cx, CLEAR_ID);
    assert!(
        f32::from(target.size.height) >= 24.0 && f32::from(target.size.width) >= 24.0,
        "a 24 px target: {target:?}"
    );
    click(cx, CLEAR_ID);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(picks_changed(&events), before + 1);
    assert_eq!(
        chunk_texts(&view, cx),
        ["#x p0\n\np1\n\np2\n\n#x p3 #y"],
        "the whole note, reveals reset"
    );
    assert!(!shows(cx, SUMMARY_ID));
    assert!(!shows(cx, CLEAR_ID));
    // Picking the same facets again shows them without the old reveal.
    pick_from_menu(&view, cx, &tag("x"));
    pick_from_menu(&view, cx, &tag("y"));
    assert_eq!(
        chunk_texts(&view, cx),
        ["#x p0", "#x p3 #y"],
        "nothing stays revealed"
    );
}

/// Picks that select nothing keep `Clear` (alone on the summary line):
/// that is when it helps most.
#[gpui_kit::test]
fn clear_shows_and_works_when_the_picks_select_nothing(cx: &mut TestAppContext) {
    let text = "# A\n\nx #a\n\n# B\n\ny #b\n";
    let (view, cx, events, _sub) = open(cx, text, vec![tag("a"), heading("b")]);
    assert!(chunk_texts(&view, cx).is_empty());
    assert!(!shows(cx, SUMMARY_ID), "no summary text");
    assert!(shows(cx, CLEAR_ID), "Clear stays");
    let bar = bounds(cx, FACET_BAR_ID);
    let clear = bounds(cx, CLEAR_ID);
    assert!(
        clear.left() > bar.left() + (bar.size.width / 2.0),
        "Clear keeps its place at the line's right end"
    );
    click(cx, CLEAR_ID);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(picks_changed(&events), 1);
    assert_eq!(chunk_texts(&view, cx), ["# A\n\nx #a\n\n# B\n\ny #b"]);
    assert!(!shows(cx, CLEAR_ID));
}

#[gpui_kit::test]
fn no_picks_shows_no_summary_and_no_clear(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    assert!(shows(cx, FACET_BAR_ID));
    assert!(!shows(cx, SUMMARY_ID));
    assert!(!shows(cx, CLEAR_ID));
}

/// Picks that select nothing draw no summary (the empty message speaks for
/// them).
#[gpui_kit::test]
fn picks_that_select_nothing_show_no_summary(cx: &mut TestAppContext) {
    let text = "# A\n\nx #a\n\n# B\n\ny #b\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("a"), heading("b")]);
    assert_eq!(picks(&view, cx).len(), 2);
    assert!(chunk_texts(&view, cx).is_empty());
    assert!(!shows(cx, SUMMARY_ID));
}

#[gpui_kit::test]
fn the_bar_draws_only_picked_headings_and_a_sections_chip(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &many_headings(400), vec![heading("h123")]);
    assert!(shows(cx, pill_id(&heading("h123"))), "the pick stays");
    assert!(!shows(cx, pill_id(&heading("h000"))));
    let sections = menu_id(ListKind::Sections);
    assert_eq!(
        label(cx, sections.clone()).as_deref(),
        Some("Sections 399"),
        "the unpicked headings"
    );
    let chip = bounds(cx, sections.clone());
    assert!(
        f32::from(chip.size.height) >= 24.0,
        "a 24 px target: {chip:?}"
    );
    // A short note counts its headings too, and draws none inline.
    view.update(cx, |view, cx| view.set_text(many_headings(3), cx));
    assert!(!shows(cx, pill_id(&heading("h000"))));
    assert_eq!(label(cx, sections.clone()).as_deref(), Some("Sections 3"));
    // A note without headings has no chip.
    view.update(cx, |view, cx| view.set_text("only #a\n".to_string(), cx));
    assert!(!shows(cx, sections));
}

#[gpui_kit::test]
fn sections_chip_opens_the_heading_list(cx: &mut TestAppContext) {
    // The whole note is one large chunk, laid out in the background.
    let rt = super::start_tests::runtime();
    let _guard = rt.enter();
    let (view, cx, events, _sub) = open(cx, &many_headings(400), vec![]);
    let sections = menu_id(ListKind::Sections);
    assert_eq!(label(cx, sections.clone()).as_deref(), Some("Sections 400"));
    click(cx, sections.clone());
    assert!(view.read_with(cx, |view, _| view.more_input().is_some()));
    assert!(shows(cx, more_item_id(&heading("h000"))));
    click(cx, more_item_id(&heading("h000")));
    assert_eq!(picks(&view, cx), [heading("h000")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(shows(cx, pill_id(&heading("h000"))));
    // The list stays open for more picks; a second click on the chip
    // closes the list it opened, a third opens it again.
    assert!(view.read_with(cx, |view, _| view.more_input().is_some()));
    click(cx, sections.clone());
    assert!(view.read_with(cx, |view, _| view.more_input().is_none()));
    click(cx, sections);
    assert!(view.read_with(cx, |view, _| view.more_input().is_some()));
}

// ── a group showing a facet view (the workspace side) ────────────────────

/// A real `Workspace` on a real temp vault (`start_tests::drive`), its
/// groups showing facet views.
pub(super) mod in_a_group {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext};
    use kimun_core::nfs::VaultPath;
    use kimun_core::note::scan::LinkSpanKind;
    use kimun_core::note::{Facet, FacetKind};
    use kimun_notes::logic::editor::claim::FollowTarget;
    use kimun_notes::logic::layout_tree::GroupId;
    use tokio::runtime::Runtime;

    use super::super::facet::{FACET_VIEW_ID, FacetEvent, FacetView, open_id};
    use super::super::file_dialogs::FileOpKind;
    use super::super::file_ops::FileOpRequest;
    use super::super::modal::ModalKind;
    use super::super::note_menu::{MenuItem, menu_item_element_id};
    use super::super::start_tests::{Started, drive, open_started, vault_start};
    use super::super::tab_strip::group_name;
    use crate::startup::FirstNote;
    use crate::test_vault::{TestVault, test_vault};

    pub(in crate::workspace) fn path(p: &str) -> VaultPath {
        VaultPath::new(p)
    }

    fn tag(key: &str) -> Facet {
        Facet::new(FacetKind::Tag, key)
    }

    const A: &str = "# A\n\nalpha #x\n\n# B\n\nbeta";

    /// A vault holding `notes`, started on `first` and driven until it shows.
    pub(in crate::workspace) fn started(
        cx: &mut TestAppContext,
        rt: &Runtime,
        notes: &[(&str, &str)],
        first: &str,
    ) -> (TestVault, Started) {
        let v = rt.block_on(test_vault(notes, &[]));
        let started = open_started(
            cx,
            vault_start(&v, FirstNote::Note(path(first))),
            v.settings.clone(),
        );
        let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
        drive(rt, cx, |cx| started.text(cx) == want);
        (v, started)
    }

    pub(in crate::workspace) fn active(s: &Started, cx: &TestAppContext) -> GroupId {
        cx.read(|cx| s.workspace.read(cx).layout.active_group())
    }

    pub(in crate::workspace) fn tabs(s: &Started, cx: &TestAppContext) -> usize {
        cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len())
    }

    pub(in crate::workspace) fn facet(
        s: &Started,
        group: GroupId,
        cx: &TestAppContext,
    ) -> Option<Entity<FacetView>> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(group)
                .and_then(|state| state.facet.clone())
        })
    }

    fn source(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<VaultPath> {
        let view = facet(s, group, cx)?;
        cx.read(|cx| view.read(cx).source().cloned())
    }

    fn facet_text(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<String> {
        let view = facet(s, group, cx)?;
        Some(cx.read(|cx| view.read(cx).text().to_string()))
    }

    fn picks(s: &Started, group: GroupId, cx: &TestAppContext) -> Vec<Facet> {
        let view = facet(s, group, cx).expect("a facet view");
        cx.read(|cx| view.read(cx).picks().to_vec())
    }

    pub(in crate::workspace) fn note_in(
        s: &Started,
        group: GroupId,
        cx: &TestAppContext,
    ) -> Option<VaultPath> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(group)
                .and_then(|state| state.notes.as_ref())
                .and_then(|host| host.note.clone())
        })
    }

    fn text_in(s: &Started, group: GroupId, cx: &TestAppContext) -> String {
        cx.read(|cx| {
            let editor = s.workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).editor.get_text()
        })
    }

    pub(in crate::workspace) fn assert_states_in_step(s: &Started, cx: &TestAppContext) {
        cx.read(|cx| {
            let w = s.workspace.read(cx);
            let mut groups = w.layout.region().all_groups();
            groups.sort_by_key(|g| format!("{g:?}"));
            let mut states: Vec<GroupId> = w.groups.keys().copied().collect();
            states.sort_by_key(|g| format!("{g:?}"));
            assert_eq!(groups, states, "one state per group of the region");
        });
    }

    /// Show a facet view of `note` in `group`, driven until it shows.
    pub(in crate::workspace) fn open_facet(
        s: &Started,
        rt: &Runtime,
        group: GroupId,
        note: &str,
        picks: Vec<Facet>,
        cx: &mut TestAppContext,
    ) {
        let note = path(note);
        let want = note.clone();
        s.workspace
            .update(cx, |w, cx| w.open_facet_view(group, note, picks, cx));
        drive(rt, cx, |cx| source(s, group, cx).as_ref() == Some(&want));
    }

    pub(in crate::workspace) fn new_tab(s: &Started, cx: &mut TestAppContext) -> GroupId {
        s.workspace.update(cx, |w, cx| w.new_tab(cx));
        cx.run_until_parked();
        active(s, cx)
    }

    pub(in crate::workspace) fn drawn(
        s: &Started,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> bool {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    pub(in crate::workspace) fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    pub(in crate::workspace) fn open_menu(s: &Started, cx: &mut TestAppContext) {
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.file_ops_chord(window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    pub(in crate::workspace) fn run(s: &Started, request: FileOpRequest, cx: &mut TestAppContext) {
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
        })
        .unwrap();
    }

    /// Replace `group`'s text with `text`, unsaved, then save it.
    pub(in crate::workspace) fn save_text_in(
        s: &Started,
        group: GroupId,
        text: &str,
        cx: &mut TestAppContext,
    ) {
        s.workspace.update(cx, |w, cx| w.focus_group(group, cx));
        let editor = cx.read(|cx| s.workspace.read(cx).state(group).unwrap().editor.clone());
        cx.update(|cx| {
            editor.update(cx, |view, cx| {
                view.editor.set_text(String::new());
                view.editor.insert_at_cursor(text);
                cx.notify();
            })
        });
        s.workspace.update(cx, |w, cx| w.save_note(cx));
    }

    #[gpui_kit::test]
    fn open_facet_view_replaces_the_group_content_and_keeps_states_in_step(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A)], "a.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        assert_eq!(note_in(&s, group, cx), None, "the editor holds no note");
        assert_eq!(text_in(&s, group, cx), "");
        assert_eq!(picks(&s, group, cx), [tag("x")]);
        assert_eq!(facet_text(&s, group, cx).as_deref(), Some(A));
        assert!(drawn(&s, FACET_VIEW_ID, cx));
        let view = facet(&s, group, cx).unwrap();
        let focused = cx
            .update_window(s.handle, |_, window, cx| {
                view.read(cx).focus_handle(cx).is_focused(window)
            })
            .unwrap();
        assert!(focused, "the view has the keyboard");
        assert!(s.placeholder(cx).is_some(), "no note is open");
        let name = cx.read(|cx| group_name(s.workspace.read(cx), group, cx));
        assert_eq!(
            name.as_deref(),
            Some("a.md"),
            "named as the source, no prefix"
        );
        assert_states_in_step(&s, cx);
    }

    /// The view's keys stay in the view: the group's hidden editor types
    /// nothing, and the workspace runs nothing for them.
    #[gpui_kit::test]
    fn keys_do_not_reach_the_hidden_editor(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A)], "a.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        let view = facet(&s, group, cx).unwrap();
        for k in ["j", "k", "space", "enter"] {
            press(&s, k, cx);
        }
        press(&s, "/", cx);
        let open = cx.read(|cx| view.read(cx).more_input().is_some());
        assert!(open, "the view took the slash");
        press(&s, "escape", cx);
        press(&s, "escape", cx);
        assert!(picks(&s, group, cx).is_empty(), "the view took Escape");
        assert_eq!(text_in(&s, group, cx), "", "the editor typed nothing");
        assert_eq!(active(&s, cx), group);
        assert_eq!(tabs(&s, cx), 1, "no tab opened or closed");
        let focused = cx
            .update_window(s.handle, |_, window, cx| {
                view.read(cx).focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap();
        assert!(focused, "the view kept the keyboard");
    }

    #[gpui_kit::test]
    fn the_groups_unsaved_note_is_written_before_its_facet_view_shows(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (v, s) = started(cx, &rt, &[("a.md", A)], "a.md");
        let group = active(&s, cx);
        let editor = cx.read(|cx| s.workspace.read(cx).state(group).unwrap().editor.clone());
        let typed = "# A\n\nedited #x";
        cx.update(|cx| {
            editor.update(cx, |view, cx| {
                view.editor.set_text(String::new());
                view.editor.insert_at_cursor(typed);
                cx.notify();
            })
        });
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        assert_eq!(facet_text(&s, group, cx).as_deref(), Some(typed));
        let disk = rt.block_on(v.vault.get_note_text(&path("a.md"))).unwrap();
        assert_eq!(disk, typed, "flushed before the view showed");
    }

    #[gpui_kit::test]
    fn a_facet_view_may_show_a_note_that_an_editor_group_also_holds(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A)], "a.md");
        let editor_group = active(&s, cx);
        let facet_group = new_tab(&s, cx);
        open_facet(&s, &rt, facet_group, "a.md", vec![], cx);
        assert_eq!(note_in(&s, editor_group, cx), Some(path("a.md")));
        let holder = cx.read(|cx| s.workspace.read(cx).locate_note(&path("a.md")));
        assert_eq!(holder, Some(editor_group), "the facet group holds no note");
        // A plain open from the facet group shows the editor group.
        s.workspace
            .update(cx, |w, cx| w.open_note(path("a.md"), cx));
        cx.run_until_parked();
        assert_eq!(active(&s, cx), editor_group);
        assert_eq!(source(&s, facet_group, cx), Some(path("a.md")));
        assert_states_in_step(&s, cx);
    }

    #[gpui_kit::test]
    fn the_menu_offers_facet_view_for_a_note_and_edit_note_for_a_facet_group(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A)], "a.md");
        let group = active(&s, cx);
        open_menu(&s, cx);
        assert!(drawn(&s, menu_item_element_id(MenuItem::FacetView), cx));
        assert!(!drawn(&s, menu_item_element_id(MenuItem::EditNote), cx));
        press(&s, "f", cx);
        drive(&rt, cx, |cx| source(&s, group, cx).is_some());
        open_menu(&s, cx);
        assert!(drawn(&s, menu_item_element_id(MenuItem::EditNote), cx));
        assert!(!drawn(&s, menu_item_element_id(MenuItem::FacetView), cx));
        for hidden in [FileOpKind::Rename, FileOpKind::Move, FileOpKind::Delete] {
            assert!(!drawn(&s, menu_item_element_id(MenuItem::Note(hidden)), cx));
        }
        assert!(drawn(&s, menu_item_element_id(MenuItem::SplitRight), cx));
        assert!(drawn(&s, menu_item_element_id(MenuItem::NewTab), cx));
        press(&s, "e", cx);
        drive(&rt, cx, |cx| note_in(&s, group, cx).is_some());
        assert!(facet(&s, group, cx).is_none());
    }

    #[gpui_kit::test]
    fn edit_note_returns_the_group_to_the_editor_on_the_same_note(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A), ("b.md", "beta")], "b.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        s.workspace
            .update(cx, |w, cx| w.close_facet_view(group, cx));
        drive(&rt, cx, |cx| note_in(&s, group, cx) == Some(path("a.md")));
        assert!(facet(&s, group, cx).is_none());
        assert_eq!(text_in(&s, group, cx), A);
        assert!(!drawn(&s, FACET_VIEW_ID, cx));
        assert_states_in_step(&s, cx);
    }

    #[gpui_kit::test]
    fn saving_the_source_in_another_group_updates_the_facet_view(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A)], "a.md");
        let editor_group = active(&s, cx);
        let facet_group = new_tab(&s, cx);
        open_facet(&s, &rt, facet_group, "a.md", vec![tag("x")], cx);
        let new = "# A\n\nalpha #x again\n\n# B\n\nbeta";
        save_text_in(&s, editor_group, new, cx);
        drive(&rt, cx, |cx| {
            facet_text(&s, facet_group, cx).as_deref() == Some(new)
        });
        assert_eq!(picks(&s, facet_group, cx), [tag("x")]);
    }

    #[gpui_kit::test]
    fn a_vanished_pick_after_a_save_shows_the_whole_note(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A)], "a.md");
        let editor_group = active(&s, cx);
        let facet_group = new_tab(&s, cx);
        open_facet(&s, &rt, facet_group, "a.md", vec![tag("x")], cx);
        let new = "# A\n\nalpha\n\n# B\n\nbeta";
        save_text_in(&s, editor_group, new, cx);
        drive(&rt, cx, |cx| {
            facet_text(&s, facet_group, cx).as_deref() == Some(new)
        });
        assert!(picks(&s, facet_group, cx).is_empty(), "the pick is gone");
        let view = facet(&s, facet_group, cx).unwrap();
        let shown: String = cx.read(|cx| {
            view.read(cx)
                .chunks()
                .iter()
                .map(|chunk| chunk.read(cx).editor.get_text())
                .collect::<Vec<_>>()
                .join("\n")
        });
        assert!(shown.contains("alpha") && shown.contains("beta"), "{shown}");
    }

    #[gpui_kit::test]
    fn renaming_the_source_retargets_the_facet_view(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A), ("b.md", "beta")], "b.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        run(
            &s,
            FileOpRequest::Rename {
                from: path("a.md"),
                to: path("c.md"),
            },
            cx,
        );
        drive(&rt, cx, |cx| source(&s, group, cx) == Some(path("c.md")));
        assert_eq!(picks(&s, group, cx), [tag("x")]);
        let name = cx.read(|cx| group_name(s.workspace.read(cx), group, cx));
        assert_eq!(name.as_deref(), Some("c.md"));
    }

    #[gpui_kit::test]
    fn deleting_the_source_empties_the_group(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A), ("b.md", "beta")], "b.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![], cx);
        run(&s, FileOpRequest::Delete(path("a.md")), cx);
        drive(&rt, cx, |cx| facet(&s, group, cx).is_none());
        assert_eq!(note_in(&s, group, cx), None);
        assert_eq!(s.placeholder(cx).as_deref(), Some("a.md was deleted"));
        assert!(!drawn(&s, FACET_VIEW_ID, cx));
    }

    #[gpui_kit::test]
    fn a_follow_from_a_facet_view_opens_a_new_tab_and_resolves_relative_to_the_source(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let notes = [
            ("dir/src.md", "see [t](t.md)"),
            ("dir/t.md", "inner"),
            ("t.md", "outer"),
        ];
        let (_v, s) = started(cx, &rt, &notes, "t.md");
        // The lookup reads the index.
        drive(&rt, cx, |cx| !s.indexing(cx));
        let facet_group = new_tab(&s, cx);
        open_facet(&s, &rt, facet_group, "dir/src.md", vec![], cx);
        assert_eq!(tabs(&s, cx), 2);
        let view = facet(&s, facet_group, cx).unwrap();
        view.update(cx, |view, cx| {
            let from = view.source().cloned().expect("a single-note view");
            cx.emit(FacetEvent::Follow {
                from,
                target: FollowTarget::Link {
                    target: "t.md".into(),
                    kind: LinkSpanKind::Markdown,
                },
            })
        });
        drive(&rt, cx, |cx| {
            let group = active(&s, cx);
            note_in(&s, group, cx).is_some_and(|n| n.is_like(&path("dir/t.md")))
                && s.text(cx) == "inner"
        });
        assert_eq!(tabs(&s, cx), 3, "a new tab");
        assert_ne!(active(&s, cx), facet_group);
        assert_eq!(source(&s, facet_group, cx), Some(path("dir/src.md")));
    }

    /// A primary press (Ctrl, or Cmd on macOS) at cell (`col`, `row`) of
    /// `group`'s facet view chunk `i`, sent as real pointer events.
    fn primary_click(
        s: &Started,
        group: GroupId,
        i: usize,
        col: u16,
        row: u16,
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::test::TestEventExt as _;
        let view = facet(s, group, cx).expect("a facet view");
        let held = gpui_kit::Modifiers {
            control: !cfg!(target_os = "macos"),
            platform: cfg!(target_os = "macos"),
            ..Default::default()
        };
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            let chunk = view.read(cx).chunks()[i].clone();
            let painted = chunk
                .read(cx)
                .painted
                .clone()
                .expect("the chunk was painted");
            let c = painted.cells;
            let at = gpui_kit::point(
                painted.origin.x + gpui_kit::px(c.x_of(col) + c.width() / 2.0),
                painted.origin.y + gpui_kit::px(c.y_of(row) + c.height() / 2.0),
            );
            window.change_modifiers(held, cx);
            window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
            window.pointer_up(at, gpui_kit::MouseButton::Left, cx);
            window.change_modifiers(gpui_kit::Modifiers::none(), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn primary_click_on_a_link_still_follows_in_a_new_tab(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let notes = [
            ("dir/src.md", "see [t](t.md)"),
            ("dir/t.md", "inner"),
            ("t.md", "outer"),
        ];
        let (_v, s) = started(cx, &rt, &notes, "t.md");
        drive(&rt, cx, |cx| !s.indexing(cx));
        let facet_group = new_tab(&s, cx);
        open_facet(&s, &rt, facet_group, "dir/src.md", vec![], cx);
        assert_eq!(tabs(&s, cx), 2);
        // `see [t](t.md)` reads `see t`: col 4 is the link text.
        primary_click(&s, facet_group, 0, 4, 0, cx);
        drive(&rt, cx, |cx| {
            let group = active(&s, cx);
            note_in(&s, group, cx).is_some_and(|n| n.is_like(&path("dir/t.md")))
                && s.text(cx) == "inner"
        });
        assert_eq!(tabs(&s, cx), 3, "a new tab");
        assert_eq!(source(&s, facet_group, cx), Some(path("dir/src.md")));
        assert!(
            picks(&s, facet_group, cx).is_empty(),
            "the filter is unchanged"
        );
    }

    #[gpui_kit::test]
    fn primary_click_on_a_tag_still_searches(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A), ("b.md", "beta")], "b.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![], cx);
        // `alpha #x` is row 2 of the whole note; col 7 is the `x`.
        primary_click(&s, group, 0, 7, 2, cx);
        drive(&rt, cx, |cx| s.shown_modal(cx) == Some(ModalKind::Search));
        assert!(picks(&s, group, cx).is_empty(), "the filter is unchanged");
    }

    /// The caret of `group`'s editor.
    fn cursor_in(s: &Started, group: GroupId, cx: &TestAppContext) -> (usize, usize) {
        cx.read(|cx| {
            let editor = s.workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).editor.cursor()
        })
    }

    /// A note with frontmatter: its `#x` pieces start on lines 5 and 9.
    const DEEP: &str = "---\ntags: [y]\n---\n# A\n\nalpha #x\n\n# B\n\nbeta #x\n";

    #[gpui_kit::test]
    fn open_note_opens_the_note_at_the_pieces_first_line(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", DEEP), ("b.md", "beta")], "b.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(open_id(1), cx)
        })
        .unwrap();
        drive(&rt, cx, |cx| {
            let now = active(&s, cx);
            now != group && note_in(&s, now, cx) == Some(path("a.md"))
        });
        assert_eq!(tabs(&s, cx), 2);
        assert_eq!(source(&s, group, cx), Some(path("a.md")), "the view stays");
        let now = active(&s, cx);
        assert_eq!(cursor_in(&s, now, cx), (9, 0), "the caret on the piece");
    }

    /// In the app's window, Escape clears and Ctrl+Z brings the picks back:
    /// no workspace or editor binding takes the chord first.
    #[gpui_kit::test]
    fn ctrl_z_in_a_group_restores_cleared_picks(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A), ("b.md", "beta")], "b.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        press(&s, "escape", cx);
        assert!(picks(&s, group, cx).is_empty());
        let key = if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        };
        press(&s, key, cx);
        assert_eq!(picks(&s, group, cx), [tag("x")]);
    }

    #[gpui_kit::test]
    fn open_note_reuses_an_open_tab_and_jumps(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", DEEP), ("b.md", "beta")], "a.md");
        let first = active(&s, cx);
        let group = new_tab(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        assert_eq!(tabs(&s, cx), 2);
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(open_id(1), cx)
        })
        .unwrap();
        drive(&rt, cx, |cx| active(&s, cx) == first);
        assert_eq!(tabs(&s, cx), 2, "no new tab");
        assert_eq!(cursor_in(&s, first, cx), (9, 0), "the caret on the piece");
        // The first piece lands on its own line.
        s.workspace.update(cx, |w, cx| w.focus_group(group, cx));
        cx.run_until_parked();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(open_id(0), cx)
        })
        .unwrap();
        drive(&rt, cx, |cx| active(&s, cx) == first);
        assert_eq!(cursor_in(&s, first, cx), (5, 0));
    }

    // ── follows that go through a dialog ─────────────────────────────────

    fn follow_from_facet(s: &Started, group: GroupId, target: &str, cx: &mut TestAppContext) {
        let view = facet(s, group, cx).unwrap();
        let target = target.to_string();
        view.update(cx, |view, cx| {
            let from = view.source().cloned().expect("a single-note view");
            cx.emit(FacetEvent::Follow {
                from,
                target: FollowTarget::Link {
                    target,
                    kind: LinkSpanKind::WikiLink,
                },
            })
        });
    }

    fn modal(s: &Started, cx: &mut TestAppContext) -> Option<ModalKind> {
        s.shown_modal(cx)
    }

    /// The link chooser's rows have landed.
    fn chooser_loaded(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
        let view = cx.read(|cx| {
            s.workspace
                .read(cx)
                .picker
                .as_ref()
                .and_then(|p| p.upgrade())
                .expect("a picker is open")
        });
        drive(rt, cx, |cx| {
            cx.read(|cx| !view.read(cx).is_loading() && !view.read(cx).rows().is_empty())
        });
    }

    fn active_note_is(s: &Started, p: &str, cx: &TestAppContext) -> bool {
        note_in(s, active(s, cx), cx).is_some_and(|n| n.is_like(&path(p)))
    }

    /// A vault holding a source that links to no note and to a name two
    /// notes share, started on `b.md`, indexed.
    fn dialog_vault(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
        let notes = [
            ("src.md", "[[nobody]] and [[dup]]"),
            ("dup.md", "Dup one"),
            ("sub/dup.md", "Dup two"),
            ("b.md", "beta"),
        ];
        let (v, s) = started(cx, rt, &notes, "b.md");
        drive(rt, cx, |cx| !s.indexing(cx));
        (v, s)
    }

    #[gpui_kit::test]
    fn a_follow_from_a_facet_view_to_a_missing_note_creates_it_in_a_new_tab(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = dialog_vault(cx, &rt);
        let facet_group = new_tab(&s, cx);
        open_facet(&s, &rt, facet_group, "src.md", vec![], cx);
        follow_from_facet(&s, facet_group, "nobody", cx);
        drive(&rt, cx, |cx| modal(&s, cx) == Some(ModalKind::CreateNote));
        press(&s, "enter", cx);
        drive(&rt, cx, |cx| active_note_is(&s, "nobody.md", cx));
        assert_eq!(tabs(&s, cx), 3, "a new tab");
        assert_ne!(active(&s, cx), facet_group);
        assert_eq!(source(&s, facet_group, cx), Some(path("src.md")));
    }

    #[gpui_kit::test]
    fn a_follow_from_a_facet_view_to_an_ambiguous_name_opens_the_pick_in_a_new_tab(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = dialog_vault(cx, &rt);
        let facet_group = new_tab(&s, cx);
        open_facet(&s, &rt, facet_group, "src.md", vec![], cx);
        follow_from_facet(&s, facet_group, "dup", cx);
        drive(&rt, cx, |cx| modal(&s, cx) == Some(ModalKind::LinkChooser));
        chooser_loaded(&s, &rt, cx);
        press(&s, "enter", cx);
        drive(&rt, cx, |cx| {
            note_in(&s, active(&s, cx), cx).is_some_and(|n| n.get_name() == "dup.md")
        });
        assert_eq!(tabs(&s, cx), 3, "a new tab");
        assert_ne!(active(&s, cx), facet_group);
        assert_eq!(source(&s, facet_group, cx), Some(path("src.md")));
    }

    #[gpui_kit::test]
    fn the_same_dialogs_from_an_editor_group_still_open_in_place(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = dialog_vault(cx, &rt);
        let group = active(&s, cx);
        s.workspace.update(cx, |w, cx| {
            w.follow_link("nobody".into(), LinkSpanKind::WikiLink, cx)
        });
        drive(&rt, cx, |cx| modal(&s, cx) == Some(ModalKind::CreateNote));
        press(&s, "enter", cx);
        drive(&rt, cx, |cx| active_note_is(&s, "nobody.md", cx));
        assert_eq!((tabs(&s, cx), active(&s, cx)), (1, group), "in place");
        s.settle_dialog(cx);
        s.workspace.update(cx, |w, cx| {
            w.follow_link("dup".into(), LinkSpanKind::WikiLink, cx)
        });
        drive(&rt, cx, |cx| modal(&s, cx) == Some(ModalKind::LinkChooser));
        chooser_loaded(&s, &rt, cx);
        press(&s, "enter", cx);
        drive(&rt, cx, |cx| {
            note_in(&s, group, cx).is_some_and(|n| n.get_name() == "dup.md")
        });
        assert_eq!((tabs(&s, cx), active(&s, cx)), (1, group), "in place");
    }

    // ---- Task 7: find it and recognize it ----------------------------------

    use super::super::group_header::{
        CORNER_FACET_ID, GROUP_FACET_ID, GROUP_FACET_MARK_ID, group_element,
    };
    use super::super::palette::PaletteRun;
    use super::super::route::NO_NOTE;
    use super::super::tab_strip::TAB_FACET_MARK_ID;
    use kimun_notes::logic::actions::ActionShortcuts;
    use kimun_notes::logic::layout::Panel;
    use kimun_notes::logic::layout_tree::Axis;

    pub(in crate::workspace) fn click(
        s: &Started,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// The accessible label and toggled state of the element `id`, if drawn.
    pub(in crate::workspace) fn button(
        s: &Started,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> Option<(String, Option<bool>, f32, f32)> {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).filter(|e| e.visible()).map(|e| {
                let b = e.bounds();
                (
                    e.label().unwrap_or_default().to_string(),
                    e.checked(),
                    f32::from(b.size.width),
                    f32::from(b.size.height),
                )
            })
        })
        .unwrap()
    }

    /// Two groups in one tab (the active one first), so each draws a header.
    pub(in crate::workspace) fn split(
        s: &Started,
        rt: &Runtime,
        cx: &mut TestAppContext,
    ) -> [GroupId; 2] {
        let first = active(s, cx);
        s.workspace.update(cx, |w, cx| {
            w.open_note_in_split(path("b.md"), Axis::Horizontal, cx)
        });
        drive(rt, cx, |cx| {
            cx.read(|cx| s.workspace.read(cx).layout.region().all_groups().len() == 2)
                && note_in(s, active(s, cx), cx) == Some(path("b.md"))
        });
        [first, active(s, cx)]
    }

    fn header_vault(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
        started(cx, rt, &[("a.md", A), ("b.md", "beta")], "a.md")
    }

    #[gpui_kit::test]
    fn the_header_button_opens_the_facet_view_for_a_note_group(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let [left, right] = split(&s, &rt, cx);
        let (label, pressed, w, h) =
            button(&s, group_element(GROUP_FACET_ID, 1), cx).expect("the button is drawn");
        assert_eq!(label, "Facet view");
        assert_eq!(pressed, Some(false));
        assert!(w >= 24.0 && h >= 24.0, "{w}x{h}");
        click(&s, group_element(GROUP_FACET_ID, 1), cx);
        drive(&rt, cx, |cx| source(&s, right, cx) == Some(path("b.md")));
        assert!(picks(&s, right, cx).is_empty());
        assert!(facet(&s, left, cx).is_none(), "only its own group");
        assert_states_in_step(&s, cx);
    }

    #[gpui_kit::test]
    fn the_header_button_returns_to_the_editor_from_a_facet_group(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let [_left, right] = split(&s, &rt, cx);
        open_facet(&s, &rt, right, "b.md", vec![], cx);
        let (label, pressed, w, h) =
            button(&s, group_element(GROUP_FACET_ID, 1), cx).expect("the button is drawn");
        assert_eq!(label, "Back to the editor");
        assert_eq!(pressed, Some(true), "it looks pressed");
        assert!(w >= 24.0 && h >= 24.0, "{w}x{h}");
        click(&s, group_element(GROUP_FACET_ID, 1), cx);
        drive(&rt, cx, |cx| note_in(&s, right, cx) == Some(path("b.md")));
        assert!(facet(&s, right, cx).is_none());
    }

    #[gpui_kit::test]
    fn a_lone_group_has_the_button_in_its_corner(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let group = active(&s, cx);
        let (label, ..) = button(&s, CORNER_FACET_ID, cx).expect("drawn in the corner");
        assert_eq!(label, "Facet view");
        click(&s, CORNER_FACET_ID, cx);
        drive(&rt, cx, |cx| source(&s, group, cx).is_some());
        let (label, pressed, ..) = button(&s, CORNER_FACET_ID, cx).unwrap();
        assert_eq!(
            (label.as_str(), pressed),
            ("Back to the editor", Some(true))
        );
    }

    /// In a lone pane the funnel is the one visible way in: it sits outside
    /// the dimmed box, at the normal muted look. Only the note menu's `⋯`
    /// stays dim until the pointer comes near.
    #[gpui_kit::test]
    fn the_lone_pane_funnel_is_not_dimmed(cx: &mut TestAppContext) {
        use super::super::group_header::{CORNER_DIM_ID, CORNER_MENU_ID};
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let dim: gpui_kit::ElementId = CORNER_DIM_ID.into();
        let path_of = |id: &'static str, cx: &mut TestAppContext| {
            cx.update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window.find(id).path().to_vec()
            })
            .unwrap()
        };
        assert!(
            !path_of(CORNER_FACET_ID, cx).contains(&dim),
            "the funnel is not dimmed"
        );
        assert!(
            path_of(CORNER_MENU_ID, cx).contains(&dim),
            "the note menu still is"
        );
    }

    #[gpui_kit::test]
    fn the_button_is_absent_for_an_empty_group_and_an_attachment(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        // An empty group: a second group with nothing in it.
        s.workspace.update(cx, |w, cx| w.new_tab(cx));
        cx.run_until_parked();
        assert!(button(&s, CORNER_FACET_ID, cx).is_none(), "empty group");
        // An attachment in place of the editor.
        let group = active(&s, cx);
        rt.block_on(_v.vault.save_attachment(&path("pic.png"), b"png"))
            .expect("the attachment is saved");
        s.workspace
            .update(cx, |w, cx| w.open_attachment(&path("pic.png"), cx));
        drive(&rt, cx, |cx| {
            cx.read(|cx| {
                s.workspace
                    .read(cx)
                    .state(group)
                    .unwrap()
                    .attachment
                    .is_some()
            })
        });
        assert!(button(&s, CORNER_FACET_ID, cx).is_none(), "attachment");
    }

    #[gpui_kit::test]
    fn the_toggle_action_works_from_the_palette(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let group = active(&s, cx);
        let toggle = ActionShortcuts::ToggleFacetView;
        let row = |cx: &TestAppContext| {
            cx.read(|cx| {
                s.workspace
                    .read(cx)
                    .palette_entries(Panel::Editor, cx)
                    .into_iter()
                    .find(|e| *e.run() == PaletteRun::Chord(toggle.clone()))
                    .map(|e| (e.label().to_string(), e.hint().is_none(), e.blocked()))
            })
        };
        assert_eq!(
            row(cx),
            Some(("+window · toggle facet view".into(), true, None)),
            "a row, no chord, allowed on a note"
        );
        let run = |cx: &mut TestAppContext| {
            let toggle = toggle.clone();
            cx.update_window(s.handle, |_, window, cx| {
                s.workspace.update(cx, |w, cx| {
                    w.run_palette_row(PaletteRun::Chord(toggle), window, cx)
                })
            })
            .unwrap();
        };
        run(cx);
        drive(&rt, cx, |cx| source(&s, group, cx) == Some(path("a.md")));
        assert!(picks(&s, group, cx).is_empty());
        assert!(row(cx).unwrap().2.is_none(), "allowed on a facet group");
        run(cx);
        drive(&rt, cx, |cx| note_in(&s, group, cx) == Some(path("a.md")));
        assert!(facet(&s, group, cx).is_none());
        // An empty group: refused with the usual message.
        new_tab(&s, cx);
        assert_eq!(row(cx).unwrap().2, Some(NO_NOTE));
        run(cx);
        cx.run_until_parked();
        assert_eq!(s.flash(cx).as_deref(), Some(NO_NOTE), "the refusal shows");
        assert!(facet(&s, active(&s, cx), cx).is_none());
    }

    #[gpui_kit::test]
    fn a_facet_group_title_and_tab_show_the_mark(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let [_left, right] = split(&s, &rt, cx);
        assert!(!drawn(&s, group_element(GROUP_FACET_MARK_ID, 1), cx));
        open_facet(&s, &rt, right, "b.md", vec![], cx);
        assert!(drawn(&s, group_element(GROUP_FACET_MARK_ID, 1), cx));
        assert!(!drawn(&s, group_element(GROUP_FACET_MARK_ID, 0), cx));
        assert_eq!(
            cx.read(|cx| group_name(s.workspace.read(cx), right, cx)),
            Some("b.md".into()),
            "the title stays the plain name"
        );
        // The tab strip shows with two tabs: a tab wears the mark while its
        // active group shows a facet view.
        let other = new_tab(&s, cx);
        assert!(!drawn(&s, group_element(TAB_FACET_MARK_ID, 1), cx));
        open_facet(&s, &rt, other, "a.md", vec![], cx);
        assert!(drawn(&s, group_element(TAB_FACET_MARK_ID, 1), cx));
        assert!(drawn(&s, group_element(TAB_FACET_MARK_ID, 0), cx));
        let label = cx.read(|cx| {
            let w = s.workspace.read(cx);
            let tab = w.layout.region().tabs()[1];
            super::super::tab_strip::tab_label(w, tab, cx).to_string()
        });
        assert_eq!(label, "a.md", "an icon element, not a text prefix");
    }

    pub(in crate::workspace) fn bounds_of(
        s: &Started,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).bounds()
        })
        .unwrap()
    }

    /// A lone group floats its corner buttons over the top-right of the
    /// facet view: the bar keeps its controls clear of them, so a click on
    /// the `Sections N` chip opens its list, not the note menu. A group in a
    /// split has a header instead, and its bar keeps its usual inset.
    #[gpui_kit::test]
    fn no_bar_control_sits_under_the_corner_buttons(cx: &mut TestAppContext) {
        use super::super::facet::{CLEAR_ID, FACET_VIEW_ID, ListKind, menu_id, pill_id};
        use super::super::group_header::CORNER_MENU_ID;
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        let sections = menu_id(ListKind::Sections);
        let controls: Vec<gpui_kit::SharedString> = vec![
            CLEAR_ID.into(),
            pill_id(&tag("x")),
            sections.clone(),
            menu_id(ListKind::All),
        ];
        let funnel = bounds_of(&s, CORNER_FACET_ID, cx);
        let menu = bounds_of(&s, CORNER_MENU_ID, cx);
        let corner = funnel.union(&menu);
        for id in &controls {
            let b = bounds_of(&s, id.clone(), cx);
            assert!(
                !b.intersects(&corner),
                "{id} {b:?} under the corner buttons {corner:?}"
            );
        }
        click(&s, sections.clone(), cx);
        let view = facet(&s, group, cx).unwrap();
        assert!(
            cx.read(|cx| view.read(cx).more_input().is_some()),
            "the Sections list opened"
        );
        assert!(
            !cx.read(|cx| s.workspace.read(cx).note_menu.is_some()),
            "the note menu did not"
        );
        press(&s, "escape", cx);
        // In a split, no corner buttons and no extra inset.
        let [left, _right] = split(&s, &rt, cx);
        assert!(facet(&s, left, cx).is_some());
        assert!(!drawn(&s, CORNER_FACET_ID, cx));
        let search = bounds_of(&s, menu_id(ListKind::All), cx);
        let pane = bounds_of(&s, FACET_VIEW_ID, cx);
        let inset = f32::from(pane.right() - search.right());
        assert!(inset <= 20.0, "the usual inset in a split: {inset}");
    }

    #[test]
    fn a_pressed_facet_button_keeps_its_fill_under_hover() {
        use super::super::group_header::{
            CORNER_DIM_OPACITY, corner_idle_opacity, facet_hover_fills,
        };
        assert!(
            facet_hover_fills(false),
            "an idle button takes the hover fill"
        );
        assert!(
            !facet_hover_fills(true),
            "a pressed one keeps its selected fill"
        );
        assert_eq!(corner_idle_opacity(false), CORNER_DIM_OPACITY);
        assert_eq!(corner_idle_opacity(true), 1.0, "pressed stays visible");
    }

    #[gpui_kit::test]
    fn the_menu_entries_still_work(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let group = active(&s, cx);
        open_menu(&s, cx);
        press(&s, "f", cx);
        drive(&rt, cx, |cx| source(&s, group, cx).is_some());
        open_menu(&s, cx);
        press(&s, "e", cx);
        drive(&rt, cx, |cx| note_in(&s, group, cx).is_some());
        assert!(facet(&s, group, cx).is_none());
    }
}

// ── empty states ─────────────────────────────────────────────────────────

const TWO_SECTIONS: &str = "# A\n\nx #a\n\n# B\n\ny #b\n";
const HINT: &str = "Pick a tag, link or section to narrow this note. \
    / finds, j and k step, a click on a tag or link opens it.";

#[gpui_kit::test]
fn an_empty_result_names_the_combination(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, TWO_SECTIONS, vec![tag("a"), heading("b")]);
    assert!(chunk_texts(&view, cx).is_empty());
    let reason = view.read_with(cx, |v, _| v.empty_text());
    assert_eq!(
        label(cx, EMPTY_ID).as_deref(),
        Some(reason.as_str()),
        "the message is the reason"
    );
    assert!(reason.starts_with("No text "), "{reason}");
    assert!(shows(cx, REMOVE_LAST_ID));
    assert!(shows(cx, CLEAR_ID), "Clear stays in the bar");
}

#[gpui_kit::test]
fn a_note_with_no_content_says_so_and_has_no_buttons(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, "", vec![]);
    assert_eq!(label(cx, EMPTY_ID).as_deref(), Some("This note is empty."));
    assert!(!shows(cx, REMOVE_LAST_ID));
}

#[gpui_kit::test]
fn remove_last_pick_undoes_only_the_latest_pick(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, TWO_SECTIONS, vec![tag("a"), heading("b")]);
    click(cx, REMOVE_LAST_ID);
    assert_eq!(picks(&view, cx), [tag("a")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(!shows(cx, REMOVE_LAST_ID), "something shows again");
    assert!(!chunk_texts(&view, cx).is_empty());
}

#[gpui_kit::test]
fn the_hint_shows_with_no_picks_and_some_facets(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    assert_eq!(label(cx, HINT_ID).as_deref(), Some(HINT));
}

#[gpui_kit::test]
fn the_hint_goes_after_the_first_pick_and_stays_gone(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    assert!(shows(cx, HINT_ID));
    pick_from_menu(&view, cx, &tag("a"));
    assert!(!shows(cx, HINT_ID));
    click(cx, CLEAR_ID);
    assert!(!shows(cx, HINT_ID), "not back after Clear");
}

/// A press in a piece picks nothing, so the hint stays; the first pick (a
/// pick from a menu) takes it away.
#[gpui_kit::test]
fn the_first_pick_still_clears_the_hint(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    let at = cell(&view, 0, 1, 4, cx);
    press(cx, at, false);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(
        label(cx, HINT_ID).as_deref(),
        Some(HINT),
        "the hint is unchanged"
    );
    pick_from_menu(&view, cx, &tag("b"));
    assert_eq!(picks(&view, cx), [tag("b")]);
    assert!(!shows(cx, HINT_ID));
}

#[gpui_kit::test]
fn the_hint_stays_gone_after_remove_last_pick(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, TWO_SECTIONS, vec![]);
    pick_from_menu(&view, cx, &tag("a"));
    pick_from_menu(&view, cx, &heading("b"));
    click(cx, REMOVE_LAST_ID);
    click(cx, pill_id(&tag("a")));
    assert!(!shows(cx, HINT_ID));
}

#[gpui_kit::test]
fn a_view_restored_with_picks_shows_no_hint(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    assert!(!shows(cx, HINT_ID));
}

#[gpui_kit::test]
fn a_note_with_no_facets_shows_no_hint(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, "just words\n", vec![]);
    assert!(!shows(cx, HINT_ID));
}

#[gpui_kit::test]
fn tab_and_enter_reach_remove_last_pick(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, TWO_SECTIONS, vec![tag("a"), heading("b")]);
    focus_view(&view, cx);
    tab_to(cx, REMOVE_LAST_ID, 12);
    assert_eq!(
        view.read_with(cx, |view, _| view.ringed()),
        Some(SharedString::from(REMOVE_LAST_ID))
    );
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("a")], "Enter clicked it");
}
