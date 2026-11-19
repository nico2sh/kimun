//! UI tests for a **Facet view** over a **Collection** (CONTEXT.md): the view
//! as a list of member notes. A single-note view is a collection of one
//! member; these tests check what each piece says about its own note.
#![cfg(test)]

use gpui_kit::{Entity, TestAppContext, VisualTestContext};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, FacetKind, MAX_RANGES};
use kimun_notes::logic::editor::claim::FollowTarget;

use super::facet::{
    EMPTY_ID, FIRST_NOTES_ID, FacetEvent, FacetView, MemberInit, REFRESH_ID, REMOVE_LAST_ID,
    SHOW_MORE_ID, SUMMARY_ID, crumb_id, gap_count_id, gap_down_id, gap_up_id, note_title_id,
    open_id, pill_id, show_all_id,
};
use super::facet_tests::{
    Events, bounds, cell, chunk_texts, click, focus_view, heading, key, label, open, open_with,
    picks, picks_changed, press, shows, tab_to, tag, to_bottom,
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
        FacetView::new_collection("#q".into(), members, total, picks, cx)
    })
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
    assert!(!shows(cx, FIRST_NOTES_ID));
}

#[gpui_kit::test]
fn every_piece_shows_its_notes_title_in_its_breadcrumb_row(cx: &mut TestAppContext) {
    let alpha = "# Day\n\n#x a1\n\nfar\n\nfar\n\n#x a2\n";
    let notes = [(A, "Alpha", alpha), (B, "Beta", "#x b1\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    assert_eq!(chunk_texts(&view, cx), ["#x a1", "#x a2", "#x b1"]);
    assert_eq!(label(cx, note_title_id(0)).as_deref(), Some("Alpha"));
    assert_eq!(label(cx, note_title_id(1)).as_deref(), Some("Alpha"));
    assert_eq!(label(cx, note_title_id(2)).as_deref(), Some("Beta"));
    // `Alpha › Day`, one row, `Open source` at its right end.
    let title = bounds(cx, note_title_id(0));
    let crumb = bounds(cx, crumb_id(0, 0));
    let open = bounds(cx, open_id(0));
    assert_eq!(label(cx, crumb_id(0, 0)).as_deref(), Some("Day"));
    assert!(title.right() <= crumb.left(), "the title leads the row");
    assert!(
        title.top() < crumb.bottom() && crumb.top() < title.bottom(),
        "one row: {title:?} {crumb:?}"
    );
    assert!(crumb.right() <= open.left());
    // The title is no facet: a click on it picks nothing.
    click(cx, note_title_id(0));
    assert_eq!(picks(&view, cx), [tag("x")]);
}

#[gpui_kit::test]
fn a_single_note_view_draws_no_title(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, "# Day\n\n#x a\n", vec![tag("x")]);
    assert!(shows(cx, crumb_id(0, 0)));
    assert!(!shows(cx, note_title_id(0)));
}

#[gpui_kit::test]
fn a_long_title_shrinks_and_open_source_stays(cx: &mut TestAppContext) {
    let long = "A very long note title that would never fit into a narrow row at all";
    let text = "# The outermost section of this rather long note\n\n#x alpha\n";
    let (_view, cx, _events, _sub) = open_collection(cx, &[(A, long, text)], 1, vec![tag("x")]);
    let open = f32::from(bounds(cx, open_id(0)).size.width);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(360.), gpui_kit::px(800.)));
    let title = bounds(cx, note_title_id(0));
    let button = bounds(cx, open_id(0));
    assert!(f32::from(title.size.width) >= 32.0, "{title:?}");
    assert_eq!(f32::from(button.size.width), open, "Open source whole");
    assert!(title.right() <= button.left(), "{title:?} {button:?}");
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
    assert_eq!(label(cx, note_title_id(1)).as_deref(), Some("Gamma"));
}

#[gpui_kit::test]
fn merged_pills_count_across_notes(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x one #x\n"), (B, "Beta", "#X two\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    assert!(shows(cx, pill_id(&tag("x"))));
    assert_eq!(label(cx, pill_id(&tag("x"))).as_deref(), Some("#x"));
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
    click(cx, pill_id(&heading("plan")));
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
    assert_eq!(label(cx, note_title_id(0)).as_deref(), Some("Alpha"));
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
    assert_eq!(label(cx, gap_count_id(1)).as_deref(), Some("1 hidden"));
    assert_eq!(label(cx, gap_count_id(2)).as_deref(), Some("1 hidden"));
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
    assert_eq!(label(cx, gap_count_id(1)).as_deref(), Some("2 hidden"));
    click(cx, show_all_id(1));
    assert_eq!(chunk_texts(&view, cx), ["#x a\n\nt1\n\nt2", "#x b"]);
    assert_eq!(label(cx, gap_count_id(2)).as_deref(), Some("1 hidden"));
}

#[gpui_kit::test]
fn open_source_names_the_pieces_own_note(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "b\n\n#x b\n")];
    let (_view, cx, events, _sub) = open_collection(cx, &notes, 2, vec![tag("x")]);
    click(cx, open_id(1));
    assert_eq!(
        *events.borrow(),
        [FacetEvent::OpenSource {
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
    assert_eq!(label(cx, note_title_id(1)).as_deref(), Some("Gamma"));
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
        [FacetEvent::OpenSource {
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
        label(cx, FIRST_NOTES_ID).as_deref(),
        Some("Showing the first 2 of 600 notes")
    );
    let members = vec![member(A, "Alpha", "a\n"), member(B, "Beta", "b\n")];
    view.update(cx, |view, cx| view.replace_members(members, 2, cx));
    assert!(!shows(cx, FIRST_NOTES_ID));
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
fn the_summary_names_the_notes(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a\n\nb\n\n#x c\n"),
        (B, "Beta", "#x d\n"),
        (C, "Gamma", "e\n"),
    ];
    let (_view, cx, _events, _sub) = open_collection(cx, &notes, 3, vec![tag("x")]);
    assert_eq!(
        label(cx, SUMMARY_ID).as_deref(),
        Some("Showing text tagged #x \u{b7} 3 pieces in 2 notes")
    );
}

/// A plain press on a note link in a collection picks its merged facet.
#[gpui_kit::test]
fn a_plain_press_on_a_link_picks_its_note_facet(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "see [[Plan]] now\n"), (B, "Beta", "other\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    let at = cell(&view, 0, 7, 0, cx);
    press(cx, at, false);
    assert_eq!(picks(&view, cx), [Facet::new(FacetKind::NoteLink, "plan")]);
    assert_eq!(chunk_texts(&view, cx), ["see [[Plan]] now"]);
}

// ── carried over from the member refactor ────────────────────────────────

#[gpui_kit::test]
fn open_source_names_its_note(cx: &mut TestAppContext) {
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
    assert!(!shows(cx, FIRST_NOTES_ID));
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
        label(cx, FIRST_NOTES_ID).as_deref(),
        Some("Showing the first 500 of 600 notes")
    );
    view.update(cx, |view, cx| view.remove_members(&[p("/n/007.md")], cx));
    assert_eq!(
        label(cx, FIRST_NOTES_ID).as_deref(),
        Some("Showing the first 499 of 599 notes")
    );
}

/// A member handed in with no title (a note with no text) shows its name.
#[gpui_kit::test]
fn a_blank_title_falls_back_to_the_file_name(cx: &mut TestAppContext) {
    let notes = [(A, "", "#x a\n"), (B, "  ", "#x b\n")];
    let (view, cx, _events, _sub) = open_collection(cx, &notes, 2, vec![]);
    assert_eq!(label(cx, note_title_id(0)).as_deref(), Some("alpha"));
    let members = vec![member(C, "", "#x c\n")];
    view.update(cx, |view, cx| view.replace_members(members, 1, cx));
    assert_eq!(label(cx, note_title_id(0)).as_deref(), Some("gamma"));
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
    assert!(!shows(cx, FIRST_NOTES_ID));
    // Nor after a refresh that loses one to a failed read.
    let members = vec![member(A, "Alpha", "a\n")];
    view.update(cx, |view, cx| view.replace_members(members, 2, cx));
    assert!(!shows(cx, FIRST_NOTES_ID));
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
        label(cx, FIRST_NOTES_ID).as_deref(),
        Some("Showing the first 499 of 600 notes")
    );
    view.update(cx, |view, cx| view.remove_members(&[p("/n/007.md")], cx));
    assert_eq!(
        label(cx, FIRST_NOTES_ID).as_deref(),
        Some("Showing the first 498 of 599 notes")
    );
    let members = vec![member(A, "Alpha", "a\n")];
    view.update(cx, |view, cx| view.replace_members(members, 700, cx));
    assert_eq!(
        label(cx, FIRST_NOTES_ID).as_deref(),
        Some("Showing the first 1 of 700 notes")
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
    fn open_source_opens_the_pieces_own_note_in_a_new_tab(cx: &mut TestAppContext) {
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
