//! UI integration tests for the search browser (M7b G4): the picker over
//! the vault's query search with its preview, on a real `Workspace` over a
//! real, indexed temp vault, driven through the window's keys.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`). The preview's debounce
//! is a GPUI timer: it fires only when a test advances the test clock.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Bounds, Entity, Pixels, TestAppContext, px, size};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::message::AppMessage;
use tokio::runtime::Runtime;

use super::modal::ModalKind;
use super::picker::{HELP_BUTTON_ID, NotePicker, PICKER_ERROR_ID, PICKER_INPUT_ID, QUERY_ERROR_ID};
use super::picker::{PICKER_ID, search_body_rems};
use super::search::{
    PREVIEW_BYTES, PREVIEW_DEBOUNCE, PREVIEW_ID, PREVIEW_ROW_ID, PREVIEW_TRUNCATED_ID,
    QUERY_HELP_ID, SearchPreview, cap_text, preview_text,
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
        80,
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

/// A match that runs across a row edge is marked on both rows; every
/// other match stays whole; the first match's row is found.
#[test]
fn preview_rows_wrap_and_keep_their_marks() {
    let shown = preview_text(
        path("x.md"),
        "intro\nxx needle needle and needle".into(),
        &["needle needle".into(), "and".into()],
        10,
    );
    assert_eq!(
        shown.lines_and_hits(),
        vec![
            ("intro".to_string(), vec![]),
            ("xx needle ".to_string(), vec!["needle ".to_string()]),
            (
                "needle and ".to_string(),
                vec!["needle".to_string(), "and".to_string()]
            ),
            ("needle".to_string(), vec![]),
        ]
    );
    assert_eq!(shown.summary(), (2, Some(1), false));
    // Wider: one row per line, the first match on line 1.
    let wide = preview_text(
        path("x.md"),
        "intro\nxx needle".into(),
        &["needle".into()],
        80,
    );
    assert_eq!(wide.summary(), (1, Some(1), false));
}

/// The body takes the window's height minus the dialog's chrome, within
/// its bounds.
#[test]
fn the_browser_body_follows_the_window_height() {
    assert_eq!(search_body_rems(40.), 28.);
    assert_eq!(search_body_rems(10.), 6., "a floor in a tiny window");
    assert_eq!(search_body_rems(200.), 44., "a cap on a huge screen");
}

/// Draw a frame (layout runs, the preview measures its width).
fn frame(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

/// The bounds of the element `id` in a fresh frame, and the window's size.
fn bounds_of(
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) -> (Bounds<Pixels>, gpui_kit::Size<Pixels>) {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        (window.find(id).bounds(), window.viewport_size())
    })
    .unwrap()
}

fn resize(s: &Started, width: f32, height: f32, cx: &mut TestAppContext) {
    cx.simulate_window_resize(s.handle, size(px(width), px(height)));
    frame(s, cx);
}

/// Let the preview land and wrap to the pane's measured width.
fn settle_wrapped(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    let view = preview(s, cx);
    drive(rt, cx, |cx| {
        cx.update_window(s.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.executor().advance_clock(PREVIEW_DEBOUNCE);
        cx.run_until_parked();
        cx.read(|cx| {
            let preview = view.read(cx);
            let (_, wrapped, measured) = preview.wrap_state();
            !preview.is_pending() && measured > 0 && wrapped == Some(measured)
        })
    });
}

/// The search browser fits a small window (preview under the results)
/// and grows with a tall one.
#[gpui_kit::test]
fn the_browser_fits_a_small_window_and_grows_in_a_tall_one(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, NOTES, &[], "a.md");
    resize(&started, 640., 420., cx);
    press(&started, "ctrl-k", cx);
    started.settle_dialog(cx);
    let (picker, window) = bounds_of(&started, PICKER_ID, cx);
    let (pane, _) = bounds_of(&started, PREVIEW_ID, cx);
    for b in [picker, pane] {
        assert!(
            b.top() >= px(0.) && b.bottom() <= window.height && b.right() <= window.width,
            "{b:?} in {window:?}"
        );
    }
    assert!(pane.size.height > px(48.), "the preview has room: {pane:?}");
    assert!(pane.top() > picker.top() + px(48.), "under the results");
    press(&started, "escape", cx);
    started.settle_dialog(cx);

    resize(&started, 1400., 1000., cx);
    press(&started, "ctrl-k", cx);
    started.settle_dialog(cx);
    let (picker, window) = bounds_of(&started, PICKER_ID, cx);
    let (pane, _) = bounds_of(&started, PREVIEW_ID, cx);
    assert!(picker.bottom() <= window.height, "{picker:?} in {window:?}");
    // Beside the results, most of the window tall (it was 20 rems).
    assert!(pane.size.height > px(600.), "{pane:?}");
    assert!(
        (pane.top() - picker.top()).abs() < px(64.),
        "beside the results"
    );
}

/// A line longer than the pane wraps to it: every row fits the measured
/// columns, nothing is lost, the match at the far end is marked and
/// scrolled into view.
#[gpui_kit::test]
fn a_long_line_wraps_and_its_tail_shows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let long = format!("start {}tailend", "word ".repeat(300));
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("a.md", "alpha"),
            ("long.md", &format!("first line\n{long}\nlast")),
        ],
        &[],
        "a.md",
    );
    resize(&started, 1400., 900., cx);
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "tailend", cx);
    settle_wrapped(&started, &rt, cx);
    assert!(previewed(&started, cx).is_some_and(|p| p.is_like(&path("long.md"))));
    let view = preview(&started, cx);
    let (first_hit, _, columns) = cx.read(|cx| view.read(cx).wrap_state());
    let rows = cx.read(|cx| view.read(cx).shown_lines());

    // The columns are the pane's width in monospace cells.
    let (pane, _) = bounds_of(&started, PREVIEW_ID, cx);
    let cell = cx.update(|cx| {
        use gpui_kit::component::ActiveTheme as _;
        let kit = cx.theme();
        let (family, size) = (kit.mono_font_family.clone(), kit.mono_font_size);
        crate::grid::CellMetrics::from_text_system(
            cx.text_system(),
            &gpui_kit::font(family),
            size,
            size,
        )
        .width()
    });
    let used = columns as f32 * cell;
    assert!(
        used <= f32::from(pane.size.width) && used > f32::from(pane.size.width) * 0.8,
        "{columns} columns of {cell}px in {pane:?}"
    );

    // Wrapped: several rows, each within the columns, together the line.
    let wrapped: Vec<&(String, Vec<String>)> = rows[1..rows.len() - 1].iter().collect();
    assert!(wrapped.len() > 5, "{} rows", wrapped.len());
    assert!(
        wrapped
            .iter()
            .all(|(text, _)| text.trim_end().chars().count() <= columns)
    );
    let joined: String = wrapped.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(joined, long);
    assert_eq!(rows[0].0, "first line");
    assert_eq!(rows.last().unwrap().0, "last");

    // The tail's row is the first match's, marked and in view.
    let tail = rows.len() - 2;
    assert_eq!(first_hit, Some(tail));
    assert_eq!(rows[tail].1, vec!["tailend".to_string()]);
    let (row, _) = bounds_of(&started, (PREVIEW_ROW_ID, tail), cx);
    assert!(
        row.top() >= pane.top() && row.bottom() <= pane.bottom(),
        "{row:?} in {pane:?}"
    );
}

/// Every match on a wrapped line stays marked, however the line wraps.
#[gpui_kit::test]
fn highlights_survive_wrapping(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let long = "some filler then needle ".repeat(40);
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("n.md", &long)], &[], "a.md");
    resize(&started, 1400., 900., cx);
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "needle", cx);
    settle_wrapped(&started, &rt, cx);
    let view = preview(&started, cx);
    let rows = cx.read(|cx| view.read(cx).shown_lines());
    assert!(rows.len() > 1, "wrapped: {rows:?}");
    let marks: Vec<String> = rows.into_iter().flat_map(|(_, hits)| hits).collect();
    assert_eq!(marks, vec!["needle".to_string(); 40]);
}

/// A narrower pane (the preview moves under the results) wraps the note
/// again to its width without reading it again; the match stays in view.
#[gpui_kit::test]
fn a_narrower_pane_rewraps_without_a_read(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let long = format!("{}needle", "lorem ipsum ".repeat(120));
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("n.md", &long)], &[], "a.md");
    resize(&started, 1400., 900., cx);
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "needle", cx);
    settle_wrapped(&started, &rt, cx);
    let view = preview(&started, cx);
    let reads = cx.read(|cx| view.read(cx).reads);
    let (_, wide, _) = cx.read(|cx| view.read(cx).wrap_state());
    let wide_rows = cx.read(|cx| view.read(cx).shown_lines()).len();

    resize(&started, 560., 900., cx);
    settle_wrapped(&started, &rt, cx);
    let (first_hit, narrow, measured) = cx.read(|cx| view.read(cx).wrap_state());
    assert!(narrow < wide, "{narrow:?} vs {wide:?}");
    assert_eq!(narrow, Some(measured));
    let rows = cx.read(|cx| view.read(cx).shown_lines());
    assert!(rows.len() > wide_rows, "{} vs {wide_rows}", rows.len());
    assert_eq!(cx.read(|cx| view.read(cx).reads), reads, "no read again");
    assert_eq!(rows.concat_text(), long);

    let hit = first_hit.expect("a match");
    assert_eq!(rows[hit].1, vec!["needle".to_string()]);
    let (pane, _) = bounds_of(&started, PREVIEW_ID, cx);
    let (row, _) = bounds_of(&started, (PREVIEW_ROW_ID, hit), cx);
    assert!(
        row.top() >= pane.top() && row.bottom() <= pane.bottom(),
        "{row:?} in {pane:?}"
    );
}

/// The first preview row drawn (the top of what the reader sees).
fn top_row(s: &Started, rows: usize, cx: &mut TestAppContext) -> usize {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        (0..rows)
            .find(|&ix| window.try_find((PREVIEW_ROW_ID, ix)).is_some())
            .expect("a row is drawn")
    })
    .unwrap()
}

/// After the reader scrolls, a re-wrap keeps the line at the top in place
/// instead of jumping back to the match.
#[gpui_kit::test]
fn a_rewrap_keeps_the_readers_place(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let text: String = (0..150)
        .map(|i| format!("line {i:03} {}\n", "pad ".repeat(20)))
        .collect();
    let text = format!("needle\n{text}");
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("n.md", &text)], &[], "a.md");
    resize(&started, 1400., 900., cx);
    press(&started, "ctrl-k", cx);
    search(&started, &rt, "needle", cx);
    settle_wrapped(&started, &rt, cx);
    let view = preview(&started, cx);
    let wide = cx.read(|cx| view.read(cx).shown_lines());
    let (_, wide_columns, _) = cx.read(|cx| view.read(cx).wrap_state());
    assert_eq!(top_row(&started, wide.len(), cx), 0, "the match, on top");

    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-1200.)));
        window.scroll(PREVIEW_ID, delta, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let top = top_row(&started, wide.len(), cx);
    assert!(top > 10, "scrolled: {top}");
    let line = cx.read(|cx| view.read(cx).row_lines())[top];
    assert!(line > 5, "a line well below the match: {line}");

    resize(&started, 560., 900., cx);
    settle_wrapped(&started, &rt, cx);
    let narrow = cx.read(|cx| view.read(cx).shown_lines());
    let (_, narrow_columns, _) = cx.read(|cx| view.read(cx).wrap_state());
    assert!(narrow_columns < wide_columns, "re-wrapped");
    let top = top_row(&started, narrow.len(), cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).row_lines())[top],
        line,
        "the same note line leads the pane"
    );
}

trait ConcatText {
    fn concat_text(&self) -> String;
}

impl ConcatText for Vec<(String, Vec<String>)> {
    fn concat_text(&self) -> String {
        self.iter().map(|(text, _)| text.as_str()).collect()
    }
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
