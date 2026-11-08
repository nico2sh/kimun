//! UI integration tests for the kimün server's status and the SEM view
//! (M7c G9): a real `Workspace` (a bare shell, or one on a real, indexed
//! temp vault), the server's status fed as the sync would report it
//! (`AppMessage::RagStatus`), and SEM's searches answered by a stub
//! transport (`RagClient::with_transport`). Nothing here starts the sync
//! or talks to a network.
//!
//! The source waits 250 ms before it asks (its own debounce, a tokio
//! timer): [`rows`] drives tokio in real time until the rows land.
#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, Focusable as _, SharedString, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::rag::{self, RagStatus};
use kimun_notes::server_client::{
    AnswerResult, ChunkResult, ContextSize, QueryTransport, RagClient, RagError,
};
use tokio::runtime::Runtime;

use super::drawer_list::DrawerList;
use super::modal::ModalKind;
use super::semantic::{
    BLANK_TEXT, NO_RESULTS_TEXT, SEMANTIC_INPUT_ID, SemanticView, semantic_row_id,
};
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use super::tests::{Shell, gui, open_default, open_shell, saved, temp_config};
use super::{LayoutStore, Workspace, rail_cell_id, slot_id};
use crate::startup::FirstNote;
use crate::status_row::{RAG_ID, ROW_ID, SAVE_ID};
use crate::test_vault::{TestVault, test_vault};

/// Every status the sync reports, and whether SEM is offered then.
const STATUSES: [(RagStatus, bool); 8] = [
    (RagStatus::Disabled, false),
    (RagStatus::Offline, false),
    (RagStatus::Unauthorized, false),
    (RagStatus::NotConfigured, false),
    (
        RagStatus::Syncing {
            llm_available: false,
        },
        true,
    ),
    (
        RagStatus::Syncing {
            llm_available: true,
        },
        true,
    ),
    (
        RagStatus::Online {
            llm_available: false,
        },
        true,
    ),
    (
        RagStatus::Online {
            llm_available: true,
        },
        true,
    ),
];

const ONLINE: RagStatus = RagStatus::Online {
    llm_available: false,
};

/// The TUI's flashes for SEM when the server cannot search.
const NOT_CONFIGURED: &str = "Set kimun_server_url in config to use semantic search";
const UNREACHABLE: &str = "Semantic search needs a reachable server with an embedder";

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

// ---- A stub server -------------------------------------------------------------

/// Answers every search with `hits` (path, title), best first, or with
/// `error`; counts the searches.
struct Stub {
    hits: Vec<(&'static str, &'static str)>,
    error: Option<&'static str>,
    searches: AtomicUsize,
}

impl Stub {
    fn new(hits: Vec<(&'static str, &'static str)>) -> Arc<Self> {
        Arc::new(Self {
            hits,
            error: None,
            searches: AtomicUsize::new(0),
        })
    }

    fn failing(error: &'static str) -> Arc<Self> {
        Arc::new(Self {
            hits: Vec::new(),
            error: Some(error),
            searches: AtomicUsize::new(0),
        })
    }

    fn searches(&self) -> usize {
        self.searches.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl QueryTransport for Stub {
    async fn search(
        &self,
        _query: &str,
        _context_size: Option<ContextSize>,
    ) -> Result<Vec<ChunkResult>, RagError> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.error {
            return Err(RagError::Protocol(error.to_string()));
        }
        Ok(self
            .hits
            .iter()
            .enumerate()
            .map(|(i, (p, title))| ChunkResult {
                path: path(p).to_string(),
                title: title.to_string(),
                date: None,
                content: String::new(),
                hash: String::new(),
                similarity_score: 1.0 - i as f64 / 10.0,
                ordinal: 0,
            })
            .collect())
    }

    async fn ask(
        &self,
        _query: &str,
        _history: &[(String, String)],
        _context_size: Option<ContextSize>,
    ) -> Result<AnswerResult, RagError> {
        Err(RagError::Protocol("not asked here".into()))
    }
}

// ---- Helpers -------------------------------------------------------------------

const NOTES: &[(&str, &str)] = &[
    ("alpha.md", "# Alpha\nthe first note"),
    ("beta.md", "# Beta\nthe second note"),
    ("gamma.md", "# Gamma\nthe third note"),
];

/// A vault holding [`NOTES`], started on `alpha.md`, indexed; SEM searches
/// through `stub`.
fn started_with(cx: &mut TestAppContext, rt: &Runtime, stub: Arc<Stub>) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("alpha.md"))),
        v.settings.clone(),
    );
    drive(rt, cx, |cx| {
        started.text(cx) == NOTES[0].1 && !started.indexing(cx)
    });
    let client = Arc::new(RagClient::with_transport(stub));
    let view = semantic(&started, cx);
    view.update(cx, |view, _| view.use_client(client));
    (v, started)
}

fn semantic(s: &Started, cx: &TestAppContext) -> Entity<SemanticView> {
    cx.read(|cx| s.workspace.read(cx).semantic.clone().expect("SEM is built"))
}

fn semantic_list(s: &Started, cx: &TestAppContext) -> Entity<DrawerList<FileListEntry>> {
    let view = semantic(s, cx);
    cx.read(|cx| view.read(cx).list.clone())
}

fn status(workspace: &Entity<Workspace>, status: RagStatus, cx: &mut TestAppContext) {
    workspace.update(cx, |w, cx| {
        w.on_app_message(AppMessage::RagStatus(status), cx)
    });
    cx.run_until_parked();
}

/// Whether the rail shows SEM's cell in a fresh frame.
fn rail_has_sem(handle: gpui_kit::AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(rail_cell_id(DrawerView::Semantic))
            .is_some_and(|e| e.visible())
    })
    .unwrap()
}

/// Whether the element `id` is drawn in a fresh frame.
fn drawn(handle: gpui_kit::AnyWindowHandle, id: SharedString, cx: &mut TestAppContext) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

fn click(handle: gpui_kit::AnyWindowHandle, id: impl Into<SharedString>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
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

/// SEM's rows once its search landed: note names, in list order.
fn rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let list = semantic_list(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| list.read(cx).is_loading()));
    cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .map(|r| r.path().get_clean_name())
            .collect()
    })
}

fn empty_text(s: &Started, cx: &TestAppContext) -> String {
    let list = semantic_list(s, cx);
    cx.read(|cx| list.read(cx).empty_text().to_string())
}

fn open_note(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    cx.read(|cx| s.workspace.read(cx).notes.as_ref()?.note.clone())
}

fn shown(workspace: &Entity<Workspace>, view: DrawerView, cx: &TestAppContext) -> bool {
    cx.read(|cx| workspace.read(cx).is_shown(view))
}

fn cursor(shell: &Shell, cx: &TestAppContext) -> Option<DrawerView> {
    cx.read(|cx| shell.workspace.read(cx).rail_cursor_view())
}

/// Name a server in the settings (after the start: no sync runs).
fn configure_server(v: &TestVault) {
    v.settings
        .write()
        .unwrap()
        .workspace_config
        .as_mut()
        .expect("the test config has a workspace config")
        .global
        .kimun_server_url = Some("http://kimun.invalid".into());
}

/// SEM shown from its rail cell (the server can search), its field
/// focused.
fn reveal(s: &Started, cx: &mut TestAppContext) {
    status(&s.workspace, ONLINE, cx);
    click(s.handle, rail_cell_id(DrawerView::Semantic), cx);
    assert!(shown(&s.workspace, DrawerView::Semantic, cx));
}

// ---- Status → rail -------------------------------------------------------------

/// SEM's cell is on the rail exactly while the server can search, ASK's
/// while it can answer; the capabilities are kept (the Thread's client).
#[gpui_kit::test]
fn the_rail_offers_sem_only_while_the_server_can_search(cx: &mut TestAppContext) {
    let shell = open_default(cx, "");
    assert!(!rail_has_sem(shell.handle, cx), "Disabled at the start");
    for (s, offered) in STATUSES {
        status(&shell.workspace, s, cx);
        assert_eq!(rail_has_sem(shell.handle, cx), offered, "{s:?}");
        assert_eq!(
            drawn(shell.handle, rail_cell_id(DrawerView::Ask), cx),
            s.llm_available(),
            "{s:?}: ASK's cell"
        );
        let caps = cx.read(|cx| shell.workspace.read(cx).rag_caps());
        assert_eq!(caps.semantic, offered, "{s:?}");
        assert_eq!(caps.ask, s.llm_available(), "{s:?}");
    }
}

/// A SEM view opened while the server could search stays in its drawer
/// when the server goes away: only the rail cell goes.
#[gpui_kit::test]
fn losing_search_drops_the_cell_and_keeps_the_placed_view(cx: &mut TestAppContext) {
    let shell = open_default(cx, "");
    status(&shell.workspace, ONLINE, cx);
    click(shell.handle, rail_cell_id(DrawerView::Semantic), cx);
    assert!(shown(&shell.workspace, DrawerView::Semantic, cx));
    status(&shell.workspace, RagStatus::Offline, cx);
    assert!(!rail_has_sem(shell.handle, cx));
    assert!(shown(&shell.workspace, DrawerView::Semantic, cx));
    assert!(drawn(shell.handle, slot_id(DrawerView::Semantic), cx));
    assert_eq!(
        cx.read(|cx| shell.workspace.read(cx).shown_on(Side::Left)),
        [DrawerView::Semantic]
    );
}

/// The rail's keyboard cursor keeps its view when SEM's cell comes or
/// goes; on SEM itself it stays at its place, clamped.
#[gpui_kit::test]
fn the_rail_cursor_follows_its_view_across_a_flip(cx: &mut TestAppContext) {
    let shell = open_default(cx, "");
    let place = |view: DrawerView, cx: &mut TestAppContext| {
        shell
            .workspace
            .update(cx, |w, _| w.place_rail_cursor(Some(view)));
    };
    status(&shell.workspace, ONLINE, cx);
    for view in [DrawerView::Tags, DrawerView::Config, DrawerView::Files] {
        place(view, cx);
        status(&shell.workspace, RagStatus::Offline, cx);
        assert_eq!(cursor(&shell, cx), Some(view), "{view:?} after the drop");
        status(&shell.workspace, ONLINE, cx);
        assert_eq!(cursor(&shell, cx), Some(view), "{view:?} after the return");
    }
    // On SEM: its place, now Tags'.
    place(DrawerView::Semantic, cx);
    status(&shell.workspace, RagStatus::Offline, cx);
    assert_eq!(cursor(&shell, cx), Some(DrawerView::Tags));
    // The keys walk the shorter rail to its end.
    shell.workspace.update(cx, |w, _| w.rail_cursor = 99);
    status(&shell.workspace, RagStatus::Unauthorized, cx);
    shell.workspace.update(cx, |w, _| w.place_rail_cursor(None));
    assert_eq!(cursor(&shell, cx), Some(DrawerView::Config));
}

/// A layout saved with SEM placed comes back with the server off: SEM in
/// its slot, no cell; and a save keeps it there.
#[gpui_kit::test]
fn a_persisted_sem_placement_survives_the_server_off(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let settings = gui(r#"
[gui.layout]
split = "horizontal"
children = [
    { weight = 0.2, views = ["files"], shown = "files" },
    { weight = 0.6, editor = true },
    { weight = 0.2, views = ["semantic"], shown = "semantic" },
]
"#);
    let shell = open_shell(
        cx,
        1000.,
        &settings,
        "",
        false,
        LayoutStore::File(config.clone()),
    );
    assert!(!rail_has_sem(shell.handle, cx));
    assert!(drawn(shell.handle, slot_id(DrawerView::Semantic), cx));
    assert!(shown(&shell.workspace, DrawerView::Semantic, cx));
    // A save (the right side hidden, then shown again).
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.toggle_drawer(Side::Right, window, cx);
            w.toggle_drawer(Side::Right, window, cx);
        })
    })
    .unwrap();
    let back = crate::shell_tree::restore(&saved(&config), crate::shell_tree::RESTORE_CAPS);
    let right = *back.slots().last().unwrap();
    assert_eq!(back.shown(right), Some(DrawerView::Semantic));
}

// ---- The sync's lifecycle ------------------------------------------------------

/// No server named: nothing is started, the status stays Disabled, the
/// Footer says nothing about a server.
#[gpui_kit::test]
fn no_server_url_starts_no_sync(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, Stub::new(Vec::new()));
    assert!(!rag::rag_configured(&v.settings));
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert!(!w.rag_sync_running());
        assert_eq!(w.rag_status, RagStatus::Disabled);
        assert_eq!(w.status.rag, None);
    });
    assert!(!rail_has_sem(started.handle, cx));
    assert!(!drawn(started.handle, RAG_ID.into(), cx));
}

// ---- The Footer ----------------------------------------------------------------

/// The Footer words the server's state as the TUI does, and says nothing
/// once it is Disabled; in a narrow window the save state stays in view.
#[gpui_kit::test]
fn the_footer_shows_the_server_state(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, Stub::new(Vec::new()));
    let segment = |cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(RAG_ID)
                .filter(|e| e.visible())
                .and_then(|e| e.label().map(str::to_string))
        })
        .unwrap()
    };
    assert_eq!(segment(cx), None);
    status(&started.workspace, ONLINE, cx);
    assert_eq!(segment(cx).as_deref(), Some("rag: online"));
    status(&started.workspace, RagStatus::Offline, cx);
    assert_eq!(segment(cx).as_deref(), Some("rag: offline"));
    status(
        &started.workspace,
        RagStatus::Syncing {
            llm_available: true,
        },
        cx,
    );
    assert_eq!(segment(cx).as_deref(), Some("rag: syncing"));
    // Narrow: the server's state gives way, the save state stays.
    cx.simulate_window_resize(
        started.handle,
        gpui_kit::size(gpui_kit::px(520.), gpui_kit::px(480.)),
    );
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        let row = window.find(ROW_ID).bounds();
        let save = window.find(SAVE_ID).bounds();
        assert!(save.right() <= row.right(), "{save:?} in {row:?}");
        if let Some(rag) = window.try_find(RAG_ID).filter(|e| e.visible()) {
            assert!(rag.bounds().right() <= row.right(), "the segment stays in");
        }
    })
    .unwrap();
    status(&started.workspace, RagStatus::Disabled, cx);
    assert_eq!(segment(cx), None);
}

// ---- Opening SEM when the server cannot search ---------------------------------

/// The palette's reason and the leader's flash are the TUI's words; SEM
/// does not open. Once the server can search, it opens with its field
/// focused.
#[gpui_kit::test]
fn opening_sem_while_unavailable_flashes_why(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, Stub::new(Vec::new()));
    let action = LeaderAction::OpenDrawer(DrawerView::Semantic);
    let reason = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.route_block(&w.route_of(action), Panel::Editor, cx)
        })
    };
    // No server named.
    assert_eq!(reason(cx), Some(NOT_CONFIGURED));
    send(&started, AppMessage::ExecuteLeaderAction(action), cx);
    cx.run_until_parked();
    assert_eq!(started.flash(cx).as_deref(), Some(NOT_CONFIGURED));
    assert!(!shown(&started.workspace, DrawerView::Semantic, cx));
    // Named, but unreachable.
    configure_server(&v);
    status(&started.workspace, RagStatus::Offline, cx);
    assert_eq!(reason(cx), Some(UNREACHABLE));
    send(&started, AppMessage::ExecuteLeaderAction(action), cx);
    cx.run_until_parked();
    assert_eq!(started.flash(cx).as_deref(), Some(UNREACHABLE));
    assert!(!shown(&started.workspace, DrawerView::Semantic, cx));
    // Online: it opens, the field has the keyboard.
    status(&started.workspace, ONLINE, cx);
    assert_eq!(reason(cx), None);
    send(&started, AppMessage::ExecuteLeaderAction(action), cx);
    cx.run_until_parked();
    assert!(shown(&started.workspace, DrawerView::Semantic, cx));
    let view = semantic(&started, cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(window.find(SEMANTIC_INPUT_ID).focused(), Some(true));
    })
    .unwrap();
}

// ---- Searching -----------------------------------------------------------------

/// A blank query asks nothing and says how to start; a typed one is asked
/// once typing pauses (not before the source's 250 ms), and lists one row
/// per note in the server's order.
#[gpui_kit::test]
fn a_query_lists_one_row_per_note_in_server_order(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::new(vec![
        ("gamma.md", "Gamma"),
        ("alpha.md", "Alpha > Intro"),
        ("alpha.md", "Alpha > More"),
        ("beta.md", ""),
    ]);
    let (_v, started) = started_with(cx, &rt, stub.clone());
    reveal(&started, cx);
    let view = semantic(&started, cx);
    assert!(cx.read(|cx| view.read(cx).is_installed()));
    assert!(rows(&started, &rt, cx).is_empty());
    assert_eq!(empty_text(&started, cx), BLANK_TEXT);
    assert_eq!(stub.searches(), 0, "a blank query asks nothing");

    type_in(&started, "first", cx);
    rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(100)));
    cx.run_until_parked();
    assert_eq!(stub.searches(), 0, "still waiting for typing to pause");
    assert_eq!(rows(&started, &rt, cx), ["gamma", "alpha", "beta"]);
    assert_eq!(stub.searches(), 1);
    let list = semantic_list(&started, cx);
    let titles: Vec<String> = cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .map(|r| match r {
                FileListEntry::Note { title, .. } => title.clone(),
                _ => String::new(),
            })
            .collect()
    });
    // The best chunk's title per note; a blank one says so.
    assert_eq!(titles, ["Gamma", "Alpha > Intro", "<no title>"]);
    assert!(drawn(
        started.handle,
        semantic_row_id(&path("gamma.md")),
        cx
    ));
}

/// A failed search lists nothing and says "No results"; while the server
/// cannot search the list says why instead.
#[gpui_kit::test]
fn a_failed_search_lists_nothing_and_says_why(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::failing("server error");
    let (v, started) = started_with(cx, &rt, stub.clone());
    reveal(&started, cx);
    type_in(&started, "anything", cx);
    drive(&rt, cx, |_| stub.searches() == 1);
    assert!(rows(&started, &rt, cx).is_empty());
    assert_eq!(empty_text(&started, cx), NO_RESULTS_TEXT);
    // The server goes; the view stays and says why.
    status(&started.workspace, RagStatus::Offline, cx);
    assert!(shown(&started.workspace, DrawerView::Semantic, cx));
    assert_eq!(empty_text(&started, cx), NOT_CONFIGURED);
    configure_server(&v);
    status(&started.workspace, RagStatus::Unauthorized, cx);
    assert_eq!(empty_text(&started, cx), UNREACHABLE);
}

/// Enter on a result opens its note (Down takes the keyboard from the
/// field to the results); Escape gives it back to the editor.
#[gpui_kit::test]
fn enter_opens_the_selected_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::new(vec![("gamma.md", "Gamma"), ("beta.md", "Beta")]);
    let (_v, started) = started_with(cx, &rt, stub);
    reveal(&started, cx);
    type_in(&started, "third", cx);
    assert_eq!(rows(&started, &rt, cx), ["gamma", "beta"]);
    press(&started, "down", cx);
    let list = semantic_list(&started, cx);
    cx.update_window(started.handle, |_, window, cx| {
        assert!(list.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    // Up on the first row: back to the field; Down again.
    press(&started, "up", cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(SEMANTIC_INPUT_ID).focused(), Some(true));
    })
    .unwrap();
    press(&started, "down", cx);
    press(&started, "down", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&path("beta.md")))
    });
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).focused_panel()),
        Panel::Editor
    );
    // Escape from the field: the editor.
    click(started.handle, SEMANTIC_INPUT_ID, cx);
    press(&started, "escape", cx);
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).focused_panel()),
        Panel::Editor
    );
}

/// A click opens (C1); a right-click shows the FILES entry menu for the
/// row's note, and opens nothing.
#[gpui_kit::test]
fn a_result_clicks_open_and_has_the_entry_menu(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let stub = Stub::new(vec![("gamma.md", "Gamma"), ("beta.md", "Beta")]);
    let (_v, started) = started_with(cx, &rt, stub);
    reveal(&started, cx);
    type_in(&started, "note", cx);
    assert_eq!(rows(&started, &rt, cx), ["gamma", "beta"]);
    let row = semantic_row_id(&path("beta.md"));
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(row.clone(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    press(&started, "down", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Rename));
    let target = cx.read(|cx| match started.workspace.read(cx).file_dialog.clone() {
        Some(super::file_ops::FileDialogView::Rename(view)) => {
            view.upgrade().unwrap().read(cx).target().clone()
        }
        _ => panic!("no rename dialog"),
    });
    assert!(target.is_like(&path("beta.md")));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(
        !open_note(&started, cx).is_some_and(|n| n.is_like(&path("beta.md"))),
        "the right click opened nothing"
    );

    let gamma = semantic_row_id(&path("gamma.md"));
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(gamma, cx);
    })
    .unwrap();
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&path("gamma.md")))
    });
}
