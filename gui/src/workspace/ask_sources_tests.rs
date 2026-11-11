//! UI integration tests for the Sources view and the Source reader (M7c
//! G10b), the part of the Ask view that expands in place under the
//! selected turn (ADR-0053): a real `Workspace` on a real, indexed temp
//! vault, the server's
//! status fed as the sync reports it, the questions answered by a stub
//! transport whose answers carry the sources each test needs. Nothing here
//! talks to a network.
//!
//! As in `ask_tests`, each test owns a current-thread tokio runtime driven
//! between GPUI's `run_until_parked` (`start_tests::drive`): the ask jobs,
//! the flushes and the reader's note reads run there.
#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, Focusable as _, SharedString, TestAppContext, point, px};
use kimun_core::nfs::VaultPath;
use kimun_notes::ask::{AskSource, locate};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::follow::FollowOutcome;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::rag::RagStatus;
use kimun_notes::server_client::{
    AnswerResult, ChunkResult, ContextSize, QueryTransport, RagClient, RagError,
};
use kimun_notes::settings::gui::GuiSettings;
use tokio::runtime::Runtime;

use super::ask::{
    COMPOSER_ID, NEW_ID, REGENERATE_ID, SOURCES_PART_ID, SOURCES_TOGGLE_ID, answer_line_id,
    turn_element_id,
};
use super::ask_sources::{
    NO_RESULTS, NO_SOURCES, READER_HEADER_ID, SOURCES_FILTER_ID, SourcesView, source_row_id,
};
use super::drawer_list::chevron_id;
use super::files::row_id;
use super::search::{PREVIEW_DEBOUNCE, READER_FAILED};
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use super::{rail_cell_id, slot_id};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

/// The TUI's flash for a route into ASK the server can't serve.
const NO_LLM: &str = "Ask needs an LLM-configured server; this one is semantic-search only";

const ONLINE: RagStatus = RagStatus::Online {
    llm_available: true,
};
const SEARCH_ONLY: RagStatus = RagStatus::Online {
    llm_available: false,
};

/// The Sources list's focus target (`DrawerList`'s rows element).
const LIST_ROWS_ID: &str = "ask-sources-list-rows";

const A_TEXT: &str = "# Intro\nalpha intro\n\n# Details\nalpha details here\n";
const NOTES: &[(&str, &str)] = &[
    ("a.md", A_TEXT),
    ("b.md", "# Beta\nbeta body\n"),
    ("c.md", "plain note\n"),
];

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

fn chunk(p: &str, title: &str, content: &str, ordinal: usize) -> ChunkResult {
    ChunkResult {
        path: path(p).to_string(),
        title: title.to_string(),
        date: None,
        content: content.to_string(),
        hash: String::new(),
        similarity_score: 0.5,
        ordinal,
    }
}

/// `a.md`'s Details section and `b.md`'s Beta.
fn a_and_b() -> Vec<ChunkResult> {
    vec![
        chunk("a.md", "Details", "alpha details here", 1),
        chunk("b.md", "Beta", "beta body", 2),
    ]
}

// ---- A stub server -------------------------------------------------------------

type Reply = (String, Vec<ChunkResult>);

/// Answers each question with the next queued reply (else "See [1]." with
/// [`a_and_b`]).
#[derive(Default)]
struct Stub {
    replies: Mutex<VecDeque<Reply>>,
}

impl Stub {
    fn with(replies: Vec<(&str, Vec<ChunkResult>)>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|(answer, sources)| (answer.to_string(), sources))
                    .collect(),
            ),
        })
    }
}

#[async_trait::async_trait]
impl QueryTransport for Stub {
    async fn search(
        &self,
        _query: &str,
        _context_size: Option<ContextSize>,
    ) -> Result<Vec<ChunkResult>, RagError> {
        Ok(Vec::new())
    }

    async fn ask(
        &self,
        _query: &str,
        _history: &[(String, String)],
        _context_size: Option<ContextSize>,
    ) -> Result<AnswerResult, RagError> {
        let (answer, sources) = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ("See [1].".to_string(), a_and_b()));
        Ok(AnswerResult { answer, sources })
    }
}

// ---- Helpers -------------------------------------------------------------------

/// A vault holding [`NOTES`], started on `a.md`, indexed; the server can
/// answer, through `stub`.
fn ready(cx: &mut TestAppContext, rt: &Runtime, stub: Arc<Stub>) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(rt, cx, |cx| {
        started.text(cx) == A_TEXT && !started.indexing(cx)
    });
    let client = Arc::new(RagClient::with_transport(stub));
    started
        .workspace
        .update(cx, |w, _| w.ask_test_client = Some(client));
    status(&started, ONLINE, cx);
    // Wide enough for the sources to sit beside the Thread (a narrow slot
    // drills into them instead: see `narrow_*` tests).
    cx.simulate_window_resize(
        started.handle,
        gpui_kit::size(gpui_kit::px(5000.), gpui_kit::px(1000.)),
    );
    (v, started)
}

fn status(s: &Started, status: RagStatus, cx: &mut TestAppContext) {
    send(s, AppMessage::RagStatus(status), cx);
    cx.run_until_parked();
}

fn sources(s: &Started, cx: &TestAppContext) -> Entity<SourcesView> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .ask_sources
            .clone()
            .expect("the Sources view is built")
    })
}

/// The rows listed: rank and path.
fn rows(s: &Started, cx: &TestAppContext) -> Vec<(usize, VaultPath)> {
    let view = sources(s, cx);
    cx.read(|cx| {
        let list = view.read(cx).list.clone();
        list.read(cx)
            .rows()
            .into_iter()
            .map(|row| (row.rank, row.source.path))
            .collect()
    })
}

fn ranked(paths: &[&str]) -> Vec<(usize, VaultPath)> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| (i + 1, path(p)))
        .collect()
}

fn selected_row(s: &Started, cx: &TestAppContext) -> Option<usize> {
    let view = sources(s, cx);
    cx.read(|cx| view.read(cx).list.read(cx).selected())
}

fn listed_turn(s: &Started, cx: &TestAppContext) -> Option<u64> {
    let view = sources(s, cx);
    cx.read(|cx| view.read(cx).turn_id())
}

fn empty_text(s: &Started, cx: &TestAppContext) -> String {
    let view = sources(s, cx);
    cx.read(|cx| view.read(cx).list.read(cx).empty_text().to_string())
}

/// The reader's expand state: collapsed, context, full.
fn reader_state(s: &Started, cx: &TestAppContext) -> (bool, bool, bool) {
    let view = sources(s, cx);
    cx.read(|cx| {
        let model = view.read(cx).reader_model();
        (model.is_collapsed(), model.is_context(), model.is_full())
    })
}

/// The note the reader shows, once it is the one wanted.
fn reader_note(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    let view = sources(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .reader
            .read(cx)
            .shown()
            .filter(|(_, current)| *current)
            .map(|(p, _)| p)
    })
}

fn reader_section(s: &Started, cx: &TestAppContext) -> Option<std::ops::Range<usize>> {
    let view = sources(s, cx);
    cx.read(|cx| view.read(cx).reader.read(cx).shown_section())
}

fn reader_notice(s: &Started, cx: &TestAppContext) -> Option<String> {
    let view = sources(s, cx);
    cx.read(|cx| view.read(cx).reader.read(cx).notice())
}

/// The reader's marked rows' text.
fn reader_marked(s: &Started, cx: &TestAppContext) -> Vec<String> {
    let view = sources(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .reader
            .read(cx)
            .shown_lines()
            .into_iter()
            .filter(|(_, hits)| !hits.is_empty())
            .map(|(line, _)| line)
            .collect()
    })
}

fn thread_shown(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).ask_shown())
}

/// Whether the sources part is expanded.
fn sources_open(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| {
        let view = s.workspace.read(cx).ask.clone().unwrap();
        view.read(cx).sources_open()
    })
}

fn turn_ids(s: &Started, cx: &TestAppContext) -> Vec<u64> {
    cx.read(|cx| {
        let view = s.workspace.read(cx).ask.clone().unwrap();
        view.read(cx)
            .thread()
            .turns()
            .iter()
            .map(|t| t.id)
            .collect()
    })
}

fn pending(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| {
        let view = s.workspace.read(cx).ask.clone().unwrap();
        view.read(cx)
            .thread()
            .turns()
            .iter()
            .any(|t| matches!(t.status, kimun_notes::ask::TurnStatus::Thinking))
    })
}

fn open_note(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    cx.read(|cx| s.workspace.read(cx).notes.as_ref()?.note.clone())
}

/// The views the slots of `side` show.
fn slot_views(s: &Started, side: Side, cx: &TestAppContext) -> Vec<DrawerView> {
    cx.read(|cx| s.workspace.read(cx).shown_on(side))
}

/// Whether a slot of `side` is drawn.
fn side_visible(s: &Started, side: Side, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).side_drawn(side))
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

fn click(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

/// A click just inside the left edge of the answer line `line` of turn
/// `turn`: on its first character.
fn click_line_start(s: &Started, turn: u64, line: usize, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click_at(answer_line_id(turn, line), point(px(3.), px(6.)), cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn drawn(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

fn focused(s: &Started, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

/// The accessible label of the element `id`, `None` when it is not drawn.
fn label_of(s: &Started, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
    label_of_id(s, id, cx)
}

/// Move the pointer onto the element `id`.
fn hover(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| window.hover(id, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Where the element `id` is laid out, drawn or not.
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

/// [`label_of`] for any element id.
fn label_of_id(
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) -> Option<String> {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .filter(|e| e.visible())
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

fn leader(s: &Started, action: LeaderAction, cx: &mut TestAppContext) {
    send(s, AppMessage::ExecuteLeaderAction(action), cx);
    cx.run_until_parked();
}

/// Show the view with the composer focused (F6), ask `question` and drive
/// until it is answered; then the sources part is expanded under the
/// selected turn (its toggle, the first time), the composer keeping the
/// keyboard.
fn ask(s: &Started, rt: &Runtime, question: &str, cx: &mut TestAppContext) {
    press(s, "f6", cx);
    drive(rt, cx, |cx| thread_shown(s, cx));
    type_in(s, question, cx);
    press(s, "enter", cx);
    drive(rt, cx, |cx| !pending(s, cx));
    if !sources_open(s, cx) && drawn(s, SOURCES_TOGGLE_ID, cx) {
        click(s, SOURCES_TOGGLE_ID, cx);
        press(s, "f6", cx);
    }
}

/// [`drive`], the test clock moved past the reader's read debounce each
/// round.
fn settle(
    rt: &Runtime,
    cx: &mut TestAppContext,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    for _ in 0..400 {
        cx.executor().advance_clock(PREVIEW_DEBOUNCE);
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
        cx.run_until_parked();
        if done(cx) {
            return;
        }
    }
    panic!("timed out driving the reader");
}

fn source(p: &str, heading: &str, text: &str, ordinal: usize) -> AskSource {
    AskSource {
        path: path(p),
        heading: heading.to_string(),
        date: None,
        score: 0.5,
        text: text.to_string(),
        ordinal,
    }
}

/// Run `f` on the Sources view with the window.
fn with_sources(
    s: &Started,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut SourcesView, &mut gpui_kit::Window, &mut gpui_kit::Context<SourcesView>),
) {
    let view = sources(s, cx);
    cx.update_window(s.handle, |_, window, cx| {
        view.update(cx, |view, cx| f(view, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

// ---- The rail ------------------------------------------------------------------

/// ASK's cell is on the rail while the server can answer; a click on it
/// shows the view in its slot (the composer has the keyboard); losing the
/// server takes the cell away and leaves the view, its sources and the
/// thread as they are.
#[gpui_kit::test]
fn the_ask_cell_follows_the_server_and_its_view_stays_on_loss(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    assert!(drawn(&started, rail_cell_id(DrawerView::Ask), cx));
    status(&started, SEARCH_ONLY, cx);
    assert!(!drawn(&started, rail_cell_id(DrawerView::Ask), cx));
    status(&started, ONLINE, cx);
    click(&started, rail_cell_id(DrawerView::Ask), cx);
    drive(&rt, cx, |cx| thread_shown(&started, cx));
    assert!(drawn(&started, slot_id(DrawerView::Ask), cx));
    // ASK's home is the right slot; FILES stays on the left.
    assert_eq!(slot_views(&started, Side::Right, cx), [DrawerView::Ask]);
    assert_eq!(slot_views(&started, Side::Left, cx), [DrawerView::Files]);
    assert!(focused(&started, COMPOSER_ID, cx));
    assert_eq!(empty_text(&started, cx), NO_SOURCES);
    ask(&started, &rt, "why?", cx);
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));

    status(&started, RagStatus::Offline, cx);
    assert!(!drawn(&started, rail_cell_id(DrawerView::Ask), cx));
    assert!(drawn(&started, slot_id(DrawerView::Ask), cx));
    assert!(thread_shown(&started, cx));
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));
}

// ---- Rows and sync ---------------------------------------------------------------

/// The rows are the selected turn's sources in rank order; another turn
/// selected re-points them; a turn being answered lists nothing until its
/// answer lands, then its sources.
#[gpui_kit::test]
fn the_rows_follow_the_selected_turn(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![
        ("one [1]", a_and_b()),
        ("two [1]", vec![chunk("c.md", "", "plain note", 1)]),
    ]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "first", cx);
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));
    let ids = turn_ids(&started, cx);
    assert_eq!(listed_turn(&started, cx), Some(ids[0]));

    type_in(&started, "second", cx);
    press(&started, "enter", cx);
    let ids = turn_ids(&started, cx);
    assert_eq!(listed_turn(&started, cx), Some(ids[1]), "the new turn");
    assert!(
        rows(&started, cx).is_empty(),
        "nothing until it is answered"
    );
    drive(&rt, cx, |cx| !pending(&started, cx));
    drive(&rt, cx, |cx| !rows(&started, cx).is_empty());
    assert_eq!(rows(&started, cx), ranked(&["c.md"]));

    // Up in the empty composer: the first turn, its sources.
    press(&started, "up", cx);
    assert_eq!(listed_turn(&started, cx), Some(ids[0]));
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));
}

/// An answer for the selected turn refreshes the rows; one for another turn
/// does not (until that turn is selected).
#[gpui_kit::test]
fn an_answer_refreshes_only_the_selected_turns_rows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![
        ("one [1]", a_and_b()),
        ("two [1]", vec![chunk("b.md", "Beta", "beta body", 1)]),
        ("one again [1]", vec![chunk("c.md", "", "plain note", 1)]),
    ]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "first", cx);
    ask(&started, &rt, "second", cx);
    let ids = turn_ids(&started, cx);
    press(&started, "up", cx);
    assert_eq!(listed_turn(&started, cx), Some(ids[0]));
    // Regenerate the first, then select the second before it lands.
    click(&started, REGENERATE_ID, cx);
    assert!(pending(&started, cx));
    cx.update_window(started.handle, |_, window, cx| {
        let view = started.workspace.read(cx).ask.clone().unwrap();
        window.focus(&view.read(cx).composer.read(cx).focus_handle(cx), cx);
    })
    .unwrap();
    press(&started, "down", cx);
    assert_eq!(listed_turn(&started, cx), Some(ids[1]));
    drive(&rt, cx, |cx| !pending(&started, cx));
    assert_eq!(listed_turn(&started, cx), Some(ids[1]), "not re-pointed");
    assert_eq!(rows(&started, cx), ranked(&["b.md"]));
    press(&started, "up", cx);
    assert_eq!(rows(&started, cx), ranked(&["c.md"]), "the new sources");
}

/// `set_turn` with the turn listed keeps the rows, the selection and the
/// reader; `refresh` replaces the rows, selects the first and collapses the
/// reader; `reset` empties it.
#[gpui_kit::test]
fn set_turn_keeps_and_refresh_replaces(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    let ab = vec![
        source("a.md", "Details", "alpha details here", 1),
        source("b.md", "Beta", "beta body", 2),
    ];
    let c = vec![source("c.md", "", "plain note", 1)];
    with_sources(&started, cx, |view, window, cx| {
        view.set_turn(7, ab.clone(), window, cx)
    });
    with_sources(&started, cx, |view, window, cx| {
        view.open_reader(1, window, cx)
    });
    assert_eq!(selected_row(&started, cx), Some(1));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    with_sources(&started, cx, |view, window, cx| {
        view.set_turn(7, c.clone(), window, cx)
    });
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));
    assert_eq!(selected_row(&started, cx), Some(1));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    with_sources(&started, cx, |view, window, cx| {
        view.refresh(7, c.clone(), window, cx)
    });
    assert_eq!(rows(&started, cx), ranked(&["c.md"]));
    assert_eq!(selected_row(&started, cx), Some(0));
    assert_eq!(reader_state(&started, cx), (true, false, false));
    with_sources(&started, cx, |view, window, cx| view.reset(window, cx));
    assert!(rows(&started, cx).is_empty());
    assert_eq!(listed_turn(&started, cx), None);
    assert_eq!(empty_text(&started, cx), NO_SOURCES);
}

/// New (the button) clears the thread and the Sources view.
#[gpui_kit::test]
fn new_resets_the_sources(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    assert!(!rows(&started, cx).is_empty());
    click(&started, NEW_ID, cx);
    assert!(rows(&started, cx).is_empty());
    assert_eq!(listed_turn(&started, cx), None);
}

/// The selected turn's sources expand in place, right under it: the
/// `Sources (n)` toggle (only on the selected, answered turn) draws the
/// part between that turn and the next one, its list with the keyboard; a
/// selection move takes the part to the newly selected turn, listing that
/// turn's sources; `s` on the turn list shuts it and opens it again.
#[gpui_kit::test]
fn sources_list_expands_in_place_under_the_selected_turn(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![
        ("one [1]", a_and_b()),
        ("two [1]", vec![chunk("c.md", "", "plain note", 1)]),
    ]);
    let (_v, started) = ready(cx, &rt, stub);
    press(&started, "f6", cx);
    for q in ["first", "second"] {
        type_in(&started, q, cx);
        press(&started, "enter", cx);
        drive(&rt, cx, |cx| !pending(&started, cx));
    }
    let ids = turn_ids(&started, cx);
    assert!(!drawn(&started, SOURCES_PART_ID, cx), "shut at first");
    assert_eq!(
        label_of(&started, SOURCES_TOGGLE_ID, cx).as_deref(),
        Some("Sources (1)")
    );
    // Up: the first turn selected, its toggle names its two sources.
    press(&started, "up", cx);
    assert_eq!(
        label_of(&started, SOURCES_TOGGLE_ID, cx).as_deref(),
        Some("Sources (2)")
    );
    click(&started, SOURCES_TOGGLE_ID, cx);
    assert!(drawn(&started, SOURCES_PART_ID, cx));
    assert!(focused(&started, LIST_ROWS_ID, cx));
    assert!(drawn(&started, source_row_id(2), cx));
    let first = bounds_of(&started, turn_element_id(ids[0]), cx);
    let second = bounds_of(&started, turn_element_id(ids[1]), cx);
    let part = bounds_of(&started, SOURCES_PART_ID, cx);
    // Beside the turns, not among them.
    assert!(part.left() >= first.right(), "{part:?} beside {first:?}");
    assert!(part.left() >= second.right(), "{part:?} beside {second:?}");
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));

    // Down in the composer: the part follows to the second turn.
    press(&started, "f6", cx);
    press(&started, "down", cx);
    assert_eq!(listed_turn(&started, cx), Some(ids[1]));
    assert_eq!(rows(&started, cx), ranked(&["c.md"]));
    let second = bounds_of(&started, turn_element_id(ids[1]), cx);
    let part = bounds_of(&started, SOURCES_PART_ID, cx);
    assert!(part.left() >= second.right(), "{part:?} beside {second:?}");

    // `s` on the turn list: shut (the turn list keeps the keyboard), open.
    press(&started, "escape", cx);
    press(&started, "s", cx);
    assert!(!drawn(&started, SOURCES_PART_ID, cx));
    press(&started, "s", cx);
    assert!(drawn(&started, SOURCES_PART_ID, cx));
    assert!(focused(&started, LIST_ROWS_ID, cx));
}

/// The selected turn's question.
fn selected_question(s: &Started, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| {
        let view = s.workspace.read(cx).ask.clone().unwrap();
        view.read(cx)
            .thread()
            .selected()
            .map(|t| t.question.clone())
    })
}

fn filter_text(s: &Started, cx: &TestAppContext) -> String {
    let view = sources(s, cx);
    cx.read(|cx| view.read(cx).filter.read(cx).value().to_string())
}

/// Real key presses typed into the sources filter are the filter's: no
/// Thread command runs (`r` regenerates nothing, `s` does not shut the
/// part, nothing moves the turn selection) and the filter holds the text.
#[gpui_kit::test]
fn keys_typed_in_the_sources_filter_never_reach_the_thread(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "first", cx);
    ask(&started, &rt, "second", cx);
    press(&started, "up", cx);
    assert_eq!(selected_question(&started, cx).as_deref(), Some("first"));
    click(&started, SOURCES_FILTER_ID, cx);
    for key in ["r", "u", "s", "t", "j", "k", "e", "y", "i"] {
        press(&started, key, cx);
    }
    assert!(!pending(&started, cx), "nothing regenerated");
    assert_eq!(selected_question(&started, cx).as_deref(), Some("first"));
    assert!(sources_open(&started, cx));
    assert!(drawn(&started, SOURCES_PART_ID, cx));
    assert_eq!(started.shown_modal(cx), None, "no Save as note");
    assert_eq!(filter_text(&started, cx), "rustjkeyi");
}

/// Real key presses on the sources list: `j`, `k`, `r`, `e`, `s` are not the
/// list's and never Thread commands (the turn selection stays, nothing is
/// regenerated or saved, the part stays open with the list focused); Down,
/// the list's own, moves the source selection.
#[gpui_kit::test]
fn keys_on_the_sources_list_never_reach_the_thread(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "first", cx);
    ask(&started, &rt, "second", cx);
    press(&started, "up", cx);
    click(&started, source_row_id(1), cx);
    assert!(focused(&started, LIST_ROWS_ID, cx));
    for key in ["j", "k", "r", "e", "s"] {
        press(&started, key, cx);
        assert!(!pending(&started, cx), "{key}: nothing regenerated");
        assert_eq!(
            selected_question(&started, cx).as_deref(),
            Some("first"),
            "{key}"
        );
        assert!(drawn(&started, SOURCES_PART_ID, cx), "{key}");
        assert!(focused(&started, LIST_ROWS_ID, cx), "{key}");
        assert_eq!(started.shown_modal(cx), None, "{key}");
    }
    assert_eq!(selected_row(&started, cx), Some(0));
    press(&started, "down", cx);
    assert_eq!(selected_row(&started, cx), Some(1));

    // Regenerate (the button) with the list focused: the part goes while
    // the turn is pending, and the keyboard never rests on its undrawn
    // list: the turn list has it.
    click(&started, REGENERATE_ID, cx);
    assert!(pending(&started, cx));
    assert!(!drawn(&started, SOURCES_PART_ID, cx));
    assert!(focused(&started, super::ask::TURNS_ID, cx));
    drive(&rt, cx, |cx| !pending(&started, cx));
}

// ---- The reader ------------------------------------------------------------------

/// Enter on a row opens the reader on that source's note, read off the UI
/// thread, its section (`locate::section_range`) marked; a section that
/// does not resolve shows the note unmarked; a note that cannot be read
/// says so (the TUI's words). Arrows keep Context on the new row; moving on
/// before a read lands shows only the row moved to. Escape steps back, then
/// gives the composer the keyboard.
#[gpui_kit::test]
fn enter_opens_the_reader_on_the_section(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![(
        "see [1]",
        vec![
            chunk("a.md", "Details", "alpha details here", 1),
            chunk("b.md", "Beta", "beta body", 2),
            chunk("c.md", "Missing", "nothing like this", 3),
            chunk("gone.md", "Gone", "gone", 4),
        ],
    )]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    click(&started, source_row_id(1), cx);
    assert!(focused(&started, LIST_ROWS_ID, cx));
    press(&started, "enter", cx);
    assert_eq!(reader_state(&started, cx), (false, true, false));
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("a.md"))
    });
    let expected = locate::section_range(A_TEXT, "Details", "alpha details here");
    assert!(expected.is_some());
    assert_eq!(reader_section(&started, cx), expected);
    assert_eq!(&A_TEXT[expected.unwrap()], "alpha details here");
    assert_eq!(reader_marked(&started, cx), ["alpha details here"]);

    // Down then Down before b.md lands: only c.md shows, unmarked.
    press(&started, "down", cx);
    press(&started, "down", cx);
    assert_eq!(
        reader_state(&started, cx),
        (false, true, false),
        "Context stays"
    );
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("c.md"))
    });
    let reads = {
        let view = sources(&started, cx);
        cx.read(|cx| view.read(cx).reader.read(cx).reads)
    };
    assert_eq!(reads, 2, "b.md was never read: a.md, then c.md");
    assert_eq!(reader_section(&started, cx), None);
    assert!(reader_marked(&started, cx).is_empty());

    // A note that is not there.
    press(&started, "down", cx);
    settle(&rt, cx, |cx| {
        reader_notice(&started, cx).as_deref() == Some(READER_FAILED)
    });

    // Escape: Context → Collapsed, then the composer.
    press(&started, "escape", cx);
    assert_eq!(reader_state(&started, cx), (true, false, false));
    press(&started, "escape", cx);
    assert!(focused(&started, COMPOSER_ID, cx));
}

/// Right grows the reader to Full (Up/Down then scroll it, the list keeps
/// the keyboard), Left shrinks it; another section of the note shown is
/// marked without a second read.
#[gpui_kit::test]
fn the_reader_grows_and_reuses_a_read_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![(
        "see [1]",
        vec![
            chunk("a.md", "Details", "alpha details here", 1),
            chunk("a.md", "Intro", "alpha intro", 2),
        ],
    )]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    click(&started, source_row_id(1), cx);
    press(&started, "right", cx);
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("a.md"))
    });
    let reads = |cx: &TestAppContext| {
        let view = sources(&started, cx);
        cx.read(|cx| view.read(cx).reader.read(cx).reads)
    };
    assert_eq!(reads(cx), 1);
    press(&started, "down", cx);
    settle(&rt, cx, |cx| reader_marked(&started, cx) == ["alpha intro"]);
    assert_eq!(reads(cx), 1, "the same note is not read again");
    press(&started, "right", cx);
    assert_eq!(reader_state(&started, cx), (false, false, true));
    press(&started, "down", cx);
    assert_eq!(selected_row(&started, cx), Some(1), "Down scrolls in Full");
    assert!(focused(&started, LIST_ROWS_ID, cx));
    press(&started, "left", cx);
    press(&started, "left", cx);
    assert_eq!(reader_state(&started, cx), (true, false, false));
}

/// The chevron of the row of the source ranked `rank`.
fn row_chevron(rank: usize) -> SharedString {
    chevron_id(&source_row_id(rank))
}

/// The chevron on the reader's header.
fn header_chevron() -> SharedString {
    chevron_id(&SharedString::from(READER_HEADER_ID))
}

const GROW: &str = "Expand the preview (→)";
const SHRINK: &str = "Collapse the preview (←)";

/// The reader's header: absent while the reader is collapsed; open, it
/// names the source (its heading, else its path) and carries the chevron,
/// `›` in Context and `‹` in Full. A click on it grows the reader to Full as
/// Right does, the chevron flips to Collapse, and a second click shrinks it
/// back to Context. A click moves no selection and leaves the list the
/// keyboard (even from the filter), so the arrows work after it; a right
/// click does nothing.
#[gpui_kit::test]
fn sources_header_shows_collapse_chevron_in_full(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![(
        "see [1]",
        vec![
            chunk("a.md", "Details", "alpha details here", 1),
            chunk("b.md", "Beta", "beta body", 2),
            chunk("c.md", "", "plain note", 3),
        ],
    )]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    click(&started, source_row_id(2), cx);
    assert_eq!(reader_state(&started, cx), (true, false, false));
    // Collapsed: the list alone, no header.
    assert!(!drawn(&started, READER_HEADER_ID, cx));
    assert!(!drawn(&started, header_chevron(), cx));
    press(&started, "enter", cx);
    assert_eq!(reader_state(&started, cx), (false, true, false));
    assert_eq!(selected_row(&started, cx), Some(1));
    assert!(focused(&started, LIST_ROWS_ID, cx));
    assert_eq!(
        label_of(&started, READER_HEADER_ID, cx).as_deref(),
        Some("Beta")
    );
    assert_eq!(
        label_of_id(&started, header_chevron(), cx).as_deref(),
        Some(GROW)
    );

    // Grow: Full, the chevron flips, the selection and the keyboard stay.
    click(&started, header_chevron(), cx);
    assert_eq!(reader_state(&started, cx), (false, false, true));
    assert_eq!(
        label_of_id(&started, header_chevron(), cx).as_deref(),
        Some(SHRINK)
    );
    assert_eq!(selected_row(&started, cx), Some(1));
    assert!(focused(&started, LIST_ROWS_ID, cx));

    // A right click on it does nothing.
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(header_chevron(), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(reader_state(&started, cx), (false, false, true));

    // Shrink: back to Context.
    click(&started, header_chevron(), cx);
    assert_eq!(reader_state(&started, cx), (false, true, false));
    assert_eq!(
        label_of_id(&started, header_chevron(), cx).as_deref(),
        Some(GROW)
    );
    assert_eq!(selected_row(&started, cx), Some(1));
    assert!(focused(&started, LIST_ROWS_ID, cx));

    // From the filter: the click gives the list the keyboard back, and the
    // arrows work.
    click(&started, SOURCES_FILTER_ID, cx);
    assert!(!focused(&started, LIST_ROWS_ID, cx));
    click(&started, header_chevron(), cx);
    assert_eq!(reader_state(&started, cx), (false, false, true));
    assert!(focused(&started, LIST_ROWS_ID, cx));
    press(&started, "left", cx);
    assert_eq!(reader_state(&started, cx), (false, true, false));
    press(&started, "right", cx);
    assert_eq!(reader_state(&started, cx), (false, false, true));
    press(&started, "left", cx);

    // A source with no heading: its path is the title.
    press(&started, "down", cx);
    assert_eq!(selected_row(&started, cx), Some(2));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    assert_eq!(
        label_of(&started, READER_HEADER_ID, cx),
        Some(path("c.md").to_string())
    );
}

/// The rows' chevron: at rest only the selected row shows it, the hovered
/// row too. A click on a row that is not selected selects THAT row and
/// grows the reader to Context on it, with the list keyboard and the header
/// on top; the row's own click does not run as well (nothing opens, the
/// reader is not cycled further), nor does a double-click on it. In Context
/// the row's chevron grows the reader to Full. A right click on it does not
/// expand.
#[gpui_kit::test]
fn sources_row_chevron_grows_reader(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![(
        "see [1]",
        vec![
            chunk("a.md", "Details", "alpha details here", 1),
            chunk("b.md", "Beta", "beta body", 2),
            chunk("c.md", "", "plain note", 3),
        ],
    )]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    let before = open_note(&started, cx);
    click(&started, source_row_id(1), cx);
    assert_eq!(selected_row(&started, cx), Some(0));
    assert_eq!(reader_state(&started, cx), (true, false, false));

    // Under the pointer: that row and the selected one; nothing moves.
    let at_rest = bounds_of(&started, row_chevron(2), cx);
    for rank in 1..=3 {
        hover(&started, source_row_id(rank), cx);
        for other in 1..=3 {
            assert_eq!(
                label_of_id(&started, row_chevron(other), cx).as_deref(),
                (other == rank || other == 1).then_some(GROW),
                "pointer on row {rank}: row {other}"
            );
        }
    }
    hover(&started, source_row_id(2), cx);
    assert_eq!(bounds_of(&started, row_chevron(2), cx), at_rest);
    assert!(!drawn(&started, READER_HEADER_ID, cx));

    // A right click on it: no expand.
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(row_chevron(2), cx)
    })
    .unwrap();
    cx.run_until_parked();
    press(&started, "escape", cx);
    assert_eq!(reader_state(&started, cx), (true, false, false));
    click(&started, source_row_id(1), cx);
    assert_eq!(reader_state(&started, cx), (true, false, false));

    // A click on the second row's chevron: that row, Context, the list
    // keyboard, the header on top; nothing opened.
    hover(&started, source_row_id(2), cx);
    click(&started, row_chevron(2), cx);
    assert_eq!(selected_row(&started, cx), Some(1));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    assert!(focused(&started, LIST_ROWS_ID, cx));
    assert_eq!(
        label_of(&started, READER_HEADER_ID, cx).as_deref(),
        Some("Beta")
    );
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("b.md"))
    });
    assert_eq!(open_note(&started, cx), before);
    assert!(thread_shown(&started, cx));

    // In Context the selected row keeps `›`: a click grows to Full.
    assert_eq!(
        label_of_id(&started, row_chevron(2), cx).as_deref(),
        Some(GROW)
    );
    click(&started, row_chevron(2), cx);
    assert_eq!(reader_state(&started, cx), (false, false, true));
    assert_eq!(selected_row(&started, cx), Some(1));
    press(&started, "left", cx);
    press(&started, "left", cx);
    assert_eq!(reader_state(&started, cx), (true, false, false));

    // A double-click on a chevron: one step, nothing opened.
    hover(&started, source_row_id(1), cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(row_chevron(1), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected_row(&started, cx), Some(0));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    drive(&rt, cx, |_| true);
    assert_eq!(open_note(&started, cx), before);
    assert!(thread_shown(&started, cx));
}

/// No source to read, no header: a filter that leaves no row collapses the
/// reader and takes the header with it; New leaves no sources and none.
#[gpui_kit::test]
fn the_reader_header_needs_a_source(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    click(&started, source_row_id(1), cx);
    press(&started, "enter", cx);
    assert!(drawn(&started, READER_HEADER_ID, cx));
    click(&started, SOURCES_FILTER_ID, cx);
    type_in(&started, "zzz", cx);
    assert!(rows(&started, cx).is_empty());
    assert!(!drawn(&started, READER_HEADER_ID, cx));
    assert!(!drawn(&started, header_chevron(), cx));

    click(&started, NEW_ID, cx);
    assert!(rows(&started, cx).is_empty());
    with_sources(&started, cx, |view, _, cx| view.cycle(cx));
    assert_eq!(reader_state(&started, cx), (true, false, false));
    assert!(!drawn(&started, READER_HEADER_ID, cx));
}

/// The reader is pointed again only when what it should show changed: a
/// list notify that moves nothing leaves it alone; another row, or the
/// reader collapsing, points it again.
#[gpui_kit::test]
fn a_list_notify_that_moves_nothing_leaves_the_reader_alone(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![(
        "see [1]",
        vec![
            chunk("a.md", "Details", "alpha details here", 1),
            chunk("a.md", "Intro", "alpha intro", 2),
        ],
    )]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    click(&started, source_row_id(1), cx);
    press(&started, "right", cx);
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("a.md"))
    });
    let view = sources(&started, cx);
    let pointed = |cx: &TestAppContext| cx.read(|cx| view.read(cx).reader_pointed);
    let before = pointed(cx);
    let list = cx.read(|cx| view.read(cx).list.clone());
    cx.update(|cx| list.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert_eq!(
        pointed(cx),
        before,
        "nothing changed: the reader is not told"
    );
    press(&started, "down", cx);
    assert_eq!(pointed(cx), before + 1, "another section: told once");
    press(&started, "left", cx);
    assert_eq!(pointed(cx), before + 2, "collapsed: told to show nothing");
}

/// The filter narrows the rows by heading and path; Escape returns to the
/// list; a narrowed-to-nothing filter says "No results".
#[gpui_kit::test]
fn the_filter_narrows_the_rows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    click(&started, SOURCES_FILTER_ID, cx);
    type_in(&started, "beta", cx);
    assert_eq!(
        rows(&started, cx),
        [(2, path("b.md"))],
        "rank kept under the filter"
    );
    type_in(&started, "zzz", cx);
    assert!(rows(&started, cx).is_empty());
    assert_eq!(empty_text(&started, cx), NO_RESULTS);
    press(&started, "escape", cx);
    assert!(focused(&started, LIST_ROWS_ID, cx));
}

// ---- Citations ---------------------------------------------------------------------

/// The Source reader opens from a source (a row and Enter) and from a
/// citation: a click on `[n]` expands the sources (shut first), selects the
/// source whose ordinal is `n` (not the `n`-th row; the server's
/// out-of-order ordinals and the position fallback for `0` both resolve)
/// and opens the reader on it. An unknown `[n]` is not a link. A citation in
/// another turn selects that turn first.
#[gpui_kit::test]
fn source_reader_opens_from_a_source_and_from_a_citation(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let cited = vec![
        chunk("a.md", "Details", "alpha details here", 3),
        // No ordinal from the server: its position, 2.
        chunk("b.md", "Beta", "beta body", 0),
        chunk("c.md", "", "plain note", 1),
    ];
    let stub = Stub::with(vec![
        ("[3]\n\n[9]\n\n[1]", cited.clone()),
        ("other [1]", vec![chunk("b.md", "Beta", "beta body", 1)]),
    ]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    let turn = turn_ids(&started, cx)[0];
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md", "c.md"]));

    // From a source: its row, Enter.
    click(&started, source_row_id(2), cx);
    press(&started, "enter", cx);
    assert_eq!(reader_state(&started, cx), (false, true, false));
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("b.md"))
    });
    press(&started, "escape", cx);
    assert_eq!(reader_state(&started, cx), (true, false, false));

    // From a citation, the sources shut first: [1] is the third row, read.
    press(&started, "escape", cx);
    press(&started, "escape", cx);
    press(&started, "s", cx);
    assert!(!sources_open(&started, cx));
    click_line_start(&started, turn, 4, cx);
    assert!(sources_open(&started, cx));
    assert!(drawn(&started, SOURCES_PART_ID, cx));
    assert_eq!(selected_row(&started, cx), Some(2));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("c.md"))
    });
    // [3]: the first row, the reader on it.
    click_line_start(&started, turn, 0, cx);
    assert_eq!(selected_row(&started, cx), Some(0));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("a.md"))
    });
    // [9]: no such source, nothing moves.
    click_line_start(&started, turn, 2, cx);
    assert_eq!(selected_row(&started, cx), Some(0));
    // The ordinal fallback: [2] is b.md.
    let second = cx.read(|cx| {
        let view = started.workspace.read(cx).ask.clone().unwrap();
        view.read(cx).thread().turns()[0]
            .source_for_citation(2)
            .map(|s| s.path.clone())
    });
    assert_eq!(second, Some(path("b.md")));

    // From a newer turn, a click on the first turn's [1]: that turn and
    // its third source.
    ask(&started, &rt, "again", cx);
    assert_eq!(rows(&started, cx), ranked(&["b.md"]));
    click_line_start(&started, turn, 4, cx);
    assert_eq!(listed_turn(&started, cx), Some(turn));
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md", "c.md"]));
    assert_eq!(selected_row(&started, cx), Some(2));
    assert_eq!(reader_state(&started, cx), (false, true, false));
}

// ---- Leader ------------------------------------------------------------------------

/// Leader `a s` reads the selected turn's top source with the sources list
/// focused; with ASK hidden and no server to answer, it is blocked with
/// the TUI's message (palette and leader).
#[gpui_kit::test]
fn leader_a_s_reads_the_top_source(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    leader(&started, LeaderAction::AskSource, cx);
    assert!(focused(&started, LIST_ROWS_ID, cx));
    assert_eq!(selected_row(&started, cx), Some(0));
    assert_eq!(reader_state(&started, cx), (false, true, false));
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("a.md"))
    });

    // ASK hidden (its rail cell) and the server search-only.
    click(&started, rail_cell_id(DrawerView::Ask), cx);
    assert!(!thread_shown(&started, cx));
    status(&started, SEARCH_ONLY, cx);
    let block = cx.read(|cx| {
        let w = started.workspace.read(cx);
        w.route_block(&w.route_of(LeaderAction::AskSource), Panel::Editor, cx)
    });
    assert_eq!(block, Some(NO_LLM));
    leader(&started, LeaderAction::AskSource, cx);
    assert_eq!(started.flash(cx).as_deref(), Some(NO_LLM));
    assert!(!thread_shown(&started, cx));
}

/// The copy chord on the Sources list copies the source's path (the TUI's
/// yank).
#[gpui_kit::test]
fn the_copy_chord_copies_the_source_path(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    click(&started, source_row_id(2), cx);
    press(&started, "ctrl-c", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, Some(path("b.md").to_string()));
    assert_eq!(started.flash(cx).as_deref(), Some("path copied"));
}

// ---- Beside the note -------------------------------------------------------------

/// Every door that opens a note opens it in the editor and leaves ASK where
/// it is (the right slot, its home), its sources still listed: FILES, an
/// open request, a followed link, the finder.
#[gpui_kit::test]
fn each_door_opens_the_note_beside_ask(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    let stays = |cx: &mut TestAppContext| {
        assert!(thread_shown(&started, cx));
        assert_eq!(slot_views(&started, Side::Right, cx), [DrawerView::Ask]);
        assert_eq!(slot_views(&started, Side::Left, cx), [DrawerView::Files]);
        assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));
    };

    // FILES.
    let listed = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone().unwrap();
            let pane = files.read(cx);
            pane.model().core().and_then(|core| {
                core.visible_rows()
                    .into_iter()
                    .find(|r| r.path().is_like(&path("b.md")))
                    .map(|r| row_id(r).to_string())
            })
        })
    };
    drive(&rt, cx, |cx| listed(cx).is_some());
    let row = listed(cx).expect("b.md is listed");
    click(&started, SharedString::from(row), cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&path("b.md")))
    });
    stays(cx);

    // An open request (the follow / OpenPath door).
    send(&started, AppMessage::open(path("a.md")), cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&path("a.md")))
    });
    stays(cx);

    // A followed link.
    started.workspace.update(cx, |w, cx| {
        w.followed("b", Ok(FollowOutcome::Note(path("b.md"))), cx)
    });
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&path("b.md")))
    });
    stays(cx);

    // The finder.
    press(&started, "ctrl-o", cx);
    type_in(&started, "c", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            started
                .workspace
                .read(cx)
                .picker
                .as_ref()
                .and_then(|p| p.upgrade())
                .is_some_and(|p| !p.read(cx).is_loading())
        })
    });
    let wanted = cx.read(|cx| {
        let picker = started
            .workspace
            .read(cx)
            .picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .unwrap();
        picker
            .read(cx)
            .selected()
            .map(|r| r.path().clone())
            .unwrap()
    });
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&wanted))
    });
    stays(cx);
}

/// Another view shown in ASK's slot (FILES moved there) hides ASK as any
/// view sharing a slot is hidden; the thread is kept, and a click on the
/// ASK cell shows it again there, the rows on the selected turn.
#[gpui_kit::test]
fn another_view_in_asks_slot_hides_it_and_keeps_the_thread(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    click(&started, slot_id(DrawerView::Files), cx);
    press(&started, "alt-shift-right", cx);
    assert!(!thread_shown(&started, cx));
    assert_eq!(slot_views(&started, Side::Right, cx), [DrawerView::Files]);
    assert_eq!(turn_ids(&started, cx).len(), 1);
    click(&started, rail_cell_id(DrawerView::Ask), cx);
    assert!(thread_shown(&started, cx));
    assert_eq!(slot_views(&started, Side::Right, cx), [DrawerView::Ask]);
    assert!(focused(&started, COMPOSER_ID, cx));
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));
}

/// ASK's placement is persisted like any view's: the layout with it shown
/// is recorded with it, and comes back with ASK shown in the same slot.
#[gpui_kit::test]
fn ask_placement_is_persisted(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    let mut gui = GuiSettings::default();
    let live = cx.read(|cx| started.workspace.read(cx).layout.clone());
    crate::shell_tree::record(&live, &mut gui);
    let back = crate::shell_tree::restore(&gui, crate::shell_tree::RESTORE_CAPS);
    let slot = live.view_slot(&DrawerView::Ask).unwrap();
    assert_eq!(back.view_slot(&DrawerView::Ask), Some(slot));
    assert_eq!(back.shown(slot), Some(DrawerView::Ask));
}

/// A click on a source only selects it (C1: the Sources view's selection
/// drives the reader, so the reader follows and nothing opens); a
/// double-click opens the source's note (not the reader). A right click
/// opens nothing either (its menu is the `Open note` door).
#[gpui_kit::test]
fn a_click_selects_a_source_and_a_double_click_opens_its_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![(
        "see [1]",
        vec![
            chunk("a.md", "Details", "alpha details here", 1),
            chunk("b.md", "Beta", "beta body", 2),
        ],
    )]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    let before = open_note(&started, cx);
    click(&started, source_row_id(2), cx);
    press(&started, "enter", cx);
    assert_eq!(reader_state(&started, cx), (false, true, false));
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("b.md"))
    });

    // A click: the selection moves, the reader follows, nothing opens.
    click(&started, source_row_id(1), cx);
    assert_eq!(selected_row(&started, cx), Some(0));
    settle(&rt, cx, |cx| {
        reader_note(&started, cx) == Some(path("a.md"))
    });
    assert_eq!(
        reader_state(&started, cx),
        (false, true, false),
        "a click does not cycle"
    );
    assert!(thread_shown(&started, cx));
    assert_eq!(open_note(&started, cx), before);

    // A right click: its menu, nothing opened.
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(source_row_id(2), cx)
    })
    .unwrap();
    cx.run_until_parked();
    press(&started, "escape", cx);
    assert!(thread_shown(&started, cx));
    assert_eq!(open_note(&started, cx), before);

    // A double-click on the second source: its note opens.
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(source_row_id(2), cx)
    })
    .unwrap();
    drive(&rt, cx, |cx| open_note(&started, cx) == Some(path("b.md")));
    assert!(thread_shown(&started, cx), "ASK stays beside the note");
}

/// The two doors onto a source's note: `o` and the row menu's `Open note`.
/// Each opens the note of the SELECTED source (the second row, not the
/// first) in the editor; ASK stays.
#[gpui_kit::test]
fn a_source_opens_its_note_by_key_and_menu(cx: &mut TestAppContext) {
    use super::drawer_list::{DrawerListEvent, OPEN_NOTE};

    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![(
        "see [1]",
        vec![
            chunk("a.md", "Details", "alpha details here", 1),
            chunk("b.md", "Beta", "beta body", 2),
        ],
    )]);
    let (_v, started) = ready(cx, &rt, stub);
    ask(&started, &rt, "why?", cx);
    let reenter = |cx: &mut TestAppContext| {
        press(&started, "f6", cx);
        drive(&rt, cx, |cx| thread_shown(&started, cx));
    };

    // `o`.
    click(&started, source_row_id(2), cx);
    assert!(focused(&started, LIST_ROWS_ID, cx));
    press(&started, "o", cx);
    drive(&rt, cx, |cx| open_note(&started, cx) == Some(path("b.md")));
    assert!(thread_shown(&started, cx));
    send(&started, AppMessage::open(path("a.md")), cx);
    drive(&rt, cx, |cx| open_note(&started, cx) == Some(path("a.md")));

    // The row menu's `Open note` (its event, as the menu item emits it).
    reenter(cx);
    click(&started, source_row_id(2), cx);
    let view = sources(&started, cx);
    let list = cx.read(|cx| view.read(cx).list.clone());
    let row = cx.read(|cx| list.read(cx).selected_row().cloned()).unwrap();
    list.update(cx, |_, cx| cx.emit(DrawerListEvent::Verb(row, OPEN_NOTE)));
    drive(&rt, cx, |cx| open_note(&started, cx) == Some(path("b.md")));
    assert!(thread_shown(&started, cx));
}

/// ASK's side toggles like any view's: off hides ASK (the thread kept),
/// on shows it again in its slot (no "only beside a Thread" rule any more,
/// ADR-0053).
#[gpui_kit::test]
fn toggling_asks_side_hides_and_restores_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    let toggle = |cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| {
            started
                .workspace
                .update(cx, |w, cx| w.toggle_side(Side::Right, window, cx))
        })
        .unwrap();
    };
    toggle(cx);
    assert!(!side_visible(&started, Side::Right, cx));
    assert!(!thread_shown(&started, cx));
    toggle(cx);
    assert_eq!(slot_views(&started, Side::Right, cx), [DrawerView::Ask]);
    assert!(thread_shown(&started, cx));
    assert_eq!(turn_ids(&started, cx).len(), 1);
    assert_eq!(rows(&started, cx), ranked(&["a.md", "b.md"]));
}

/// ASK's slot header names the view, "Ask"; "Sources" is the accessible
/// name of the sources part inside it, not of the slot.
#[gpui_kit::test]
fn the_ask_slot_header_says_ask_and_the_part_says_sources(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(vec![("one [1]", a_and_b())]));
    ask(&started, &rt, "first", cx);
    assert!(sources_open(&started, cx));
    let headers: Vec<String> = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            let mut all = window.find_all("slot-header");
            all.extend(window.find_all("slot-header-focused"));
            all.iter()
                .filter_map(|h| h.label().map(str::to_string))
                .collect()
        })
        .unwrap();
    assert!(headers.iter().any(|h| h == "Ask"), "{headers:?}");
    assert!(!headers.iter().any(|h| h == "Sources"), "{headers:?}");
    assert_eq!(
        label_of(&started, SOURCES_PART_ID, cx).as_deref(),
        Some("Sources")
    );
}
