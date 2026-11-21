//! UI tests for the **Facet view**'s chrome: a collection's meta line, the
//! copy of the gaps and the summary, the colour of its labels and the size
//! of a dateline's title. Each test draws a `FacetView` alone in a headless
//! window (see `facet_tests` for the helpers).
#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::NaiveTime;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, Role, TestAppContext, VisualTestContext};
use kimun_core::nfs::VaultPath;
use kimun_core::note::Facet;
use kimun_notes::settings::themes::Theme;

use super::facet::{
    COLLECTION_LINE_ID, EMPTY_ID, FacetView, MemberInit, SHOW_MORE_COUNT_ID, SUMMARY_ID, chunk_id,
    crumb_id, dateline_title_id, gap_count_id, gap_down_id, gap_up_id,
};
use super::facet_tests::{
    Events, bounds, chunk_texts, click, heading, label, open, open_with, picks, shows, tag,
    to_bottom,
};
use super::facet_ui::{Strip, crumb_color, label_color};
use crate::theme;

fn member(path: &str, title: &str, text: &str) -> MemberInit {
    MemberInit {
        path: VaultPath::new(path),
        title: title.to_string(),
        text: text.to_string(),
    }
}

fn ten_forty_two() -> NaiveTime {
    NaiveTime::from_hms_opt(10, 42, 7).unwrap()
}

/// A collection view of `notes` (path, title, text), `total` hits, with
/// `picks`, its clock `clock`, in its own window, drawn once.
fn collection_at<'a>(
    cx: &'a mut TestAppContext,
    notes: &[(&str, &str, &str)],
    total: usize,
    picks: Vec<Facet>,
    clock: fn() -> NaiveTime,
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
        FacetView::new_collection_at("#q".into(), members, total, picks, clock, cx)
    })
}

/// [`collection_at`] with a clock that says 10:42.
fn collection<'a>(
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
    collection_at(cx, notes, total, picks, ten_forty_two)
}

const A: &str = "/a/alpha.md";
const B: &str = "/b/beta.md";
const C: &str = "/c/gamma.md";
const D: &str = "/d/delta.md";

fn line(cx: &mut VisualTestContext) -> Option<String> {
    label(cx, COLLECTION_LINE_ID)
}

// ── the collection line ──────────────────────────────────────────────────

#[gpui_kit::test]
fn a_collection_always_shows_its_note_count(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n"), (C, "Gamma", "c\n")];
    let (_view, cx, _events, _sub) = collection(cx, &notes, 3, vec![]);
    assert_eq!(line(cx).as_deref(), Some("3 notes \u{b7} list from 10:42"));
    // At rest it is a plain label: a screen reader does not read the
    // numbers again after each pick.
    assert_eq!(line_role(cx), Some(Role::Label));
}

fn line_role(cx: &mut VisualTestContext) -> Option<Role> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.find(COLLECTION_LINE_ID).role()
    })
}

/// The line is a status only while it reports something new: a load or
/// the end of a refresh. Then a screen reader says it.
#[gpui_kit::test]
fn the_collection_line_is_a_status_only_while_it_reports(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n")];
    let (view, cx, _events, _sub) = collection(cx, &notes, 2, vec![]);
    view.update(cx, |view, cx| view.set_loading(true, cx));
    assert_eq!(line_role(cx), Some(Role::Status), "loading");
    let members = vec![member(A, "Alpha", "a\n")];
    view.update(cx, |view, cx| {
        view.replace_members(members, 1, cx);
        view.set_loading(false, cx);
    });
    assert!(line(cx).unwrap().starts_with("Refreshed"));
    assert_eq!(line_role(cx), Some(Role::Status), "the refresh report");
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    assert_eq!(line_role(cx), Some(Role::Label), "at rest again");
}

#[gpui_kit::test]
fn a_collection_of_one_says_one_note(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = collection(cx, &[(A, "Alpha", "a\n")], 1, vec![]);
    assert_eq!(line(cx).as_deref(), Some("1 note \u{b7} list from 10:42"));
}

#[gpui_kit::test]
fn a_single_note_view_has_no_collection_line(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, "#x a\n", vec![]);
    assert!(!shows(cx, COLLECTION_LINE_ID));
}

#[gpui_kit::test]
fn the_collection_line_counts_matched_notes_with_picks(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "#x a\n\nb\n\n#x c\n"),
        (B, "Beta", "#x d\n"),
        (C, "Gamma", "e\n"),
    ];
    let (_view, cx, _events, _sub) = collection(cx, &notes, 3, vec![tag("x")]);
    assert_eq!(
        line(cx).as_deref(),
        Some("3 pieces in 2 of 3 notes \u{b7} list from 10:42")
    );
    // The summary keeps the filter text. The line holds the counts, so
    // the summary does not say them again.
    assert_eq!(
        label(cx, SUMMARY_ID).as_deref(),
        Some("Showing text tagged #x")
    );
}

#[gpui_kit::test]
fn the_collection_line_leaves_the_cut_to_the_show_more_row(cx: &mut TestAppContext) {
    let text: String = (0..150).map(|i| format!("#t m{i}\n\nplain\n\n")).collect();
    let notes = [(A, "Alpha", text.as_str())];
    let (view, cx, _events, _sub) = collection(cx, &notes, 1, vec![tag("t")]);
    assert_eq!(
        line(cx).as_deref(),
        Some("150 pieces in 1 of 1 note \u{b7} list from 10:42")
    );
    to_bottom(&view, cx);
    assert_eq!(
        label(cx, SHOW_MORE_COUNT_ID).as_deref(),
        Some("100 of 150 pieces shown")
    );
}

#[gpui_kit::test]
fn the_collection_line_says_first_n_of_m_when_the_query_hit_the_cap(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n")];
    let (_view, cx, _events, _sub) = collection(cx, &notes, 600, vec![]);
    assert_eq!(
        line(cx).as_deref(),
        Some("2 notes \u{b7} first 2 of 600 \u{b7} list from 10:42")
    );
}

/// The minutes past the hour the test clock reads, for clocks that move.
static MOVING_MINUTE: AtomicU32 = AtomicU32::new(5);

fn moving_clock() -> NaiveTime {
    NaiveTime::from_hms_opt(9, MOVING_MINUTE.load(Ordering::SeqCst), 0).unwrap()
}

#[gpui_kit::test]
fn the_collection_line_shows_the_age_of_the_list(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n")];
    let (view, cx, _events, _sub) = collection_at(cx, &notes, 1, vec![], moving_clock);
    // Local time, 24-hour, two digits each.
    assert_eq!(line(cx).as_deref(), Some("1 note \u{b7} list from 09:05"));
    // The age is the load's: a later draw keeps it.
    MOVING_MINUTE.store(30, Ordering::SeqCst);
    assert_eq!(line(cx).as_deref(), Some("1 note \u{b7} list from 09:05"));
    // A refresh is a new load.
    let members = vec![member(A, "Alpha", "a\n")];
    view.update(cx, |view, cx| view.replace_members(members, 1, cx));
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    assert_eq!(line(cx).as_deref(), Some("1 note \u{b7} list from 09:30"));
}

#[gpui_kit::test]
fn the_collection_line_shows_loading_while_the_query_runs(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n"), (B, "Beta", "b\n")];
    let (view, cx, _events, _sub) = collection(cx, &notes, 2, vec![]);
    view.update(cx, |view, cx| view.set_loading(true, cx));
    assert_eq!(line(cx).as_deref(), Some("Loading\u{2026}"));
    assert_eq!(chunk_texts(&view, cx), ["a", "b"], "the old content stays");
    view.update(cx, |view, cx| view.set_loading(false, cx));
    assert_eq!(line(cx).as_deref(), Some("2 notes \u{b7} list from 10:42"));
}

#[gpui_kit::test]
fn a_refresh_reports_added_removed_and_dropped_picks_then_clears(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "#x a\n"), (B, "Beta", "#y b\n")];
    let (view, cx, _events, _sub) = collection(cx, &notes, 2, vec![tag("y")]);
    view.update(cx, |view, cx| view.set_loading(true, cx));
    let members = vec![
        member(A, "Alpha", "#x a\n"),
        member(C, "Gamma", "c\n"),
        member(D, "Delta", "d\n"),
    ];
    view.update(cx, |view, cx| {
        view.replace_members(members, 3, cx);
        view.set_loading(false, cx);
    });
    assert!(picks(&view, cx).is_empty(), "y went with its note");
    assert_eq!(
        line(cx).as_deref(),
        Some("Refreshed: 2 notes added, 1 removed \u{b7} 1 pick dropped")
    );
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert!(line(cx).unwrap().starts_with("Refreshed"), "still there");
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(line(cx).as_deref(), Some("3 notes \u{b7} list from 10:42"));

    // A refresh that changes nothing says so.
    let members = vec![
        member(A, "Alpha", "#x a\n"),
        member(C, "Gamma", "c\n"),
        member(D, "Delta", "d\n"),
    ];
    view.update(cx, |view, cx| view.replace_members(members, 3, cx));
    assert_eq!(line(cx).as_deref(), Some("Refreshed: no changes"));
    // A new load takes over the line at once.
    view.update(cx, |view, cx| view.set_loading(true, cx));
    assert_eq!(line(cx).as_deref(), Some("Loading\u{2026}"));
    view.update(cx, |view, cx| view.set_loading(false, cx));
    assert_eq!(line(cx).as_deref(), Some("3 notes \u{b7} list from 10:42"));

    // One note removed, two picks dropped.
    let notes_back = vec![member(A, "Alpha", "#x a #z\n"), member(C, "Gamma", "c\n")];
    view.update(cx, |view, cx| view.replace_members(notes_back, 2, cx));
    view.update(cx, |view, cx| {
        view.toggle(tag("x"), cx);
        view.toggle(tag("z"), cx);
    });
    let members = vec![member(C, "Gamma", "c\n")];
    view.update(cx, |view, cx| view.replace_members(members, 1, cx));
    assert_eq!(
        line(cx).as_deref(),
        Some("Refreshed: 1 note removed \u{b7} 2 picks dropped")
    );
}

// ── copy ─────────────────────────────────────────────────────────────────

#[gpui_kit::test]
fn hidden_text_says_blocks_hidden_with_the_right_plural(cx: &mut TestAppContext) {
    let text = "#x a\n\nb\n\n#x c\n\nd\n\ne\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(
        label(cx, gap_count_id(1)).as_deref(),
        Some("1 block hidden")
    );
    assert_eq!(
        label(cx, gap_count_id(2)).as_deref(),
        Some("2 blocks hidden")
    );
    assert_eq!(
        label(cx, gap_down_id(1)).as_deref(),
        Some("Show next block")
    );
    assert_eq!(
        label(cx, gap_up_id(1)).as_deref(),
        Some("Show previous block")
    );
}

#[gpui_kit::test]
fn the_summary_joins_kinds_with_and(cx: &mut TestAppContext) {
    let text = "# D\n\n#run #garden\n";
    let picks = vec![tag("run"), tag("garden"), heading("d")];
    let (_view, cx, _events, _sub) = open(cx, text, picks);
    assert_eq!(
        label(cx, SUMMARY_ID).as_deref(),
        Some("Showing text tagged #run or #garden, and in D \u{b7} 1 piece")
    );
}

#[gpui_kit::test]
fn the_empty_reason_joins_kinds_with_and_too(cx: &mut TestAppContext) {
    let text = "# A\n\n#run\n\n# B\n\nother\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("run"), heading("b")]);
    assert_eq!(
        label(cx, EMPTY_ID).as_deref(),
        Some("No text tagged #run, and in B.")
    );
}

// ── colour and type ──────────────────────────────────────────────────────

/// Chip labels, `Clear`, `Undo`, `Show all`, `Show more`, crumbs and `Open
/// note` take the text colour. A dimmed chip keeps the muted one, so the
/// dimming still shows.
#[test]
fn interactive_labels_use_the_foreground_color() {
    for t in Theme::builtins() {
        assert_eq!(label_color(false, &t), theme::foreground(&t), "{}", t.name);
        assert_eq!(label_color(true, &t), theme::muted(&t), "{}", t.name);
    }
}

/// The crumbs strip and the hover and press of the stops on it start from
/// the page, not from the panel: the slot header tint is not used here.
#[test]
fn the_crumbs_strip_takes_its_colors_from_the_page() {
    for t in Theme::builtins() {
        let strip = Strip::of(&t);
        assert_eq!(strip.bg, theme::facet_strip_background(&t), "{}", t.name);
        let hover = theme::facet_strip_hover(&t);
        assert_eq!(strip.fills.hover, hover, "{}", t.name);
        let press = theme::pressed_background(hover, &t);
        assert_eq!(strip.fills.press, press, "{}", t.name);
    }
}

/// A crumb is quieter than the text at rest. It takes the text colour
/// while it has the keyboard, as it does under the pointer.
#[test]
fn a_crumb_is_muted_at_rest_and_in_the_text_colour_with_the_keyboard() {
    for t in Theme::builtins() {
        assert_eq!(crumb_color(false, &t), theme::muted(&t), "{}", t.name);
        assert_eq!(crumb_color(true, &t), theme::foreground(&t), "{}", t.name);
    }
}

/// Measured on the drawn title, not on the size function: at the default
/// and at a large editor font the title row is at least one line of the
/// piece text high.
#[gpui_kit::test]
fn the_dateline_title_is_not_smaller_than_the_piece_text(cx: &mut TestAppContext) {
    let notes = [(A, "Alpha", "a\n")];
    let (view, cx, _events, _sub) = collection(cx, &notes, 1, vec![]);
    for size in [None, Some(gpui_kit::px(18.))] {
        view.update(cx, |_, cx| {
            if size.is_some() {
                cx.set_global(crate::editor_font::EditorFontSetting { family: None, size });
            }
            cx.notify();
        });
        let line_height = view.update(cx, |_, cx| {
            (f32::from(crate::editor_font::font_size(cx)) * crate::editor_element::LINE_HEIGHT)
                .round()
        });
        let title = bounds(cx, dateline_title_id(0));
        assert!(
            f32::from(title.size.height) >= line_height,
            "title {:?} is lower than a piece line of {line_height} px at {size:?}",
            title.size.height
        );
    }
}

/// A crumb toggles a pick of its heading, so it shows that state as a
/// picked pill does: it reports itself toggled and its name says that a
/// press removes the pick.
#[gpui_kit::test]
fn a_crumb_shows_whether_its_heading_is_picked(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# Day\n\n#x a\n", vec![tag("x")]);
    let checked = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.find(crumb_id(0, 0)).checked()
        })
    };
    assert_eq!(label(cx, crumb_id(0, 0)).as_deref(), Some("Day"));
    assert_eq!(checked(cx), Some(false));
    click(cx, crumb_id(0, 0));
    assert_eq!(picks(&view, cx), [tag("x"), heading("day")]);
    assert_eq!(label(cx, crumb_id(0, 0)).as_deref(), Some("Remove Day"));
    assert_eq!(checked(cx), Some(true));
}

/// Each piece is a named group, so a screen reader says where it is. In a
/// single-note view the name is its crumbs. In a collection it is the note
/// title and the last crumb. A piece with no crumbs takes the note title.
#[gpui_kit::test]
fn each_piece_group_is_named_after_its_place(cx: &mut TestAppContext) {
    let text = "# Day\n\n## Morning\n\n#x a\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("x")]);
    assert_eq!(label(cx, chunk_id(0)).as_deref(), Some("Day, Morning"));
}

#[gpui_kit::test]
fn a_collection_piece_group_is_named_after_its_note_and_last_crumb(cx: &mut TestAppContext) {
    let notes = [
        (A, "Alpha", "# Day\n\n## Morning\n\n#x a\n"),
        (B, "Beta", "#x b\n"),
    ];
    let (_view, cx, _events, _sub) = collection(cx, &notes, 2, vec![tag("x")]);
    assert_eq!(label(cx, chunk_id(0)).as_deref(), Some("Alpha, Morning"));
    assert_eq!(label(cx, chunk_id(1)).as_deref(), Some("Beta"));
}
