//! UI integration tests for the update flow (M10b, [`update`]): a real
//! `Workspace` over a real temp vault, its release backend a scripted fake
//! (`logic::update::ScriptedUpdates`: no network, no files), driven by the
//! palette route, Preferences, the toast's action and the dialog's buttons
//! and keys. Each test owns a current-thread tokio runtime and drives it
//! between GPUI's `run_until_parked`: the backend's calls run on its
//! blocking pool.
//!
//! [`update`]: super::update
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Context, TestAppContext, Window, WindowOptions};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::leader_route::{LeaderJob, LeaderRoute};
use kimun_notes::logic::update::{
    self as rules, SCRIPTED_RELEASES_URL, ScriptedUpdates, UpdateBackend, UpdatePhase,
};
use kimun_notes::settings::SharedSettings;
use kimun_notes::settings::themes::Theme;
use kimun_notes::update::InstallChannel;
use tokio::runtime::Runtime;

use crate::relaunch::RecordingRelauncher;

use super::modal::ModalKind;
use super::note::Closing;
use super::preferences::{CHECK_UPDATES_ID, Section, UPDATE_CHECK_HELP, UPDATE_CHECK_HELP_ID};
use super::start_tests::{Started, drive, runtime, vault_start};
use super::update::{
    ACTION_TOAST_LIFETIME, INSTALL_LABEL, RETRY_LABEL, TITLE, TITLE_INSTALLED, UPDATE_CLOSE_ID,
    UPDATE_COMMAND_ID, UPDATE_DIALOG_ID, UPDATE_ERROR_ID, UPDATE_INSTALL_ID, UPDATE_LATER_ID,
    UPDATE_NOTES_ID, UPDATE_SKIP_ID, UPDATE_TEXT_ID, UPDATE_VERSIONS_ID, UpdateDialog,
};
use super::{FLASH_MESSAGE_ID, LayoutStore, TOAST_ACTION_ID, ToastAction, Workspace};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::startup::{FirstNote, Start};
use crate::test_vault::{TestVault, test_vault};

/// The version the fakes publish.
const NEW: &str = "999.1.0";

/// The window as `main` builds it, the update backend set before `start`
/// (as `main` sets the manual gate's fake).
fn open_with(
    cx: &mut TestAppContext,
    start: Start,
    settings: SharedSettings,
    backend: Option<Arc<ScriptedUpdates>>,
) -> Started {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let layout = crate::shell_tree::restore(&Default::default(), crate::shell_tree::RESTORE_CAPS);
    let (handle, workspace) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let editor = cx.new(|cx| EditorView::on_channel(false, tx.clone(), cx));
                let workspace = cx.new(|cx| {
                    let mut workspace = Workspace::new(
                        editor,
                        Theme::default(),
                        layout,
                        LayoutStore::Off,
                        window,
                        cx,
                    );
                    if let Some(backend) = backend {
                        workspace.set_update_backend(backend as Arc<dyn UpdateBackend>);
                    }
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

fn vault(rt: &Runtime) -> TestVault {
    rt.block_on(test_vault(&[("a.md", "alpha")], &[]))
}

/// The window on `v` with `a.md` open, the startup check (if any) landed.
fn on_vault(
    cx: &mut TestAppContext,
    rt: &Runtime,
    v: &TestVault,
    backend: Option<Arc<ScriptedUpdates>>,
) -> Started {
    let s = open_with(
        cx,
        vault_start(v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
        backend,
    );
    drive(rt, cx, |cx| !s.indexing(cx) && s.text(cx) == "alpha");
    drive(rt, cx, |cx| !s.up_checking(cx));
    s
}

fn set_update_check(v: &TestVault, on: bool) {
    let mut settings = v.settings.write().unwrap();
    settings
        .workspace_config
        .as_mut()
        .expect("the test config has a workspace config")
        .global
        .update_check = on;
}

impl Started {
    fn up_on_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
    ) -> R {
        let workspace = self.workspace.clone();
        let r = cx
            .update_window(self.handle, |_, window, cx| {
                workspace.update(cx, |w, cx| f(w, window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        r
    }

    fn up_checking(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).updates.machine.is_checking())
    }

    fn up_phase(&self, cx: &TestAppContext) -> UpdatePhase {
        cx.read(|cx| self.workspace.read(cx).updates.machine.phase().clone())
    }

    fn up_flashes(&self, cx: &TestAppContext) -> Vec<(String, bool)> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .flashes
                .iter()
                .map(|(t, e)| (t.to_string(), *e))
                .collect()
        })
    }

    fn up_dialog(&self, cx: &TestAppContext) -> Option<gpui_kit::Entity<UpdateDialog>> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .updates
                .dialog
                .as_ref()
                .and_then(|d| d.upgrade())
        })
    }

    fn up_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn up_press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    /// The accessible label of the element `id`, when it is shown.
    fn up_label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .filter(|e| e.visible())
                .and_then(|e| e.label().map(str::to_string))
        })
        .unwrap()
    }

    /// The accessible names of the flash toasts up now.
    fn up_toasts(&self, cx: &mut TestAppContext) -> Vec<String> {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .find_all(FLASH_MESSAGE_ID)
                .into_iter()
                .filter_map(|e| e.label().map(str::to_string))
                .collect()
        })
        .unwrap()
    }

    /// How many toast action buttons are up now.
    fn up_actions(&self, cx: &mut TestAppContext) -> usize {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_all(TOAST_ACTION_ID).len()
        })
        .unwrap()
    }

    fn up_disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).disabled() == Some(true)
        })
        .unwrap()
    }

    /// The palette route `v u`, checked unblocked.
    fn up_route(&self, cx: &mut TestAppContext) {
        self.up_on_window(cx, |w, window, cx| {
            let route = w.route_of(LeaderAction::AppCheckUpdates);
            assert_eq!(route, LeaderRoute::Job(LeaderJob::CheckUpdates));
            assert_eq!(w.route_block(&route, Panel::Editor, cx), None);
            w.perform_route(route, window, cx)
        });
    }

    /// A manual check that finds [`NEW`], landed, its toast up.
    fn up_found(&self, rt: &Runtime, cx: &mut TestAppContext) {
        self.up_route(cx);
        drive(rt, cx, |cx| !self.up_checking(cx));
        self.age_toasts(Duration::from_millis(500), cx);
    }

    /// The toast's `View`: the dialog.
    fn up_view(&self, cx: &mut TestAppContext) {
        self.up_click(TOAST_ACTION_ID, cx);
        assert_eq!(self.shown_modal(cx), Some(ModalKind::Update));
    }
}

// ---- The startup check -------------------------------------------------------

/// Off: the backend is never asked. On: one throttled check (`force`
/// false), once per launch.
#[gpui_kit::test]
fn the_startup_check_runs_only_when_the_setting_is_on(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    assert!(fake.checks().is_empty(), "off: never asked");
    assert!(s.up_flashes(cx).is_empty());

    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::up_to_date());
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    assert_eq!(fake.checks(), [false], "on: one throttled check");
    assert!(s.up_flashes(cx).is_empty(), "up to date: silent");
    // A later entry (a reopen) does not check again.
    s.up_on_window(cx, |w, _, cx| w.check_updates_on_start(cx));
    assert_eq!(fake.checks(), [false]);
}

/// Offline at startup: no toast, nothing known.
#[gpui_kit::test]
fn an_offline_startup_check_is_silent(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::offline("network unreachable"));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    assert_eq!(fake.checks(), [false]);
    assert!(s.up_flashes(cx).is_empty());
    assert_eq!(s.toasts(cx), 0);
    assert!(matches!(s.up_phase(cx), UpdatePhase::Idle));
}

/// A newer version: exactly one toast, "Kimün <v> is available", with
/// `View`; never repeated in the session.
#[gpui_kit::test]
fn a_found_version_is_announced_once_with_view(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.age_toasts(Duration::from_millis(500), cx);
    let text = rules::available_text(NEW);
    assert_eq!(s.up_toasts(cx), std::slice::from_ref(&text));
    assert_eq!(s.up_actions(cx), 1, "with View");
    assert_eq!(s.up_flashes(cx), [(text, false)]);
    // Re-entering the vault (a reopen) neither checks nor announces again.
    s.up_on_window(cx, |w, _, cx| w.check_updates_on_start(cx));
    assert_eq!(fake.checks().len(), 1);
    assert_eq!(s.up_flashes(cx).len(), 1);
    // Its View opens the dialog.
    s.up_view(cx);
    assert_eq!(
        s.up_label(UPDATE_VERSIONS_ID, cx),
        Some(format!(
            "Kimün {} → {NEW}",
            kimun_notes::update::CURRENT_VERSION
        ))
    );
}

/// A skipped version: no toast.
#[gpui_kit::test]
fn a_skipped_version_is_not_announced(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    fake.dismiss(NEW).unwrap();
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    assert_eq!(fake.checks(), [false]);
    assert!(s.up_flashes(cx).is_empty());
    assert!(matches!(s.up_phase(cx), UpdatePhase::Skipped(v) if v == NEW));
}

/// The check never holds the window up: the note opens and the window works
/// while the backend has not answered; the toast comes when it does.
#[gpui_kit::test]
fn the_startup_check_does_not_delay_the_window(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    fake.hold();
    let s = open_with(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
        Some(fake.clone()),
    );
    drive(&rt, cx, |cx| !s.indexing(cx) && s.text(cx) == "alpha");
    assert!(s.up_checking(cx), "still waiting on the backend");
    assert!(s.up_flashes(cx).is_empty());
    // The window is usable meanwhile: the palette opens.
    s.up_on_window(cx, |w, window, cx| w.open_palette(window, cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.up_press("escape", cx);
    s.settle_dialog(cx);
    fake.release();
    drive(&rt, cx, |cx| !s.up_checking(cx));
    assert_eq!(s.up_flashes(cx), [(rules::available_text(NEW), false)]);
}

/// Not while the setup assistant has the window; once it is gone, the next
/// entry checks.
#[gpui_kit::test]
fn the_startup_check_waits_for_the_setup_assistant(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::up_to_date());
    let s = open_with(
        cx,
        Start::NoWorkspace,
        v.settings.clone(),
        Some(fake.clone()),
    );
    drive(&rt, cx, |cx| {
        cx.read(|cx| s.workspace.read(cx).onboarding_open())
    });
    assert!(fake.checks().is_empty(), "not under the assistant");
    s.up_on_window(cx, |w, _, cx| {
        w.drop_onboarding();
        w.check_updates_on_start(cx);
    });
    drive(&rt, cx, |cx| !s.up_checking(cx));
    assert_eq!(fake.checks(), [false]);
}

// ---- The manual check ----------------------------------------------------------

/// The palette row: unthrottled, and it says the result: up to date.
#[gpui_kit::test]
fn the_palette_row_checks_and_reports_up_to_date(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::up_to_date());
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    let row = cx.read(|cx| {
        s.workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| e.label() == "+vault · check for updates")
            .expect("the palette lists it")
    });
    assert_eq!(row.blocked(), None, "no longer \"not yet\"");
    s.up_route(cx);
    drive(&rt, cx, |cx| !s.up_checking(cx));
    assert_eq!(fake.checks(), [true], "bypasses the throttle");
    assert_eq!(
        s.up_flashes(cx),
        [
            (rules::CHECKING_TEXT.to_string(), false),
            (
                rules::up_to_date_text(kimun_notes::update::CURRENT_VERSION),
                false
            ),
        ]
    );
}

/// Available: the toast with View opens the dialog; the palette row then
/// opens the dialog without checking again.
#[gpui_kit::test]
fn a_manual_check_that_finds_one_offers_view(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.up_found(&rt, cx);
    assert!(s.up_toasts(cx).contains(&rules::available_text(NEW)));
    s.up_view(cx);
    s.age_toasts(Duration::from_millis(500), cx);
    assert!(
        !s.up_toasts(cx).contains(&rules::available_text(NEW)),
        "View closed its toast"
    );
    s.up_press("escape", cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    s.up_route(cx);
    assert_eq!(
        s.shown_modal(cx),
        Some(ModalKind::Update),
        "known: the dialog"
    );
    assert_eq!(fake.checks(), [true], "no second check");
}

/// Offline: an error toast that says why.
#[gpui_kit::test]
fn a_manual_check_offline_says_so(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::offline("network unreachable"));
    let s = on_vault(cx, &rt, &v, Some(fake));
    s.up_found(&rt, cx);
    assert_eq!(
        s.up_flashes(cx).last(),
        Some(&(rules::check_failed_text("network unreachable"), true))
    );
}

/// A manual check while one runs joins it: one backend call, one report.
#[gpui_kit::test]
fn manual_checks_while_one_runs_are_coalesced(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::offline("network unreachable"));
    fake.hold();
    let s = open_with(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
        Some(fake.clone()),
    );
    drive(&rt, cx, |cx| !s.indexing(cx) && s.text(cx) == "alpha");
    assert!(s.up_checking(cx), "the startup check waits");
    s.up_route(cx);
    s.up_on_window(cx, |w, _, cx| w.check_updates_now(cx));
    assert_eq!(fake.checks(), [false], "joined, not started again");
    fake.release();
    drive(&rt, cx, |cx| !s.up_checking(cx));
    // The throttled startup check is followed by one forced check (both
    // asks coalesced into it), whose answer is the report.
    assert_eq!(fake.checks(), [false, true]);
    // Each manual ask said a check runs (no dead click), then the report.
    assert_eq!(
        s.up_flashes(cx),
        [
            (rules::CHECKING_TEXT.into(), false),
            (rules::CHECKING_TEXT.into(), false),
            (rules::check_failed_text("network unreachable"), true),
        ]
    );
}

/// A manual check while a throttled startup check runs: the startup
/// check's "nothing known" (the throttle, no network) is not reported as
/// up to date; a forced check runs after it and its answer is the report.
#[gpui_kit::test]
fn a_manual_check_during_a_throttled_startup_check_really_checks(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW).throttled());
    fake.hold();
    let s = open_with(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
        Some(fake.clone()),
    );
    drive(&rt, cx, |cx| !s.indexing(cx) && s.text(cx) == "alpha");
    assert!(s.up_checking(cx), "the startup check waits");
    s.up_route(cx);
    fake.release();
    drive(&rt, cx, |cx| !s.up_checking(cx));
    assert_eq!(fake.checks(), [false, true], "a forced check followed");
    assert_eq!(
        s.up_flashes(cx),
        [
            (rules::CHECKING_TEXT.into(), false),
            (rules::available_text(NEW), false),
        ]
    );
    assert!(matches!(s.up_phase(cx), UpdatePhase::Available(_)));
}

/// The dialog open on a known version while a check runs: it says the
/// check runs, Install and Skip are disabled (they could not act) and
/// Enter installs nothing; Release notes and Later still work. Once the
/// check lands, Install is back.
#[gpui_kit::test]
fn the_dialog_waits_on_a_running_check(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.up_found(&rt, cx);
    s.up_view(cx);
    assert!(!s.up_disabled(UPDATE_INSTALL_ID, cx));
    fake.hold();
    s.up_on_window(cx, |w, _, cx| w.check_updates_now(cx));
    assert!(s.up_checking(cx));
    assert_eq!(
        s.up_label(UPDATE_TEXT_ID, cx).as_deref(),
        Some(rules::CHECKING_TEXT)
    );
    assert!(
        s.up_label(UPDATE_VERSIONS_ID, cx).is_some(),
        "the version stays"
    );
    assert!(s.up_disabled(UPDATE_INSTALL_ID, cx), "Install waits");
    assert!(s.up_disabled(UPDATE_SKIP_ID, cx), "Skip waits");
    assert!(!s.up_disabled(UPDATE_NOTES_ID, cx));
    assert!(!s.up_disabled(UPDATE_LATER_ID, cx));
    s.up_press("enter", cx);
    assert!(!s.up_on_window(cx, |w, _, _| w.update_busy()));
    assert_eq!(fake.installs(), 0);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Update));
    fake.release();
    drive(&rt, cx, |cx| !s.up_checking(cx));
    assert!(!s.up_disabled(UPDATE_INSTALL_ID, cx), "Install is back");
    assert!(!s.up_disabled(UPDATE_SKIP_ID, cx));
    assert_ne!(
        s.up_label(UPDATE_TEXT_ID, cx).as_deref(),
        Some(rules::CHECKING_TEXT)
    );
    // Later still closes while a check runs.
    fake.hold();
    s.up_on_window(cx, |w, _, cx| w.check_updates_now(cx));
    s.up_click(UPDATE_LATER_ID, cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    fake.release();
    drive(&rt, cx, |cx| !s.up_checking(cx));
}

/// A failed install, then a check, then Later: the failure is cleaned up,
/// so a check that fails goes back to the version offered clean, not to
/// the failure.
#[gpui_kit::test]
fn later_on_a_failure_during_a_check_cleans_it_up(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW).install_fails("checksum mismatch"));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    let relauncher = Arc::new(RecordingRelauncher::default());
    s.up_on_window(cx, |w, _, _| w.set_relauncher(relauncher.clone()));
    s.up_found(&rt, cx);
    s.up_view(cx);
    s.up_click(UPDATE_INSTALL_ID, cx);
    drive(&rt, cx, |cx| {
        matches!(s.up_phase(cx), UpdatePhase::Failed { .. })
    });
    cx.run_until_parked();
    assert_eq!(relauncher.calls(), 0, "a failed install does not restart");
    assert_eq!(cx.read(|cx| s.workspace.read(cx).quits), 0);
    fake.hold();
    fake.go_offline("network unreachable");
    s.up_on_window(cx, |w, _, cx| w.check_updates_now(cx));
    assert!(s.up_checking(cx));
    s.up_click(UPDATE_LATER_ID, cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    fake.release();
    drive(&rt, cx, |cx| !s.up_checking(cx));
    assert!(
        matches!(s.up_phase(cx), UpdatePhase::Available(_)),
        "{:?}",
        s.up_phase(cx)
    );
}

/// Preferences > Display: the help text says what happens, `Check now`
/// checks (unthrottled) and the toast's View opens the dialog over them.
#[gpui_kit::test]
fn preferences_check_now_and_help_text(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.pf_route(cx);
    s.pf_loaded(&rt, cx);
    s.pf_section(cx, Section::Display);
    assert_eq!(
        s.up_label(UPDATE_CHECK_HELP_ID, cx).as_deref(),
        Some(UPDATE_CHECK_HELP)
    );
    assert!(UPDATE_CHECK_HELP.starts_with("Checks GitHub for a newer release on startup"));
    s.pf_click(CHECK_UPDATES_ID, cx);
    drive(&rt, cx, |cx| !s.up_checking(cx));
    assert_eq!(fake.checks(), [true]);
    assert!(s.pf_is_open(cx), "Preferences stay");
    s.age_toasts(Duration::from_millis(500), cx);
    s.up_view(cx);
    assert!(s.pf_is_open(cx));
    // Later goes back to Preferences, nothing persisted.
    s.up_click(UPDATE_LATER_ID, cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    assert!(s.pf_is_open(cx));
    assert_eq!(fake.dismissed(), None);
}

// ---- The dialog ------------------------------------------------------------------

/// The dialog's lines and buttons on an installable version; Release notes
/// hands the releases page to the opener.
#[gpui_kit::test]
fn the_dialog_shows_the_versions_and_opens_the_release_notes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake));
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = opened.clone();
    s.up_on_window(cx, move |w, _, _| {
        w.opener = Rc::new(move |what| {
            log.borrow_mut().push(what.to_string_lossy().into_owned());
            Ok(())
        });
    });
    s.up_found(&rt, cx);
    s.up_view(cx);
    assert!(s.shows(UPDATE_DIALOG_ID, cx));
    let dialog = s.up_dialog(cx).unwrap();
    let title = cx.read(|cx| dialog.read(cx).title());
    assert_eq!(title, TITLE);
    for id in [
        UPDATE_INSTALL_ID,
        UPDATE_NOTES_ID,
        UPDATE_SKIP_ID,
        UPDATE_LATER_ID,
    ] {
        assert!(s.shows(id, cx), "{id}");
    }
    assert_eq!(
        s.up_label(UPDATE_INSTALL_ID, cx).as_deref(),
        Some(INSTALL_LABEL)
    );
    s.up_click(UPDATE_NOTES_ID, cx);
    assert_eq!(*opened.borrow(), [SCRIPTED_RELEASES_URL.to_string()]);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Update), "it stays open");
}

/// Skip persists through the backend and closes; nothing is announced
/// for that version again.
#[gpui_kit::test]
fn skip_persists_the_version(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.up_found(&rt, cx);
    s.up_view(cx);
    s.up_click(UPDATE_SKIP_ID, cx);
    drive(&rt, cx, |_| fake.dismissed().is_some());
    assert_eq!(fake.dismissed().as_deref(), Some(NEW));
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    assert!(matches!(s.up_phase(cx), UpdatePhase::Skipped(v) if v == NEW));
}

/// Later and Escape close it without persisting anything.
#[gpui_kit::test]
fn later_and_escape_close_without_persisting(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.up_found(&rt, cx);
    s.up_view(cx);
    s.up_press("escape", cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    s.up_route(cx);
    s.up_click(UPDATE_LATER_ID, cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    assert_eq!(fake.dismissed(), None);
    assert!(matches!(s.up_phase(cx), UpdatePhase::Available(_)));
    assert!(cx.read(|cx| s.workspace.read(cx).updates.dialog.is_none()));
}

/// Tab `tabs` times (Install, Release notes, Skip, Later), check `id` has
/// focus, press Enter: the primary button (Install) must not run.
fn enter_on_non_primary_leaves_install_alone(
    rt: &Runtime,
    cx: &mut TestAppContext,
    tabs: usize,
    id: &'static str,
) {
    let v = vault(rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, rt, &v, Some(fake.clone()));
    s.up_found(rt, cx);
    s.up_view(cx);
    for _ in 0..tabs {
        s.up_press("tab", cx);
    }
    let focused = cx
        .update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused()
        })
        .unwrap();
    assert_eq!(focused, Some(true), "{id} has focus");
    s.up_press("enter", cx);
    assert!(
        !s.up_on_window(cx, |w, _, _| w.update_busy()),
        "Enter on {id} must not install"
    );
    assert_eq!(fake.installs(), 0);
    // Leave the window as the other tests do.
    if s.shown_modal(cx).is_some() {
        s.up_press("escape", cx);
        s.settle_dialog(cx);
    }
}

/// Enter on a focused non-primary button never runs the primary one: the
/// dialog's `on_ok` is not reached through the kit's button.
#[gpui_kit::test]
fn enter_on_release_notes_does_not_install(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    enter_on_non_primary_leaves_install_alone(&rt, cx, 2, UPDATE_NOTES_ID);
}

#[gpui_kit::test]
fn enter_on_skip_does_not_install(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    enter_on_non_primary_leaves_install_alone(&rt, cx, 3, UPDATE_SKIP_ID);
}

#[gpui_kit::test]
fn enter_on_later_does_not_install(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    enter_on_non_primary_leaves_install_alone(&rt, cx, 4, UPDATE_LATER_ID);
}

/// Install flushes the dirty note first, shows its progress with every
/// button waiting and the window's close refused, then the success state.
#[gpui_kit::test]
fn install_flushes_first_shows_progress_and_succeeds(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake =
        Arc::new(ScriptedUpdates::with_release(NEW).install_delay(Duration::from_millis(400)));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    let relauncher = Arc::new(RecordingRelauncher::default());
    s.up_on_window(cx, |w, _, _| w.set_relauncher(relauncher.clone()));
    cx.update_window(s.handle, |_, window, cx| window.input("X", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| s.workspace.read(cx).editor_dirty(cx)));
    s.up_found(&rt, cx);
    s.up_view(cx);
    assert_eq!(relauncher.calls(), 0, "nothing restarts before the install");
    // Enter: the primary button, Install.
    s.up_press("enter", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| s.workspace.read(cx).updates.machine.is_installing())
    });
    let on_disk = rt
        .block_on(v.vault.get_note_text(&VaultPath::new("a.md")))
        .unwrap();
    assert_eq!(on_disk, "Xalpha", "the note was written before the install");
    assert_eq!(
        s.up_label(UPDATE_TEXT_ID, cx).as_deref(),
        Some(rules::INSTALLING_TEXT)
    );
    for id in [
        UPDATE_INSTALL_ID,
        UPDATE_NOTES_ID,
        UPDATE_SKIP_ID,
        UPDATE_LATER_ID,
    ] {
        assert!(s.up_disabled(id, cx), "{id} waits");
    }
    // Escape and the window's close wait too.
    s.up_press("escape", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Update));
    let closes = s.up_on_window(cx, |w, _, cx| w.should_close(cx));
    assert!(!closes, "the close is refused while installing");
    assert_eq!(s.flash(cx).as_deref(), Some(rules::CLOSE_REFUSED_TEXT));
    drive(&rt, cx, |cx| {
        matches!(s.up_phase(cx), UpdatePhase::Installed { .. })
    });
    assert_eq!(fake.installs(), 1);
    assert_eq!(
        s.up_label(UPDATE_TEXT_ID, cx),
        Some(rules::installed_restarting_text(NEW))
    );
    // The install ended the app, and asked for it to start again.
    drive(&rt, cx, |cx| cx.read(|cx| s.workspace.read(cx).quits) == 1);
    assert_eq!(relauncher.calls(), 1, "the app restarts once");
    let dialog = s.up_dialog(cx).unwrap();
    let title = cx.read(|cx| dialog.read(cx).title());
    assert_eq!(title, TITLE_INSTALLED);
    assert!(!s.shows(UPDATE_INSTALL_ID, cx));
    // The palette row shows it again rather than checking for the same
    // version; Close closes, and the window can close now.
    s.up_click(UPDATE_CLOSE_ID, cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    s.up_route(cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Update));
    assert_eq!(fake.checks(), [true]);
    s.up_press("escape", cx);
    s.settle_dialog(cx);
    assert!(s.up_on_window(cx, |w, _, _| !w.update_busy()
        && w.closing == super::Closing::Open));
}

/// A note that cannot be let go refuses the install, inline; nothing is
/// installed.
#[gpui_kit::test]
fn a_failed_flush_refuses_the_install(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.up_found(&rt, cx);
    s.up_view(cx);
    // Another operation holds the note.
    s.up_on_window(cx, |w, _, _| {
        w.properties_opening = Some(gpui_kit::Task::ready(()));
    });
    s.up_click(UPDATE_INSTALL_ID, cx);
    let error = s.up_label(UPDATE_ERROR_ID, cx).expect("the refusal shows");
    assert!(
        error.starts_with("Couldn't install the update: the open note couldn't be saved first"),
        "{error}"
    );
    assert_eq!(fake.installs(), 0);
    s.up_on_window(cx, |w, _, _| w.properties_opening = None);
}

/// A failed install: the error inline and Retry, which installs again.
#[gpui_kit::test]
fn a_failed_install_offers_retry(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW).install_fails("checksum mismatch"));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    let relauncher = Arc::new(RecordingRelauncher::default());
    s.up_on_window(cx, |w, _, _| w.set_relauncher(relauncher.clone()));
    s.up_found(&rt, cx);
    s.up_view(cx);
    s.up_click(UPDATE_INSTALL_ID, cx);
    drive(&rt, cx, |cx| {
        matches!(s.up_phase(cx), UpdatePhase::Failed { .. })
    });
    cx.run_until_parked();
    assert_eq!(relauncher.calls(), 0, "a failed install does not restart");
    assert_eq!(cx.read(|cx| s.workspace.read(cx).quits), 0);
    assert_eq!(
        s.up_label(UPDATE_ERROR_ID, cx),
        Some(rules::install_failed_text("checksum mismatch"))
    );
    assert_eq!(
        s.up_label(UPDATE_INSTALL_ID, cx).as_deref(),
        Some(RETRY_LABEL)
    );
    fake.set_install(Ok(()));
    s.up_click(UPDATE_INSTALL_ID, cx);
    drive(&rt, cx, |cx| {
        matches!(s.up_phase(cx), UpdatePhase::Installed { .. })
    });
    assert_eq!(fake.installs(), 2);
    assert!(!s.shows(UPDATE_ERROR_ID, cx));
}

/// A package-managed install: the package manager's words and command,
/// Release notes as the primary, no Install.
#[gpui_kit::test]
fn a_managed_install_has_no_install_button(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW).channel(InstallChannel::Brew));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.up_found(&rt, cx);
    s.up_view(cx);
    assert_eq!(
        s.up_label(UPDATE_TEXT_ID, cx).as_deref(),
        Some(rules::PACKAGE_MANAGER_TEXT)
    );
    assert_eq!(
        s.up_label(UPDATE_COMMAND_ID, cx).as_deref(),
        Some("Run: brew upgrade kimun")
    );
    assert!(!s.shows(UPDATE_INSTALL_ID, cx));
    assert!(s.shows(UPDATE_NOTES_ID, cx) && s.shows(UPDATE_SKIP_ID, cx));
    // Enter is the release notes here, never an install.
    let opened = Rc::new(RefCell::new(0));
    let count = opened.clone();
    s.up_on_window(cx, move |w, _, _| {
        w.opener = Rc::new(move |_| {
            *count.borrow_mut() += 1;
            Ok(())
        });
    });
    s.up_press("enter", cx);
    assert_eq!(*opened.borrow(), 1);
    assert_eq!(fake.installs(), 0);
    // The desktop app's own binary unpublished: the releases page's words.
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW).published(false));
    let s = on_vault(cx, &rt, &v, Some(fake));
    s.up_found(&rt, cx);
    s.up_view(cx);
    assert_eq!(
        s.up_label(UPDATE_TEXT_ID, cx).as_deref(),
        Some(rules::DOWNLOAD_TEXT)
    );
    assert!(!s.shows(UPDATE_INSTALL_ID, cx));
}

/// Refused over another modal and under the setup assistant.
#[gpui_kit::test]
fn the_dialog_is_refused_over_a_modal_or_the_setup_assistant(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake));
    s.up_found(&rt, cx);
    s.up_on_window(cx, |w, window, cx| w.open_palette(window, cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.up_on_window(cx, |w, window, cx| w.open_update_dialog(window, cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette), "refused");
    assert!(s.up_dialog(cx).is_none());
    s.up_press("escape", cx);
    s.settle_dialog(cx);
    s.up_on_window(cx, |w, window, cx| w.open_onboarding(window, cx));
    s.up_on_window(cx, |w, window, cx| w.open_update_dialog(window, cx));
    assert_eq!(s.shown_modal(cx), None, "not under the assistant");
}

// ---- The toast action ------------------------------------------------------------------

/// A toast with an action: its button closes it and runs the action; a
/// plain flash has no button; nobody dismissing it, it goes after ten
/// seconds.
#[gpui_kit::test]
fn a_toast_action_runs_and_closes_its_toast(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let s = on_vault(cx, &rt, &v, None);
    // Left alone: the plain one goes after the kit's five seconds, the
    // action one after ten.
    s.up_on_window(cx, |w, _, cx| {
        w.flash("a plain notice", cx);
        w.flash_with_action("left alone", ToastAction::new("Do it", |_, _, _| {}), cx);
    });
    s.age_toasts(Duration::from_millis(500), cx);
    assert_eq!(s.toasts(cx), 2);
    assert_eq!(s.up_actions(cx), 1, "only the action toast has a button");
    s.age_toasts(Duration::from_secs(6), cx);
    assert_eq!(s.up_toasts(cx), ["left alone"]);
    s.age_toasts(ACTION_TOAST_LIFETIME, cx);
    assert!(s.up_toasts(cx).is_empty());

    // Its button closes it and runs the action.
    let ran = Rc::new(RefCell::new(0));
    let count = ran.clone();
    s.up_on_window(cx, move |w, _, cx| {
        let action = ToastAction::new("Do it", move |_, _, _| *count.borrow_mut() += 1);
        w.flash_with_action("a notice with an action", action, cx);
    });
    s.age_toasts(Duration::from_millis(500), cx);
    s.up_click(TOAST_ACTION_ID, cx);
    s.age_toasts(Duration::from_millis(500), cx);
    assert_eq!(*ran.borrow(), 1);
    assert!(s.up_toasts(cx).is_empty());
}

/// A toast shown again keeps its full lifetime: the first timer must not take
/// the second showing down early.
#[gpui_kit::test]
fn a_reshown_action_toast_keeps_its_full_lifetime(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let s = on_vault(cx, &rt, &v, None);
    let show = |s: &Started, cx: &mut TestAppContext| {
        s.up_on_window(cx, |w, _, cx| {
            w.flash_with_action("again", ToastAction::new("Do it", |_, _, _| {}), cx);
        });
    };
    show(&s, cx);
    s.age_toasts(Duration::from_secs(8), cx);
    show(&s, cx);
    // Past the first timer's ten seconds, inside the second's.
    s.age_toasts(Duration::from_secs(3), cx);
    assert_eq!(s.up_toasts(cx), ["again"], "the first timer is gone");
    s.age_toasts(ACTION_TOAST_LIFETIME, cx);
    assert!(s.up_toasts(cx).is_empty());
}

/// Install never starts while the window closes, even with no open note to
/// flush (where `flush_block` is not consulted).
#[gpui_kit::test]
fn install_does_not_start_while_the_window_closes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    set_update_check(&v, false);
    let fake = Arc::new(ScriptedUpdates::with_release(NEW));
    let s = on_vault(cx, &rt, &v, Some(fake.clone()));
    s.up_found(&rt, cx);
    s.up_on_window(cx, |w, window, cx| {
        // No note open: the shortcut straight to the install.
        w.notes.as_mut().expect("a vault").note = None;
        w.closing = Closing::Flushing;
        w.install_update(window, cx);
        w.closing = Closing::Open;
    });
    assert!(matches!(s.up_phase(cx), UpdatePhase::Available(_)));
    assert_eq!(fake.installs(), 0);
}
