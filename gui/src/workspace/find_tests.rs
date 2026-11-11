//! UI integration tests for the FIND and TAGS drawer views (M7b G5): a real
//! `Workspace` on a real, indexed temp vault, driven through the window's
//! keys and pointer.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`). FIND's typing debounce
//! and the preview's are GPUI timers: they fire only when a test advances
//! the test clock.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, Focusable as _, SharedString, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout::Side;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::nav::{DrawerView, LinksTab};
use kimun_notes::logic::query::QueryHit;
use kimun_notes::logic::views::TagEntry;
use tokio::runtime::Runtime;

use super::drawer_list::{DrawerList, chevron_id};
use super::find::{
    BLANK_TEXT, FIND_DEBOUNCE, FIND_ERROR_ID, FIND_HELP_ID, FIND_ID, FIND_INPUT_ID, FIND_MORE_ID,
    FIND_PREVIEW_HEADER_ID, FIND_SORT_ID, FindView, ROW_CAP, find_row_id,
};
use super::modal::ModalKind;
use super::picker::TO_FIND_BUTTON_ID;
use super::rail_cell_id;
use super::search::{PREVIEW_DEBOUNCE, SearchPreview};
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use super::start_tests::{copy_key, select_all_key};
use super::tags::{TAGS_FILTER_ID, TAGS_ID, TagsView, tag_row_id};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// A vault holding `notes`, started on `first`, driven until its text
/// shows and the index is built.
fn started_on(
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
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx)
    });
    (v, started)
}

/// `alpha` is linked from `beta` and `epsilon`; labels: `rust` ×3,
/// `go` ×1, `solo tag` (frontmatter, with a space) ×2.
const NOTES: &[(&str, &str)] = &[
    ("alpha.md", "# Alpha\nthe alpha note #rust, see [[beta]]"),
    (
        "beta.md",
        "# Beta\nintro\n\nback to [[alpha]] here #rust\n\nmore",
    ),
    ("epsilon.md", "# Epsilon\n[[alpha]] too #rust #go"),
    ("gamma.md", "+++\ntags = \"Solo Tag\"\n+++\n# Gamma\nplain"),
    ("delta.md", "+++\ntags = \"Solo Tag\"\n+++\n# Delta\nother"),
];

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

fn click(s: &Started, id: impl Into<SharedString>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn leader(s: &Started, keys: &str, cx: &mut TestAppContext) {
    press(s, "ctrl-shift-b", cx);
    type_in(s, keys, cx);
}

fn find(s: &Started, cx: &TestAppContext) -> Entity<FindView> {
    cx.read(|cx| s.workspace.read(cx).find.clone().expect("FIND is built"))
}

fn find_list(s: &Started, cx: &TestAppContext) -> Entity<DrawerList<QueryHit>> {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).list.clone())
}

fn find_preview(s: &Started, cx: &TestAppContext) -> Entity<SearchPreview> {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).preview.clone())
}

fn tags(s: &Started, cx: &TestAppContext) -> Entity<TagsView> {
    cx.read(|cx| s.workspace.read(cx).tags.clone().expect("TAGS is built"))
}

fn tags_list(s: &Started, cx: &TestAppContext) -> Entity<DrawerList<TagEntry>> {
    let view = tags(s, cx);
    cx.read(|cx| view.read(cx).list.clone())
}

fn runs(s: &Started, cx: &TestAppContext) -> usize {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).runs)
}

fn query(s: &Started, cx: &TestAppContext) -> String {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).query(cx))
}

/// FIND's rows once they have landed: (note name, context), in list order.
fn find_rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<(String, String)> {
    let list = find_list(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| list.read(cx).is_loading()));
    cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .map(|r| (r.entry.path.get_clean_name(), r.entry.context.clone()))
            .collect()
    })
}

fn find_names(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    find_rows(s, rt, cx).into_iter().map(|(n, _)| n).collect()
}

/// Replace FIND's query with `q` and let it run (the debounce out).
fn search(s: &Started, rt: &Runtime, q: &str, cx: &mut TestAppContext) {
    press(s, select_all_key(), cx);
    press(s, "backspace", cx);
    type_in(s, q, cx);
    cx.executor().advance_clock(FIND_DEBOUNCE);
    cx.run_until_parked();
    find_rows(s, rt, cx);
}

fn tag_rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<(String, usize)> {
    let list = tags_list(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| list.read(cx).is_loading()));
    cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .map(|r| (r.label.clone(), r.count))
            .collect()
    })
}

fn shown(s: &Started, view: DrawerView, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).is_shown(view))
}

fn on(s: &Started, p: &str, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).notes.as_ref()?.note.clone())
        .is_some_and(|n| n.is_like(&path(p)))
}

fn focused(s: &Started, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

fn find_list_focused(s: &Started, cx: &mut TestAppContext) -> bool {
    let list = find_list(s, cx);
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        list.read(cx).focus_handle(cx).is_focused(window)
    })
    .unwrap()
}

fn needles(s: &Started, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .editor
            .read(cx)
            .editor
            .session()
            .needles
            .clone()
    })
}

/// The preview's debounce out and its read landed.
fn settle_preview(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    let view = find_preview(s, cx);
    drive(rt, cx, |cx| {
        cx.executor().advance_clock(PREVIEW_DEBOUNCE);
        cx.run_until_parked();
        !cx.read(|cx| view.read(cx).is_pending())
    });
}

fn previewed(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    let view = find_preview(s, cx);
    cx.read(|cx| view.read(cx).shown())
        .and_then(|(p, current)| current.then_some(p))
}

/// The preview's state: (collapsed, context, full).
fn expand(s: &Started, cx: &TestAppContext) -> (bool, bool, bool) {
    let view = find(s, cx);
    cx.read(|cx| {
        let m = view.read(cx).preview_model();
        (m.is_collapsed(), m.is_context(), m.is_full())
    })
}

// ── FIND ─────────────────────────────────────────────────────────────────

/// FIND opens on the open note's backlinks ("Backlinks"); a typed query
/// runs and lists title, filename and the context paragraph.
#[gpui_kit::test]
fn find_runs_a_query_and_lists_rows_with_context(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    click(&started, rail_cell_id(DrawerView::Find), cx);
    assert!(started.shows(FIND_ID, cx));
    assert!(
        focused(&started, FIND_INPUT_ID, cx),
        "the query has the keyboard"
    );
    assert_eq!(query(&started, cx), "", "FIND starts empty, like the TUI's");
    search(&started, &rt, "<{note}", cx);
    assert_eq!(find_names(&started, &rt, cx), ["beta", "epsilon"]);
    let title = cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .find(super::find::FIND_TITLE_ID)
            .label()
            .map(str::to_string)
    });
    assert_eq!(title.unwrap().as_deref(), Some("Backlinks"));

    search(&started, &rt, "#rust", cx);
    let rows = find_rows(&started, &rt, cx);
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["alpha", "beta", "epsilon"],
        "by filename, no directive"
    );
    assert_eq!(
        rows[1].1, "back to [[alpha]] here #rust",
        "the paragraph with the match"
    );
}

/// `<{note}` follows the open note: another note opened reloads it; a
/// query without variables stays as listed.
#[gpui_kit::test]
fn backlinks_follow_the_open_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    // In the right slot, so FILES stays where it is.
    started.workspace.update(cx, |w, cx| {
        w.move_to_side(DrawerView::Find, Side::Right);
        w.refresh_views(cx);
    });
    assert!(shown(&started, DrawerView::Find, cx));
    open_find_on_backlinks(&started, cx);
    assert_eq!(find_names(&started, &rt, cx), ["beta", "epsilon"]);
    send(&started, AppMessage::open(path("beta.md")), cx);
    drive(&rt, cx, |cx| on(&started, "beta.md", cx));
    assert_eq!(find_names(&started, &rt, cx), ["alpha"], "beta's backlinks");

    let before = runs(&started, cx);
    let view = find(&started, cx);
    cx.update_window(started.handle, |_, window, cx| {
        view.update(cx, |v, cx| v.apply_query("#go".into(), None, window, cx))
    })
    .unwrap();
    assert_eq!(find_names(&started, &rt, cx), ["epsilon"]);
    assert_eq!(runs(&started, cx), before + 1);
    let list = find_list(&started, cx);
    send(&started, AppMessage::open(path("alpha.md")), cx);
    drive(&rt, cx, |cx| on(&started, "alpha.md", cx));
    assert!(!cx.read(|cx| list.read(cx).is_loading()), "no reload");
    assert_eq!(find_names(&started, &rt, cx), ["epsilon"]);
}

/// The sort chip reads the query's order directive (Name ascending when
/// it has none).
#[gpui_kit::test]
fn the_sort_chip_shows_the_directive(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    let chip = |cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(FIND_SORT_ID).label().map(str::to_string)
        })
        .unwrap()
        .unwrap_or_default()
    };
    let name_up = chip(cx);
    assert!(name_up.starts_with("Sorted by Name"), "{name_up}");
    search(&started, &rt, "#rust -or:title", cx);
    let title_down = chip(cx);
    assert!(title_down.starts_with("Sorted by Title"), "{title_down}");
    assert_ne!(
        title_down.trim_start_matches("Sorted by Title"),
        name_up.trim_start_matches("Sorted by Name"),
        "the other direction"
    );
    assert_eq!(find_names(&started, &rt, cx), ["epsilon", "beta", "alpha"]);
}

/// The preview grows Collapsed → Context → Full with Right, shrinks with
/// Left and cycles with Space while the results have the keyboard; it
/// shows the selected note (read only once expanded) and in Full the
/// arrows scroll it; the button cycles it too.
#[gpui_kit::test]
fn the_preview_cycles_and_scrolls(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let long: String = (0..200).map(|i| format!("line {i} #big\n")).collect();
    let mut notes = NOTES.to_vec();
    notes.push(("long.md", long.as_str()));
    let (_v, started) = started_on(cx, &rt, &notes, "alpha.md");
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#big", cx);
    press(&started, "down", cx);
    assert!(find_list_focused(&started, cx), "Down goes to the results");
    assert_eq!(expand(&started, cx), (true, false, false));
    let reads = |cx: &TestAppContext| {
        let view = find_preview(&started, cx);
        cx.read(|cx| view.read(cx).reads)
    };
    cx.executor().advance_clock(PREVIEW_DEBOUNCE * 3);
    cx.run_until_parked();
    assert_eq!(reads(cx), 0, "collapsed: nothing read");

    press(&started, "right", cx);
    assert_eq!(expand(&started, cx), (false, true, false));
    settle_preview(&started, &rt, cx);
    assert!(previewed(&started, cx).is_some_and(|p| p.is_like(&path("long.md"))));
    assert!(started.shows(super::find::FIND_PREVIEW_ID, cx));

    press(&started, "right", cx);
    assert_eq!(expand(&started, cx), (false, false, true));
    let preview = find_preview(&started, cx);
    let top = |cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.read(|cx| preview.read(cx).top_row())
    };
    let first = top(cx);
    press(&started, "down", cx);
    press(&started, "down", cx);
    assert_eq!(top(cx), first + 2, "Down scrolls the preview in Full");
    press(&started, "up", cx);
    assert_eq!(top(cx), first + 1);
    assert!(
        find_list_focused(&started, cx),
        "the list keeps the keyboard"
    );

    press(&started, "left", cx);
    assert_eq!(expand(&started, cx), (false, true, false));
    press(&started, "space", cx);
    assert_eq!(expand(&started, cx), (false, false, true));
    press(&started, "space", cx);
    assert_eq!(expand(&started, cx), (true, false, false));
    let list = find_list(&started, cx);
    let sel = cx.read(|cx| list.read(cx).selected_row().unwrap().entry.path.clone());
    click(&started, chevron_id(&find_row_id(&sel)), cx);
    assert_eq!(
        expand(&started, cx),
        (false, true, false),
        "the row's chevron grows it"
    );
}

fn drawn_find(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

/// The toolbar has no expand button: the chevron replaced it.
#[gpui_kit::test]
fn find_has_no_toolbar_expand_button(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#rust", cx);
    assert!(!drawn_find(&started, "find-expand", cx));
    assert!(drawn_find(&started, super::find::FIND_SAVE_ID, cx));
    assert!(drawn_find(&started, FIND_HELP_ID, cx));
}

/// The selected row shows the chevron, the others do not (at rest).
#[gpui_kit::test]
fn find_selected_row_shows_chevron(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#rust", cx);
    let list = find_list(&started, cx);
    let (sel, other) = cx.read(|cx| {
        let l = list.read(cx);
        let sel = l.selected_row().unwrap().entry.path.clone();
        let other = l
            .rows()
            .into_iter()
            .map(|r| r.entry.path.clone())
            .find(|p| *p != sel)
            .unwrap();
        (sel, other)
    });
    assert!(drawn_find(&started, chevron_id(&find_row_id(&sel)), cx));
    assert!(!drawn_find(&started, chevron_id(&find_row_id(&other)), cx));
}

/// A chevron click selects its row and grows the preview to Context; it
/// does not open the note, and the list has the keyboard.
#[gpui_kit::test]
fn find_chevron_click_opens_context_preview(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#rust", cx);
    let list = find_list(&started, cx);
    let sel = cx.read(|cx| list.read(cx).selected_row().unwrap().entry.path.clone());
    assert_eq!(expand(&started, cx), (true, false, false));
    click(&started, chevron_id(&find_row_id(&sel)), cx);
    assert_eq!(expand(&started, cx), (false, true, false));
    assert!(find_list_focused(&started, cx));
    assert!(on(&started, "alpha.md", cx), "the note was not opened");
}

/// In Context and Full the header names the note and its chevron steps
/// Context -> Full -> Context; Full keeps it visible and clickable.
#[gpui_kit::test]
fn find_header_chevron_steps_context_to_full_then_back(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#go", cx);
    assert!(!drawn_find(&started, FIND_PREVIEW_HEADER_ID, cx));
    press(&started, "down", cx);
    press(&started, "right", cx);
    assert_eq!(expand(&started, cx), (false, true, false));
    assert!(drawn_find(&started, FIND_PREVIEW_HEADER_ID, cx));
    let header_chevron = chevron_id(&SharedString::from(FIND_PREVIEW_HEADER_ID));
    click(&started, header_chevron.clone(), cx);
    assert_eq!(expand(&started, cx), (false, false, true));
    assert!(drawn_find(&started, FIND_PREVIEW_HEADER_ID, cx));
    assert!(find_list_focused(&started, cx));
    click(&started, header_chevron, cx);
    assert_eq!(expand(&started, cx), (false, true, false));
    assert!(find_list_focused(&started, cx));
}

/// Enter in the results opens the note with the query's matches
/// emphasised, saving the open note first; Up from the first row goes
/// back to the query.
#[gpui_kit::test]
fn enter_opens_with_emphasis(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_in(&started, "X", cx);
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#go", cx);
    press(&started, "enter", cx);
    assert!(find_list_focused(&started, cx), "Enter goes to the results");
    press(&started, "up", cx);
    assert!(
        focused(&started, FIND_INPUT_ID, cx),
        "Up from the top: the query"
    );
    press(&started, "down", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "epsilon.md", cx));
    assert_eq!(needles(&started, cx), vec!["#go".to_string()]);
    assert!(
        rt.block_on(v.vault.get_note_text(&path("alpha.md")))
            .is_ok_and(|t| t.starts_with('X')),
        "saved before the switch"
    );
}

/// A click on a row opens it, as Enter does (C1).
#[gpui_kit::test]
fn a_click_opens_a_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    let list = find_list(&started, cx);
    find_rows(&started, &rt, cx);
    let row = cx.read(|cx| {
        list.read(cx)
            .rows()
            .into_iter()
            .find(|r| r.entry.path.is_like(&path("epsilon.md")))
            .map(|r| find_row_id(&r.entry.path))
            .unwrap()
    });
    click(&started, row, cx);
    drive(&rt, cx, |cx| on(&started, "epsilon.md", cx));
}

/// A query core cannot read says why under the field.
#[gpui_kit::test]
fn an_invalid_query_shows_the_reason(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    assert!(!started.shows(FIND_ERROR_ID, cx));
    search(&started, &rt, "\"open", cx);
    assert!(started.shows(FIND_ERROR_ID, cx));
    type_in(&started, "\"", cx);
    assert!(!started.shows(FIND_ERROR_ID, cx));
}

/// Typing runs the query once it pauses: fast keystrokes run it once, for
/// the last text; the rows are never an older query's.
#[gpui_kit::test]
fn typing_is_debounced(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    find_rows(&started, &rt, cx);
    let before = runs(&started, cx);
    press(&started, select_all_key(), cx);
    press(&started, "backspace", cx);
    for key in ["#", "r", "u", "s", "t"] {
        type_in(&started, key, cx);
        cx.executor().advance_clock(FIND_DEBOUNCE / 3);
        cx.run_until_parked();
    }
    assert_eq!(runs(&started, cx), before, "nothing ran while typing");
    let view = find(&started, cx);
    assert!(cx.read(|cx| view.read(cx).is_waiting()));
    cx.executor().advance_clock(FIND_DEBOUNCE);
    cx.run_until_parked();
    assert_eq!(runs(&started, cx), before + 1, "one run for the pause");
    let list = find_list(&started, cx);
    assert_eq!(cx.read(|cx| list.read(cx).query().to_string()), "#rust");
    assert_eq!(find_names(&started, &rt, cx), ["alpha", "beta", "epsilon"]);

    // Enter runs a waiting query at once.
    press(&started, select_all_key(), cx);
    press(&started, "backspace", cx);
    type_in(&started, "#go", cx);
    press(&started, "enter", cx);
    assert_eq!(runs(&started, cx), before + 2);
    assert_eq!(find_names(&started, &rt, cx), ["epsilon"]);
    cx.executor().advance_clock(FIND_DEBOUNCE * 2);
    cx.run_until_parked();
    assert_eq!(
        runs(&started, cx),
        before + 2,
        "the pause after Enter runs nothing"
    );

    // A blank query lists nothing and says how to search.
    press(&started, "up", cx);
    press(&started, select_all_key(), cx);
    press(&started, "backspace", cx);
    cx.executor().advance_clock(FIND_DEBOUNCE);
    cx.run_until_parked();
    assert!(find_rows(&started, &rt, cx).is_empty());
    assert_eq!(
        cx.read(|cx| list.read(cx).empty_text().to_string()),
        BLANK_TEXT
    );
}

/// Hiding and revealing FIND keeps the query as typed, and its rows.
#[gpui_kit::test]
fn a_reveal_keeps_the_query(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#go", cx);
    type_in(&started, " #rust", cx);
    click(&started, rail_cell_id(DrawerView::Find), cx);
    assert!(!shown(&started, DrawerView::Find, cx));
    cx.executor().advance_clock(FIND_DEBOUNCE * 2);
    cx.run_until_parked();
    click(&started, rail_cell_id(DrawerView::Find), cx);
    assert!(shown(&started, DrawerView::Find, cx));
    assert_eq!(query(&started, cx), "#go #rust");
    assert_eq!(find_names(&started, &rt, cx), ["epsilon"]);
}

/// More results than the cap: the first ones are listed and the rest are
/// counted under the list.
#[gpui_kit::test]
fn a_broad_query_is_capped_with_a_footer(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let notes: Vec<(String, String)> = (0..ROW_CAP + 7)
        .map(|i| (format!("n{i:04}.md"), format!("note {i} #many")))
        .collect();
    let notes: Vec<(&str, &str)> = notes
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (_v, started) = started_on(cx, &rt, &notes, "n0000.md");
    open_find_on_backlinks(&started, cx);
    search(&started, &rt, "#many", cx);
    let rows = find_names(&started, &rt, cx);
    assert_eq!(rows.len(), ROW_CAP);
    assert_eq!(rows[0], "n0000");
    let list = find_list(&started, cx);
    assert!(cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .all(|r| r.entry.full_text.is_none())
    }));
    let footer = cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(FIND_MORE_ID).label().map(str::to_string)
    });
    assert_eq!(
        footer.unwrap().as_deref(),
        Some("7 more results — refine the query")
    );
    search(&started, &rt, "#many n0003", cx);
    assert!(!started.shows(FIND_MORE_ID, cx));
}

/// F1 and the `Syntax` button show the query syntax as the one modal;
/// Escape returns to FIND.
#[gpui_kit::test]
fn f1_shows_the_query_syntax(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    press(&started, "f1", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::QueryHelp));
    assert!(started.shows(super::search::QUERY_HELP_ID, cx));
    // Help to read: it keeps the kit's close button (no other dialog does).
    assert!(started.shows("close", cx));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, FIND_INPUT_ID, cx));
    click(&started, FIND_HELP_ID, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::QueryHelp));
}

/// The query syntax is the `QuerySyntax` action: rebound under
/// `[gui.keys]` its new chord opens it from FIND, and F1 there is the
/// keyboard reference again.
#[gpui_kit::test]
fn a_rebound_query_syntax_moves_off_f1(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    let overrides = [("QuerySyntax".to_string(), vec!["f9".to_string()])]
        .into_iter()
        .collect();
    let (keymap, errors) =
        crate::keymap::Keymap::load(crate::keymap::Platform::LinuxWindows, &overrides);
    assert!(errors.is_empty(), "{errors:?}");
    cx.update(|cx| crate::keymap::init(cx, keymap));
    open_find_on_backlinks(&started, cx);
    press(&started, "f1", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::KeyboardHelp));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    open_find_on_backlinks(&started, cx);
    press(&started, "f9", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::QueryHelp));
}

// ── TAGS ─────────────────────────────────────────────────────────────────

/// The query typed in the open search browser.
fn browser_query(started: &Started, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| {
        let picker = started.workspace.read(cx).picker.as_ref()?.upgrade()?;
        let query = picker.read(cx).input.read(cx).value().to_string();
        Some(query)
    })
}

/// TAGS lists every label with its count, most-used first; the filter
/// narrows it; Enter runs the selected tag in the search browser, quoted,
/// over the layout: FIND is not shown. The browser's `Open in FIND`
/// carries the query to FIND.
#[gpui_kit::test]
fn tags_list_counts_filter_and_run_in_find(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    leader(&started, "ft", cx);
    assert!(shown(&started, DrawerView::Tags, cx));
    assert!(started.shows(TAGS_ID, cx));
    assert!(focused(&started, TAGS_FILTER_ID, cx));
    assert_eq!(
        tag_rows(&started, &rt, cx),
        [
            ("rust".to_string(), 3),
            ("solo tag".to_string(), 2),
            ("go".to_string(), 1)
        ]
    );
    type_in(&started, "solo", cx);
    assert_eq!(tag_rows(&started, &rt, cx), [("solo tag".to_string(), 2)]);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    assert_eq!(
        browser_query(&started, cx).as_deref(),
        Some("#\"solo tag\"")
    );
    assert!(!shown(&started, DrawerView::Find, cx), "no view replaced");
    assert!(shown(&started, DrawerView::Tags, cx));

    // Escape closes it: TAGS is as it was.
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(shown(&started, DrawerView::Tags, cx));

    // A click on a row runs it too (C1); Escape clears, then leaves.
    leader(&started, "ft", cx);
    press(&started, "escape", cx);
    assert_eq!(tag_rows(&started, &rt, cx).len(), 3, "the filter cleared");
    click(&started, tag_row_id("go"), cx);
    started.settle_dialog(cx);
    assert_eq!(browser_query(&started, cx).as_deref(), Some("#go"));

    // `Open in FIND`: the browser closes, FIND shows the query.
    click(&started, TO_FIND_BUTTON_ID, cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(shown(&started, DrawerView::Find, cx));
    assert_eq!(query(&started, cx), "#go");
    assert_eq!(find_names(&started, &rt, cx), ["epsilon"]);
}

/// A double-click on a tag opens the browser once and opens no note: its
/// second press lands on the browser's modal, not under it.
#[gpui_kit::test]
fn a_double_click_on_a_tag_runs_it_and_opens_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    leader(&started, "ft", cx);
    assert_eq!(tag_rows(&started, &rt, cx).len(), 3);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(tag_row_id("rust"), cx)
    })
    .unwrap();
    cx.run_until_parked();
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    assert_eq!(browser_query(&started, cx).as_deref(), Some("#rust"));
    drive(&rt, cx, |_| true);
    assert!(on(&started, "alpha.md", cx), "no result opened");
}

/// The channel's `RunTagQuery` opens the browser on the label too.
#[gpui_kit::test]
fn run_tag_query_opens_the_browser(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    send(&started, AppMessage::RunTagQuery("Solo Tag".into()), cx);
    cx.run_until_parked();
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    assert_eq!(
        browser_query(&started, cx).as_deref(),
        Some("#\"Solo Tag\"")
    );
    assert!(!shown(&started, DrawerView::Find, cx));
}

/// The browser's `OpenQueryInSidebar` chord does what its button does: the
/// browser closes and FIND shows the typed query. With nothing typed it
/// says so and keeps the browser.
#[gpui_kit::test]
fn the_browser_chord_opens_its_query_in_find(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    press(&started, "ctrl-k", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    press(&started, "ctrl-shift-enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search), "blank");
    assert!(!shown(&started, DrawerView::Find, cx));

    type_in(&started, "#rust", cx);
    press(&started, "ctrl-shift-enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(shown(&started, DrawerView::Find, cx));
    assert_eq!(query(&started, cx), "#rust");
}

/// FIND on the open note's backlinks query, as the old leader `f b` did
/// (that chord now shows LINKS' Backlinks tab instead).
fn open_find_on_backlinks(started: &Started, cx: &mut TestAppContext) {
    cx.update_window(started.handle, |_, window, cx| {
        started.workspace.update(cx, |w, cx| {
            w.open_find_with_query("<{note}".to_string(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

// ── Both views ───────────────────────────────────────────────────────────

/// Leader `f b` shows LINKS on its Backlinks tab (not FIND on a query), `f t`
/// shows TAGS; FIND and TAGS work in the right slot and split under FILES.
#[gpui_kit::test]
fn the_leader_and_either_side(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    let links_tab = |cx: &TestAppContext| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.links.as_ref().map(|l| l.read(cx).tab())
        })
    };
    // From another tab: `f b` brings LINKS back to Backlinks.
    leader(&started, "lo", cx);
    assert_eq!(links_tab(cx), Some(LinksTab::Outgoing));
    leader(&started, "fb", cx);
    assert_eq!(links_tab(cx), Some(LinksTab::Backlinks));
    assert!(shown(&started, DrawerView::Links, cx), "LINKS shows");
    assert!(
        !shown(&started, DrawerView::Find, cx),
        "no FIND query is opened for backlinks"
    );
    assert_eq!(runs(&started, cx), 0, "FIND ran nothing");
    // FIND on a query, then to the right slot (the focused view moves).
    cx.update_window(started.handle, |_, window, cx| {
        started.workspace.update(cx, |w, cx| {
            w.open_find_with_query("<{note}".to_string(), window, cx)
        })
    })
    .unwrap();
    assert!(shown(&started, DrawerView::Find, cx));
    assert_eq!(query(&started, cx), "<{note}");
    assert_eq!(find_names(&started, &rt, cx), ["beta", "epsilon"]);
    assert_eq!(runs(&started, cx), 1, "the query ran once");
    // FIND to the right slot (the focused view moves).
    press(&started, "alt-shift-right", cx);
    let side = |view: DrawerView, cx: &TestAppContext| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.layout.view_slot(&view).map(|s| w.slot_side(s))
        })
    };
    assert_eq!(side(DrawerView::Find, cx), Some(Side::Right));
    assert!(shown(&started, DrawerView::Find, cx));
    leader(&started, "ft", cx);
    assert!(shown(&started, DrawerView::Tags, cx));
    assert!(!tag_rows(&started, &rt, cx).is_empty());
    // TAGS split under FILES on the left.
    cx.update_window(started.handle, |_, window, cx| {
        started.workspace.update(cx, |w, cx| {
            w.open_file_browser(window, cx);
            let left = w.focused_slot().expect("FILES has the keyboard");
            let low = w
                .layout
                .split_slot(left, kimun_notes::logic::layout_tree::Axis::Vertical)
                .expect("a split");
            w.layout.move_view(DrawerView::Tags, low);
            w.refresh_views(cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    let slots = cx.read(|cx| started.workspace.read(cx).shown_on(Side::Left));
    assert!(
        slots.contains(&DrawerView::Files) && slots.contains(&DrawerView::Tags),
        "{slots:?}"
    );
    assert!(started.shows(TAGS_ID, cx));
    assert!(
        shown(&started, DrawerView::Find, cx),
        "FIND still on the right"
    );
    assert!(started.shows(FIND_ID, cx));
}

/// Hidden, FIND runs nothing and TAGS reads nothing, through opens,
/// typing and file operations; shown, they catch up.
#[gpui_kit::test]
fn hidden_find_and_tags_do_no_work(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_in(&started, "x #new ", cx);
    send(&started, AppMessage::open(path("beta.md")), cx);
    drive(&rt, cx, |cx| on(&started, "beta.md", cx));
    send(
        &started,
        AppMessage::IndexingDone(Ok(Default::default())),
        cx,
    );
    assert_eq!(runs(&started, cx), 0);
    let list = find_list(&started, cx);
    assert!(!cx.read(|cx| list.read(cx).is_started()));
    let list = tags_list(&started, cx);
    assert!(!cx.read(|cx| list.read(cx).is_started()));
    click(&started, rail_cell_id(DrawerView::Find), cx);
    assert_eq!(runs(&started, cx), 1);
    assert!(
        find_names(&started, &rt, cx).is_empty(),
        "blank query: no rows"
    );
    click(&started, rail_cell_id(DrawerView::Tags), cx);
    assert!(!tag_rows(&started, &rt, cx).is_empty());
}

/// The copy chord copies the selected result's path from FIND's list, and
/// leaves the query field's copy alone.
#[gpui_kit::test]
fn the_copy_chord_in_find(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    open_find_on_backlinks(&started, cx);
    find_rows(&started, &rt, cx);
    press(&started, select_all_key(), cx);
    press(&started, copy_key(), cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("<{note}"), "the field's own copy");
    press(&started, "down", cx);
    press(&started, "ctrl-c", cx);
    let list = find_list(&started, cx);
    let selected = cx.read(|cx| {
        list.read(cx)
            .selected_row()
            .map(|r| r.entry.path.to_string())
    });
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, selected);
    assert_eq!(started.flash(cx).as_deref(), Some("path copied"));
}

// ── performance ──────────────────────────────────────────────────────────

/// A 5 000-note vault: how long FIND's broad and narrow queries take end to
/// end through the window, and how many rows it holds. Prints the timings;
/// run with `cargo nextest run -p kimun-gui --run-ignored only find_large`.
#[ignore = "a timing probe over a 5k-note vault (slow)"]
#[gpui_kit::test]
fn find_large_vault_timings(cx: &mut TestAppContext) {
    use std::time::Instant;
    let rt = runtime();
    let _guard = rt.enter();
    let notes: Vec<(String, String)> = (0..5000)
        .map(|i| {
            let tag = if i % 10 == 0 { " #ten" } else { "" };
            (
                format!("d{}/note{i}.md", i % 50),
                format!(
                    "# Note {i}\n\nSome body text about topic {}{tag}\n\n{}",
                    i % 97,
                    "filler words ".repeat(400)
                ),
            )
        })
        .collect();
    let notes: Vec<(&str, &str)> = notes
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let built = Instant::now();
    let (v, started) = started_on(cx, &rt, &notes, "d0/note0.md");
    eprintln!(
        "vault of {} notes built and indexed in {:?}",
        notes.len(),
        built.elapsed()
    );
    // The TUI's loader, for comparison: every hit read and kept.
    for q in ["body", "#ten"] {
        let t = Instant::now();
        let rows = rt.block_on(kimun_notes::logic::query::load_query(&v.vault, q));
        let bytes: usize = rows
            .iter()
            .map(|r| r.full_text.as_ref().map_or(0, String::len) + r.context.len())
            .sum();
        eprintln!(
            "TUI load_query {q:?}: {} rows in {:?}, {bytes} bytes held",
            rows.len(),
            t.elapsed()
        );
        let t = Instant::now();
        let page = rt.block_on(kimun_notes::logic::query::load_query_page(
            &v.vault,
            q,
            super::find::ROW_CAP,
            super::find::CONTEXT_CHARS,
        ));
        eprintln!(
            "load_query_page {q:?}: {} of {} rows in {:?}",
            page.entries.len(),
            page.total,
            t.elapsed()
        );
    }
    open_find_on_backlinks(&started, cx);
    find_rows(&started, &rt, cx);
    for q in ["body", "#ten", "topic 42", "=note4999"] {
        let t = Instant::now();
        search(&started, &rt, q, cx);
        let list = find_list(&started, cx);
        let (n, total, bytes) = cx.read(|cx| {
            let list = list.read(cx);
            let rows = list.rows();
            let bytes: usize = rows
                .iter()
                .map(|r| r.entry.context.len() + r.entry.title.len() + r.entry.filename.len())
                .sum();
            (rows.len(), rows.first().map_or(0, |r| r.total), bytes)
        });
        eprintln!(
            "query {q:?}: {n} rows of {total} in {:?}, {bytes} bytes of row text",
            t.elapsed()
        );
    }
    let t = Instant::now();
    for _ in 0..50 {
        press(&started, "down", cx);
    }
    eprintln!("50 Down presses in {:?}", t.elapsed());
}

// ── autocomplete in the field ────────────────────────────────────────────

fn suggested(s: &Started, cx: &TestAppContext) -> Vec<String> {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).suggested())
}

/// Clear FIND's field and type `text`; wait for the suggestions to list.
fn type_and_suggest(s: &Started, rt: &Runtime, text: &str, cx: &mut TestAppContext) {
    open_find_on_backlinks(s, cx);
    press(s, select_all_key(), cx);
    press(s, "backspace", cx);
    type_in(s, text, cx);
    drive(rt, cx, |cx| !suggested(s, cx).is_empty());
}

#[gpui_kit::test]
fn a_hash_in_the_field_lists_the_vault_labels(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "#r", cx);
    assert_eq!(suggested(&started, cx), ["rust"]);
    // Drawn under the field, as a listbox.
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(super::input_autocomplete::POPUP_ID).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_completes_the_label_and_the_query_runs(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "#r", cx);
    press(&started, "tab", cx);
    assert_eq!(query(&started, cx), "#rust");
    assert!(suggested(&started, cx).is_empty(), "closed, not reopened");
    cx.executor().advance_clock(FIND_DEBOUNCE);
    cx.run_until_parked();
    assert_eq!(find_names(&started, &rt, cx), ["alpha", "beta", "epsilon"]);
}

#[gpui_kit::test]
fn down_then_enter_takes_the_second_suggestion_not_the_list(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "#", cx);
    let second = suggested(&started, cx)[1].clone();
    press(&started, "down", cx);
    let view = find(&started, cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).suggestion_highlighted()),
        Some(1)
    );
    assert!(!find_list_focused(&started, cx), "Down moved the highlight");
    press(&started, "enter", cx);
    assert!(
        query(&started, cx).contains(&second),
        "{} vs {second}",
        query(&started, cx)
    );
    assert!(suggested(&started, cx).is_empty());
}

#[gpui_kit::test]
fn escape_closes_the_suggestions_before_it_dismisses_find(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "#r", cx);
    press(&started, "escape", cx);
    assert!(suggested(&started, cx).is_empty());
    assert_eq!(query(&started, cx), "#r", "the field is untouched");
    assert!(shown(&started, DrawerView::Find, cx), "FIND stays up");
}

#[gpui_kit::test]
fn a_press_on_a_suggestion_takes_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "#r", cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click((SharedString::from("rust"), 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(query(&started, cx), "#rust");
    assert!(suggested(&started, cx).is_empty());
}

#[gpui_kit::test]
fn a_question_mark_completes_a_saved_search_and_names_find_for_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    rt.block_on(v.vault.save_search("rusty", "#rust")).unwrap();
    type_and_suggest(&started, &rt, "?ru", cx);
    assert_eq!(suggested(&started, cx), ["rusty"]);
    press(&started, "tab", cx);
    assert_eq!(
        query(&started, cx),
        "#rust",
        "the stored query fills the field"
    );
    let view = find(&started, cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).saved_name()).as_deref(),
        Some("rusty")
    );
}

#[gpui_kit::test]
fn a_link_filter_operator_completes_note_names(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "<be", cx);
    assert_eq!(suggested(&started, cx), ["beta"]);
    press(&started, "tab", cx);
    assert_eq!(query(&started, cx), "<beta");
}

#[gpui_kit::test]
fn moving_the_caret_out_of_the_trigger_closes_the_suggestions(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "word #r", cx);
    press(&started, "home", cx);
    cx.update_window(started.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(suggested(&started, cx).is_empty());
}

#[gpui_kit::test]
fn a_saved_search_typed_in_full_still_offers_its_stored_query(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    rt.block_on(v.vault.save_search("rusty", "#rust")).unwrap();
    // The one match is exactly what was typed, yet accepting it expands.
    type_and_suggest(&started, &rt, "?rusty", cx);
    assert_eq!(suggested(&started, cx), ["rusty"]);
    press(&started, "tab", cx);
    assert_eq!(query(&started, cx), "#rust");
    let view = find(&started, cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).saved_name()).as_deref(),
        Some("rusty")
    );
}

#[gpui_kit::test]
fn replacing_the_query_from_outside_closes_the_suggestions(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, "alpha.md");
    type_and_suggest(&started, &rt, "#r", cx);
    let view = find(&started, cx);
    cx.update_window(started.handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.apply_query("#rust #be".into(), None, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(started.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(
        suggested(&started, cx).is_empty(),
        "the popup was for the old text"
    );
    // Enter commits the query; it does not accept a stale suggestion.
    assert_eq!(query(&started, cx), "#rust #be");
}
