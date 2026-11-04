//! UI integration tests for the startup (M5b-2 G1): a real `Workspace` put
//! on a [`Start`] — an empty state, or a real vault on a temp dir — and its
//! dispatch of the app channel.
//!
//! The vault's calls run on tokio. GPUI's test scheduler rejects wakes from
//! other threads, so each test that touches a vault owns a current-thread
//! runtime, entered on the test thread, and drives it between GPUI's
//! `run_until_parked` ([`drive`]).
#![cfg(test)]

use std::sync::{Arc, RwLock};
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::settings::gui::GuiSettings;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::runtime::Runtime;

use super::{EMPTY_STATE_ID, LayoutStore, Workspace};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::shell_layout;
use crate::startup::{FirstNote, Start, VaultStart};
use crate::status_row::{FLASH_DURATION, FLASH_ID, INDEXING_ID, READ_ONLY_ID};
use crate::test_vault::{NAME, TestVault, test_vault};

pub(super) struct Started {
    pub(super) handle: AnyWindowHandle,
    pub(super) workspace: Entity<Workspace>,
}

impl Started {
    pub(super) fn text(&self, cx: &TestAppContext) -> String {
        cx.read(|cx| {
            let editor = self.workspace.read(cx).editor.clone();
            editor.read(cx).editor.get_text()
        })
    }

    pub(super) fn flash(&self, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .status
                .flash
                .as_ref()
                .map(|f| f.to_string())
        })
    }

    /// The editor Panel's empty-state title when a vault is open but no
    /// note is.
    pub(super) fn placeholder(&self, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| self.workspace.read(cx).note_empty_state().map(|e| e.title))
    }

    pub(super) fn indexing(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).status.indexing)
    }

    /// Whether the element `id` is in the last frame.
    pub(super) fn shows(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }
}

/// A runtime for the vault's calls, run only when [`drive`] drives it.
pub(super) fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// Run tokio and GPUI in turn until `done`, or fail after ~10 s.
pub(super) fn drive(
    rt: &Runtime,
    cx: &mut TestAppContext,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    for _ in 0..2000 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(5)));
        cx.run_until_parked();
        if done(cx) {
            return;
        }
    }
    panic!("timed out driving the startup");
}

/// A workspace started on `start`, as `main` builds it. Enter a runtime
/// first: `start` spawns the indexing on it.
pub(super) fn open_started(
    cx: &mut TestAppContext,
    start: Start,
    settings: SharedSettings,
) -> Started {
    open_started_with(cx, start, settings, false)
}

/// [`open_started`], with the vim editor when `vim`.
pub(super) fn open_started_with(
    cx: &mut TestAppContext,
    start: Start,
    settings: SharedSettings,
    vim: bool,
) -> Started {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let layout = shell_layout::restore(&GuiSettings::default());
    let (handle, workspace) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let editor =
                    cx.new(|cx| EditorView::on_channel(Theme::default(), vim, tx.clone(), cx));
                let workspace = cx.new(|cx| {
                    let mut workspace = Workspace::new(
                        editor,
                        Theme::default(),
                        layout,
                        LayoutStore::Off,
                        window,
                        cx,
                    );
                    workspace.start(start, settings, tx, rx, window, cx);
                    workspace
                });
                workspace.update(cx, |w, cx| {
                    w.apply_focus(window, cx);
                    w.guard_close(window, cx);
                });
                workspace
            })
        })
        .expect("open test window");
    Started { handle, workspace }
}

fn no_settings() -> SharedSettings {
    Arc::new(RwLock::new(AppSettings::default()))
}

pub(super) fn vault_start(v: &TestVault, first: FirstNote) -> Start {
    Start::Vault(VaultStart {
        vault: v.vault.clone(),
        name: NAME.into(),
        first,
    })
}

// ── empty states ─────────────────────────────────────────────────────────

#[gpui_kit::test]
fn no_workspace_shows_the_empty_state_instead_of_the_editor(cx: &mut TestAppContext) {
    let started = open_started(cx, Start::NoWorkspace, no_settings());
    assert!(started.shows(EMPTY_STATE_ID, cx));
    assert!(!started.shows("editor", cx), "no editor without a vault");
    assert!(started.shows("status-row", cx), "the footer stays docked");
    assert!(!started.indexing(cx), "nothing to index");
    let empty = cx.read(|cx| started.workspace.read(cx).empty.clone().unwrap());
    assert_eq!(empty.title, "No workspace configured");
}

#[gpui_kit::test]
fn a_vault_that_would_not_open_says_why(cx: &mut TestAppContext) {
    let started = open_started(
        cx,
        Start::OpenFailed("/notes: database is locked".into()),
        no_settings(),
    );
    assert!(started.shows(EMPTY_STATE_ID, cx));
    let empty = cx.read(|cx| started.workspace.read(cx).empty.clone().unwrap());
    assert_eq!(empty.detail, "/notes: database is locked");
    assert!(empty.failed);
}

// ── indexing and the restore ─────────────────────────────────────────────

/// The footer says "Indexing…" while the index catches up; then the session
/// lands on the newest note still on disk, and says the last one is gone.
#[gpui_kit::test]
fn the_restore_lands_on_the_last_live_note_after_indexing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(
        &[("still.md", "still here")],
        &["gone.md", "still.md"],
    ));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    assert!(started.indexing(cx));
    assert!(started.shows(INDEXING_ID, cx));
    // No editor to type into until the restore lands (G2 fix wave H1).
    assert!(started.shows(EMPTY_STATE_ID, cx));
    assert!(!started.shows("editor", cx));
    assert_eq!(
        started.placeholder(cx).as_deref(),
        Some("Opening your last note…")
    );

    drive(&rt, cx, |cx| started.text(cx) == "still here");
    assert!(!started.shows(EMPTY_STATE_ID, cx));
    assert!(started.shows("editor", cx));
    assert!(!started.indexing(cx));
    assert!(!started.shows(INDEXING_ID, cx));
    let flash = started.flash(cx).expect("the gone note is flashed");
    assert!(flash.contains("gone.md"), "{flash}");
    assert!(started.shows(FLASH_ID, cx));
    assert_eq!(v.history(), vec![VaultPath::new("still.md")]);
}

/// An empty history lands on the root: no note to show, no flash.
#[gpui_kit::test]
fn an_empty_history_leaves_the_editor_empty(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "a")], &[]));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    drive(&rt, cx, |cx| !started.indexing(cx));
    // Room for a stray open to land.
    drive(&rt, cx, |_| true);
    assert_eq!(started.text(cx), "");
    assert_eq!(started.flash(cx), None);
    assert!(started.shows(EMPTY_STATE_ID, cx));
    assert_eq!(started.placeholder(cx).as_deref(), Some("No note open"));
}

// ── the command line ─────────────────────────────────────────────────────

#[gpui_kit::test]
fn the_note_named_on_the_command_line_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(
        &[("a.md", "the history's"), ("b.md", "the argument's")],
        &["a.md"],
    ));
    let first = FirstNote::Note(VaultPath::new("b.md"));
    let started = open_started(cx, vault_start(&v, first), v.settings.clone());
    drive(&rt, cx, |cx| {
        !started.indexing(cx) && !started.text(cx).is_empty()
    });
    // Indexing done, and no restore came after it.
    drive(&rt, cx, |_| true);
    assert_eq!(started.text(cx), "the argument's");
}

#[gpui_kit::test]
fn a_missing_note_on_the_command_line_flashes_not_found(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[], &[]));
    let first = FirstNote::Note(VaultPath::new("missing.md"));
    let started = open_started(cx, vault_start(&v, first), v.settings.clone());
    drive(&rt, cx, |cx| started.flash(cx).is_some());
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.contains("missing.md") && flash.contains("not found"),
        "{flash}"
    );
    assert_eq!(started.text(cx), "");
    assert!(started.shows(EMPTY_STATE_ID, cx));
    assert_eq!(started.placeholder(cx).as_deref(), Some("No note open"));
}

#[gpui_kit::test]
fn a_refused_argument_flashes_and_the_editor_starts_empty(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "a")], &["a.md"]));
    let first = FirstNote::Rejected("/etc/hosts is not in the workspace".into());
    let started = open_started(cx, vault_start(&v, first), v.settings.clone());
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("/etc/hosts is not in the workspace")
    );
    drive(&rt, cx, |cx| !started.indexing(cx));
    drive(&rt, cx, |_| true);
    assert_eq!(started.text(cx), "", "no restore behind a refused argument");
    assert_eq!(started.placeholder(cx).as_deref(), Some("No note open"));
}

// ── messages ─────────────────────────────────────────────────────────────

pub(super) fn send(started: &Started, message: AppMessage, cx: &mut TestAppContext) {
    started
        .workspace
        .update(cx, |w, cx| w.on_app_message(message, cx));
}

#[gpui_kit::test]
fn a_flash_goes_away_on_its_own(cx: &mut TestAppContext) {
    let started = open_started(cx, Start::NoWorkspace, no_settings());
    send(&started, AppMessage::FlashMessage("copied".into()), cx);
    assert_eq!(started.flash(cx).as_deref(), Some("copied"));
    cx.executor().advance_clock(FLASH_DURATION / 2);
    cx.run_until_parked();
    // A newer flash replaces it and gets its own full time.
    send(&started, AppMessage::FlashMessage("pasted".into()), cx);
    cx.executor()
        .advance_clock(FLASH_DURATION / 2 + Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(started.flash(cx).as_deref(), Some("pasted"));
    cx.executor().advance_clock(FLASH_DURATION);
    cx.run_until_parked();
    assert_eq!(started.flash(cx), None);
    assert!(!started.shows(FLASH_ID, cx) || started.flash(cx).is_none());
}

/// Names differing only by case: no saving (G2 honours `read_only`), said
/// once in a flash and for good in the footer.
#[gpui_kit::test]
fn a_case_conflict_makes_notes_read_only(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[], &[]));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    send(
        &started,
        AppMessage::VaultConflict(
            "Case-sensitivity conflicts detected in vault:\na.md / A.md\nb.md / B.md".into(),
        ),
        cx,
    );
    assert!(!started.indexing(cx));
    assert!(cx.read(|cx| started.workspace.read(cx).status.read_only));
    assert!(started.shows(READ_ONLY_ID, cx));
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.contains("2 names") && flash.contains("case"),
        "{flash}"
    );
}

/// The editor's background parse/layout jobs report on the window's
/// channel now: their `Redraw` still repaints the editor.
#[gpui_kit::test]
fn a_redraw_on_the_channel_repaints_the_editor(cx: &mut TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;
    let started = open_started(cx, Start::NoWorkspace, no_settings());
    let editor = cx.read(|cx| started.workspace.read(cx).editor.clone());
    let notified = Rc::new(Cell::new(0));
    let _observer = cx.update(|cx| {
        let notified = notified.clone();
        cx.observe(&editor, move |_, _| notified.set(notified.get() + 1))
    });
    let redraw = cx
        .update(|cx| editor.read(cx).editor.session().notifier.clone())
        .expect("the redraw notifier is installed");
    cx.run_until_parked();
    let before = notified.get();
    // What a finished job calls (from a tokio worker in production).
    redraw();
    cx.run_until_parked();
    assert!(notified.get() > before, "the editor was notified");
}

#[gpui_kit::test]
fn failed_indexing_is_flashed(cx: &mut TestAppContext) {
    let started = open_started(cx, Start::NoWorkspace, no_settings());
    started
        .workspace
        .update(cx, |w, _| w.status.indexing = true);
    send(
        &started,
        AppMessage::IndexingDone(Err("disk gone".into())),
        cx,
    );
    assert!(!started.indexing(cx));
    assert!(started.flash(cx).unwrap().contains("disk gone"));
}
