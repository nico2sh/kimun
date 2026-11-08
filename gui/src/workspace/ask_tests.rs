//! UI integration tests for the Thread (M7c G10a): a real `Workspace` on a
//! real, indexed temp vault, the server's status fed as the sync reports it
//! (`AppMessage::RagStatus`), and the questions answered by a stub
//! transport (`RagClient::with_transport`) the Thread gets whenever the
//! status says the server can answer. Nothing here talks to a network.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the ask jobs, the
//! flushes and the free-path lookups run there.
#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::ask::{TurnStatus, citations, save};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::logic::message::{AppMessage, AskData, FileOp};
use kimun_notes::rag::RagStatus;
use kimun_notes::server_client::{
    AnswerResult, ChunkResult, ContextSize, QueryTransport, RagClient, RagError,
};
use tokio::runtime::Runtime;

use super::ask::{
    AskView, COMPOSER_DISABLED, COMPOSER_ID, COMPOSER_LABEL_ID, COMPOSER_TITLE, COPIED, COPY_ID,
    LineKind, NEW_ID, REGENERATE_ID, RunStyle, SAVE_ID, SEND_ID, THINKING, TURNS_ID, answer_lines,
    turn_answer_id, turn_status_id,
};
use super::files::row_id;
use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
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

/// The answer the stub gives when no reply is queued.
const ANSWER: &str = "**Alpha** says hi [1].";

const NOTES: &[(&str, &str)] = &[("a.md", "alpha"), ("b.md", "beta")];

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

// ---- A stub server -------------------------------------------------------------

type Asked = Vec<(String, Vec<(String, String)>)>;

/// Answers each question with the next queued reply (else [`ANSWER`]),
/// citing `a.md`; records what was asked.
#[derive(Default)]
struct Stub {
    replies: Mutex<VecDeque<Result<String, String>>>,
    asked: Mutex<Asked>,
}

impl Stub {
    fn with(replies: Vec<Result<&str, &str>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|r| r.map(str::to_string).map_err(str::to_string))
                    .collect(),
            ),
            asked: Mutex::default(),
        })
    }

    fn asked(&self) -> Asked {
        self.asked.lock().unwrap().clone()
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
        query: &str,
        history: &[(String, String)],
        _context_size: Option<ContextSize>,
    ) -> Result<AnswerResult, RagError> {
        self.asked
            .lock()
            .unwrap()
            .push((query.to_string(), history.to_vec()));
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(ANSWER.to_string()));
        match reply {
            Ok(answer) => Ok(AnswerResult {
                answer,
                sources: vec![ChunkResult {
                    path: path("a.md").to_string(),
                    title: "Alpha".to_string(),
                    date: None,
                    content: "alpha".to_string(),
                    hash: String::new(),
                    similarity_score: 0.9,
                    ordinal: 1,
                }],
            }),
            Err(e) => Err(RagError::Protocol(e)),
        }
    }
}

// ---- Helpers -------------------------------------------------------------------

/// A vault holding [`NOTES`], started on `a.md`, indexed; the Thread asks
/// `stub` once the server can answer, which it now can.
fn ready(cx: &mut TestAppContext, rt: &Runtime, stub: Arc<Stub>) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(rt, cx, |cx| {
        started.text(cx) == "alpha" && !started.indexing(cx)
    });
    let client = Arc::new(RagClient::with_transport(stub));
    started
        .workspace
        .update(cx, |w, _| w.ask_test_client = Some(client));
    status(&started, ONLINE, cx);
    assert!(has_client(&started, cx));
    (v, started)
}

fn status(s: &Started, status: RagStatus, cx: &mut TestAppContext) {
    send(s, AppMessage::RagStatus(status), cx);
    cx.run_until_parked();
}

fn view(s: &Started, cx: &TestAppContext) -> Entity<AskView> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .ask
            .clone()
            .expect("the Thread is built")
    })
}

/// Whether the turn `id`'s answer is drawn (made when it landed).
fn drawn(s: &Started, id: u64, cx: &TestAppContext) -> bool {
    let view = view(s, cx);
    cx.read(|cx| view.read(cx).has_drawn_answer(id))
}

fn has_client(s: &Started, cx: &TestAppContext) -> bool {
    let view = view(s, cx);
    cx.read(|cx| view.read(cx).has_client())
}

fn shown(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).ask_shown())
}

/// The turns: question, and the answer, `…` while pending, or the error.
fn turns(s: &Started, cx: &TestAppContext) -> Vec<(String, String)> {
    let view = view(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .thread()
            .turns()
            .iter()
            .map(|t| {
                let state = match &t.status {
                    TurnStatus::Done => t.answer.clone(),
                    TurnStatus::Error(e) => format!("error: {e}"),
                    _ => "…".to_string(),
                };
                (t.question.clone(), state)
            })
            .collect()
    })
}

fn selected(s: &Started, cx: &TestAppContext) -> Option<String> {
    let view = view(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .thread()
            .selected()
            .map(|t| t.question.clone())
    })
}

fn turn_ids(s: &Started, cx: &TestAppContext) -> Vec<u64> {
    let view = view(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .thread()
            .turns()
            .iter()
            .map(|t| t.id)
            .collect()
    })
}

fn pending(s: &Started, cx: &TestAppContext) -> bool {
    turns(s, cx).iter().any(|(_, state)| state == "…")
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

fn click(s: &Started, id: impl Into<gpui_kit::SharedString>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

/// The accessible name of the element `id` in a fresh frame, if drawn.
fn label(
    s: &Started,
    id: impl Into<gpui_kit::SharedString>,
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

fn focused(s: &Started, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

fn open_note(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    cx.read(|cx| s.workspace.read(cx).notes.as_ref()?.note.clone())
}

fn disk(rt: &Runtime, v: &TestVault, p: &VaultPath) -> String {
    rt.block_on(v.vault.get_note_text(p)).unwrap()
}

/// F6, then `question` and Enter; driven until it is answered.
fn ask(s: &Started, rt: &Runtime, question: &str, cx: &mut TestAppContext) {
    if !shown(s, cx) {
        press(s, "f6", cx);
        drive(rt, cx, |cx| shown(s, cx));
    }
    type_in(s, question, cx);
    press(s, "enter", cx);
    drive(rt, cx, |cx| !pending(s, cx));
}

fn leader(s: &Started, action: LeaderAction, cx: &mut TestAppContext) {
    send(s, AppMessage::ExecuteLeaderAction(action), cx);
    cx.run_until_parked();
}

// ---- The answer's lines (no window) ----------------------------------------------

/// Bold, italic and struck runs lose their sigils; inline code stays as it
/// is (never a citation); `[n]` is a citation; code, heading and quote
/// lines are verbatim in their role.
#[test]
fn an_answer_is_drawn_from_the_editors_markdown_model() {
    let lines = answer_lines(
        "A **bold** and *it* ~~x~~ `[2]` see [1].\n\n# Head [3]\n> quoted\n```\nlet **a** = 1;\n```",
    );
    let first = &lines[0];
    assert_eq!(first.kind, LineKind::Normal);
    assert_eq!(first.text.as_ref(), "A bold and it x `[2]` see [1].");
    let at = |needle: &str| {
        let start = first.text.find(needle).unwrap();
        start..start + needle.len()
    };
    let style_of = |range: std::ops::Range<usize>| {
        first
            .runs
            .iter()
            .find(|(r, _)| *r == range)
            .map(|(_, s)| *s)
    };
    assert_eq!(
        style_of(at("bold")),
        Some(RunStyle {
            bold: true,
            ..RunStyle::default()
        })
    );
    assert!(style_of(at("it")).is_some_and(|s| s.italic));
    assert!(style_of(at("x")).is_some_and(|s| s.strike));
    assert_eq!(
        style_of(at("`[2]`")),
        Some(RunStyle {
            code: true,
            ..RunStyle::default()
        }),
        "inline code is verbatim and never a citation"
    );
    assert_eq!(
        style_of(at("[1]")),
        Some(RunStyle {
            citation: true,
            ..RunStyle::default()
        })
    );
    assert_eq!(lines[1].text.as_ref(), "");
    assert_eq!(
        (lines[2].kind, lines[2].text.as_ref()),
        (LineKind::Heading, "# Head [3]")
    );
    assert_eq!(
        (lines[3].kind, lines[3].text.as_ref()),
        (LineKind::Quote, "> quoted")
    );
    assert_eq!(
        (lines[5].kind, lines[5].text.as_ref()),
        (LineKind::Code, "let **a** = 1;")
    );
    assert!(lines[5].runs.is_empty());
}

// ---- Asking --------------------------------------------------------------------

/// F6 shows the Thread with the composer focused; Enter asks; the pending
/// turn says "… thinking"; the answer lands and is drawn; the next question
/// carries the first exchange as history.
#[gpui_kit::test]
fn a_question_is_asked_and_its_answer_drawn(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(Vec::new());
    let (_v, started) = ready(cx, &rt, stub.clone());
    press(&started, "f6", cx);
    drive(&rt, cx, |cx| shown(&started, cx));
    assert!(
        focused(&started, COMPOSER_ID, cx),
        "the composer has the keyboard"
    );
    assert_eq!(
        label(&started, COMPOSER_LABEL_ID, cx).as_deref(),
        Some(COMPOSER_TITLE)
    );

    type_in(&started, "why?", cx);
    press(&started, "enter", cx);
    let id = turn_ids(&started, cx)[0];
    assert_eq!(turns(&started, cx), [("why?".to_string(), "…".to_string())]);
    assert_eq!(
        label(&started, turn_status_id(id), cx).as_deref(),
        Some(THINKING)
    );
    assert_eq!(
        {
            let view = view(&started, cx);
            cx.read(|cx| view.read(cx).composer_text(cx))
        },
        "",
        "the composer is emptied"
    );
    drive(&rt, cx, |cx| !pending(&started, cx));
    assert_eq!(
        turns(&started, cx),
        [("why?".to_string(), ANSWER.to_string())]
    );
    assert_eq!(
        label(&started, turn_answer_id(id), cx).as_deref(),
        Some("Alpha says hi [1].")
    );
    assert_eq!(stub.asked(), [("why?".to_string(), Vec::new())]);

    type_in(&started, "and then?", cx);
    click(&started, SEND_ID, cx);
    drive(&rt, cx, |cx| {
        !pending(&started, cx) && turns(&started, cx).len() == 2
    });
    assert_eq!(
        stub.asked()[1],
        (
            "and then?".to_string(),
            vec![("why?".to_string(), citations::strip(ANSWER))]
        )
    );
    assert_eq!(selected(&started, cx).as_deref(), Some("and then?"));
}

/// A blank question asks nothing; without a server nothing is asked
/// either, and no turn is left thinking.
#[gpui_kit::test]
fn a_blank_question_or_no_server_asks_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(Vec::new());
    let (_v, started) = ready(cx, &rt, stub.clone());
    press(&started, "f6", cx);
    drive(&rt, cx, |cx| shown(&started, cx));
    type_in(&started, "   ", cx);
    press(&started, "enter", cx);
    assert!(turns(&started, cx).is_empty());

    status(&started, SEARCH_ONLY, cx);
    assert!(!has_client(&started, cx));
    let view = view(&started, cx);
    cx.update_window(started.handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.composer
                .update(cx, |c, cx| c.set_value("why?", window, cx))
        })
    })
    .unwrap();
    press(&started, "enter", cx);
    click(&started, SEND_ID, cx);
    assert!(turns(&started, cx).is_empty());
    assert!(stub.asked().is_empty());
}

/// An answer for a turn the thread no longer waits on changes nothing: an
/// unknown id, a turn already answered, a turn cleared by New.
#[gpui_kit::test]
fn a_stale_answer_is_dropped(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    let id = turn_ids(&started, cx)[0];
    let stale = |turn_id: u64| {
        AppMessage::Ask(AskData::AnswerReady {
            turn_id,
            result: Ok(("stale".to_string(), Vec::new())),
        })
    };
    send(&started, stale(id + 99), cx);
    send(&started, stale(id), cx);
    cx.run_until_parked();
    assert_eq!(
        turns(&started, cx),
        [("why?".to_string(), ANSWER.to_string())]
    );
    // A stale answer is never drawn; the accepted one still shows.
    assert!(!drawn(&started, id + 99, cx));
    assert_eq!(
        label(&started, turn_answer_id(id), cx).as_deref(),
        Some("Alpha says hi [1].")
    );
    leader(&started, LeaderAction::AskNew, cx);
    assert!(turns(&started, cx).is_empty());
    assert!(!drawn(&started, id, cx), "New drops the drawn answers");
    send(&started, stale(id), cx);
    cx.run_until_parked();
    assert!(turns(&started, cx).is_empty());
    assert!(!drawn(&started, id, cx));
}

/// A failed answer shows "✗ …" and Regenerate (r on the turns) asks the
/// same question again.
#[gpui_kit::test]
fn a_failure_shows_and_regenerate_retries(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![Err("the model is asleep")]);
    let (_v, started) = ready(cx, &rt, stub.clone());
    ask(&started, &rt, "why?", cx);
    let id = turn_ids(&started, cx)[0];
    assert_eq!(
        turns(&started, cx),
        [("why?".to_string(), "error: the model is asleep".to_string())]
    );
    assert_eq!(
        label(&started, turn_status_id(id), cx).as_deref(),
        Some("✗ the model is asleep")
    );
    // Escape: the turn list; r regenerates.
    press(&started, "escape", cx);
    assert!(focused(&started, TURNS_ID, cx));
    press(&started, "r", cx);
    drive(&rt, cx, |cx| !pending(&started, cx));
    assert_eq!(
        turns(&started, cx),
        [("why?".to_string(), ANSWER.to_string())]
    );
    assert_eq!(stub.asked().len(), 2);
    assert_eq!(stub.asked()[1].0, "why?");
}

/// Regenerate on an older turn rewinds it and asks its question again, with
/// the history before it; the newer turn stays.
#[gpui_kit::test]
fn regenerate_rewinds_and_asks_again(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(vec![Ok("one"), Ok("two"), Ok("one again")]);
    let (_v, started) = ready(cx, &rt, stub.clone());
    ask(&started, &rt, "first", cx);
    ask(&started, &rt, "second", cx);
    // The composer is empty: Up selects the older turn.
    press(&started, "up", cx);
    assert_eq!(selected(&started, cx).as_deref(), Some("first"));
    let first = turn_ids(&started, cx)[0];
    assert_eq!(
        label(&started, turn_answer_id(first), cx).as_deref(),
        Some("one")
    );
    click(&started, REGENERATE_ID, cx);
    assert_eq!(turns(&started, cx)[0].1, "…");
    assert!(!drawn(&started, first, cx), "the rewound answer is dropped");
    drive(&rt, cx, |cx| !pending(&started, cx));
    assert_eq!(
        label(&started, turn_answer_id(first), cx).as_deref(),
        Some("one again"),
        "the new answer is drawn"
    );
    assert_eq!(
        turns(&started, cx),
        [
            ("first".to_string(), "one again".to_string()),
            ("second".to_string(), "two".to_string())
        ]
    );
    assert_eq!(
        stub.asked().last().cloned(),
        Some(("first".to_string(), Vec::new()))
    );
}

/// Up/Down in the empty composer, and Up/Down/k/j on the turn list, move
/// the selected turn.
#[gpui_kit::test]
fn selection_keys_move_the_selected_turn(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    for q in ["one", "two", "three"] {
        ask(&started, &rt, q, cx);
    }
    assert_eq!(selected(&started, cx).as_deref(), Some("three"));
    press(&started, "up", cx);
    press(&started, "up", cx);
    assert_eq!(selected(&started, cx).as_deref(), Some("one"));
    press(&started, "down", cx);
    assert_eq!(selected(&started, cx).as_deref(), Some("two"));
    // With text in it, Up is the composer's.
    type_in(&started, "x", cx);
    press(&started, "up", cx);
    assert_eq!(selected(&started, cx).as_deref(), Some("two"));
    press(&started, "escape", cx);
    assert!(focused(&started, TURNS_ID, cx));
    press(&started, "j", cx);
    assert_eq!(selected(&started, cx).as_deref(), Some("three"));
    press(&started, "k", cx);
    press(&started, "k", cx);
    assert_eq!(selected(&started, cx).as_deref(), Some("one"));
    press(&started, "down", cx);
    assert_eq!(selected(&started, cx).as_deref(), Some("two"));
    // i: back to the composer.
    press(&started, "i", cx);
    assert!(focused(&started, COMPOSER_ID, cx));
    // Escape twice: the turns, then the drawer (the editor is gone).
    press(&started, "escape", cx);
    press(&started, "escape", cx);
    let panel = cx.read(|cx| started.workspace.read(cx).focused_panel());
    assert!(matches!(panel, Panel::Drawer(_) | Panel::Rail), "{panel:?}");
    assert!(shown(&started, cx), "the Thread stays shown");
}

/// The server losing its LLM keeps the thread readable but disables
/// asking (the composer says so; Send, Regenerate and New are off, with the
/// TUI's reason); it coming back enables them again.
#[gpui_kit::test]
fn losing_the_server_keeps_the_thread_and_disables_asking(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::with(Vec::new());
    let (_v, started) = ready(cx, &rt, stub.clone());
    ask(&started, &rt, "why?", cx);
    let id = turn_ids(&started, cx)[0];
    status(&started, RagStatus::Offline, cx);
    assert!(!has_client(&started, cx));
    assert!(shown(&started, cx), "the Thread stays");
    assert_eq!(
        label(&started, COMPOSER_LABEL_ID, cx).as_deref(),
        Some(COMPOSER_DISABLED)
    );
    assert_eq!(
        label(&started, turn_answer_id(id), cx).as_deref(),
        Some("Alpha says hi [1].")
    );
    let view = view(&started, cx);
    cx.update_window(started.handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.composer
                .update(cx, |c, cx| c.set_value("again?", window, cx))
        })
    })
    .unwrap();
    let actions = cx.read(|cx| view.read(cx).actions(cx));
    assert!(!actions.send && !actions.regenerate && !actions.new);
    assert!(
        actions.copy && actions.save,
        "reading the thread still works"
    );
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        for id in [SEND_ID, REGENERATE_ID, NEW_ID] {
            assert_eq!(window.find(id).disabled(), Some(true), "{id}");
        }
        assert_ne!(window.find(COPY_ID).disabled(), Some(true));
    })
    .unwrap();
    // The leader's steps that need the server say why, in the TUI's words.
    let block = |action: LeaderAction, cx: &mut TestAppContext| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.route_block(&w.route_of(action), Panel::Editor, cx)
        })
    };
    assert_eq!(block(LeaderAction::AskNew, cx), Some(NO_LLM));
    assert_eq!(block(LeaderAction::AskRegenerate, cx), Some(NO_LLM));
    assert_eq!(block(LeaderAction::AskFocus, cx), None, "already shown");
    assert_eq!(block(LeaderAction::AskCopy, cx), None);
    leader(&started, LeaderAction::AskRegenerate, cx);
    assert_eq!(started.flash(cx).as_deref(), Some(NO_LLM));
    assert_eq!(stub.asked().len(), 1);

    status(&started, ONLINE, cx);
    assert!(has_client(&started, cx));
    assert_eq!(
        label(&started, COMPOSER_LABEL_ID, cx).as_deref(),
        Some(COMPOSER_TITLE)
    );
    click(&started, SEND_ID, cx);
    drive(&rt, cx, |cx| {
        turns(&started, cx).len() == 2 && !pending(&started, cx)
    });
    assert_eq!(stub.asked()[1].0, "again?");
}

// ---- The editor Panel ------------------------------------------------------------

/// Leader `a a` shows the Thread: the dirty note is flushed first, then let
/// go (no open note, an empty buffer), and note-scoped routes say there is
/// no note open.
#[gpui_kit::test]
fn leader_a_a_flushes_the_note_and_shows_the_thread(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    type_in(&started, "X", cx);
    press(&started, "ctrl-shift-b", cx);
    press(&started, "a", cx);
    press(&started, "a", cx);
    drive(&rt, cx, |cx| shown(&started, cx));
    assert_eq!(disk(&rt, &v, &path("a.md")), "Xalpha", "flushed first");
    assert_eq!(open_note(&started, cx), None);
    assert_eq!(started.text(cx), "");
    assert!(focused(&started, COMPOSER_ID, cx));
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert!(w.ask_flow().showing());
        assert!(!w.can_save());
        assert!(w.route_facts().open_note().is_none());
        assert_eq!(
            w.route_of(LeaderAction::NoteRename),
            LeaderRoute::NoNoteOpen
        );
        assert_eq!(w.focused_panel(), Panel::Editor);
    });
}

/// A flush that fails refuses the Thread: the note stays with its edits.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_flush_refuses_the_thread(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    let file = v.workspace.as_path().join("a.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    type_in(&started, "X", cx);
    press(&started, "f6", cx);
    drive(&rt, cx, |cx| started.flash(cx).is_some());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save a.md:") && flash.ends_with("Ask was not opened."),
        "{flash}"
    );
    assert!(!shown(&started, cx));
    assert!(open_note(&started, cx).is_some_and(|n| n.is_like(&path("a.md"))));
    assert_eq!(started.text(cx), "Xalpha");
}

/// A note opened from FILES, or from any other door (an open request),
/// takes the Thread down and brings the editor back; the thread stays, and
/// F6 shows its turns again. An attachment takes it down too.
#[gpui_kit::test]
fn opening_a_note_takes_the_thread_down_and_keeps_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    rt.block_on(v.vault.save_attachment(&path("notes.txt"), b"text"))
        .unwrap();
    // FILES stays on the left: the Thread's Sources take the right slot.
    ask(&started, &rt, "why?", cx);
    // FILES: double-click b.md.
    let row = cx.read(|cx| {
        let files = started.workspace.read(cx).files.clone().unwrap();
        let pane = files.read(cx);
        let core = pane.model().core().expect("a listing");
        core.visible_rows()
            .into_iter()
            .find(|r| r.path().is_like(&path("b.md")))
            .map(|r| row_id(r).to_string())
            .expect("b.md is listed")
    });
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(row, cx)
    })
    .unwrap();
    drive(&rt, cx, |cx| started.text(cx) == "beta");
    assert!(!shown(&started, cx));
    assert!(open_note(&started, cx).is_some_and(|n| n.is_like(&path("b.md"))));
    assert_eq!(turns(&started, cx).len(), 1, "the thread is kept");

    // F6: the old turns are back; an open request takes it down again.
    press(&started, "f6", cx);
    drive(&rt, cx, |cx| shown(&started, cx));
    let id = turn_ids(&started, cx)[0];
    assert_eq!(
        label(&started, turn_answer_id(id), cx).as_deref(),
        Some("Alpha says hi [1].")
    );
    send(&started, AppMessage::open(path("a.md")), cx);
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    assert!(!shown(&started, cx));

    // An attachment shown takes it down as well.
    press(&started, "f6", cx);
    drive(&rt, cx, |cx| shown(&started, cx));
    send(&started, AppMessage::OpenAttachment(path("notes.txt")), cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| started.workspace.read(cx).shown_attachment(cx))
            .is_some()
    });
    assert!(!shown(&started, cx));
    assert_eq!(turns(&started, cx).len(), 1);
}

// ---- New, Copy, Save as note -----------------------------------------------------

/// New (the button, leader `a n`) clears the thread and gives the composer
/// the keyboard.
#[gpui_kit::test]
fn new_clears_the_thread(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    press(&started, "escape", cx);
    click(&started, NEW_ID, cx);
    assert!(turns(&started, cx).is_empty());
    assert!(focused(&started, COMPOSER_ID, cx));
    ask(&started, &rt, "again", cx);
    leader(&started, LeaderAction::AskNew, cx);
    assert!(turns(&started, cx).is_empty());
}

/// Copy puts the selected answer on the clipboard, citations stripped, and
/// says so (the TUI's words).
#[gpui_kit::test]
fn copy_puts_the_answer_on_the_clipboard(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    let thread = view(&started, cx);
    let expected = cx.read(|cx| thread.read(cx).copy_text()).unwrap();
    assert_eq!(expected, citations::strip(ANSWER));
    click(&started, COPY_ID, cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some(expected.as_str()));
    assert_eq!(started.flash(cx).as_deref(), Some(COPIED));
    // Leader `a y` does the same.
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("other".into()));
    leader(&started, LeaderAction::AskCopy, cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some(expected.as_str()));
}

/// Save as note offers the create dialog on `ask/<slug>` holding the turn;
/// confirming makes it (core) and opens it, taking the Thread down. With
/// that name taken, the dialog offers the free one and the note lands
/// there.
#[gpui_kit::test]
fn save_as_note_creates_the_note_under_ask(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    let thread = view(&started, cx);
    let (suggested, content) = cx.read(|cx| thread.read(cx).save_payload()).unwrap();
    assert_eq!(suggested, save::suggested_path("why?"));
    click(&started, SAVE_ID, cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&suggested))
    });
    assert_eq!(disk(&rt, &v, &suggested), content);
    assert!(!shown(&started, cx), "the note took the editor Panel");

    // Taken: the free name is offered (ShowCreateWithContent's door).
    let free = rt.block_on(v.vault.free_note_path(&suggested));
    assert!(!free.is_like(&suggested));
    send(
        &started,
        AppMessage::FileOp(FileOp::ShowCreateWithContent {
            path: suggested.clone(),
            content: content.clone(),
        }),
        cx,
    );
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&free))
    });
    assert_eq!(disk(&rt, &v, &free), content);
    assert_eq!(disk(&rt, &v, &suggested), content, "the first is untouched");
}

/// A saved answer that meets an open dialog is not "not found": the flash
/// says the answer was not saved and what to do.
#[gpui_kit::test]
fn a_saved_answer_meeting_a_modal_says_it_was_not_saved(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    ask(&started, &rt, "why?", cx);
    let thread = view(&started, cx);
    let (path, content) = cx.read(|cx| thread.read(cx).save_payload()).unwrap();
    send(
        &started,
        AppMessage::FileOp(FileOp::ShowCreateWithContent {
            path: path.clone(),
            content: content.clone(),
        }),
        cx,
    );
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    // The dialog is open: a second save meets it.
    send(
        &started,
        AppMessage::FileOp(FileOp::ShowCreateWithContent { path, content }),
        cx,
    );
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            started
                .workspace
                .read(cx)
                .last_flash()
                .is_some_and(|(text, error)| error && text.contains("save the answer"))
        })
    });
}

/// Send follows the composer's text across sends: enabled once something
/// is typed, back to disabled once the question went, enabled again on the
/// next question's first key (the redraw is gated on blank/non-blank
/// flipping, and a send clears the composer without a change event).
#[gpui_kit::test]
fn send_is_enabled_again_for_the_question_after_a_send(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    let send_disabled = |cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(SEND_ID).disabled()
        })
        .unwrap()
    };
    ask(&started, &rt, "why?", cx);
    assert_eq!(send_disabled(cx), Some(true), "empty after the send");
    type_in(&started, "a", cx);
    assert_ne!(send_disabled(cx), Some(true), "typing enables it");
}

// ---- Doors while the server cannot answer ---------------------------------------

/// With no LLM: the palette's `+ask` rows that need the server are blocked
/// with the TUI's message (the top source too, the Thread not showing),
/// Copy and Save are not; F6 and leader `a a` flash the message and show
/// nothing.
#[gpui_kit::test]
fn the_doors_say_why_while_the_server_cannot_answer(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = ready(cx, &rt, Stub::with(Vec::new()));
    status(&started, SEARCH_ONLY, cx);
    let reasons = cx.read(|cx| {
        let entries = started
            .workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx);
        let reason = |action: LeaderAction| {
            entries
                .iter()
                .find(|e| *e.run() == PaletteRun::Leaf(action))
                .map(|e| e.blocked())
        };
        [
            reason(LeaderAction::AskFocus),
            reason(LeaderAction::AskNew),
            reason(LeaderAction::AskRegenerate),
            reason(LeaderAction::AskCopy),
            reason(LeaderAction::AskSave),
            reason(LeaderAction::AskSource),
        ]
    });
    assert_eq!(
        reasons,
        [
            Some(Some(NO_LLM)),
            Some(Some(NO_LLM)),
            Some(Some(NO_LLM)),
            Some(None),
            Some(None),
            Some(Some(NO_LLM)),
        ]
    );
    press(&started, "f6", cx);
    assert_eq!(started.flash(cx).as_deref(), Some(NO_LLM));
    assert!(!shown(&started, cx));
    leader(&started, LeaderAction::AskFocus, cx);
    assert_eq!(started.flash(cx).as_deref(), Some(NO_LLM));
    assert!(!shown(&started, cx));
    assert!(open_note(&started, cx).is_some_and(|n| n.is_like(&path("a.md"))));
}
