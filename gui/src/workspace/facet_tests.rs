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
    CLEAR_ID, COLUMN_ID, EMPTY_ID, FACET_BAR_ID, FacetEvent, FacetView, HINT_ID, MORE_LIST_ID,
    REMOVE_LAST_ID, SEARCH_ID, SECTIONS_ID, SHOW_MORE_ID, SUMMARY_ID, chunk_id, crumb_id,
    gap_count_id, gap_down_id, gap_up_id, more_button_id, more_item_id, open_id, pill_id,
    row_leader_id, show_all_id,
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
/// events it emits.
pub(super) fn open_with(
    cx: &mut TestAppContext,
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

pub(super) fn picks_changed(events: &Events) -> usize {
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
        [FacetEvent::Follow {
            from: VaultPath::new("/notes/n.md"),
            target: FollowTarget::Label("b".into())
        }]
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
    assert_eq!(
        *events.borrow(),
        [FacetEvent::OpenSource {
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
fn clicking_a_crumb_toggles_that_heading_facet(cx: &mut TestAppContext) {
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
fn open_source_is_a_labelled_text_button_in_the_breadcrumb_row(cx: &mut TestAppContext) {
    let text = "# Top\n\nalpha #x\n";
    let (_view, cx, events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(label(cx, open_id(0)).as_deref(), Some("Open source"));
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
        [FacetEvent::OpenSource {
            note: VaultPath::new("/notes/n.md"),
            line: 2
        }]
    );
}

/// A breadcrumb wider than its row shortens with an ellipsis, outer crumbs
/// first: the innermost crumb ends before `Open source`, which stays whole.
#[gpui_kit::test]
fn a_long_breadcrumb_shortens_before_open_source(cx: &mut TestAppContext) {
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
        "the last crumb ends before Open source: {last:?} {button:?}"
    );
    assert_eq!(
        f32::from(button.size.width),
        open,
        "Open source keeps its width"
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
fn the_gap_row_says_n_hidden_without_the_word_block(cx: &mut TestAppContext) {
    let text = "#x p0\n\np1\n\np2\n\n#x p3\n\np4\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(label(cx, gap_count_id(1)).as_deref(), Some("2 hidden"));
    assert_eq!(label(cx, gap_count_id(2)).as_deref(), Some("1 hidden"));
    assert!(!shows(cx, gap_count_id(0)), "nothing hides above");
}

#[gpui_kit::test]
fn show_all_appears_for_a_gap_of_three_or_fewer_and_not_for_four(cx: &mut TestAppContext) {
    let text = "#x p0\n\na\n\nb\n\nc\n\n#x p4\n\nd\n\ne\n\nf\n\ng\n\n#x p9\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x p0", "#x p4", "#x p9"]);
    assert_eq!(label(cx, gap_count_id(1)).as_deref(), Some("3 hidden"));
    assert_eq!(label(cx, show_all_id(1)).as_deref(), Some("Show all"));
    assert_eq!(label(cx, gap_count_id(2)).as_deref(), Some("4 hidden"));
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
        Some("1 hidden"),
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
        (gap_down_id(1), "Show one more below"),
        (gap_up_id(1), "Show one more above"),
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
        view.read_with(cx, |view, _| view.more_input().is_some()),
        "it stays open for more picks"
    );
    // Picked pills come first, so it shows in the row now.
    assert!(shows(cx, pill_id(&tag("t13"))));
}

/// Fourteen tags: the tag row folds two into `+2 more`.
fn many_tags() -> String {
    (0..14).map(|i| format!("#t{i:02} ")).collect::<String>() + "\n\nplain\n"
}

fn list_open(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.more_input().is_some())
}

/// Whether the view's own focus handle has the keyboard.
fn view_focused(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| view.read(cx).focus_handle(cx).is_focused(window))
}

/// Whether the open list's filter field has the keyboard.
fn field_focused(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
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
    let (view, cx, _events, _sub) = open(cx, &many_tags(), vec![]);
    cx.update(|window, cx| window.render_frame(cx));
    next_frame(cx);
    let bar = bounds(cx, FACET_BAR_ID);
    let first = bounds(cx, chunk_id(0));
    let rows = view.read_with(cx, |view, _| view.sized_rows().to_vec());
    click(cx, more_button_id(FacetKind::Tag));
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
    let chip = bounds(cx, more_button_id(FacetKind::Tag));
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
    let (view, cx, _events, _sub) = open(cx, &many_tags(), vec![]);
    click(cx, more_button_id(FacetKind::Tag));
    assert!(field_focused(&view, cx), "the chip's list");
    key(cx, "escape");
    // The cross-kind search list (`/` opens it).
    let text = "# Head\n\nsome #a and [[plan]]\n";
    view.update(cx, |view, cx| view.set_text(text.to_string(), cx));
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_list(None, window, cx)));
    cx.run_until_parked();
    assert!(field_focused(&view, cx), "the search list");
    assert!(shows(cx, MORE_LIST_ID));
    assert!(shows(cx, more_item_id(&tag("a"))));
    assert!(shows(
        cx,
        more_item_id(&Facet::new(FacetKind::NoteLink, "plan"))
    ));
    assert!(shows(cx, more_item_id(&heading("head"))));
}

/// Escape closes the list and nothing else: the picks stay. The keyboard
/// goes back to the chip that opened the list (a tab stop).
#[gpui_kit::test]
fn escape_closes_the_list_and_returns_focus_to_the_chip(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, &many_tags(), vec![]);
    click(cx, more_button_id(FacetKind::Tag));
    click(cx, more_item_id(&tag("t13")));
    assert!(list_open(&view, cx));
    key(cx, "escape");
    assert!(!list_open(&view, cx), "closed");
    assert!(!shows(cx, MORE_LIST_ID));
    assert_eq!(picks(&view, cx), [tag("t13")], "the picks stay");
    assert_eq!(picks_changed(&events), 1);
    assert!(
        has_focus(cx, more_button_id(FacetKind::Tag)),
        "the keyboard is back on the chip"
    );
}

/// A list whose chip is no tab stop (the row's every facet is picked, so
/// the list hangs from the row's icon) gives the keyboard back to the view.
#[gpui_kit::test]
fn a_list_with_no_chip_gives_the_keyboard_back_to_the_view(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "#a #b\n", vec![]);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_list(Some(FacetKind::Tag), window, cx)
        })
    });
    cx.run_until_parked();
    assert!(list_open(&view, cx));
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(view_focused(&view, cx), "the keyboard is back on the view");
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

fn query(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> String {
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
fn slash_opens_the_search_list_and_focuses_its_field(cx: &mut TestAppContext) {
    let text = "# Head\n\nsome #a and [[plan]]\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![]);
    assert!(shows(cx, SEARCH_ID), "the search button is always there");
    assert_eq!(label(cx, SEARCH_ID).as_deref(), Some("Search facets (/)"));
    let size = bounds(cx, SEARCH_ID).size;
    assert!(
        size.width >= px(24.) && size.height >= px(24.),
        "24 px: {size:?}"
    );
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    assert_eq!(query(&view, cx), "", "the slash is not typed into it");
    assert!(shows(cx, more_item_id(&tag("a"))));
    assert!(shows(
        cx,
        more_item_id(&Facet::new(FacetKind::NoteLink, "plan"))
    ));
    assert!(shows(cx, more_item_id(&heading("head"))));
    // It hangs from the search button.
    let button = bounds(cx, SEARCH_ID);
    let list = bounds(cx, MORE_LIST_ID);
    assert!(list.top() >= button.bottom(), "{list:?} {button:?}");
    // Escape closes it and gives the keyboard to the button.
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(has_focus(cx, SEARCH_ID));
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
    click(cx, more_button_id(FacetKind::Tag));
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
    // Reading order: the bar (summary line, then each row), its search
    // button, then the column (gap, piece crumbs, Open source, gap).
    let want: Vec<SharedString> = vec![
        CLEAR_ID.into(),
        pill_id(&tag("a")),
        pill_id(&tag("b")),
        pill_id(&heading("head")),
        SEARCH_ID.into(),
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
    let (view, cx, events, _sub) = open(cx, &many_tags(), vec![]);
    focus_view(&view, cx);
    tab_to(cx, pill_id(&tag("t01")), 20);
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("t01")]);
    assert!(has_focus(cx, pill_id(&tag("t01"))), "it keeps the keyboard");
    key(cx, "space");
    assert!(picks(&view, cx).is_empty(), "Space too");
    assert_eq!(picks_changed(&events), 2);
    // A chip does what a click does: `+N more` opens its list.
    tab_to(cx, more_button_id(FacetKind::Tag), 20);
    key(cx, "enter");
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    key(cx, "escape");
    assert!(has_focus(cx, more_button_id(FacetKind::Tag)));
    // `Clear` too.
    key(cx, "shift-tab");
    key(cx, "enter");
    assert_eq!(picks(&view, cx).len(), 1, "a pill picked");
    tab_to(cx, CLEAR_ID, 30);
    key(cx, "enter");
    assert!(picks(&view, cx).is_empty(), "Clear cleared");
    assert!(
        view_focused(&view, cx),
        "Clear is gone: the keyboard is on the view"
    );
}

/// A press outside the list closes it; the press still does what it does
/// there (here, toggles a row pill).
#[gpui_kit::test]
fn an_outside_click_closes_the_list(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &many_tags(), vec![]);
    click(cx, more_button_id(FacetKind::Tag));
    assert!(list_open(&view, cx));
    click(cx, chunk_id(0));
    assert!(!list_open(&view, cx), "a press on the content closes it");
    assert!(!shows(cx, MORE_LIST_ID));
    click(cx, more_button_id(FacetKind::Tag));
    assert!(list_open(&view, cx));
    click(cx, pill_id(&tag("t00")));
    assert!(!list_open(&view, cx), "a press on a row pill closes it");
    assert_eq!(picks(&view, cx), [tag("t00")], "and toggles that pill");
}

#[gpui_kit::test]
fn picking_in_the_list_toggles_and_keeps_it_open(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, &many_tags(), vec![]);
    click(cx, more_button_id(FacetKind::Tag));
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
    // A second press on a picked one unpicks it, still open.
    click(cx, more_item_id(&tag("t12")));
    assert_eq!(picks(&view, cx), [tag("t13")]);
    assert!(list_open(&view, cx));
}

/// A note with no facet draws no bar, so `/` has no search button to hang
/// a list from: it opens nothing, and the keyboard stays on the view.
#[gpui_kit::test]
fn slash_does_nothing_in_a_note_with_no_facets(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "just text\n\nmore text\n", vec![]);
    assert!(!shows(cx, FACET_BAR_ID));
    focus_view(&view, cx);
    key(cx, "/");
    assert!(!list_open(&view, cx), "no list without a bar");
    assert!(view_focused(&view, cx), "the keyboard stays on the view");
}

/// A live edit that takes away every facet of the open list's kind leaves
/// it nowhere to hang: it closes and the view takes the keyboard back.
#[gpui_kit::test]
fn an_edit_that_removes_the_open_lists_kind_closes_it(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# H\n\n#a #b\n", vec![]);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_list(Some(FacetKind::Tag), window, cx)
        })
    });
    cx.run_until_parked();
    assert!(field_focused(&view, cx));
    view.update(cx, |view, cx| view.set_text("# H\n\nplain\n".into(), cx));
    cx.update(|window, cx| window.render_frame(cx));
    cx.run_until_parked();
    assert!(!list_open(&view, cx), "no tag row: the tag list closed");
    assert!(view_focused(&view, cx), "the keyboard is back on the view");
    // The search list closes once the bar goes.
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    view.update(cx, |view, cx| view.set_text("plain\n".into(), cx));
    cx.update(|window, cx| window.render_frame(cx));
    cx.run_until_parked();
    assert!(!list_open(&view, cx), "no bar: the search closed");
    assert!(view_focused(&view, cx));
    key(cx, "/");
    assert!(!list_open(&view, cx), "and `/` opens nothing now");
}

/// Picked facets move to the front of the list; the highlight follows the
/// facet Enter toggled, so a second Enter toggles it back.
#[gpui_kit::test]
fn the_highlight_follows_the_facet_enter_toggled(cx: &mut TestAppContext) {
    let text = "#a #a #a\n\n#b #b\n\n#c\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    assert_eq!(
        view.read_with(cx, |view, cx| view.list_results(cx)),
        [tag("a"), tag("b"), tag("c")]
    );
    key(cx, "down");
    key(cx, "down");
    assert!(selected(cx, more_item_id(&tag("c"))));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("c")]);
    assert!(
        selected(cx, more_item_id(&tag("c"))),
        "c stays lit where it moved"
    );
    key(cx, "enter");
    assert!(picks(&view, cx).is_empty(), "the second Enter unpicks c");
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
    click(cx, pill_id(&tag("x")));
    click(cx, pill_id(&tag("y")));
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
fn each_row_has_a_leading_icon_for_its_kind(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# Head\n\nsome #a and [[plan]]\n", vec![]);
    let kinds = [
        (FacetKind::Tag, "Tags", tag("a")),
        (
            FacetKind::NoteLink,
            "Links",
            Facet::new(FacetKind::NoteLink, "plan"),
        ),
        (FacetKind::Heading, "Sections", heading("head")),
    ];
    let mut last_top = None;
    for (kind, name, first) in kinds {
        assert_eq!(label(cx, row_leader_id(kind)).as_deref(), Some(name));
        let leader = bounds(cx, row_leader_id(kind));
        let pill = bounds(cx, pill_id(&first));
        assert!(leader.right() <= pill.left(), "{name}: the icon leads");
        if let Some(top) = last_top {
            assert!(leader.top() > top, "{name}: rows go tag, link, heading");
        }
        last_top = Some(leader.top());
    }
    // A kind the note lacks has no row and no leader.
    view.update(cx, |view, cx| view.set_text("only #a\n".to_string(), cx));
    assert!(shows(cx, row_leader_id(FacetKind::Tag)));
    assert!(!shows(cx, row_leader_id(FacetKind::NoteLink)));
    assert!(!shows(cx, row_leader_id(FacetKind::Heading)));
}

#[gpui_kit::test]
fn the_heading_row_collapses_to_a_sections_chip_for_a_long_note(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &many_headings(400), vec![heading("h123")]);
    assert!(shows(cx, pill_id(&heading("h123"))), "the pick stays");
    assert!(!shows(cx, pill_id(&heading("h000"))));
    assert!(!shows(cx, more_button_id(FacetKind::Heading)));
    assert_eq!(label(cx, SECTIONS_ID).as_deref(), Some("Sections 400"));
    let chip = bounds(cx, SECTIONS_ID);
    assert!(
        f32::from(chip.size.height) >= 24.0,
        "a 24 px target: {chip:?}"
    );
    // A short note keeps its heading pills.
    view.update(cx, |view, cx| view.set_text(many_headings(3), cx));
    assert!(shows(cx, pill_id(&heading("h000"))));
    assert!(!shows(cx, SECTIONS_ID));
}

#[gpui_kit::test]
fn sections_chip_opens_the_heading_list(cx: &mut TestAppContext) {
    // The whole note is one large chunk, laid out in the background.
    let rt = super::start_tests::runtime();
    let _guard = rt.enter();
    let (view, cx, events, _sub) = open(cx, &many_headings(400), vec![]);
    assert_eq!(label(cx, SECTIONS_ID).as_deref(), Some("Sections 400"));
    click(cx, SECTIONS_ID);
    assert!(view.read_with(cx, |view, _| view.more_input().is_some()));
    assert!(shows(cx, more_item_id(&heading("h000"))));
    click(cx, more_item_id(&heading("h000")));
    assert_eq!(picks(&view, cx), [heading("h000")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(shows(cx, pill_id(&heading("h000"))));
    // The list stays open for more picks; a second click on the chip
    // closes the list it opened, a third opens it again.
    assert!(view.read_with(cx, |view, _| view.more_input().is_some()));
    click(cx, SECTIONS_ID);
    assert!(view.read_with(cx, |view, _| view.more_input().is_none()));
    click(cx, SECTIONS_ID);
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

    fn bounds_of(
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
    /// the search button opens the search, not the note menu. A group in a
    /// split has a header instead, and its bar keeps its usual inset.
    #[gpui_kit::test]
    fn no_bar_control_sits_under_the_corner_buttons(cx: &mut TestAppContext) {
        use super::super::facet::{CLEAR_ID, FACET_VIEW_ID, SEARCH_ID, pill_id};
        use super::super::group_header::CORNER_MENU_ID;
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = header_vault(cx, &rt);
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![tag("x")], cx);
        let controls: Vec<gpui_kit::SharedString> = vec![
            SEARCH_ID.into(),
            CLEAR_ID.into(),
            pill_id(&tag("x")),
            pill_id(&Facet::new(FacetKind::Heading, "A")),
            pill_id(&Facet::new(FacetKind::Heading, "B")),
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
        click(&s, SEARCH_ID, cx);
        let view = facet(&s, group, cx).unwrap();
        assert!(
            cx.read(|cx| view.read(cx).more_input().is_some()),
            "the search list opened"
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
        let search = bounds_of(&s, SEARCH_ID, cx);
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
const HINT: &str = "Pick a tag, link or section to narrow this note.";

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
    let (_view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    assert!(shows(cx, HINT_ID));
    click(cx, pill_id(&tag("a")));
    assert!(!shows(cx, HINT_ID));
    click(cx, CLEAR_ID);
    assert!(!shows(cx, HINT_ID), "not back after Clear");
}

#[gpui_kit::test]
fn the_hint_also_goes_after_a_press_on_an_inline_tag(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    let at = cell(&view, 0, 6, 4, cx);
    press(cx, at, false);
    assert_eq!(picks(&view, cx), [tag("b")]);
    assert!(!shows(cx, HINT_ID));
}

#[gpui_kit::test]
fn the_hint_stays_gone_after_remove_last_pick(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, TWO_SECTIONS, vec![]);
    click(cx, pill_id(&tag("a")));
    click(cx, pill_id(&heading("b")));
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
