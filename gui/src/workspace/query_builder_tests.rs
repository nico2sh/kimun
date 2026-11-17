//! UI integration tests for the query builder: opened from FIND (its button
//! and the `QueryBuilder` chord) and from the search browser, on a real
//! `Workspace` over a real, indexed temp vault.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the lists the dialog
//! suggests from, the notes it asks for and the count all land only when
//! tokio runs, which is also the proof that none is read on the UI thread.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::query::{QueryCount, RowKind};
use kimun_notes::logic::sort::{SortField, SortOrder};
use tokio::runtime::Runtime;

use super::file_dialogs::CONFIRM_ID;
use super::find::{FIND_BUILD_ID, FIND_DEBOUNCE};
use super::modal::ModalKind;
use super::picker::{BUILD_BUTTON_ID, NotePicker};
use super::query_builder::{BUILDER_SUGGESTIONS_ID, COUNT_DEBOUNCE, QueryBuilderDialog};
use super::start_tests::select_all_key;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

const NOTES: &[(&str, &str)] = &[
    ("alpha.md", "# Alpha\nthe alpha note #rust, see [[beta]]"),
    ("beta.md", "# Beta\nback to [[alpha]] here #rust #go"),
    ("gamma.md", "# Gamma\nplain #go"),
];

fn started(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("alpha.md"))),
        v.settings.clone(),
    );
    let want = NOTES[0].1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx)
    });
    (v, started)
}

fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn type_in(s: &Started, text: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn click(s: &Started, id: &'static str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn click_rail_find(s: &Started, cx: &mut TestAppContext) {
    let id = super::rail_cell_id(DrawerView::Find);
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

/// FIND shown, with `query` typed in and run.
fn find_on(s: &Started, query: &str, cx: &mut TestAppContext) {
    click_rail_find(s, cx);
    press(s, select_all_key(), cx);
    press(s, "backspace", cx);
    type_in(s, query, cx);
    cx.executor().advance_clock(FIND_DEBOUNCE);
    cx.run_until_parked();
}

fn find_query(s: &Started, cx: &TestAppContext) -> String {
    let view = cx.read(|cx| s.workspace.read(cx).find.clone().expect("FIND is built"));
    cx.read(|cx| view.read(cx).query(cx))
}

fn dialog(s: &Started, cx: &TestAppContext) -> Entity<QueryBuilderDialog> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .query_builder
            .as_ref()
            .and_then(|open| open.dialog.upgrade())
            .expect("the builder is open")
    })
}

fn builder_open(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).query_builder.is_some())
}

/// What the dialog's rows say: (kind, not, value).
fn rows_of(s: &Started, cx: &TestAppContext) -> Vec<(RowKind, bool, String)> {
    let d = dialog(s, cx);
    cx.read(|cx| {
        d.read(cx)
            .rows()
            .iter()
            .map(|r| (r.kind, r.exclude, r.value.clone()))
            .collect()
    })
}

fn query_of(s: &Started, cx: &TestAppContext) -> String {
    let d = dialog(s, cx);
    cx.read(|cx| d.read(cx).query())
}

/// The count once the debounce is over and the search has run.
fn counted(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> QueryCount {
    cx.executor().advance_clock(COUNT_DEBOUNCE);
    let d = dialog(s, cx);
    drive(rt, cx, |cx| cx.read(|cx| d.read(cx).count().is_some()));
    cx.read(|cx| d.read(cx).count().cloned().expect("counted"))
}

fn with_dialog<R>(
    s: &Started,
    cx: &mut TestAppContext,
    f: impl FnOnce(
        &mut QueryBuilderDialog,
        &mut gpui_kit::Window,
        &mut gpui_kit::Context<QueryBuilderDialog>,
    ) -> R,
) -> R {
    let d = dialog(s, cx);
    cx.update_window(s.handle, |_, window, cx| {
        d.update(cx, |dialog, cx| f(dialog, window, cx))
    })
    .unwrap()
}

/// FIND on `query`, the builder open on it.
fn builder_on(s: &Started, rt: &Runtime, query: &str, cx: &mut TestAppContext) {
    find_on(s, query, cx);
    click(s, FIND_BUILD_ID, cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::QueryBuilder));
    // The lists load off the UI thread.
    drive(rt, cx, |_| true);
    cx.run_until_parked();
}

// ── Opening ──────────────────────────────────────────────────────────────

/// FIND's `Build…` opens the builder on the field's query: one row per
/// condition, a tag a "not" row, a note link a link row.
#[gpui_kit::test]
fn the_build_button_opens_the_builder_on_the_query(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "#rust -#go <alpha", cx);
    assert_eq!(
        rows_of(&s, cx),
        [
            (RowKind::Tag, false, "rust".to_string()),
            (RowKind::Tag, true, "go".to_string()),
            (RowKind::LinksTo, false, "alpha".to_string()),
        ]
    );
    // Written grouped: what must match, then what must not.
    assert_eq!(query_of(&s, cx), "#rust <alpha -#go");
}

/// The chord (F7) opens it from FIND's field too.
#[gpui_kit::test]
fn the_chord_opens_the_builder_from_find(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    find_on(&s, "#rust", cx);
    press(&s, "f7", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::QueryBuilder));
    assert!(builder_open(&s, cx));
}

/// An empty query opens on one empty row to type in.
#[gpui_kit::test]
fn an_empty_query_starts_on_one_empty_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "", cx);
    assert_eq!(rows_of(&s, cx), [(RowKind::Text, false, String::new())]);
    assert_eq!(query_of(&s, cx), "", "an incomplete row writes nothing");
}

// ── Editing ──────────────────────────────────────────────────────────────

/// Rows are added, typed in, marked "not" and removed; the query follows.
#[gpui_kit::test]
fn editing_the_rows_rewrites_the_query(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "#rust", cx);
    with_dialog(&s, cx, |d, window, cx| d.add_row(window, cx));
    // The new row has the keyboard.
    type_in(&s, "back", cx);
    assert_eq!(query_of(&s, cx), "back #rust");
    let first = with_dialog(&s, cx, |d, _, _| d.row_id(0));
    with_dialog(&s, cx, |d, _, cx| d.set_exclude(first, true, cx));
    assert_eq!(query_of(&s, cx), "back -#rust");
    with_dialog(&s, cx, |d, _, cx| d.remove_row(first, cx));
    assert_eq!(query_of(&s, cx), "back");
    assert_eq!(rows_of(&s, cx).len(), 1);
}

/// The type pick changes what a row means; a raw row can't be "not".
#[gpui_kit::test]
fn a_row_changes_kind_and_raw_rows_cannot_be_excluded(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "alpha", cx);
    let id = with_dialog(&s, cx, |d, _, _| d.row_id(0));
    with_dialog(&s, cx, |d, window, cx| d.pick_kind(id, "Tag", window, cx));
    assert_eq!(query_of(&s, cx), "#alpha");
    with_dialog(&s, cx, |d, window, cx| d.pick_kind(id, "Raw", window, cx));
    with_dialog(&s, cx, |d, _, cx| d.set_exclude(id, true, cx));
    assert_eq!(query_of(&s, cx), "alpha", "a raw row is written as it is");
    assert_eq!(
        rows_of(&s, cx)[0],
        (RowKind::Raw, false, "alpha".to_string())
    );
}

/// What no row says is kept: an unterminated quote and a bare prefix come
/// back out as they went in.
#[gpui_kit::test]
fn what_no_row_says_survives_apply(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "alpha # \"open", cx);
    assert_eq!(
        rows_of(&s, cx)
            .iter()
            .filter(|r| r.0 == RowKind::Raw)
            .count(),
        2
    );
    click(&s, CONFIRM_ID, cx);
    assert!(!builder_open(&s, cx));
    assert_eq!(find_query(&s, cx), "alpha # \"open");
}

/// Turning a row into a property row shows its comparison: the select reads
/// "=", not the "exists" a text row's select started on.
#[gpui_kit::test]
fn a_property_row_shows_its_comparison(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "alpha", cx);
    let id = with_dialog(&s, cx, |d, _, _| d.row_id(0));
    with_dialog(&s, cx, |d, window, cx| {
        d.pick_kind(id, "Property", window, cx)
    });
    let d = dialog(&s, cx);
    assert_eq!(cx.read(|cx| d.read(cx).op_shown(id, cx)), Some("="));
}

// ── Sort ─────────────────────────────────────────────────────────────────

/// The sort control is the query's own order directive.
#[gpui_kit::test]
fn the_sort_is_read_from_the_query_and_written_back(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "#rust -or:title", cx);
    let sort = with_dialog(&s, cx, |d, _, _| d.sort().cloned());
    assert_eq!(sort, Some((SortField::Title, SortOrder::Descending)));
    assert_eq!(rows_of(&s, cx).len(), 1, "the sort is not a row");
    click(&s, CONFIRM_ID, cx);
    assert_eq!(find_query(&s, cx), "#rust -or:title");
}

// ── Count ────────────────────────────────────────────────────────────────

/// The count is the number of notes the query matches, and follows an edit.
#[gpui_kit::test]
fn the_count_follows_the_rows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "#rust", cx);
    assert_eq!(counted(&s, &rt, cx), QueryCount::Matches(2));
    let id = with_dialog(&s, cx, |d, _, _| d.row_id(0));
    with_dialog(&s, cx, |d, _, cx| d.set_exclude(id, true, cx));
    assert_eq!(counted(&s, &rt, cx), QueryCount::Matches(1));
    with_dialog(&s, cx, |d, _, cx| d.remove_row(id, cx));
    assert_eq!(counted(&s, &rt, cx), QueryCount::Blank);
}

// ── Suggestions ──────────────────────────────────────────────────────────

/// A tag row suggests the vault's tags as it is typed in.
#[gpui_kit::test]
fn a_tag_row_suggests_the_vaults_tags(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "", cx);
    let id = with_dialog(&s, cx, |d, _, _| d.row_id(0));
    with_dialog(&s, cx, |d, window, cx| d.pick_kind(id, "Tag", window, cx));
    type_in(&s, "r", cx);
    let d = dialog(&s, cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| d.read(cx).suggested()) == ["rust".to_string()]
    });
    // The list floats over the dialog: it is shown, and taking it writes.
    assert!(s.shows(BUILDER_SUGGESTIONS_ID, cx), "the suggestions show");
    with_dialog(&s, cx, |d, window, cx| d.take_suggestion(0, window, cx));
    assert_eq!(query_of(&s, cx), "#rust");
    assert!(
        !s.shows(BUILDER_SUGGESTIONS_ID, cx),
        "a taken suggestion closes the list"
    );
}

/// A links row offers "current open note" first: it writes `{note}`, which
/// the query resolves against the open note, so the count is the backlinks of
/// alpha.
#[gpui_kit::test]
fn a_links_row_offers_the_open_note_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "", cx);
    let id = with_dialog(&s, cx, |d, _, _| d.row_id(0));
    with_dialog(&s, cx, |d, window, cx| {
        d.pick_kind(id, "Links to", window, cx)
    });
    // Focus the value field (the pick left it where it was).
    with_dialog(&s, cx, |d, window, cx| d.focus_first_value(window, cx));
    type_in(&s, "cu", cx);
    let d = dialog(&s, cx);
    assert_eq!(
        cx.read(|cx| d.read(cx).suggested())
            .first()
            .map(String::as_str),
        Some("{note}")
    );
    with_dialog(&s, cx, |d, window, cx| d.take_suggestion(0, window, cx));
    assert_eq!(query_of(&s, cx), "<{note}");
    // Alpha is open: its backlinks are beta.
    assert_eq!(counted(&s, &rt, cx), QueryCount::Matches(1));
}

// ── Apply, cancel and the browser ────────────────────────────────────────

/// Apply writes the built query into FIND and runs it; the dialog closes.
#[gpui_kit::test]
fn apply_runs_the_built_query_in_find(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "#rust", cx);
    with_dialog(&s, cx, |d, window, cx| d.add_row(window, cx));
    type_in(&s, "back", cx);
    click(&s, CONFIRM_ID, cx);
    s.settle_dialog(cx);
    assert!(!builder_open(&s, cx));
    assert_eq!(s.shown_modal(cx), None);
    assert_eq!(find_query(&s, cx), "back #rust");
}

/// Escape and Cancel change nothing.
#[gpui_kit::test]
fn escape_and_cancel_change_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    builder_on(&s, &rt, "#rust", cx);
    with_dialog(&s, cx, |d, window, cx| d.add_row(window, cx));
    type_in(&s, "back", cx);
    press(&s, "escape", cx);
    s.settle_dialog(cx);
    assert!(!builder_open(&s, cx));
    assert_eq!(find_query(&s, cx), "#rust");

    press(&s, "f7", cx);
    assert!(builder_open(&s, cx));
    click(&s, super::file_dialogs::CANCEL_ID, cx);
    s.settle_dialog(cx);
    assert!(!builder_open(&s, cx));
    assert_eq!(find_query(&s, cx), "#rust");
}

fn picker(s: &Started, cx: &TestAppContext) -> Entity<NotePicker> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .expect("the browser is open")
    })
}

/// From the search browser it stacks over it: Apply puts the query in the
/// browser's field, and Escape comes back to the browser.
#[gpui_kit::test]
fn from_the_search_browser_it_is_stacked_and_applies_there(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    press(&s, "ctrl-k", cx);
    type_in(&s, "#rust", cx);
    click(&s, BUILD_BUTTON_ID, cx);
    assert!(builder_open(&s, cx));
    assert_eq!(
        s.shown_modal(cx),
        Some(ModalKind::Search),
        "the browser keeps the slot"
    );
    assert_eq!(query_of(&s, cx), "#rust");

    // Escape: back on the browser, its query as it was.
    press(&s, "escape", cx);
    s.settle_dialog(cx);
    assert!(!builder_open(&s, cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Search));
    let p = picker(&s, cx);
    assert_eq!(cx.read(|cx| p.read(cx).query().to_string()), "#rust");

    // Again, and apply a change.
    click(&s, BUILD_BUTTON_ID, cx);
    with_dialog(&s, cx, |d, window, cx| d.add_row(window, cx));
    type_in(&s, "back", cx);
    click(&s, CONFIRM_ID, cx);
    s.settle_dialog(cx);
    assert!(!builder_open(&s, cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Search));
    let p = picker(&s, cx);
    assert_eq!(cx.read(|cx| p.read(cx).query().to_string()), "back #rust");
}
