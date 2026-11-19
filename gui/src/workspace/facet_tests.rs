//! UI tests for the **Facet view** entity (ADR-0061): a `FacetView` alone in
//! a headless window, driven by clicks on its pills, gap buttons and chunks.
//! The workspace side (a group showing one) is tested further down, by the
//! next task.
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{
    Entity, MouseButton, Pixels, Point, SharedString, Subscription, TestAppContext,
    VisualTestContext, point, px,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, FacetKind, MAX_RANGES};
use kimun_notes::logic::editor::claim::FollowTarget;

use super::facet::{
    FacetEvent, FacetView, SHOW_MORE_ID, chunk_id, gap_down_id, more_button_id, more_item_id,
    open_id, pill_id,
};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};

type Events = Rc<RefCell<Vec<FacetEvent>>>;

/// A facet view of `text` (a note at `/notes/n.md`) with `picks`, in its own
/// window, drawn once, and the events it emits.
fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    picks: Vec<Facet>,
) -> (
    Entity<FacetView>,
    &'a mut VisualTestContext,
    Events,
    Subscription,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let text = text.to_string();
    let (view, cx) = cx.add_window_view(move |_, cx| {
        FacetView::new(VaultPath::new("/notes/n.md"), text, picks, cx)
    });
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

fn click(cx: &mut VisualTestContext, id: impl Into<SharedString>) {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    });
    cx.run_until_parked();
}

fn shows(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> bool {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
}

/// Scroll the view's column to its end and draw it there.
fn to_bottom(view: &Entity<FacetView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        view.read(cx).scroll.scroll_to_bottom();
        window.refresh();
        window.render_frame(cx);
    });
}

fn picks(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<Facet> {
    view.read_with(cx, |view, _| view.picks().to_vec())
}

/// The text each chunk's editor holds, in order.
fn chunk_texts(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<String> {
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
fn cell(
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
fn press(cx: &mut VisualTestContext, at: Point<Pixels>, primary: bool) {
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

fn tag(key: &str) -> Facet {
    Facet::new(FacetKind::Tag, key)
}

fn picks_changed(events: &Events) -> usize {
    events
        .borrow()
        .iter()
        .filter(|e| matches!(e, FacetEvent::PicksChanged))
        .count()
}

const NOTE: &str = "# Head\n\nsome #a\n\nmore #b\n";

// ── the helpers (no window) ──────────────────────────────────────────────

#[test]
fn a_plain_press_picks_the_facet_under_it() {
    use super::facet::pressed_facet;
    use kimun_core::note::scan::LinkSpanKind;
    let at = VaultPath::new("/notes/n.md");
    let label = FollowTarget::Label("Idea".into());
    assert_eq!(
        pressed_facet(Some(&label), "x #Idea", &at),
        Some(tag("idea"))
    );
    let wiki = FollowTarget::Link {
        target: "Plan#goals".into(),
        kind: LinkSpanKind::WikiLink,
    };
    assert_eq!(
        pressed_facet(Some(&wiki), "see [[Plan#goals]]", &at),
        Some(Facet::new(FacetKind::NoteLink, "plan"))
    );
    let md = FollowTarget::Link {
        target: "../work/dup.md".into(),
        kind: LinkSpanKind::Markdown,
    };
    assert_eq!(
        pressed_facet(Some(&md), "", &at),
        Some(Facet::new(FacetKind::NoteLink, "dup"))
    );
    let url = FollowTarget::Link {
        target: "https://example.com".into(),
        kind: LinkSpanKind::Markdown,
    };
    assert_eq!(
        pressed_facet(Some(&url), "# Head [x](https://example.com)", &at),
        None
    );
    let image = FollowTarget::Link {
        target: "pic.png".into(),
        kind: LinkSpanKind::Image,
    };
    assert_eq!(pressed_facet(Some(&image), "", &at), None);
    assert_eq!(
        pressed_facet(None, "## Big Plans", &at),
        Some(Facet::new(FacetKind::Heading, "Big Plans"))
    );
    assert_eq!(pressed_facet(None, "plain text", &at), None);
}

// ── drawing ──────────────────────────────────────────────────────────────

#[gpui_kit::test]
fn no_picks_draws_one_chunk_with_the_whole_note(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    assert_eq!(chunk_texts(&view, cx), ["# Head\n\nsome #a\n\nmore #b"]);
    assert!(shows(cx, chunk_id(0)));
    assert!(shows(cx, pill_id(&tag("a"))));
    assert!(shows(cx, pill_id(&Facet::new(FacetKind::Heading, "Head"))));
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

/// Runs the next-frame callbacks the last frame asked for; how many ran.
fn next_frame(cx: &mut VisualTestContext) -> usize {
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
    click(cx, pill_id(&tag("a")));
    assert_eq!(picks(&view, cx), [tag("a")]);
    assert_eq!(chunk_texts(&view, cx), ["some #a"]);
    assert_eq!(picks_changed(&events), 1);
    // The editor is reused for the new text.
    assert_eq!(chunk(&view, 0, cx), first);
    // A second click unpicks it: the whole note again.
    click(cx, pill_id(&tag("a")));
    assert!(picks(&view, cx).is_empty());
    assert_eq!(chunk_texts(&view, cx), ["# Head\n\nsome #a\n\nmore #b"]);
    assert_eq!(picks_changed(&events), 2);
}

#[gpui_kit::test]
fn clicking_an_inline_tag_in_a_chunk_toggles_its_pill(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![]);
    // "more #b" is row 4; col 6 is the `b`.
    let at = cell(&view, 0, 6, 4, cx);
    press(cx, at, false);
    assert_eq!(picks(&view, cx), [tag("b")]);
    assert_eq!(chunk_texts(&view, cx), ["more #b"]);
    assert_eq!(picks_changed(&events), 1);
}

#[gpui_kit::test]
fn clicking_a_note_link_toggles_it_and_a_url_toggles_nothing(cx: &mut TestAppContext) {
    let text = "see [[Plan]] now\n\nweb [x](https://e.com)\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    let url = cell(&view, 0, 4, 2, cx);
    press(cx, url, false);
    assert!(picks(&view, cx).is_empty(), "a URL is no facet");
    assert_eq!(picks_changed(&events), 0);
    let link = cell(&view, 0, 7, 0, cx);
    press(cx, link, false);
    assert_eq!(picks(&view, cx), [Facet::new(FacetKind::NoteLink, "plan")]);
}

#[gpui_kit::test]
fn clicking_a_heading_line_toggles_the_heading_facet(cx: &mut TestAppContext) {
    let text = "# Head\n\nsome #a\n\n# Other\n\nmore\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![]);
    let at = cell(&view, 0, 3, 4, cx);
    press(cx, at, false);
    assert_eq!(picks(&view, cx), [Facet::new(FacetKind::Heading, "Other")]);
    assert_eq!(chunk_texts(&view, cx), ["# Other\n\nmore"]);
}

/// A heading line inside a code block reads as a heading but is no facet
/// of the note: the press toggles nothing.
#[gpui_kit::test]
fn a_press_on_a_facet_the_note_does_not_hold_does_nothing(cx: &mut TestAppContext) {
    let text = "```\n# not a heading\n```\n\nafter #a\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    let at = cell(&view, 0, 4, 1, cx);
    press(cx, at, false);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(picks_changed(&events), 0);
}

/// A double press is two presses (the editor emits one event per press):
/// it toggles twice, back to where it started.
#[gpui_kit::test]
fn a_double_press_toggles_twice(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "a #x b\n", vec![]);
    let at = cell(&view, 0, 3, 0, cx);
    press(cx, at, false);
    let at = cell(&view, 0, 3, 0, cx);
    press(cx, at, false);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(picks_changed(&events), 2);
}

#[gpui_kit::test]
fn a_primary_press_emits_follow_and_leaves_picks_alone(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, NOTE, vec![]);
    let at = cell(&view, 0, 6, 4, cx);
    press(cx, at, true);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(
        *events.borrow(),
        [FacetEvent::Follow(FollowTarget::Label("b".into()))]
    );
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
fn open_source_emits_the_first_line_of_the_chunk(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\n#x p2\n";
    let (_view, cx, events, _sub) = open(cx, text, vec![tag("x")]);
    click(cx, open_id(1));
    assert_eq!(*events.borrow(), [FacetEvent::OpenSource { line: 4 }]);
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
        assert_eq!(view.source(), &VaultPath::new("/other/m.md"))
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

// ── the "+N more" list ───────────────────────────────────────────────────

#[gpui_kit::test]
fn plus_n_more_opens_a_fuzzy_list_and_picking_from_it_toggles(cx: &mut TestAppContext) {
    let text: String = (0..14).map(|i| format!("#t{i:02} ")).collect();
    let (view, cx, events, _sub) = open(cx, &text, vec![]);
    assert!(shows(cx, pill_id(&tag("t11"))));
    assert!(!shows(cx, pill_id(&tag("t13"))), "folded into +2 more");
    click(cx, more_button_id(FacetKind::Tag));
    assert!(shows(cx, more_item_id(&tag("t00"))));
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
        view.read_with(cx, |view, _| view.more_input().is_none()),
        "it closes"
    );
    // Picked pills come first, so it shows in the row now.
    assert!(shows(cx, pill_id(&tag("t13"))));
}

// ── a group showing a facet view (the workspace side) ────────────────────

/// A real `Workspace` on a real temp vault (`start_tests::drive`), its
/// groups showing facet views.
mod in_a_group {
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

    fn path(p: &str) -> VaultPath {
        VaultPath::new(p)
    }

    fn tag(key: &str) -> Facet {
        Facet::new(FacetKind::Tag, key)
    }

    const A: &str = "# A\n\nalpha #x\n\n# B\n\nbeta";

    /// A vault holding `notes`, started on `first` and driven until it shows.
    fn started(
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

    fn active(s: &Started, cx: &TestAppContext) -> GroupId {
        cx.read(|cx| s.workspace.read(cx).layout.active_group())
    }

    fn tabs(s: &Started, cx: &TestAppContext) -> usize {
        cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len())
    }

    fn facet(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<Entity<FacetView>> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(group)
                .and_then(|state| state.facet.clone())
        })
    }

    fn source(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<VaultPath> {
        let view = facet(s, group, cx)?;
        Some(cx.read(|cx| view.read(cx).source().clone()))
    }

    fn facet_text(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<String> {
        let view = facet(s, group, cx)?;
        Some(cx.read(|cx| view.read(cx).text().to_string()))
    }

    fn picks(s: &Started, group: GroupId, cx: &TestAppContext) -> Vec<Facet> {
        let view = facet(s, group, cx).expect("a facet view");
        cx.read(|cx| view.read(cx).picks().to_vec())
    }

    fn note_in(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<VaultPath> {
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

    fn assert_states_in_step(s: &Started, cx: &TestAppContext) {
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
    fn open_facet(
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

    fn new_tab(s: &Started, cx: &mut TestAppContext) -> GroupId {
        s.workspace.update(cx, |w, cx| w.new_tab(cx));
        cx.run_until_parked();
        active(s, cx)
    }

    fn drawn(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn open_menu(s: &Started, cx: &mut TestAppContext) {
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.file_ops_chord(window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn run(s: &Started, request: FileOpRequest, cx: &mut TestAppContext) {
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
        })
        .unwrap();
    }

    /// Replace `group`'s text with `text`, unsaved, then save it.
    fn save_text_in(s: &Started, group: GroupId, text: &str, cx: &mut TestAppContext) {
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
        view.update(cx, |_, cx| {
            cx.emit(FacetEvent::Follow(FollowTarget::Link {
                target: "t.md".into(),
                kind: LinkSpanKind::Markdown,
            }))
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

    #[gpui_kit::test]
    fn open_source_opens_the_note_in_a_new_tab(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started(cx, &rt, &[("a.md", A), ("b.md", "beta")], "b.md");
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(open_id(0), cx)
        })
        .unwrap();
        drive(&rt, cx, |cx| {
            let now = active(&s, cx);
            now != group && note_in(&s, now, cx) == Some(path("a.md"))
        });
        assert_eq!(tabs(&s, cx), 2);
        assert_eq!(source(&s, group, cx), Some(path("a.md")), "the view stays");
    }

    // ── follows that go through a dialog ─────────────────────────────────

    fn follow_from_facet(s: &Started, group: GroupId, target: &str, cx: &mut TestAppContext) {
        let view = facet(s, group, cx).unwrap();
        let target = target.to_string();
        view.update(cx, |_, cx| {
            cx.emit(FacetEvent::Follow(FollowTarget::Link {
                target,
                kind: LinkSpanKind::WikiLink,
            }))
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
}
