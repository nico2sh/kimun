//! UI integration tests for the search browser (M7b G4): the picker over
//! the vault's query search with its preview, on a real `Workspace` over a
//! real, indexed temp vault, driven through the window's keys.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`). The preview's debounce
//! is a GPUI timer: it fires only when a test advances the test clock.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::message::AppMessage;
use tokio::runtime::Runtime;

use super::modal::ModalKind;
use super::picker::{HELP_BUTTON_ID, NotePicker, PICKER_ERROR_ID, PICKER_INPUT_ID, QUERY_ERROR_ID};
use super::search::{
    PREVIEW_BYTES, PREVIEW_DEBOUNCE, PREVIEW_TRUNCATED_ID, QUERY_HELP_ID, SearchPreview, cap_text,
    preview_text,
};
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

fn disk(rt: &Runtime, v: &TestVault, p: &str) -> Option<String> {
    rt.block_on(v.vault.get_note_text(&path(p))).ok()
}

fn exists(rt: &Runtime, v: &TestVault, p: &str) -> bool {
    rt.block_on(v.vault.exists(&path(p)))
}

/// A vault holding `notes` with `history` (newest first), started on
/// `first`, driven until its text shows and the index is built.
fn started_on(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    history: &[&str],
    first: &str,
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(notes, history));
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

fn picker(s: &Started, cx: &TestAppContext) -> Entity<NotePicker> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .expect("a picker is open")
    })
}

fn preview(s: &Started, cx: &TestAppContext) -> Entity<SearchPreview> {
    let view = picker(s, cx);
    cx.read(|cx| view.read(cx).preview().expect("the search browser"))
}

/// The browser's rows once the current query's have landed: each note's
/// path, `Create: <path>` for the create row.
fn rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let view = picker(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| view.read(cx).is_loading()));
    cx.read(|cx| {
        view.read(cx)
            .rows()
            .iter()
            .map(|row| match row {
                FileListEntry::CreateNote { path, .. } => format!("Create: {path}"),
                other => other.path().to_string(),
            })
            .collect()
    })
}

/// The note rows only, sorted (search results come in core's order).
fn notes_found(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let mut found: Vec<String> = rows(s, rt, cx)
        .into_iter()
        .filter(|r| !r.starts_with("Create:"))
        .map(|r| path(&r).get_clean_name())
        .collect();
    found.sort();
    found
}

fn selected(s: &Started, cx: &TestAppContext) -> Option<FileListEntry> {
    let view = picker(s, cx);
    cx.read(|cx| view.read(cx).selected())
}

fn selected_path(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    selected(s, cx).map(|r| r.path().clone())
}

/// Replace the query with `query` and wait for its rows.
fn search(s: &Started, rt: &Runtime, query: &str, cx: &mut TestAppContext) {
    press(s, "ctrl-a", cx);
    press(s, "backspace", cx);
    type_in(s, query, cx);
    rows(s, rt, cx);
}

/// Let the preview's debounce run out and its read land.
fn settle_preview(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    let view = preview(s, cx);
    drive(rt, cx, |cx| {
        cx.executor().advance_clock(PREVIEW_DEBOUNCE);
        cx.run_until_parked();
        !cx.read(|cx| view.read(cx).is_pending())
    });
}

/// The note the preview shows, if it is the selected one's.
fn previewed(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    let view = preview(s, cx);
    cx.read(|cx| view.read(cx).shown())
        .and_then(|(p, current)| current.then_some(p))
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

fn dirty(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).editor.read(cx).editor.is_dirty())
}

fn query_help(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).query_help)
}

const NOTES: &[(&str, &str)] = &[
    ("a.md", "alpha [[b]] #rust"),
    ("b.md", "+++\nstatus = \"done\"\n+++\nbeta body"),
    ("c.md", "charlie links [[a]]"),
    ("d.md", "delta"),
];

// ── opening, rows, searching ─────────────────────────────────────────────

/// Ctrl+K opens the browser with its query focused, listing the recent
/// notes newest first; its own chord closes it and the editor has the
/// keyboard back.
#[gpui_kit::test]
fn the_chord_opens_on_the_recent_notes_and_toggles(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, NOTES, &["d.md", "c.md"], "a.md");
    press(&started, "ctrl-k", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    assert!(focused(&started, PICKER_INPUT_ID, cx));
    let want = v.history();
    assert!(want.len() >= 3, "{want:?}");
    let got: Vec<VaultPath> = rows(&started, &rt, cx).iter().map(|r| path(r)).collect();
    assert_eq!(got.len(), want.len(), "{got:?}");
    assert!(
        got.iter().zip(&want).all(|(g, w)| g.is_like(w)),
        "recent notes, newest first: {got:?} vs {want:?}"
    );
    assert!(got[0].is_like(&path("a.md")), "the open note is newest");

    press(&started, "ctrl-k", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
}

/// Typing runs core's query search: labels, properties, backlinks and
/// forward links in both spellings, and the bare `<` against the open note.
#[gpui_kit::test]
fn typing_runs_the_query_search(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    press(&started, "ctrl-k", cx);
    for (query, want) in [
        ("#rust", vec!["a"]),
        ("prop:status=done", vec!["b"]),
        ("%status=done", vec!["b"]),
        ("<a", vec!["c"]),
        ("lk:a", vec!["c"]),
        (">a", vec!["b"]),
        ("fwd:a", vec!["b"]),
        ("<", vec!["c"]),
        ("delta", vec!["d"]),
        ("nothing-like-it", vec![]),
    ] {
        search(&started, &rt, query, cx);
        assert_eq!(notes_found(&started, &rt, cx), want, "{query}");
    }
}

/// A typed query leads with `Create:` (in the open note's folder) but the
/// selection starts on the best result; Up then Enter makes the note and
/// opens it.
#[gpui_kit::test]
fn a_typed_query_offers_create_below_the_selection(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("w/a.md", "alpha"), ("b.md", "beta")],
        &[],
        "w/a.md",
    );
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "beta", cx);
    let rows = rows(&started, &rt, cx);
    let create = rows[0].strip_prefix("Create: ").expect("Create: leads");
    assert!(path(create).is_like(&path("w/beta.md")), "{rows:?}");
    assert!(selected_path(&started, cx).is_some_and(|p| p.is_like(&path("b.md"))));

    press(&started, "up", cx);
    assert!(matches!(
        selected(&started, cx),
        Some(FileListEntry::CreateNote { .. })
    ));
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "w/beta.md", cx));
    assert_eq!(disk(&rt, &v, "w/beta.md").as_deref(), Some(""));
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);

    // Nothing matches: `Create:` is all there is, and selected.
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "zebra", cx);
    assert!(matches!(
        selected(&started, cx),
        Some(FileListEntry::CreateNote { .. })
    ));
}

/// An unterminated quote says so under the results.
#[gpui_kit::test]
fn an_invalid_query_says_why(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    press(&started, "ctrl-k", cx);
    assert!(!started.shows(QUERY_ERROR_ID, cx));
    search(&started, &rt, "\"alpha", cx);
    assert!(started.shows(QUERY_ERROR_ID, cx));
    let label = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(QUERY_ERROR_ID).label().map(str::to_string)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Unterminated quote in the query"));
    type_in(&started, "\"", cx);
    assert!(!started.shows(QUERY_ERROR_ID, cx));
}

// ── the preview ──────────────────────────────────────────────────────────

/// The preview shows the selected note with the query's needles marked,
/// and follows the selection: arrowing reads nothing until the selection
/// holds for the debounce, and then only the note selected last shows.
#[gpui_kit::test]
fn the_preview_follows_the_selection_debounced(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let notes: Vec<(String, String)> = (0..8)
        .map(|i| (format!("n{i}.md"), format!("note {i}\nshared word here")))
        .collect();
    let notes: Vec<(&str, &str)> = notes
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (_v, started) = started_on(cx, &rt, &notes, &[], "n0.md");
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "shared", cx);
    settle_preview(&started, &rt, cx);
    let first = selected_path(&started, cx).unwrap();
    assert_eq!(previewed(&started, cx), Some(first));
    let view = preview(&started, cx);
    let lines = cx.read(|cx| view.read(cx).shown_lines());
    assert_eq!(lines[1].1, vec!["shared".to_string()], "{lines:?}");
    let reads = cx.read(|cx| view.read(cx).reads);

    for _ in 0..5 {
        press(&started, "down", cx);
    }
    assert_eq!(
        cx.read(|cx| view.read(cx).reads),
        reads,
        "no read while arrowing"
    );
    assert!(cx.read(|cx| view.read(cx).is_pending()));
    assert_eq!(
        previewed(&started, cx),
        None,
        "the old note is not the current one"
    );

    settle_preview(&started, &rt, cx);
    let last = selected_path(&started, cx).unwrap();
    assert_eq!(previewed(&started, cx), Some(last));
    assert_eq!(
        cx.read(|cx| view.read(cx).reads),
        reads + 1,
        "one read, the last"
    );

    // `Create:` shows nothing.
    for _ in 0..8 {
        press(&started, "up", cx);
    }
    assert!(matches!(
        selected(&started, cx),
        Some(FileListEntry::CreateNote { .. })
    ));
    settle_preview(&started, &rt, cx);
    assert_eq!(cx.read(|cx| view.read(cx).shown()), None);
}

/// The preview's lines are laid out off the UI thread: cut on a character
/// boundary at the cap, matched case-insensitively, the first match found.
#[test]
fn preview_lines_are_capped_and_matched() {
    let (text, cut) = cap_text("é".repeat(PREVIEW_BYTES), PREVIEW_BYTES - 1);
    assert!(cut && text.len() == PREVIEW_BYTES - 2, "{}", text.len());
    let (text, cut) = cap_text("short".into(), PREVIEW_BYTES);
    assert!(!cut && text == "short");

    let shown = preview_text(
        path("x.md"),
        "title\na Big\tdeal big".into(),
        &["big".into()],
    );
    assert_eq!(
        shown.lines_and_hits(),
        vec![
            ("title".to_string(), vec![]),
            (
                "a Big    deal big".to_string(),
                vec!["Big".to_string(), "big".to_string()]
            ),
        ]
    );
    assert_eq!(shown.summary(), (2, Some(1), false));
}

/// A note over the cap is cut and the preview says so.
#[gpui_kit::test]
fn a_huge_note_is_cut_and_says_so(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let huge = "line of text\n".repeat(PREVIEW_BYTES / 10);
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha"), ("huge.md", &huge)],
        &[],
        "a.md",
    );
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "=huge", cx);
    settle_preview(&started, &rt, cx);
    assert!(previewed(&started, cx).is_some_and(|p| p.is_like(&path("huge.md"))));
    assert!(started.shows(PREVIEW_TRUNCATED_ID, cx));
}

// ── opening a result ─────────────────────────────────────────────────────

/// Enter opens the selected note with the query's needles emphasised in the
/// editor; the first edit clears them.
#[gpui_kit::test]
fn enter_opens_with_the_needles_until_an_edit(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "Delta", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "d.md", cx));
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
    assert_eq!(needles(&started, cx), vec!["delta".to_string()]);

    type_in(&started, "X", cx);
    cx.update_window(started.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(needles(&started, cx).is_empty(), "an edit clears them");

    // A plain open carries none.
    send(&started, AppMessage::open(path("c.md")), cx);
    drive(&rt, cx, |cx| on(&started, "c.md", cx));
    assert!(needles(&started, cx).is_empty());
}

/// With unsaved edits the browser waits while they are written, then
/// opens the result.
#[gpui_kit::test]
fn the_open_note_is_saved_before_a_result_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    type_in(&started, "X", cx);
    assert!(dirty(&started, cx));
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "delta", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "d.md", cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha [[b]] #rust"));
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

/// A save that fails keeps the browser open saying why; nothing opens and
/// the note keeps its edits.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_save_keeps_the_browser_open(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    let file = v.workspace.as_path().join("a.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    type_in(&started, "X", cx);
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "delta", cx);
    press(&started, "enter", cx);
    let view = picker(&started, cx);
    drive(&rt, cx, |cx| cx.read(|cx| view.read(cx).error().is_some()));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = cx
        .read(|cx| view.read(cx).error().map(str::to_string))
        .unwrap();
    assert!(
        error.starts_with("Couldn't save a.md:") && error.ends_with("Nothing was opened."),
        "{error}"
    );
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    assert!(started.shows(PICKER_ERROR_ID, cx));
    assert!(on(&started, "a.md", cx));
    assert!(dirty(&started, cx));
    assert!(
        !cx.read(|cx| view.read(cx).is_busy()),
        "the browser takes keys again"
    );
}

/// Escape closes the browser and the editor has the keyboard back.
#[gpui_kit::test]
fn escape_closes_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    press(&started, "ctrl-k", cx);
    type_in(&started, "zz", cx);
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
    assert_eq!(
        started.text(cx),
        "alpha [[b]] #rust",
        "nothing typed into the note"
    );
    assert!(!exists(&rt, &_v, "zz.md"));
}

// ── query syntax, doors, one modal ───────────────────────────────────────

/// F1 shows the query syntax over the browser and Escape goes back to the
/// browser's query; the `?` button shows it too; the browser's chord closes
/// both.
#[gpui_kit::test]
fn f1_shows_the_syntax_over_the_browser(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    press(&started, "ctrl-k", cx);
    type_in(&started, "alp", cx);
    press(&started, "f1", cx);
    assert!(query_help(&started, cx));
    assert!(started.shows(QUERY_HELP_ID, cx));
    press(&started, "f1", cx);
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert!(!query_help(&started, cx));
    assert!(!started.shows(QUERY_HELP_ID, cx));
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    assert!(focused(&started, PICKER_INPUT_ID, cx));
    let view = picker(&started, cx);
    assert_eq!(cx.read(|cx| view.read(cx).query().to_string()), "alp");

    click(&started, HELP_BUTTON_ID, cx);
    assert!(started.shows(QUERY_HELP_ID, cx));
    press(&started, "ctrl-k", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(!query_help(&started, cx));
    assert!(focused(&started, "editor", cx));
}

/// Following a `#tag` opens the browser on that label.
#[gpui_kit::test]
fn following_a_label_searches_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "#rust"), ("b.md", "x #rust")],
        &[],
        "a.md",
    );
    press(&started, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::Search)
    });
    let view = picker(&started, cx);
    assert_eq!(cx.read(|cx| view.read(cx).query().to_string()), "#rust");
    assert_eq!(notes_found(&started, &rt, cx), vec!["a", "b"]);
    assert_eq!(
        cx.read(|cx| view.read(cx).needles()),
        vec!["#rust".to_string()]
    );
    // The caret waits after the label: typing narrows it.
    type_in(&started, " x", cx);
    assert_eq!(cx.read(|cx| view.read(cx).query().to_string()), "#rust x");
}

/// One modal at a time: over the browser the finder and palette chords do
/// nothing, and the browser's chord does nothing over another modal.
/// Leader `f g` and `f r` and the palette open it.
#[gpui_kit::test]
fn one_modal_at_a_time_and_its_doors(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    press(&started, "ctrl-k", cx);
    for chord in ["ctrl-o", "ctrl-shift-p", "f2", "ctrl-shift-w"] {
        press(&started, chord, cx);
        assert_eq!(started.shown_modal(cx), Some(ModalKind::Search), "{chord}");
    }
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    press(&started, "ctrl-o", cx);
    press(&started, "ctrl-k", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
    press(&started, "escape", cx);
    started.settle_dialog(cx);

    for keys in ["fg", "fr"] {
        press(&started, "ctrl-shift-b", cx);
        type_in(&started, keys, cx);
        assert_eq!(started.shown_modal(cx), Some(ModalKind::Search), "{keys}");
        press(&started, "escape", cx);
        started.settle_dialog(cx);
    }

    press(&started, "ctrl-shift-p", cx);
    type_in(&started, "find grep", cx);
    press(&started, "enter", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
}

// ── performance ──────────────────────────────────────────────────────────

/// A 5 000-note vault: how long the empty query, a broad and a narrow
/// search and a preview take, end to end through the window (tokio,
/// GPUI, the list). Prints the timings; run with
/// `cargo nextest run -p kimun-gui --run-ignored only large_vault`.
#[ignore = "a timing probe over a 5k-note vault (slow)"]
#[gpui_kit::test]
fn large_vault_timings(cx: &mut TestAppContext) {
    use std::time::Instant;
    let rt = runtime();
    let _guard = rt.enter();
    let notes: Vec<(String, String)> = (0..5000)
        .map(|i| {
            let tag = if i % 10 == 0 { " #ten" } else { "" };
            (
                format!("d{}/note{i}.md", i % 50),
                format!(
                    "# Note {i}\n\nSome body text about topic {}{tag}\n{}",
                    i % 97,
                    "filler words ".repeat(40)
                ),
            )
        })
        .collect();
    let notes: Vec<(&str, &str)> = notes
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let built = Instant::now();
    let (_v, started) = started_on(cx, &rt, &notes, &[], "d0/note0.md");
    eprintln!(
        "vault of {} notes built and indexed in {:?}",
        notes.len(),
        built.elapsed()
    );

    let t = Instant::now();
    press(&started, "ctrl-k", cx);
    let n = rows(&started, &rt, cx).len();
    eprintln!("open + recents: {n} rows in {:?}", t.elapsed());
    for query in ["body", "#ten", "topic 42", "=note4999"] {
        let t = Instant::now();
        search(&started, &rt, query, cx);
        let n = rows(&started, &rt, cx).len();
        eprintln!("query {query:?}: {n} rows in {:?}", t.elapsed());
    }
    search(&started, &rt, "body", cx);
    let t = Instant::now();
    for _ in 0..50 {
        press(&started, "down", cx);
    }
    eprintln!("50 Down presses (no preview read) in {:?}", t.elapsed());
    let t = Instant::now();
    settle_preview(&started, &rt, cx);
    eprintln!("preview after the debounce in {:?}", t.elapsed());
    assert!(previewed(&started, cx).is_some());
}
