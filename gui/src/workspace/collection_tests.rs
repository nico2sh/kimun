//! UI tests for a **Facet view** over a **Collection** (CONTEXT.md): the view
//! as a list of member notes. A single-note view is a collection of one
//! member; these tests check what each piece says about its own note.
#![cfg(test)]

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Entity, Role, SharedString, TestAppContext, VisualTestContext};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, MAX_RANGES};
use kimun_notes::logic::editor::claim::FollowTarget;

use super::facet::{
    COLLECTION_LINE_ID, COLUMN_ID, EMPTY_ID, FacetEvent, FacetView, ListKind, MemberInit,
    REFRESH_ID, REMOVE_LAST_ID, SHOW_MORE_ID, SUMMARY_ID, chunk_id, crumb_id, dateline_folder_id,
    dateline_id, dateline_meta_id, dateline_rule_id, dateline_title_id, gap_count_id, gap_down_id,
    gap_up_id, menu_id, more_item_id, open_id, show_all_id,
};
use super::facet_tests::{
    Events, bounds, cell, chunk_texts, click, focus_view, has_focus, heading, key, label, open,
    open_with, picks, picks_changed, press, shows, tab_to, tag, to_bottom,
};

fn member(path: &str, title: &str, text: &str) -> MemberInit {
    MemberInit {
        path: VaultPath::new(path),
        title: title.to_string(),
        text: text.to_string(),
    }
}

/// A collection view of `notes` (path, title, text) for the query `q`,
/// `total` hits, with `picks`, in its own window, drawn once.
fn open_collection<'a>(
    cx: &'a mut TestAppContext,
    notes: &[(&str, &str, &str)],
    total: usize,
    picks: Vec<Facet>,
) -> (
    Entity<FacetView>,
    &'a mut VisualTestContext,
    Events,
    gpui_kit::Subscription,
) {
    let members: Vec<MemberInit> = notes
        .iter()
        .map(|(path, title, text)| member(path, title, text))
        .collect();
    open_with(cx, move |cx| {
        FacetView::new_collection_at("#q".into(), members, total, picks, ten_forty_two, cx)
    })
}

/// The test clock: the collection line's age reads `10:42`.
fn ten_forty_two() -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt(10, 42, 0).unwrap()
}

/// What the collection line says.
fn line(cx: &mut VisualTestContext) -> Option<String> {
    label(cx, COLLECTION_LINE_ID)
}

/// Let the report of a refresh run out (6 seconds).
fn refresh_report_ends(cx: &mut VisualTestContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(6));
    cx.run_until_parked();
}

const A: &str = "/a/alpha.md";
const B: &str = "/b/beta.md";
const C: &str = "/c/gamma.md";

fn p(path: &str) -> VaultPath {
    VaultPath::new(path)
}

fn paths(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<VaultPath> {
    view.read_with(cx, |view, _| view.member_paths())
}

/// Whether the last frame drew `id` at all, on screen or scrolled away.
fn exists(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> bool {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
}

/// Every element the last frame drew with the heading role.
fn headings(cx: &mut VisualTestContext) -> Vec<gpui_kit::test::ElementSnapshot> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.find_all_by_role(Role::Heading)
    })
}

/// Every element the last frame drew with the accessible name `name`.
fn named(cx: &mut VisualTestContext, name: &str) -> Vec<gpui_kit::test::ElementSnapshot> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.find_all_by_label(name)
    })
}

/// `a - b` in pixels, as a plain number.
fn dist(a: gpui_kit::Pixels, b: gpui_kit::Pixels) -> f32 {
    f32::from(a - b)
}

#[gpui_kit::test]
fn a_collection_shows_every_note_whole_with_no_picks(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a1\n\na2\n"), (B, "Beta", "b1\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    assert_eq!(chunk_texts(&view, cx), ["a1\n\na2", "b1"]);
    view.read_with(cx, |view, _| {
        assert!(view.is_collection());
        assert_eq!(view.query(), Some("#q"));
        assert_eq!(view.source(), None, "a collection has no one source");
        assert_eq!(view.member_paths(), [p(A), p(B)], "list order kept");
    });
}

#[gpui_kit::test]
fn a_single_note_view_is_no_collection(cx: &mut TestAppContext) {
    let (single, cx, _e, _s) = open(cx, "x\n", vec![]);
    single.read_with(cx, |view, _| {
        assert!(!view.is_collection());
        assert_eq!(view.query(), None);
        assert_eq!(view.member_paths(), [p("/notes/n.md")]);
    });
    assert!(!shows(cx, COLLECTION_LINE_ID));
}

/// Supersedes slice 3's `every_piece_shows_its_notes_title_in_its_breadcrumb_row`:
/// the note's title moved to its dateline, so a breadcrumb row holds only
/// the heading crumbs and `Open note`.
#[gpui_kit::test]
fn breadcrumbs_no_longer_start_with_the_note_title(cx: &mut TestAppContext) {
    let alpha = "# Day\n\n#x a1\n\nfar\n\nfar\n\n#x a2\n";
    let notes = [(A, "Alpha", alpha), (B, "Beta", "#x b1\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x a1", "#x a2", "#x b1"]);
    // Each title is drawn once, by its dateline, outside every piece. A
    // piece group with no crumbs is named after its note too, but draws
    // no title text: it is left out here.
    for title in ["Alpha", "Beta"] {
        let mut found = named(cx, title);
        found.retain(|e| e.role() != Some(Role::Group));
        assert_eq!(found.len(), 1, "{title}: {found:?}");
        assert_eq!(found[0].role(), Some(Role::Heading), "{title}");
        for i in 0..3 {
            let piece = bounds(cx, chunk_id(i));
            assert!(
                !piece.intersects(&found[0].bounds()),
                "{title} inside piece {i}"
            );
        }
    }
    // The first crumb starts the row; `Open note` ends it.
    let piece = bounds(cx, chunk_id(0));
    let crumb = bounds(cx, crumb_id(0, 0));
    assert_eq!(label(cx, crumb_id(0, 0)).as_deref(), Some("Day"));
    // Its label starts at the column's one left edge, where the text does.
    let (spine, inset) = cx.update(|window, _| {
        let rem = window.rem_size();
        (
            super::facet::rows::spine().to_pixels(rem),
            super::facet_ui::label_inset(rem),
        )
    });
    assert_eq!(
        crumb.left() + inset,
        piece.left() + spine,
        "nothing before the first crumb"
    );
    assert!(crumb.right() <= bounds(cx, open_id(0)).left());
    // A piece with no crumbs still draws its row.
    assert!(shows(cx, open_id(2)));
    assert!(!exists(cx, crumb_id(2, 0)));
}

#[gpui_kit::test]
fn a_single_note_view_has_no_dateline(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, "# Day\n\n#x a\n", vec![tag("x")]);
    assert!(shows(cx, crumb_id(0, 0)));
    assert!(!exists(cx, dateline_id(0)));
    // The view title row is the one heading: it names the note.
    let only = headings(cx);
    assert_eq!(only.len(), 1, "{only:?}");
    assert_eq!(only[0].label(), Some("Note: Day"));
    // The piece is the column's first row. The first gap (it hides `# Day`)
    // has nothing above it, so it draws only its up row, under the crumbs.
    let column = bounds(cx, COLUMN_ID);
    assert_eq!(dist(bounds(cx, chunk_id(0)).top(), column.top()), 12.0);
    assert!(bounds(cx, gap_up_id(0)).top() >= bounds(cx, open_id(0)).bottom());
}

#[gpui_kit::test]
fn a_long_title_shrinks_in_the_dateline_and_the_meta_stays_whole(cx: &mut TestAppContext) {
    let long = "A very long note title that would never fit into a narrow row at all";
    let text = "# The outermost section of this rather long note\n\n#x alpha\n";
    let (_view, cx, _events, _sub) = open_collection(cx, &[(A, long, text)], 1, vec![tag("x")]);
    let meta = f32::from(bounds(cx, dateline_meta_id(0)).size.width);
    let open = f32::from(bounds(cx, open_id(0)).size.width);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(360.), gpui_kit::px(800.)));
    let row = bounds(cx, dateline_id(0));
    let title = bounds(cx, dateline_title_id(0));
    let now = bounds(cx, dateline_meta_id(0));
    assert_eq!(f32::from(now.size.width), meta, "the meta whole");
    assert!(title.right() <= now.left(), "{title:?} {now:?}");
    assert!(now.right() <= row.right(), "{now:?} {row:?}");
    assert_eq!(f32::from(bounds(cx, open_id(0)).size.width), open);
}

// ── the dateline ─────────────────────────────────────────────────────────

#[gpui_kit::test]
fn every_note_starts_with_a_dateline_and_the_first_has_no_extra_space(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "a1\n"),
        (B, "Beta", "b1\n"),
        (C, "Gamma", "c1\n"),
    ];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![]);
    for (m, title) in ["Alpha", "Beta", "Gamma"].into_iter().enumerate() {
        assert_eq!(label(cx, dateline_id(m)).as_deref(), Some(title));
        let line = bounds(cx, dateline_id(m));
        let piece = bounds(cx, chunk_id(m));
        assert_eq!(
            dist(piece.top(), line.bottom()),
            12.0,
            "{title}: the stream's gap"
        );
    }
    // The first dateline is the column's first row: only the padding.
    let column = bounds(cx, COLUMN_ID);
    assert_eq!(dist(bounds(cx, dateline_id(0)).top(), column.top()), 12.0);
    // Every later one keeps 24 px from the note above.
    for m in 1..3 {
        let above = bounds(cx, chunk_id(m - 1));
        let line = bounds(cx, dateline_id(m));
        assert_eq!(dist(line.top(), above.bottom()), 24.0, "dateline {m}");
    }
}

#[gpui_kit::test]
fn the_dateline_sits_above_its_notes_first_gap(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "b head\n\n#x b\n")];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    // Gap 2 is B's first gap: it hides B's head.
    assert_eq!(
        label(cx, gap_count_id(2)).as_deref(),
        Some("1 block hidden")
    );
    let line = bounds(cx, dateline_id(1));
    let gap = bounds(cx, gap_up_id(2));
    assert!(bounds(cx, chunk_id(0)).bottom() <= line.top());
    assert!(line.bottom() <= gap.top(), "{line:?} {gap:?}");
    // The gap's up row lies inside piece 1, under its crumbs.
    assert!(line.bottom() <= bounds(cx, chunk_id(1)).top());
    assert!(gap.top() >= bounds(cx, open_id(1)).bottom());
}

#[gpui_kit::test]
fn the_previous_notes_last_gap_sits_above_the_next_dateline(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n\na tail\n"), (B, "Beta", "#x b\n")];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    // Gap 1 is A's last gap: it hides A's tail.
    assert_eq!(
        label(cx, gap_count_id(1)).as_deref(),
        Some("1 block hidden")
    );
    let gap = bounds(cx, gap_down_id(1));
    let line = bounds(cx, dateline_id(1));
    assert_eq!(dist(line.top(), gap.bottom()), 24.0, "{gap:?} {line:?}");
    assert!(line.bottom() <= bounds(cx, chunk_id(1)).top());
}

#[gpui_kit::test]
fn the_dateline_shows_folder_and_piece_count(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a1\n\nmid\n\n#x a2\n"),
        (B, "Beta", "#x b\n"),
        ("/root.md", "Root", "#x r\n"),
        ("/journal/2026/day.md", "Day", "#x d\n"),
    ];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 4, vec![tag("x")]);
    let metas: Vec<_> = (0..4).map(|m| label(cx, dateline_meta_id(m))).collect();
    assert_eq!(
        metas,
        [
            Some("a \u{b7} 2 pieces".to_string()),
            Some("b \u{b7} 1 piece".to_string()),
            Some("1 piece".to_string()),
            Some("journal/2026 \u{b7} 1 piece".to_string()),
        ]
    );
}

#[gpui_kit::test]
fn the_dateline_has_a_hairline_that_fills_the_line(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a1\n"), (B, "Beta", "b1\n")];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    let check = |cx: &mut VisualTestContext| {
        let row = bounds(cx, dateline_id(0));
        let title = bounds(cx, dateline_title_id(0));
        let rule = bounds(cx, dateline_rule_id(0));
        let folder = bounds(cx, dateline_folder_id(0));
        let meta = bounds(cx, dateline_meta_id(0));
        let piece = bounds(cx, chunk_id(0));
        assert_eq!(f32::from(rule.size.height), 1.0, "{rule:?}");
        assert_eq!((row.left(), row.right()), (piece.left(), piece.right()));
        assert_eq!(title.left(), row.left(), "the title leads");
        assert_eq!(meta.right(), row.right(), "the meta ends the row");
        // The rule takes what is left between them, less the row's gaps.
        let before = dist(rule.left(), title.right());
        let after = dist(folder.left(), rule.right());
        assert!((0.0..=8.0).contains(&before), "{title:?} {rule:?}");
        assert!((0.0..=8.0).contains(&after), "{rule:?} {folder:?}");
        assert!(folder.right() <= meta.left(), "{folder:?} {meta:?}");
        let middle = (rule.top() + rule.bottom()) / 2.0;
        assert!(title.top() < middle && middle < title.bottom(), "centred");
    };
    check(cx);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(500.), gpui_kit::px(800.)));
    check(cx);
}

#[gpui_kit::test]
fn a_note_with_no_drawn_piece_has_no_dateline(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "a #y\n"),
        (B, "Beta", "#x b\n"),
        (C, "Gamma", "c\n"),
    ];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![tag("x")]);
    assert!(!exists(cx, dateline_id(0)));
    assert_eq!(label(cx, dateline_id(1)).as_deref(), Some("Beta"));
    assert!(!exists(cx, dateline_id(2)));
    // The view title row and Beta's dateline.
    assert_eq!(headings(cx).len(), 2);
    // Beta's dateline is now the column's first row.
    let column = bounds(cx, COLUMN_ID);
    assert_eq!(dist(bounds(cx, dateline_id(1)).top(), column.top()), 12.0);
}

#[gpui_kit::test]
fn the_dateline_is_not_a_tab_stop_and_not_clickable(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "#x b\n")];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    for snapshot in headings(cx) {
        assert_eq!(snapshot.focused(), None, "no focus binding: {snapshot:?}");
    }
    click(cx, dateline_id(1));
    click(cx, dateline_title_id(1));
    click(cx, dateline_meta_id(1));
    assert_eq!(picks(&view, cx), [tag("x")]);
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    // Tab walks from B's `Open note` round to the bar and never lands
    // on a dateline.
    focus_view(&view, cx);
    tab_to(cx, open_id(1), 20);
    for _ in 0..20 {
        key(cx, "tab");
        for m in 0..2 {
            assert!(!has_focus(cx, dateline_id(m)));
            assert!(!has_focus(cx, dateline_title_id(m)));
        }
    }
}

/// `j` onto a note's first piece brings its dateline (and the first gap
/// under it) to the top, so the keyboard shows which note it landed in.
/// Later pieces of a note still come to the top themselves.
#[gpui_kit::test]
fn j_onto_a_notes_first_piece_shows_its_dateline(cx: &mut TestAppContext) {
    let alpha: String = (0..3).map(|i| format!("#t a{i}\n\nplain\n\n")).collect();
    let beta: String = "b head\n\n".to_string()
        + &(0..40)
            .map(|i| format!("#t b{i}\n\nplain\n\n"))
            .collect::<String>();
    let notes = [(A, "Alpha", alpha.as_str()), (B, "Beta", beta.as_str())];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("t")]);
    focus_view(&view, cx);
    let top = |cx: &mut VisualTestContext, id: SharedString| {
        dist(bounds(cx, id).top(), bounds(cx, COLUMN_ID).top())
    };
    key(cx, "j");
    assert!(
        top(cx, chunk_id(1)).abs() < 1.0,
        "a later piece of Alpha: itself"
    );
    key(cx, "j");
    key(cx, "j");
    // Piece 3 is Beta's first; gap 4 is Beta's first gap (it hides the head).
    let line = top(cx, dateline_id(1));
    assert!(line >= -0.5, "Beta's dateline in view: {line}");
    assert!(line < 1.0, "Beta's dateline leads the view: {line}");
    assert!(shows(cx, gap_up_id(4)));
    assert!(bounds(cx, dateline_id(1)).bottom() <= bounds(cx, gap_up_id(4)).top());
    key(cx, "j");
    assert!(
        top(cx, chunk_id(4)).abs() < 1.0,
        "Beta's second piece: itself"
    );
    key(cx, "k");
    assert!(
        top(cx, dateline_id(1)).abs() < 1.0,
        "k back: the dateline again"
    );
}

/// A deep folder in a narrow pane gives up its room first: the title keeps
/// 32 px, the count stays whole, nothing overlaps.
#[gpui_kit::test]
fn a_long_folder_in_a_narrow_pane_leaves_the_title_and_the_count(cx: &mut TestAppContext) {
    let path = "/projects/clients/acme/2026/q4/planning.md";
    let title = "Quarterly planning notes for the client";
    let text = "#x a\n\nmid\n\n#x b\n";
    let (_view, cx, _events, _sub) = open_collection(cx, &[(path, title, text)], 1, vec![tag("x")]);
    assert_eq!(
        label(cx, dateline_meta_id(0)).as_deref(),
        Some("projects/clients/acme/2026/q4 \u{b7} 2 pieces")
    );
    let count = f32::from(bounds(cx, dateline_meta_id(0)).size.width);
    for width in [300., 240.] {
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(width), gpui_kit::px(800.)));
        let row = bounds(cx, dateline_id(0));
        let title = bounds(cx, dateline_title_id(0));
        let rule = bounds(cx, dateline_rule_id(0));
        let folder = bounds(cx, dateline_folder_id(0));
        let now = bounds(cx, dateline_meta_id(0));
        assert!(f32::from(title.size.width) >= 32.0, "{width}: {title:?}");
        assert_eq!(f32::from(now.size.width), count, "{width}: the count whole");
        assert!(title.right() <= rule.left(), "{width}: {title:?} {rule:?}");
        assert!(
            rule.right() <= folder.left(),
            "{width}: {rule:?} {folder:?}"
        );
        assert!(folder.right() <= now.left(), "{width}: {folder:?} {now:?}");
        assert!(now.right() <= row.right(), "{width}: {now:?} {row:?}");
    }
}

#[gpui_kit::test]
fn the_dateline_follows_a_cap_that_cuts_inside_a_note(cx: &mut TestAppContext) {
    let many = |prefix: &str, n: usize| -> String {
        (0..n)
            .map(|i| format!("#t {prefix}{i}\n\nplain {i}\n\n"))
            .collect()
    };
    let (a, b, c) = (many("a", MAX_RANGES - 1), many("b", 3), many("c", 2));
    let notes = [
        (A, "Alpha", a.as_str()),
        (B, "Beta", b.as_str()),
        (C, "Gamma", c.as_str()),
    ];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![tag("t")]);
    assert_eq!(chunk_texts(&view, cx).len(), MAX_RANGES);
    let pieces = format!("a \u{b7} {} pieces", MAX_RANGES - 1);
    assert_eq!(
        label(cx, dateline_meta_id(0)).as_deref(),
        Some(pieces.as_str())
    );
    to_bottom(&view, cx);
    // Beta keeps its dateline and counts the one piece drawn of three.
    assert_eq!(label(cx, dateline_id(1)).as_deref(), Some("Beta"));
    assert_eq!(
        label(cx, dateline_meta_id(1)).as_deref(),
        Some("b \u{b7} 1 piece")
    );
    // Gamma is cut whole: no dateline.
    assert!(!exists(cx, dateline_id(2)));
    // Show more draws the rest, and the counts follow.
    click(cx, SHOW_MORE_ID);
    to_bottom(&view, cx);
    assert_eq!(
        label(cx, dateline_meta_id(2)).as_deref(),
        Some("c \u{b7} 2 pieces")
    );
}

#[gpui_kit::test]
fn a_tag_pick_shows_only_the_notes_that_hold_it(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a\n"),
        (B, "Beta", "b #y\n"),
        (C, "Gamma", "#x c\n"),
    ];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x a", "#x c"]);
    assert_eq!(label(cx, dateline_id(2)).as_deref(), Some("Gamma"));
    assert!(!exists(cx, dateline_id(1)));
}

#[gpui_kit::test]
fn merged_pills_count_across_notes(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x one #x\n"), (B, "Beta", "#X two\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 1")
    );
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(label(cx, more_item_id(&tag("x"))).as_deref(), Some("#x"));
    let count = view.read_with(cx, |view, _| view.merged_count(&tag("x")));
    assert_eq!(count, Some(3));
}

#[gpui_kit::test]
fn the_same_heading_in_two_notes_is_one_pill_that_shows_both(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "# Plan\n\nalpha\n\n# Other\n\nx\n"),
        (B, "Beta", "## plan\n\nbeta\n"),
    ];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    let count = view.read_with(cx, |view, _| view.merged_count(&heading("plan")));
    assert_eq!(count, Some(2), "one merged pill");
    // An unpicked heading is not in the bar: pick it from the Sections list.
    click(cx, menu_id(ListKind::Sections));
    click(cx, more_item_id(&heading("plan")));
    assert_eq!(
        chunk_texts(&view, cx),
        ["# Plan\n\nalpha", "## plan\n\nbeta"]
    );
}

#[gpui_kit::test]
fn a_day_and_a_tag_intersect_inside_each_note(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "# Mon\n\n#x a\n\nb\n\n# Tue\n\n#x c\n"),
        (B, "Beta", "# Mon\n\nd\n\n# Tue\n\n#x e\n"),
    ];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![heading("mon"), tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x a"]);
    assert_eq!(label(cx, dateline_id(0)).as_deref(), Some("Alpha"));
}

#[gpui_kit::test]
fn one_cap_spans_notes_and_show_more_doubles_it(cx: &mut TestAppContext) {
    let many = |prefix: &str| -> String {
        (0..60)
            .map(|i| format!("#t {prefix}{i}\n\nplain {i}\n\n"))
            .collect()
    };
    let (a, b) = (many("a"), many("b"));
    let notes = [(A, "Alpha", a.as_str()), (B, "Beta", b.as_str())];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("t")]);
    let texts = chunk_texts(&view, cx);
    assert_eq!(texts.len(), MAX_RANGES);
    assert_eq!(texts[60], "#t b0", "the cap runs on into the next note");
    let label_now = view.read_with(cx, |view, _| view.show_more_label());
    assert_eq!(label_now.as_deref(), Some("Show 20 more"));
    to_bottom(&view, cx);
    click(cx, SHOW_MORE_ID);
    assert_eq!(chunk_texts(&view, cx).len(), 120);
    assert_eq!(view.read_with(cx, |view, _| view.show_more_label()), None);
}

/// The gap rows of a boundary: A's last gap, then B's first gap.
#[gpui_kit::test]
fn reveal_never_crosses_a_note_boundary(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a\n\na tail\n"),
        (B, "Beta", "b head\n\n#x b\n"),
    ];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x a", "#x b"]);
    // Gaps: 0 above A's piece, 1 below it, 2 above B's piece, 3 below it.
    assert!(!shows(cx, gap_count_id(0)));
    assert_eq!(
        label(cx, gap_count_id(1)).as_deref(),
        Some("1 block hidden")
    );
    assert_eq!(
        label(cx, gap_count_id(2)).as_deref(),
        Some("1 block hidden")
    );
    assert!(!shows(cx, gap_count_id(3)));
    assert!(shows(cx, gap_down_id(1)) && !shows(cx, gap_up_id(1)));
    assert!(shows(cx, gap_up_id(2)) && !shows(cx, gap_down_id(2)));
    click(cx, gap_down_id(1));
    assert_eq!(chunk_texts(&view, cx), ["#x a\n\na tail", "#x b"]);
    assert!(!shows(cx, gap_down_id(1)), "A's tail is all A hid below");
    click(cx, gap_up_id(2));
    assert_eq!(chunk_texts(&view, cx), ["#x a\n\na tail", "b head\n\n#x b"]);
}

#[gpui_kit::test]
fn show_all_fills_only_its_own_gap(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a\n\nt1\n\nt2\n"),
        (B, "Beta", "u1\n\n#x b\n"),
    ];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    assert_eq!(
        label(cx, gap_count_id(1)).as_deref(),
        Some("2 blocks hidden")
    );
    click(cx, show_all_id(1));
    assert_eq!(chunk_texts(&view, cx), ["#x a\n\nt1\n\nt2", "#x b"]);
    assert_eq!(
        label(cx, gap_count_id(2)).as_deref(),
        Some("1 block hidden")
    );
}

#[gpui_kit::test]
fn open_note_names_the_pieces_own_note(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "b\n\n#x b\n")];
    let (_view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    click(cx, open_id(1));
    assert_eq!(
        *events.borrow(),
        [FacetEvent::OpenNote {
            note: p(B),
            line: 2
        }]
    );
}

#[gpui_kit::test]
fn a_follow_resolves_from_the_pieces_own_note(cx: &mut TestAppContext) {
    // The same relative link in two folders: `/a/plan.md`, `/b/plan.md`.
    let text = "see [plan](plan.md) now\n";
    let notes = [(A, "Alpha", text), (B, "Beta", text)];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![]);
    let at = cell(&view, 1, 7, 0, cx);
    press(cx, at, true);
    assert_eq!(
        *events.borrow(),
        [FacetEvent::Follow {
            from: p(B),
            target: FollowTarget::Link {
                target: "plan.md".into(),
                kind: kimun_core::note::scan::LinkSpanKind::Markdown,
            },
        }]
    );
    let at = cell(&view, 0, 7, 0, cx);
    press(cx, at, true);
    assert!(
        matches!(&events.borrow()[1], FacetEvent::Follow { from, .. } if *from == p(A)),
        "{:?}",
        events.borrow()
    );
}

#[gpui_kit::test]
fn set_member_text_updates_only_that_member(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "#x b\n")];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![]);
    let first = view.read_with(cx, |view, _| view.chunks()[0].clone());
    view.update(cx, |view, cx| {
        view.set_member_text(&p(B), "#x b2\n".into(), cx)
    });
    assert_eq!(chunk_texts(&view, cx), ["#x a", "#x b2"]);
    let still = view.read_with(cx, |view, _| view.chunks()[0].clone());
    assert_eq!(first, still, "A's piece kept its editor");
    view.update(cx, |view, cx| {
        view.set_member_text(&p("/nowhere.md"), "gone\n".into(), cx)
    });
    assert_eq!(
        chunk_texts(&view, cx),
        ["#x a", "#x b2"],
        "unknown: nothing"
    );
    assert_eq!(picks_changed(&events), 0);
}

#[gpui_kit::test]
fn set_member_text_prunes_a_vanished_pick(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#y a\n"), (B, "Beta", "#x b\n")];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x b"]);
    view.update(cx, |view, cx| view.set_member_text(&p(B), "b\n".into(), cx));
    assert!(picks(&view, cx).is_empty());
    assert_eq!(picks_changed(&events), 1);
    assert_eq!(chunk_texts(&view, cx), ["#y a", "b"]);
}

#[gpui_kit::test]
fn remove_members_drops_one_and_keeps_the_rest(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a\n"),
        (B, "Beta", "#x b\n"),
        (C, "Gamma", "#x c\n"),
    ];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![tag("x")]);
    let removed = view.update(cx, |view, cx| view.remove_members(&[p(B)], cx));
    assert_eq!(removed, 1);
    assert_eq!(paths(&view, cx), [p(A), p(C)]);
    assert_eq!(chunk_texts(&view, cx), ["#x a", "#x c"]);
    assert_eq!(label(cx, dateline_id(1)).as_deref(), Some("Gamma"));
    let again = view.update(cx, |view, cx| view.remove_members(&[p(B)], cx));
    assert_eq!(again, 0);
}

#[gpui_kit::test]
fn set_member_path_rekeys_a_member(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "#x b\n")];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![]);
    view.update(cx, |view, _| view.set_member_path(&p(B), p("/d/moved.md")));
    assert_eq!(paths(&view, cx), [p(A), p("/d/moved.md")]);
    click(cx, open_id(1));
    assert_eq!(
        *events.borrow(),
        [FacetEvent::OpenNote {
            note: p("/d/moved.md"),
            line: 0
        }]
    );
}

#[gpui_kit::test]
fn an_empty_collection_shows_the_message_and_refresh(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open_collection(cx, &[], 0, vec![tag("x")]);
    assert_eq!(
        label(cx, EMPTY_ID).as_deref(),
        Some("No notes match this query.")
    );
    assert!(shows(cx, REFRESH_ID));
    assert!(!shows(cx, REMOVE_LAST_ID));
    assert!(!shows(cx, SUMMARY_ID));
    assert!(chunk_texts(&view, cx).is_empty());
    focus_view(&view, cx);
    tab_to(cx, REFRESH_ID, 6);
    key(cx, "enter");
    key(cx, "j");
    key(cx, "escape");
    assert_eq!(*events.borrow(), [FacetEvent::Refresh]);
}

#[gpui_kit::test]
fn removing_the_last_member_leaves_the_empty_state(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 1, vec![tag("x")]);
    view.update(cx, |view, cx| view.remove_members(&[p(A)], cx));
    assert!(picks(&view, cx).is_empty(), "the pick went with its note");
    assert_eq!(
        label(cx, EMPTY_ID).as_deref(),
        Some("No notes match this query.")
    );
    assert!(shows(cx, REFRESH_ID));
    view.update(cx, |view, cx| {
        view.set_member_text(&p(A), "x\n".into(), cx);
        view.set_member_path(&p(A), p(B));
        assert_eq!(view.remove_members(&[p(A)], cx), 0);
        assert_eq!(view.text(), "");
        assert_eq!(view.source(), None);
    });
}

#[gpui_kit::test]
fn the_first_500_line_shows_when_total_exceeds_members(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 600, vec![]);
    assert_eq!(
        line(cx).as_deref(),
        Some("2 notes \u{b7} first 2 of 600 \u{b7} list from 10:42")
    );
    let members = vec![member(A, "Alpha", "a\n"), member(B, "Beta", "b\n")];
    view.update(cx, |view, cx| view.replace_members(members, 2, cx));
    refresh_report_ends(cx);
    assert_eq!(line(cx).as_deref(), Some("2 notes \u{b7} list from 10:42"));
}

#[gpui_kit::test]
fn replace_members_prunes_picks_and_resets_reveals(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n\ntail\n"), (B, "Beta", "#y b\n")];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    click(cx, gap_down_id(1));
    assert_eq!(chunk_texts(&view, cx), ["#x a\n\ntail"]);
    let members = vec![
        member(A, "Alpha", "#x a\n\ntail\n"),
        member(C, "Gamma", "c\n"),
    ];
    view.update(cx, |view, cx| view.replace_members(members, 2, cx));
    assert_eq!(paths(&view, cx), [p(A), p(C)]);
    assert_eq!(chunk_texts(&view, cx), ["#x a"], "reveals reset");
    let members = vec![member(C, "Gamma", "c\n")];
    view.update(cx, |view, cx| view.replace_members(members, 1, cx));
    assert!(picks(&view, cx).is_empty(), "x went with its notes");
    assert_eq!(picks_changed(&events), 1);
}

#[gpui_kit::test]
fn r_and_the_button_emit_refresh_in_a_collection_only(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n")];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 1, vec![]);
    focus_view(&view, cx);
    key(cx, "r");
    assert_eq!(*events.borrow(), [FacetEvent::Refresh]);
    assert!(
        !shows(cx, REFRESH_ID),
        "the button stands in the empty state only"
    );
}

#[gpui_kit::test]
fn a_single_note_view_ignores_r_and_has_no_refresh(cx: &mut TestAppContext) {
    let (single, cx, events, _sub) = open(cx, "", vec![]);
    focus_view(&single, cx);
    key(cx, "r");
    assert!(events.borrow().is_empty());
    assert!(!shows(cx, REFRESH_ID));
}

#[gpui_kit::test]
fn the_summary_leaves_the_counts_to_the_collection_line(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a\n\nb\n\n#x c\n"),
        (B, "Beta", "#x d\n"),
        (C, "Gamma", "e\n"),
    ];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![tag("x")]);
    assert_eq!(
        label(cx, SUMMARY_ID).as_deref(),
        Some("Showing text tagged #x")
    );
    assert_eq!(
        line(cx).as_deref(),
        Some("3 pieces in 2 of 3 notes \u{b7} list from 10:42")
    );
}

/// A plain click on a note link in a collection follows it from the
/// piece's own note. The filter is unchanged.
#[gpui_kit::test]
fn a_plain_click_on_a_link_in_a_collection_follows_it_and_keeps_the_filter(
    cx: &mut TestAppContext,
) {
    let notes = [(A, "Alpha", "see [[Plan]] now\n"), (B, "Beta", "other\n")];
    let (view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![]);
    let at = cell(&view, 0, 7, 0, cx);
    press(cx, at, false);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(picks_changed(&events), 0);
    assert_eq!(chunk_texts(&view, cx), ["see [[Plan]] now", "other"]);
    let follows: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            FacetEvent::Follow { from, .. } => Some(from.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(follows, [VaultPath::new(A)]);
}

// ── carried over from the member refactor ────────────────────────────────

#[gpui_kit::test]
fn open_note_names_its_note(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn follow_names_the_note_it_started_in(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "# Head\n\nsome #a\n\nmore #b\n", vec![]);
    let at = cell(&view, 0, 6, 4, cx);
    press(cx, at, true);
    assert_eq!(
        *events.borrow(),
        [FacetEvent::Follow {
            from: VaultPath::new("/notes/n.md"),
            target: FollowTarget::Label("b".into()),
        }]
    );
}

// ── review round 1 ───────────────────────────────────────────────────────

/// A delete is no cap: an uncapped collection that loses a note does not
/// claim to show "the first" of anything.
#[gpui_kit::test]
fn removing_from_an_uncapped_collection_shows_no_first_notes_line(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n"), (C, "Gamma", "c\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![]);
    view.update(cx, |view, cx| view.remove_members(&[p(B)], cx));
    assert_eq!(line(cx).as_deref(), Some("2 notes \u{b7} list from 10:42"));
}

/// A capped collection that loses a note: the deleted note is no hit.
#[gpui_kit::test]
fn removing_from_a_capped_collection_lowers_both_counts(cx: &mut TestAppContext) {
    let names: Vec<(String, String)> = (0..500)
        .map(|i| (format!("/n/{i:03}.md"), format!("N{i}")))
        .collect();
    let notes: Vec<(&str, &str, &str)> = names
        .iter()
        .map(|(path, title)| (path.as_str(), title.as_str(), "x\n"))
        .collect();
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 600, vec![]);
    assert_eq!(
        line(cx).as_deref(),
        Some("500 notes \u{b7} first 500 of 600 \u{b7} list from 10:42")
    );
    view.update(cx, |view, cx| view.remove_members(&[p("/n/007.md")], cx));
    assert_eq!(
        line(cx).as_deref(),
        Some("499 notes \u{b7} first 499 of 599 \u{b7} list from 10:42")
    );
}

/// A member handed in with no title (a note with no text) shows its name.
#[gpui_kit::test]
fn a_blank_title_falls_back_to_the_file_name(cx: &mut TestAppContext) {
    let notes = [(A, "", "#x a\n"), (B, "  ", "#x b\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    assert_eq!(label(cx, dateline_id(0)).as_deref(), Some("alpha"));
    let members = vec![member(C, "", "#x c\n")];
    view.update(cx, |view, cx| view.replace_members(members, 1, cx));
    assert_eq!(label(cx, dateline_id(0)).as_deref(), Some("gamma"));
}

/// The report of a refresh ends as soon as the user picks, unpicks,
/// clears or undoes: the line then tells what the picks show.
#[gpui_kit::test]
fn a_pick_clear_or_undo_ends_the_refresh_report(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "#y b\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    let refresh = |view: &Entity<FacetView>, cx: &mut VisualTestContext| {
        let members = vec![member(A, "Alpha", "#x a\n"), member(B, "Beta", "#y b\n")];
        view.update(cx, |view, cx| view.replace_members(members, 2, cx));
        assert_eq!(line(cx).as_deref(), Some("Refreshed: no changes"));
    };
    let refreshed =
        |cx: &mut VisualTestContext| line(cx).is_some_and(|l| l.starts_with("Refreshed"));
    refresh(&view, cx);
    view.update(cx, |view, cx| view.toggle(tag("x"), cx));
    assert!(!refreshed(cx), "a pick ends it");
    refresh(&view, cx);
    view.update(cx, |view, cx| view.toggle(tag("x"), cx));
    assert!(!refreshed(cx), "an unpick ends it");
    view.update(cx, |view, cx| view.toggle(tag("y"), cx));
    refresh(&view, cx);
    click(cx, super::facet::CLEAR_ID);
    assert!(!refreshed(cx), "a clear ends it");
    refresh(&view, cx);
    focus_view(&view, cx);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("y")]);
    assert!(!refreshed(cx), "an undo ends it");
}

/// A refresh brings the cap back to `MAX_RANGES`, as it resets reveals.
#[gpui_kit::test]
fn replace_members_resets_the_show_more_limit(cx: &mut TestAppContext) {
    let text: String = (0..150).map(|i| format!("#t m{i}\n\nplain\n\n")).collect();
    let notes = [(A, "Alpha", text.as_str())];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 1, vec![tag("t")]);
    to_bottom(&view, cx);
    click(cx, SHOW_MORE_ID);
    assert_eq!(chunk_texts(&view, cx).len(), 150);
    let members = vec![member(A, "Alpha", &text)];
    view.update(cx, |view, cx| view.replace_members(members, 1, cx));
    assert_eq!(chunk_texts(&view, cx).len(), MAX_RANGES);
}

// ── final review ─────────────────────────────────────────────────────────

/// A batch remove takes every named member out, lowers the total by as
/// many, and plans once.
#[gpui_kit::test]
fn remove_members_removes_all_named_and_plans_once(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n"), (C, "Gamma", "c\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![]);
    let before = view.read_with(cx, |view, _| view.plans_run());
    let removed = view.update(cx, |view, cx| {
        view.remove_members(&[p(A), p(C), p("/not/a/member.md")], cx)
    });
    assert_eq!(removed, 2);
    assert_eq!(paths(&view, cx), [p(B)]);
    assert_eq!(view.read_with(cx, |view, _| view.plans_run()), before + 1);
    assert_eq!(chunk_texts(&view, cx), ["b"]);
    let none = view.update(cx, |view, cx| view.remove_members(&[p(A)], cx));
    assert_eq!(none, 0);
    assert_eq!(
        view.read_with(cx, |view, _| view.plans_run()),
        before + 1,
        "nothing removed, nothing planned"
    );
}

/// An unreadable note is no cap: 3 hits with 2 read shows no "first" line.
#[gpui_kit::test]
fn an_unreadable_hit_shows_no_first_notes_line(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![]);
    assert_eq!(line(cx).as_deref(), Some("2 notes \u{b7} list from 10:42"));
    // Nor after a refresh that loses one to a failed read.
    let members = vec![member(A, "Alpha", "a\n")];
    view.update(cx, |view, cx| view.replace_members(members, 2, cx));
    refresh_report_ends(cx);
    assert_eq!(line(cx).as_deref(), Some("1 note \u{b7} list from 10:42"));
}

/// A capped load with one unreadable note still says what it holds, and so
/// does a refresh that comes back capped.
#[gpui_kit::test]
fn a_capped_load_with_an_unreadable_note_reads_first_of_total(cx: &mut TestAppContext) {
    let names: Vec<(String, String)> = (0..499)
        .map(|i| (format!("/n/{i:03}.md"), format!("N{i}")))
        .collect();
    let notes: Vec<(&str, &str, &str)> = names
        .iter()
        .map(|(path, title)| (path.as_str(), title.as_str(), "x\n"))
        .collect();
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 600, vec![]);
    assert_eq!(
        line(cx).as_deref(),
        Some("499 notes \u{b7} first 499 of 600 \u{b7} list from 10:42")
    );
    view.update(cx, |view, cx| view.remove_members(&[p("/n/007.md")], cx));
    assert_eq!(
        line(cx).as_deref(),
        Some("498 notes \u{b7} first 498 of 599 \u{b7} list from 10:42")
    );
    let members = vec![member(A, "Alpha", "a\n")];
    view.update(cx, |view, cx| view.replace_members(members, 700, cx));
    refresh_report_ends(cx);
    assert_eq!(
        line(cx).as_deref(),
        Some("1 note \u{b7} first 1 of 700 \u{b7} list from 10:42")
    );
}

// ── a group showing a collection (the workspace side) ────────────────────

/// A real `Workspace` on a real temp vault, its groups showing collections
/// (`collection_group`).
mod in_a_group {
    use gpui_kit::TestAppContext;
    use kimun_core::nfs::VaultPath;
    use kimun_core::note::scan::LinkSpanKind;
    use kimun_core::note::{Facet, FacetKind};
    use kimun_notes::logic::actions::ActionShortcuts;
    use kimun_notes::logic::editor::claim::FollowTarget;
    use kimun_notes::logic::layout::Panel;
    use kimun_notes::logic::layout_tree::GroupId;
    use tokio::runtime::Runtime;

    use super::super::facet::{EMPTY_ID, FACET_VIEW_ID, FacetEvent, open_id};
    use super::super::facet_tests::in_a_group::{
        active, assert_states_in_step, button, click, drawn, facet, new_tab, note_in, open_menu,
        path, run, save_text_in, split, started, tabs,
    };
    use super::super::file_dialogs::FileOpKind;
    use super::super::file_ops::FileOpRequest;
    use super::super::group_header::{
        CORNER_FACET_ID, CORNER_REFRESH_ID, GROUP_FACET_ID, GROUP_FACET_MARK_ID, GROUP_REFRESH_ID,
        REFRESH_LABEL, group_element,
    };
    use super::super::note_menu::{MenuItem, menu_item_element_id};
    use super::super::palette::PaletteRun;
    use super::super::route::NO_EDITOR_TO_RETURN_TO;
    use super::super::start_tests::{Started, drive};
    use super::super::tab_strip::{TAB_FACET_MARK_ID, group_name, tab_label};
    use crate::test_vault::TestVault;

    fn tag(key: &str) -> Facet {
        Facet::new(FacetKind::Tag, key)
    }

    const A: &str = "# Day\n\nalpha #x\n";
    const B: &str = "beta #x\n";
    const C: &str = "gamma #y\n";

    /// `a.md` and `b.md` hold `#x`, `c.md` holds `#y`; started on `first`
    /// and indexed (a collection reads the index).
    fn vault(
        cx: &mut TestAppContext,
        rt: &Runtime,
        extra: &[(&str, &str)],
        first: &str,
    ) -> (TestVault, Started) {
        let mut notes = vec![("a.md", A), ("b.md", B), ("c.md", C)];
        notes.extend_from_slice(extra);
        let (v, s) = started(cx, rt, &notes, first);
        drive(rt, cx, |cx| !s.indexing(cx));
        (v, s)
    }

    fn query(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| s.workspace.read(cx).collection_query_in(group, cx))
    }

    fn members(s: &Started, group: GroupId, cx: &TestAppContext) -> Vec<VaultPath> {
        let view = facet(s, group, cx).expect("a facet view");
        cx.read(|cx| view.read(cx).member_paths())
    }

    fn has_member(s: &Started, group: GroupId, p: &str, cx: &TestAppContext) -> bool {
        members(s, group, cx).iter().any(|m| m.is_like(&path(p)))
    }

    fn member_text(s: &Started, group: GroupId, p: &str, cx: &TestAppContext) -> Option<String> {
        let view = facet(s, group, cx)?;
        cx.read(|cx| view.read(cx).member_text_of(&path(p)))
    }

    fn picks(s: &Started, group: GroupId, cx: &TestAppContext) -> Vec<Facet> {
        let view = facet(s, group, cx).expect("a facet view");
        cx.read(|cx| view.read(cx).picks().to_vec())
    }

    fn reads(s: &Started, group: GroupId, cx: &TestAppContext) -> usize {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(group)
                .map_or(0, |state| state.collection_reading.len())
        })
    }

    fn loading(s: &Started, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(group)
                .is_some_and(|state| state.facet_loading.is_some())
        })
    }

    /// Show the collection of `q` in `group`, driven until it shows.
    fn open_coll(
        s: &Started,
        rt: &Runtime,
        group: GroupId,
        q: &str,
        picks: Vec<Facet>,
        cx: &mut TestAppContext,
    ) {
        let q = q.to_string();
        let want = q.clone();
        s.workspace
            .update(cx, |w, cx| w.open_collection(group, q, picks, cx));
        drive(rt, cx, |cx| {
            query(s, group, cx).as_deref() == Some(want.as_str()) && !loading(s, group, cx)
        });
    }

    #[gpui_kit::test]
    fn open_collection_shows_the_queries_notes_and_keeps_the_group_noteless(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let group = active(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        assert_eq!(members(&s, group, cx), [path("/a.md"), path("/b.md")]);
        assert_eq!(member_text(&s, group, "a.md", cx).as_deref(), Some(A));
        assert_eq!(note_in(&s, group, cx), None, "the host holds no note");
        cx.read(|cx| {
            let w = s.workspace.read(cx);
            assert_eq!(w.facet_source_in(group, cx), None);
            assert_eq!(w.shown_note_in(group, cx), None);
            assert!(w.facet_group_in(group, cx));
        });
        assert!(drawn(&s, FACET_VIEW_ID, cx));
        assert_states_in_step(&s, cx);
    }

    #[gpui_kit::test]
    fn a_zero_hit_query_shows_the_empty_view_not_an_empty_group(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let group = active(&s, cx);
        open_coll(&s, &rt, group, "#nothing", vec![], cx);
        assert!(members(&s, group, cx).is_empty());
        assert!(facet(&s, group, cx).is_some(), "the view stays");
        assert!(drawn(&s, EMPTY_ID, cx));
        assert_eq!(note_in(&s, group, cx), None);
    }

    #[gpui_kit::test]
    fn the_group_title_is_the_query_and_has_the_funnel_mark(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let [_left, right] = split(&s, &rt, cx);
        open_coll(&s, &rt, right, "#x", vec![], cx);
        let name = cx.read(|cx| group_name(s.workspace.read(cx), right, cx));
        assert_eq!(name.as_deref(), Some("#x"));
        assert!(drawn(&s, group_element(GROUP_FACET_MARK_ID, 1), cx));
        assert!(!drawn(&s, group_element(GROUP_FACET_MARK_ID, 0), cx));
        // A tab named for a collection group wears the mark too.
        let other = new_tab(&s, cx);
        open_coll(&s, &rt, other, "#y", vec![], cx);
        assert!(drawn(&s, group_element(TAB_FACET_MARK_ID, 1), cx));
        let label = cx.read(|cx| {
            let w = s.workspace.read(cx);
            let tab = w.layout.region().tabs()[1];
            tab_label(w, tab, cx).to_string()
        });
        assert_eq!(label, "#y");
    }

    #[gpui_kit::test]
    fn files_marks_nothing_for_a_collection(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let marked = |cx: &TestAppContext| {
            cx.read(|cx| {
                let w = s.workspace.read(cx);
                w.files
                    .as_ref()
                    .and_then(|f| f.read(cx).model().open_note().cloned())
            })
        };
        assert_eq!(marked(cx), Some(path("a.md")));
        let group = active(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        assert_eq!(marked(cx), None);
        // Back to the group after another: still nothing.
        let other = new_tab(&s, cx);
        s.workspace.update(cx, |w, cx| w.focus_group(group, cx));
        cx.run_until_parked();
        assert_ne!(other, group);
        assert_eq!(marked(cx), None);
    }

    #[gpui_kit::test]
    fn the_note_menu_has_only_layout_items_in_a_collection(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let [_left, right] = split(&s, &rt, cx);
        new_tab(&s, cx);
        s.workspace.update(cx, |w, cx| w.focus_group(right, cx));
        cx.run_until_parked();
        open_coll(&s, &rt, right, "#x", vec![], cx);
        open_menu(&s, cx);
        for shown in [
            MenuItem::SplitRight,
            MenuItem::SplitDown,
            MenuItem::NewTab,
            MenuItem::CloseGroup,
            MenuItem::CloseTab,
        ] {
            assert!(drawn(&s, menu_item_element_id(shown), cx), "{shown:?}");
        }
        for hidden in [
            MenuItem::FacetView,
            MenuItem::EditNote,
            MenuItem::Note(FileOpKind::Properties),
            MenuItem::Note(FileOpKind::Rename),
            MenuItem::Note(FileOpKind::Move),
            MenuItem::Note(FileOpKind::Delete),
        ] {
            assert!(!drawn(&s, menu_item_element_id(hidden), cx), "{hidden:?}");
        }
    }

    #[gpui_kit::test]
    fn the_header_and_corner_show_refresh_not_the_funnel(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (v, s) = vault(cx, &rt, &[], "a.md");
        let group = active(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        assert!(button(&s, CORNER_FACET_ID, cx).is_none(), "no funnel");
        let (label, _, w, h) = button(&s, CORNER_REFRESH_ID, cx).expect("drawn in the corner");
        assert_eq!(label, REFRESH_LABEL);
        assert!(w >= 24.0 && h >= 24.0, "{w}x{h}");
        // A new hit shows after a click on Refresh.
        rt.block_on(v.vault.create_note(&path("n.md"), "new #x\n"))
            .expect("the note is made");
        click(&s, CORNER_REFRESH_ID, cx);
        drive(&rt, cx, |cx| has_member(&s, group, "n.md", cx));
        // In a split, the header carries it.
        let [left, _right] = split(&s, &rt, cx);
        assert_eq!(left, group);
        let (label, _, w, h) =
            button(&s, group_element(GROUP_REFRESH_ID, 0), cx).expect("in the header");
        assert_eq!(label, REFRESH_LABEL);
        assert!(w >= 24.0 && h >= 24.0, "{w}x{h}");
        assert!(button(&s, group_element(GROUP_FACET_ID, 0), cx).is_none());
        assert!(
            button(&s, group_element(GROUP_FACET_ID, 1), cx).is_some(),
            "a note group keeps its funnel"
        );
        assert!(button(&s, group_element(GROUP_REFRESH_ID, 1), cx).is_none());
    }

    #[gpui_kit::test]
    fn the_toggle_command_refuses_in_a_collection(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let group = active(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        let blocked = cx.read(|cx| {
            s.workspace
                .read(cx)
                .palette_entries(Panel::Editor, cx)
                .into_iter()
                .find(|e| *e.run() == PaletteRun::Chord(ActionShortcuts::ToggleFacetView))
                .and_then(|e| e.blocked())
        });
        assert_eq!(blocked, Some(NO_EDITOR_TO_RETURN_TO));
        s.workspace
            .update(cx, |w, cx| w.toggle_facet_view(group, cx));
        cx.run_until_parked();
        assert_eq!(s.flash(cx).as_deref(), Some(NO_EDITOR_TO_RETURN_TO));
        assert_eq!(query(&s, group, cx).as_deref(), Some("#x"), "it stays");
        assert_eq!(note_in(&s, group, cx), None);
    }

    #[gpui_kit::test]
    fn saving_a_member_in_an_editor_group_updates_the_member_only(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let editor_group = active(&s, cx);
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![tag("x")], cx);
        let new = "# Day\n\nalpha #x edited\n";
        save_text_in(&s, editor_group, new, cx);
        drive(&rt, cx, |cx| {
            member_text(&s, group, "a.md", cx).as_deref() == Some(new)
        });
        assert_eq!(member_text(&s, group, "b.md", cx).as_deref(), Some(B));
        assert_eq!(members(&s, group, cx), [path("/a.md"), path("/b.md")]);
        assert_eq!(picks(&s, group, cx), [tag("x")]);
        assert_eq!(reads(&s, group, cx), 0, "the read is done");
        // A save that drops the tag keeps the note in the list.
        save_text_in(&s, editor_group, "# Day\n\nplain\n", cx);
        drive(&rt, cx, |cx| {
            member_text(&s, group, "a.md", cx).as_deref() == Some("# Day\n\nplain\n")
        });
        assert_eq!(members(&s, group, cx), [path("/a.md"), path("/b.md")]);
    }

    #[gpui_kit::test]
    fn saving_a_note_outside_the_collection_changes_nothing(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let editor_group = active(&s, cx);
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        s.workspace
            .update(cx, |w, cx| w.refresh_facets(&path("c.md"), cx));
        assert_eq!(reads(&s, group, cx), 0, "no read for a non-member");
        save_text_in(&s, editor_group, "gamma #x now\n", cx);
        drive(&rt, cx, |cx| {
            note_in(&s, editor_group, cx).is_some()
                && cx.read(|cx| {
                    !s.workspace
                        .read(cx)
                        .state(editor_group)
                        .unwrap()
                        .editor
                        .read(cx)
                        .editor
                        .is_dirty()
                })
        });
        assert_eq!(members(&s, group, cx), [path("/a.md"), path("/b.md")]);
        assert_eq!(member_text(&s, group, "a.md", cx).as_deref(), Some(A));
        assert_eq!(member_text(&s, group, "b.md", cx).as_deref(), Some(B));
    }

    const TASKS: &str = "# T\n\n- [ ] task #x\n";

    /// Ticks the task on line 2 of `t.md`, as a click in a piece asks.
    fn toggle_t(s: &Started, seen: &str, cx: &mut TestAppContext) {
        let seen = seen.to_string();
        s.workspace.update(cx, |w, cx| {
            w.toggle_facet_task(path("/t.md"), 2, seen, cx);
        });
    }

    #[gpui_kit::test]
    fn ticking_a_task_in_a_piece_writes_the_note_and_redraws_the_piece(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[("t.md", TASKS)], "a.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![tag("x")], cx);
        toggle_t(&s, "- [ ] task #x", cx);
        let ticked = "# T\n\n- [x] task #x\n";
        drive(&rt, cx, |cx| {
            member_text(&s, group, "t.md", cx).as_deref() == Some(ticked)
        });
        assert_eq!(s.flash(cx), None, "a tick says nothing");
        toggle_t(&s, "- [x] task #x", cx);
        drive(&rt, cx, |cx| {
            member_text(&s, group, "t.md", cx).as_deref() == Some(TASKS)
        });
    }

    #[gpui_kit::test]
    fn ticking_a_task_reloads_the_clean_editor_that_holds_the_note(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[("t.md", TASKS)], "t.md");
        let editor_group = active(&s, cx);
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![tag("x")], cx);
        toggle_t(&s, "- [ ] task #x", cx);
        let ticked = "# T\n\n- [x] task #x\n";
        let editor = cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(editor_group)
                .unwrap()
                .editor
                .clone()
        });
        // Else the next edit and save in that editor would write the old
        // box back over the tick.
        drive(&rt, cx, |cx| {
            cx.read(|cx| editor.read(cx).editor.get_text() == ticked)
        });
    }

    #[gpui_kit::test]
    fn ticking_a_task_that_changed_leaves_the_note_and_says_so(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[("t.md", TASKS)], "a.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![tag("x")], cx);
        toggle_t(&s, "- [ ] a task that is gone", cx);
        drive(&rt, cx, |cx| s.flash(cx).is_some());
        let said = s.flash(cx).unwrap();
        assert!(said.contains("changed"), "{said}");
        assert_eq!(member_text(&s, group, "t.md", cx).as_deref(), Some(TASKS));
    }

    #[gpui_kit::test]
    fn ticking_a_task_is_refused_while_an_editor_holds_unsaved_text(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[("t.md", TASKS)], "t.md");
        let editor_group = active(&s, cx);
        let editor = cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(editor_group)
                .unwrap()
                .editor
                .clone()
        });
        cx.update(|cx| {
            editor.update(cx, |view, cx| {
                view.editor.insert_at_cursor("unsaved ");
                cx.notify();
            })
        });
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![tag("x")], cx);
        toggle_t(&s, "- [ ] task #x", cx);
        let said = s.flash(cx).expect("a refusal");
        assert!(said.starts_with("Save "), "{said}");
        assert_eq!(member_text(&s, group, "t.md", cx).as_deref(), Some(TASKS));
    }

    #[gpui_kit::test]
    fn renaming_a_member_rekeys_it_and_rereads(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        run(
            &s,
            FileOpRequest::Rename {
                from: path("a.md"),
                to: path("z.md"),
            },
            cx,
        );
        drive(&rt, cx, |cx| {
            has_member(&s, group, "z.md", cx) && reads(&s, group, cx) == 0
        });
        assert!(!has_member(&s, group, "a.md", cx));
        assert_eq!(member_text(&s, group, "z.md", cx).as_deref(), Some(A));
        assert_eq!(members(&s, group, cx).len(), 2);
    }

    #[gpui_kit::test]
    fn moving_a_folder_updates_every_member_under_it(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let extra = [("d/e.md", "eta #x\n"), ("d/f.md", "phi #x\n")];
        let (_v, s) = vault(cx, &rt, &extra, "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        assert_eq!(members(&s, group, cx).len(), 4);
        run(
            &s,
            FileOpRequest::Move {
                from: path("d"),
                to: path("z/d"),
            },
            cx,
        );
        drive(&rt, cx, |cx| {
            has_member(&s, group, "z/d/e.md", cx)
                && has_member(&s, group, "z/d/f.md", cx)
                && reads(&s, group, cx) == 0
        });
        assert!(!has_member(&s, group, "d/e.md", cx));
        assert!(has_member(&s, group, "a.md", cx), "the rest stay");
        assert_eq!(
            member_text(&s, group, "z/d/f.md", cx).as_deref(),
            Some("phi #x\n")
        );
    }

    #[gpui_kit::test]
    fn deleting_a_member_removes_it_and_a_last_delete_shows_the_empty_view(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![tag("x")], cx);
        run(&s, FileOpRequest::Delete(path("a.md")), cx);
        drive(&rt, cx, |cx| !has_member(&s, group, "a.md", cx));
        assert_eq!(members(&s, group, cx), [path("/b.md")]);
        run(&s, FileOpRequest::Delete(path("b.md")), cx);
        drive(&rt, cx, |cx| members(&s, group, cx).is_empty());
        assert!(facet(&s, group, cx).is_some(), "the group keeps the view");
        assert!(drawn(&s, EMPTY_ID, cx));
        assert_eq!(query(&s, group, cx).as_deref(), Some("#x"));
        assert_states_in_step(&s, cx);
    }

    /// A folder delete takes every member under it out at once: one plan.
    #[gpui_kit::test]
    fn deleting_a_folder_removes_its_members_with_one_plan(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let extra = [
            ("d/e.md", "eta #x\n"),
            ("d/f.md", "phi #x\n"),
            ("d/g/h.md", "chi #x\n"),
            ("dd.md", "not under d #x\n"),
        ];
        let (_v, s) = vault(cx, &rt, &extra, "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        assert_eq!(members(&s, group, cx).len(), 6);
        let view = facet(&s, group, cx).unwrap();
        let before = cx.read(|cx| view.read(cx).plans_run());
        run(&s, FileOpRequest::Delete(path("d")), cx);
        drive(&rt, cx, |cx| members(&s, group, cx).len() == 3);
        assert_eq!(
            members(&s, group, cx),
            [path("/a.md"), path("/b.md"), path("/dd.md")]
        );
        assert_eq!(cx.read(|cx| view.read(cx).plans_run()), before + 1);
    }

    /// A refresh pressed while a newer open still loads leaves that open
    /// alone: it lands and replaces the shown collection.
    #[gpui_kit::test]
    fn a_refresh_does_not_cancel_a_newer_open_still_loading(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        s.workspace.update(cx, |w, cx| {
            w.open_collection(group, "#y".into(), vec![], cx);
            w.refresh_collection(group, cx);
        });
        for _ in 0..20 {
            rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
            cx.run_until_parked();
        }
        drive(&rt, cx, |cx| !loading(&s, group, cx));
        assert_eq!(query(&s, group, cx).as_deref(), Some("#y"));
        assert_eq!(members(&s, group, cx), [path("/c.md")]);
    }

    /// A refresh that lands on a view that no longer shows its query ends
    /// the load too: the line must stop saying `Loading…` at once.
    #[gpui_kit::test]
    fn a_refresh_for_another_query_still_ends_loading(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        let view = facet(&s, group, cx).unwrap();
        // A load that never ends by itself.
        s.workspace.update(cx, |w, cx| {
            let task = cx.spawn(async move |_, _| std::future::pending::<()>().await);
            w.state_mut(group).unwrap().facet_loading = Some(task);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.read(|cx| view.read(cx).is_loading()), "the line loads");
        s.workspace.update(cx, |w, cx| {
            w.collection_refreshed(group, "#other", Err("gone".into()), cx)
        });
        cx.run_until_parked();
        assert!(!loading(&s, group, cx));
        assert!(!cx.read(|cx| view.read(cx).is_loading()), "stale line");
    }

    #[gpui_kit::test]
    fn a_follow_from_a_piece_opens_a_new_tab_and_resolves_from_its_note(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let extra = [
            ("dir/src.md", "#x see [t](t.md)\n"),
            ("dir/t.md", "inner"),
            ("t.md", "outer"),
        ];
        let (_v, s) = vault(cx, &rt, &extra, "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        assert_eq!(tabs(&s, cx), 2);
        let view = facet(&s, group, cx).unwrap();
        view.update(cx, |_, cx| {
            cx.emit(FacetEvent::Follow {
                from: path("dir/src.md"),
                target: FollowTarget::Link {
                    target: "t.md".into(),
                    kind: LinkSpanKind::Markdown,
                },
            })
        });
        drive(&rt, cx, |cx| {
            let now = active(&s, cx);
            note_in(&s, now, cx).is_some_and(|n| n.is_like(&path("dir/t.md")))
                && s.text(cx) == "inner"
        });
        assert_eq!(tabs(&s, cx), 3, "a new tab");
        assert_eq!(
            query(&s, group, cx).as_deref(),
            Some("#x"),
            "the view stays"
        );
    }

    #[gpui_kit::test]
    fn open_note_opens_the_pieces_own_note_in_a_new_tab(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        // Piece 1 is `b.md`'s.
        click(&s, open_id(1), cx);
        drive(&rt, cx, |cx| {
            let now = active(&s, cx);
            now != group && note_in(&s, now, cx).is_some_and(|n| n.is_like(&path("b.md")))
        });
        assert_eq!(tabs(&s, cx), 3);
        assert_eq!(
            query(&s, group, cx).as_deref(),
            Some("#x"),
            "the view stays"
        );
    }

    #[gpui_kit::test]
    fn refresh_runs_the_query_again_and_prunes_picks(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (v, s) = vault(cx, &rt, &[], "c.md");
        let group = new_tab(&s, cx);
        let day = Facet::new(FacetKind::Heading, "day");
        open_coll(&s, &rt, group, "#x", vec![day.clone()], cx);
        assert_eq!(picks(&s, group, cx), [day]);
        // Behind the view's back: `a.md` loses its heading, `n.md` is new.
        rt.block_on(v.vault.save_note(&path("a.md"), "alpha #x\n"))
            .expect("saved");
        rt.block_on(v.vault.create_note(&path("n.md"), "new #x\n"))
            .expect("made");
        let view = facet(&s, group, cx).unwrap();
        view.update(cx, |_, cx| cx.emit(FacetEvent::Refresh));
        drive(&rt, cx, |cx| has_member(&s, group, "n.md", cx));
        assert_eq!(
            members(&s, group, cx),
            [path("/a.md"), path("/b.md"), path("/n.md")]
        );
        assert!(picks(&s, group, cx).is_empty(), "the heading went");
        assert_eq!(
            member_text(&s, group, "a.md", cx).as_deref(),
            Some("alpha #x\n")
        );
    }

    #[gpui_kit::test]
    fn a_second_open_replaces_the_first_load(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let group = new_tab(&s, cx);
        s.workspace.update(cx, |w, cx| {
            w.open_collection(group, "#x".into(), vec![], cx);
            w.open_collection(group, "#y".into(), vec![], cx);
        });
        drive(&rt, cx, |cx| {
            query(&s, group, cx).is_some() && !loading(&s, group, cx)
        });
        for _ in 0..20 {
            rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
            cx.run_until_parked();
        }
        assert_eq!(query(&s, group, cx).as_deref(), Some("#y"));
        assert_eq!(members(&s, group, cx), [path("/c.md")]);
    }

    #[gpui_kit::test]
    fn closing_the_group_drops_the_view_and_its_tasks(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "c.md");
        let group = active(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        // A member read and a refresh under way.
        s.workspace.update(cx, |w, cx| {
            w.refresh_facets(&path("a.md"), cx);
            w.refresh_collection(group, cx);
        });
        assert_eq!(reads(&s, group, cx), 1);
        assert!(loading(&s, group, cx));
        // The last group of the last tab is emptied, not removed.
        s.workspace
            .update(cx, |w, cx| w.close_editor_group(group, cx));
        drive(&rt, cx, |cx| facet(&s, group, cx).is_none());
        assert_eq!(reads(&s, group, cx), 0);
        assert!(!loading(&s, group, cx));
        assert!(!drawn(&s, FACET_VIEW_ID, cx));
        assert_states_in_step(&s, cx);
        // A group that goes takes its state with it.
        let other = new_tab(&s, cx);
        open_coll(&s, &rt, other, "#x", vec![], cx);
        s.workspace
            .update(cx, |w, cx| w.close_editor_group(other, cx));
        drive(&rt, cx, |cx| {
            cx.read(|cx| s.workspace.read(cx).state(other).is_none())
        });
        assert_states_in_step(&s, cx);
    }

    #[gpui_kit::test]
    fn the_same_note_in_an_editor_group_and_a_collection_both_follow_a_rename(
        cx: &mut TestAppContext,
    ) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let editor_group = active(&s, cx);
        let group = new_tab(&s, cx);
        open_coll(&s, &rt, group, "#x", vec![], cx);
        run(
            &s,
            FileOpRequest::Rename {
                from: path("a.md"),
                to: path("z.md"),
            },
            cx,
        );
        drive(&rt, cx, |cx| {
            note_in(&s, editor_group, cx).is_some_and(|n| n.is_like(&path("z.md")))
                && has_member(&s, group, "z.md", cx)
                && reads(&s, group, cx) == 0
        });
        assert_eq!(member_text(&s, group, "z.md", cx).as_deref(), Some(A));
    }

    #[gpui_kit::test]
    fn assert_states_in_step_holds_with_a_collection(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = vault(cx, &rt, &[], "a.md");
        let [left, right] = split(&s, &rt, cx);
        open_coll(&s, &rt, right, "#x", vec![], cx);
        assert_states_in_step(&s, cx);
        let third = new_tab(&s, cx);
        open_coll(&s, &rt, third, "#y", vec![], cx);
        assert_states_in_step(&s, cx);
        s.workspace.update(cx, |w, cx| w.close_shown_tab(cx));
        drive(&rt, cx, |cx| tabs(&s, cx) == 1);
        assert_states_in_step(&s, cx);
        s.workspace
            .update(cx, |w, cx| w.close_editor_group(right, cx));
        drive(&rt, cx, |cx| {
            cx.read(|cx| s.workspace.read(cx).state(right).is_none())
        });
        assert_eq!(active(&s, cx), left);
        assert_states_in_step(&s, cx);
    }
}

/// The entry points: FIND's "Open as collection" button, the two palette
/// commands.
mod entry_points {
    use gpui_kit::AppContext as _;
    use gpui_kit::TestAppContext;
    use kimun_core::nfs::VaultPath;
    use kimun_notes::logic::actions::ActionShortcuts;
    use kimun_notes::logic::layout::Panel;
    use kimun_notes::logic::layout_tree::GroupId;
    use kimun_notes::logic::nav::DrawerView;
    use kimun_notes::logic::query::{QueryContext, resolve_collection_query};
    use tokio::runtime::Runtime;

    use super::super::facet_tests::in_a_group::{active, button, facet, new_tab, note_in, path};
    use super::super::find::{COLLECTION_LABEL, FIND_COLLECTION_ID, FIND_ID};
    use super::super::find_tests::{
        NOTES, click, open_find_on_backlinks, query as find_query, search, started_on,
    };
    use super::super::palette::PaletteRun;
    use super::super::route::NEEDS_FIND_FOCUS;
    use super::super::start_tests::{Started, drive};
    use crate::test_vault::TestVault;

    const NEEDS_NOTE: &str = "This query needs an open note.";

    fn setup(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
        let (v, s) = started_on(cx, rt, NOTES, "alpha.md");
        open_find_on_backlinks(&s, cx);
        (v, s)
    }

    fn coll_query(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| s.workspace.read(cx).collection_query_in(group, cx))
    }

    fn loaded(s: &Started, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let w = s.workspace.read(cx);
            w.collection_query_in(group, cx).is_some()
                && w.state(group).is_some_and(|st| st.facet_loading.is_none())
        })
    }

    fn members(s: &Started, group: GroupId, cx: &TestAppContext) -> Vec<VaultPath> {
        let view = facet(s, group, cx).expect("a facet view");
        cx.read(|cx| view.read(cx).member_paths())
    }

    fn row(
        s: &Started,
        action: ActionShortcuts,
        opener: Panel,
        cx: &TestAppContext,
    ) -> Option<&'static str> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .palette_entries(opener, cx)
                .into_iter()
                .find(|e| *e.run() == PaletteRun::Chord(action.clone()))
                .expect("the row is listed")
                .blocked()
        })
    }

    fn run_row(s: &Started, action: ActionShortcuts, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            s.workspace.update(cx, |w, cx| {
                w.run_palette_row(PaletteRun::Chord(action), window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn find_shown(s: &Started, cx: &TestAppContext) -> bool {
        cx.read(|cx| s.workspace.read(cx).is_shown(DrawerView::Find))
    }

    #[gpui_kit::test]
    fn the_find_button_opens_the_results_as_a_collection(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = setup(cx, &rt);
        search(&s, &rt, "#rust", cx);
        let group = active(&s, cx);
        click(&s, FIND_COLLECTION_ID, cx);
        drive(&rt, cx, |cx| loaded(&s, group, cx));
        assert_eq!(coll_query(&s, group, cx).as_deref(), Some("#rust"));
        assert_eq!(
            members(&s, group, cx),
            [path("/alpha.md"), path("/beta.md"), path("/epsilon.md")]
        );
        assert_eq!(note_in(&s, group, cx), None);
        // The drawer stays as it was, with its query.
        assert!(find_shown(&s, cx));
        assert!(s.shows(FIND_ID, cx));
        assert_eq!(find_query(&s, cx), "#rust");
    }

    #[gpui_kit::test]
    fn the_palette_command_does_the_same_and_needs_find_focus(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = setup(cx, &rt);
        search(&s, &rt, "#go", cx);
        let group = active(&s, cx);
        let action = ActionShortcuts::OpenQueryAsCollection;
        assert_eq!(
            row(&s, action.clone(), Panel::Editor, cx),
            Some(NEEDS_FIND_FOCUS)
        );
        let opener = cx.read(|cx| s.workspace.read(cx).focused_panel());
        assert_eq!(
            row(&s, action.clone(), opener, cx),
            None,
            "FIND has the keyboard"
        );
        run_row(&s, action, cx);
        drive(&rt, cx, |cx| loaded(&s, group, cx));
        assert_eq!(coll_query(&s, group, cx).as_deref(), Some("#go"));
        assert_eq!(members(&s, group, cx), [path("/epsilon.md")]);
    }

    #[gpui_kit::test]
    fn an_empty_query_refuses(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = started_on(cx, &rt, NOTES, "alpha.md");
        click(&s, super::super::rail_cell_id(DrawerView::Find), cx);
        assert_eq!(find_query(&s, cx), "");
        let group = active(&s, cx);
        run_row(&s, ActionShortcuts::OpenQueryAsCollection, cx);
        assert_eq!(s.flash(cx).as_deref(), Some("No query to open"));
        assert!(facet(&s, group, cx).is_none());
        assert!(note_in(&s, group, cx).is_some(), "the note stays");
    }

    #[gpui_kit::test]
    fn a_note_query_without_an_open_note_refuses_with_the_flash(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = setup(cx, &rt);
        search(&s, &rt, "#rust", cx);
        let group = active(&s, cx);
        click(&s, FIND_COLLECTION_ID, cx);
        drive(&rt, cx, |cx| loaded(&s, group, cx));
        // The group is now a collection: it holds no note.
        search(&s, &rt, "<{note}", cx);
        click(&s, FIND_COLLECTION_ID, cx);
        cx.run_until_parked();
        assert_eq!(s.flash(cx).as_deref(), Some(NEEDS_NOTE));
        assert_eq!(
            coll_query(&s, group, cx).as_deref(),
            Some("#rust"),
            "it stays"
        );
    }

    #[gpui_kit::test]
    fn a_note_query_is_stored_resolved(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = setup(cx, &rt);
        let group = active(&s, cx);
        let resolved =
            resolve_collection_query("<{note}", &QueryContext::with_note(Some(path("/alpha.md"))))
                .expect("a note is open");
        assert!(!resolved.contains("{note}"), "{resolved}");
        click(&s, FIND_COLLECTION_ID, cx);
        drive(&rt, cx, |cx| loaded(&s, group, cx));
        assert_eq!(
            coll_query(&s, group, cx).as_deref(),
            Some(resolved.as_str())
        );
        assert_eq!(
            members(&s, group, cx),
            [path("/beta.md"), path("/epsilon.md")]
        );
        // Another note, opened in another group, changes nothing.
        new_tab(&s, cx);
        s.workspace
            .update(cx, |w, cx| w.open_note(path("/gamma.md"), cx));
        cx.run_until_parked();
        assert_eq!(
            coll_query(&s, group, cx).as_deref(),
            Some(resolved.as_str())
        );
    }

    #[gpui_kit::test]
    fn the_button_has_a_label_and_is_24px(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (_v, s) = setup(cx, &rt);
        search(&s, &rt, "#rust", cx);
        let (label, _, w, h) = button(&s, FIND_COLLECTION_ID, cx).expect("drawn");
        assert_eq!(label, COLLECTION_LABEL);
        assert_eq!(COLLECTION_LABEL, "Open as collection");
        assert_eq!((w, h), (24.0, 24.0));
    }

    #[gpui_kit::test]
    fn refresh_collection_runs_in_a_collection_and_is_refused_elsewhere(cx: &mut TestAppContext) {
        let rt = super::super::start_tests::runtime();
        let _guard = rt.enter();
        let (v, s) = setup(cx, &rt);
        let group = active(&s, cx);
        let action = ActionShortcuts::RefreshCollection;
        let opener = Panel::Editor;
        let blocked = row(&s, action.clone(), opener, cx);
        assert!(blocked.is_some(), "a note group is no collection");
        run_row(&s, action.clone(), cx);
        assert_eq!(s.flash(cx).as_deref(), blocked);
        // In a collection it is listed as runnable and runs the query again.
        search(&s, &rt, "#rust", cx);
        click(&s, FIND_COLLECTION_ID, cx);
        drive(&rt, cx, |cx| loaded(&s, group, cx));
        assert_eq!(row(&s, action.clone(), opener, cx), None);
        rt.block_on(v.vault.create_note(&path("n.md"), "new #rust\n"))
            .expect("the note is made");
        run_row(&s, action, cx);
        drive(&rt, cx, |cx| {
            members(&s, group, cx)
                .iter()
                .any(|m| m.is_like(&path("n.md")))
        });
    }
}

#[gpui_kit::test]
fn a_reload_or_a_removal_recounts_the_chips(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#a [[x]]\n"), (B, "Beta", "#a [[x]] [[y]]\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("a")]);
    let links = || menu_id(ListKind::Links);
    assert_eq!(label(cx, links()).as_deref(), Some("Links 2"));
    // A refresh brings other members.
    let fresh = vec![
        member(A, "Alpha", "#a [[x]]\n\n[[y]]\n"),
        member(C, "Gamma", "#a [[y]] [[z]]\n"),
    ];
    view.update(cx, |view, cx| view.replace_members(fresh, 2, cx));
    assert_eq!(label(cx, links()).as_deref(), Some("Links 3"));
    // A save of one member.
    view.update(cx, |view, cx| {
        view.set_member_text(&p(C), "#a [[y]]\n\n[[z]]\n".into(), cx)
    });
    assert_eq!(label(cx, links()).as_deref(), Some("Links 2"));
    // A delete that drops no pick plans again but does not select.
    view.update(cx, |view, cx| view.remove_members(&[p(C)], cx));
    assert_eq!(label(cx, links()).as_deref(), Some("Links 1"));
}
