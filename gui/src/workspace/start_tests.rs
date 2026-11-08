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

use super::modal::ModalKind;
use super::{EMPTY_STATE_ID, LayoutStore, Workspace};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::startup::{FirstNote, Start, VaultStart};
use gpui_kit::component::WindowExt as _;

use super::{FLASH_MESSAGE_ID, MAX_FLASH_TOASTS};

/// The kit's autohide for a toast (`NotificationList::push` gives every
/// autohiding toast five seconds, with no setter): what these tests age
/// toasts by. Pinned here, not in the app: nothing else depends on it.
const KIT_AUTOHIDE: Duration = Duration::from_secs(5);
use crate::status_row::{INDEXING_ID, READ_ONLY_ID, ROW_ID};
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

    /// The newest flash, `None` when nothing was flashed.
    pub(super) fn flash(&self, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .last_flash()
                .map(|(text, _)| text.to_string())
        })
    }

    /// How many toasts the window's notification layer holds now.
    pub(super) fn toasts(&self, cx: &mut TestAppContext) -> usize {
        cx.update_window(self.handle, |_, window, cx| window.notifications(cx).len())
            .unwrap()
    }

    /// Let toasts run their course for `by`, in steps short enough for the
    /// kit's lifecycle clock (it ticks every 50 ms) to see each phase.
    pub(super) fn age_toasts(&self, by: Duration, cx: &mut TestAppContext) {
        let step = Duration::from_millis(100);
        let mut left = by;
        while !left.is_zero() {
            let now = left.min(step);
            cx.executor().advance_clock(now);
            cx.run_until_parked();
            left -= now;
        }
    }

    /// The editor Panel's empty-state title when a vault is open but no
    /// note is.
    pub(super) fn placeholder(&self, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| self.workspace.read(cx).note_empty_state().map(|e| e.title))
    }

    pub(super) fn indexing(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).status.indexing)
    }

    /// The modal the window shows now (the kit's dialog layer agrees).
    pub(super) fn shown_modal(&self, cx: &mut TestAppContext) -> Option<ModalKind> {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |w, cx| w.open_modal(window, cx))
        })
        .unwrap()
    }

    /// Let a closing dialog finish leaving: the kit gives the keyboard back
    /// to its opener once its exit transition ends.
    pub(super) fn settle_dialog(&self, cx: &mut TestAppContext) {
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
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
    let layout =
        crate::shell_tree::restore(&GuiSettings::default(), crate::shell_tree::RESTORE_CAPS);
    let (handle, workspace) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let editor = cx.new(|cx| EditorView::on_channel(vim, tx.clone(), cx));
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

/// A window with no vault and no setup assistant over it (a workspace that
/// would not open): the flash and channel tests' bare window.
fn no_vault() -> Start {
    Start::OpenFailed("no vault in this test".into())
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

/// No workspace: the setup assistant has the window (M8c), not the editor
/// Panel's empty state; the empty state under it says what is missing.
#[gpui_kit::test]
fn no_workspace_shows_the_setup_assistant_instead_of_the_editor(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let started = open_started(cx, Start::NoWorkspace, no_settings());
    // Its model is read on tokio: let that land.
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let view = started.workspace.read(cx).onboarding.clone().unwrap();
            view.read(cx).model().is_some()
        })
    });
    assert!(started.shows(super::onboarding::ONBOARDING_ID, cx));
    assert!(
        !started.shows(EMPTY_STATE_ID, cx),
        "the assistant takes the body"
    );
    assert!(!started.shows("editor", cx), "no editor without a vault");
    assert!(!started.shows("rail", cx), "no Panel under the assistant");
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
    assert_eq!(started.toasts(cx), 1, "shown as a toast");
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

/// A note named on the command line that is not there: the create dialog
/// offers it (M6 G4); nothing is made until it is confirmed.
#[gpui_kit::test]
fn a_missing_note_on_the_command_line_offers_to_create_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[], &[]));
    let first = FirstNote::Note(VaultPath::new("missing.md"));
    let started = open_started(cx, vault_start(&v, first), v.settings.clone());
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    assert!(!rt.block_on(v.vault.exists(&VaultPath::new("missing.md"))));
    started.settle_dialog(cx);
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
    // Flashed while the window was being built, before its Root existed:
    // the toast is pushed once that update is done.
    assert_eq!(started.toasts(cx), 1);
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
    let started = open_started(cx, no_vault(), no_settings());
    send(&started, AppMessage::FlashMessage("copied".into()), cx);
    cx.run_until_parked();
    assert_eq!(started.flash(cx).as_deref(), Some("copied"));
    assert_eq!(started.toasts(cx), 1);
    started.age_toasts(KIT_AUTOHIDE / 2, cx);
    // The same flash again replaces its toast and gets its own full time.
    send(&started, AppMessage::FlashMessage("copied".into()), cx);
    cx.run_until_parked();
    assert_eq!(started.toasts(cx), 1, "replaced, not stacked");
    started.age_toasts(KIT_AUTOHIDE / 2 + Duration::from_secs(1), cx);
    assert_eq!(started.toasts(cx), 1, "the repeat restarted its time");
    started.age_toasts(KIT_AUTOHIDE, cx);
    assert_eq!(started.toasts(cx), 0);
}

/// Different flashes stack, each its own toast; a repeat of one of them
/// replaces it rather than adding a third.
#[gpui_kit::test]
fn different_flashes_stack_and_a_repeat_replaces(cx: &mut TestAppContext) {
    let started = open_started(cx, no_vault(), no_settings());
    send(&started, AppMessage::FlashMessage("copied".into()), cx);
    send(&started, AppMessage::FlashMessage("pasted".into()), cx);
    cx.run_until_parked();
    assert_eq!(started.toasts(cx), 2);
    send(&started, AppMessage::FlashMessage("copied".into()), cx);
    cx.run_until_parked();
    assert_eq!(started.toasts(cx), 2);
    let flashes = cx.read(|cx| started.workspace.read(cx).flashes.clone());
    assert_eq!(
        flashes,
        [
            ("copied".into(), false),
            ("pasted".into(), false),
            ("copied".into(), false)
        ]
    );
}

/// Past [`MAX_FLASH_TOASTS`] distinct flashes, the oldest toast goes for
/// each new one; a repeat of one that is up still only replaces it.
#[gpui_kit::test]
fn the_toast_stack_is_bounded(cx: &mut TestAppContext) {
    let started = open_started(cx, no_vault(), no_settings());
    for n in 0..MAX_FLASH_TOASTS + 2 {
        send(
            &started,
            AppMessage::FlashMessage(format!("notice {n}")),
            cx,
        );
    }
    cx.run_until_parked();
    // Past the evicted toasts' exit, well inside the others' time.
    started.age_toasts(Duration::from_millis(500), cx);
    assert_eq!(started.toasts(cx), MAX_FLASH_TOASTS);
    let newest = format!("notice {}", MAX_FLASH_TOASTS + 1);
    send(&started, AppMessage::FlashMessage(newest), cx);
    cx.run_until_parked();
    started.age_toasts(Duration::from_millis(500), cx);
    assert_eq!(started.toasts(cx), MAX_FLASH_TOASTS, "a repeat replaces");
    let labels = toast_labels(&started, cx);
    assert!(
        !labels.iter().any(|l| l == "notice 0" || l == "notice 1"),
        "the oldest went: {labels:?}"
    );
    assert!(labels.iter().any(|l| l == "notice 2"), "{labels:?}");
}

/// The accessible names of the flash messages up now.
fn toast_labels(started: &Started, cx: &mut TestAppContext) -> Vec<String> {
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all(FLASH_MESSAGE_ID)
            .into_iter()
            .filter_map(|e| e.label().map(str::to_string))
            .collect()
    })
    .unwrap()
}

/// A toast is announced with its words: the kit's toast is an alert, and
/// its message a label named by the flash's text.
#[gpui_kit::test]
fn a_toast_is_an_alert_named_by_its_text(cx: &mut TestAppContext) {
    const TEXT: &str = "a notice to read aloud";
    let started = open_started(cx, no_vault(), no_settings());
    started.workspace.update(cx, |w, cx| w.flash(TEXT, cx));
    cx.run_until_parked();
    started.age_toasts(Duration::from_millis(500), cx);
    assert_eq!(toast_labels(&started, cx), [TEXT]);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        let message = window.find(FLASH_MESSAGE_ID);
        assert_eq!(message.role(), Some(gpui_kit::Role::Label));
        let alert = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|e| e.role() == Some(gpui_kit::Role::Alert))
            .expect("the toast is an alert");
        assert!(
            message.path().starts_with(alert.path()),
            "the message is inside the alert: {:?} / {:?}",
            message.path(),
            alert.path()
        );
    })
    .unwrap();
}

/// `flash` is information and `flash_error` an error, recorded as such;
/// the kit draws them with its info and error icons.
#[gpui_kit::test]
fn flash_and_flash_error_map_to_info_and_error(cx: &mut TestAppContext) {
    use gpui_kit::component::notification::NotificationType;
    assert!(matches!(super::toast_kind(false), NotificationType::Info));
    assert!(matches!(super::toast_kind(true), NotificationType::Error));
    let started = open_started(cx, no_vault(), no_settings());
    started.workspace.update(cx, |w, cx| {
        w.flash("a notice", cx);
        w.flash_error("a problem", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).flashes.clone()),
        [("a notice".into(), false), ("a problem".into(), true)]
    );
    assert_eq!(started.toasts(cx), 2);
}

/// A toast sits in the window's bottom-right corner, clear of the Footer's
/// row; the Footer itself shows no flash text any more.
#[gpui_kit::test]
fn a_toast_sits_bottom_right_and_the_footer_shows_no_flash(cx: &mut TestAppContext) {
    const TEXT: &str = "a notice for the corner";
    let started = open_started(cx, no_vault(), no_settings());
    started.workspace.update(cx, |w, cx| w.flash(TEXT, cx));
    cx.run_until_parked();
    // Past the enter transition, so the toast is at rest.
    started.age_toasts(Duration::from_millis(500), cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        let size = window.viewport_size();
        // The toast's close button (its one element the kit registers),
        // at the toast's top right.
        let close = window.find_all("close");
        assert_eq!(close.len(), 1);
        let close = close[0].bounds();
        assert!(
            close.right() > size.width * 0.9 && close.top() > size.height * 0.5,
            "bottom-right: {close:?} in {size:?}"
        );
        let row = window.find(ROW_ID);
        assert!(!row.label().is_some_and(|l| l.contains(TEXT)));
        let in_row = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|e| e.path().contains(&ROW_ID.into()))
            .filter_map(|e| e.label().map(str::to_string))
            .collect::<Vec<_>>();
        assert!(
            in_row.iter().all(|label| !label.contains(TEXT)),
            "{in_row:?}"
        );
    })
    .unwrap();
}

/// A window whose root is not the kit's `Root` has no toast layer: a
/// flash there is logged and dropped, never a panic.
#[gpui_kit::test]
fn a_flash_without_a_toast_layer_is_dropped(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let layout =
        crate::shell_tree::restore(&GuiSettings::default(), crate::shell_tree::RESTORE_CAPS);
    let handle = cx
        .update(|cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let editor = cx.new(|cx| EditorView::new(None, false, cx));
                cx.new(|cx| {
                    Workspace::new(
                        editor,
                        Theme::default(),
                        layout,
                        LayoutStore::Off,
                        window,
                        cx,
                    )
                })
            })
        })
        .expect("open a bare window");
    handle
        .update(cx, |workspace, _, cx| workspace.flash("nowhere to go", cx))
        .unwrap();
    cx.run_until_parked();
    let flashes = handle
        .update(cx, |workspace, _, _| workspace.flashes.clone())
        .unwrap();
    assert_eq!(flashes, [("nowhere to go".into(), false)]);
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
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.contains("2 names") && flash.contains("case"),
        "{flash}"
    );
    // Preferences > Workspaces show the vault's words (M9b-2); closing them
    // leaves the read-only vault with its Footer mark.
    let prefs = cx
        .read(|cx| started.workspace.read(cx).preferences.clone())
        .expect("Preferences show the conflict");
    let (section, conflict) = cx.read(|cx| {
        let p = prefs.read(cx);
        (p.section(), p.conflict().map(str::to_string))
    });
    assert_eq!(section, super::preferences::Section::Workspaces);
    assert!(conflict.unwrap().contains("a.md / A.md"));
    drive(&rt, cx, |cx| cx.read(|cx| prefs.read(cx).model().is_some()));
    cx.update_window(started.handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(started.shows(READ_ONLY_ID, cx));
}

/// The editor's background parse/layout jobs report on the window's
/// channel now: their `Redraw` still repaints the editor.
#[gpui_kit::test]
fn a_redraw_on_the_channel_repaints_the_editor(cx: &mut TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;
    let started = open_started(cx, no_vault(), no_settings());
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
    let started = open_started(cx, no_vault(), no_settings());
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
