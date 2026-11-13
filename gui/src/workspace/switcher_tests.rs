//! UI integration tests for the workspace switcher (M8d) on a real
//! `Workspace` over two real temp vaults, `a` and `b`, named in one temp
//! config (`reopen_tests::pair`), switched between with keys and clicks.
//!
//! Like the startup tests, each test owns a current-thread tokio runtime and
//! drives it between GPUI's `run_until_parked` (see `start_tests::drive`):
//! the switch's flush, the settings write and the new vault's open land only
//! when tokio runs.
#![cfg(test)]

use std::sync::{Arc, RwLock};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::Panel;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::runtime::Runtime;

use super::LayoutStore;
use super::modal::ModalKind;
use super::onboarding::ONBOARDING_ID;
use super::palette::PaletteRun;
use super::reopen_tests::{
    ALPHA, BETA, Pair, current, files_rows, on_disk_current, open_on_a, pair,
};
use super::start_tests::{Started, drive, open_started, runtime};
use super::switcher::{
    EMPTY_TEXT, SWITCHER_EMPTY_ID, SWITCHER_SETUP_ID, SWITCHER_SWITCH_ID, TITLE, WorkspaceSwitcher,
    workspace_row_id,
};
use crate::startup::Start;

fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
    s.sw_key(key, cx);
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

fn disabled(s: &Started, id: &'static str, cx: &mut TestAppContext) -> Option<bool> {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).disabled()
    })
    .unwrap()
}

/// The switcher's view while it is open.
fn switcher(s: &Started, cx: &TestAppContext) -> Entity<WorkspaceSwitcher> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .switcher
            .as_ref()
            .and_then(|view| view.upgrade())
            .expect("the switcher is open")
    })
}

/// The rows listed: (name, current).
fn rows(s: &Started, cx: &TestAppContext) -> Vec<(String, bool)> {
    let view = switcher(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .rows()
            .iter()
            .map(|r| (r.name.clone(), r.current))
            .collect()
    })
}

fn selected(s: &Started, cx: &TestAppContext) -> Option<usize> {
    let view = switcher(s, cx);
    cx.read(|cx| view.read(cx).selected())
}

/// Whether the row element `id` is in the last frame.
fn drawn(s: &Started, id: gpui_kit::SharedString, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

/// The pair's window on A, persisting to its config.
fn on_a(cx: &mut TestAppContext, rt: &Runtime) -> (Pair, Started) {
    let p = rt.block_on(pair());
    let started = open_on_a(cx, rt, &p, LayoutStore::File(p.config.clone()));
    (p, started)
}

/// Drive until the switch to B landed: B's note restored, the index built.
fn landed_on_b(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| {
        !s.sw_reopening(cx) && !s.indexing(cx) && s.text(cx) == BETA
    });
}

fn onboarding_open(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).onboarding_open())
}

// ── Opening ──────────────────────────────────────────────────────────────

/// F5 opens it: both workspaces, by name, the current one marked and
/// selected, every row drawn. Escape closes it with nothing changed.
#[gpui_kit::test]
fn f5_lists_the_workspaces_with_the_current_one_selected(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (p, started) = on_a(cx, &rt);
    press(&started, "f5", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::WorkspaceSwitcher));
    assert_eq!(
        rows(&started, cx),
        [("a".to_string(), true), ("b".to_string(), false)]
    );
    assert_eq!(selected(&started, cx), Some(0));
    assert!(drawn(&started, workspace_row_id("a"), cx));
    assert!(drawn(&started, workspace_row_id("b"), cx));
    assert!(!started.shows(SWITCHER_EMPTY_ID, cx));
    assert_eq!(TITLE, "Switch workspace");
    // Switching to the current one has nothing to do.
    assert_eq!(disabled(&started, SWITCHER_SWITCH_ID, cx), Some(true));
    press(&started, "down", cx);
    assert_ne!(disabled(&started, SWITCHER_SWITCH_ID, cx), Some(true));

    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(cx.read(|cx| started.workspace.read(cx).switcher.is_none()));
    drive(&rt, cx, |_| true);
    assert!(started.sw_is_on("alpha.md", cx));
    assert_eq!(current(&p.settings).as_deref(), Some("a"));
    assert_eq!(on_disk_current(&p.config).as_deref(), Some("a"));
}

/// Leader `v s` opens it, and F5 again closes it (the modal's toggle).
#[gpui_kit::test]
fn leader_v_s_opens_it_and_f5_toggles_it_closed(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_p, started) = on_a(cx, &rt);
    press(&started, "ctrl-shift-b", cx);
    started.sw_typed("vs", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::WorkspaceSwitcher));
    assert_eq!(rows(&started, cx).len(), 2);
    press(&started, "f5", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

/// The palette's `+vault · switch vault` runs (not blocked), shows F5, and
/// opens the switcher.
#[gpui_kit::test]
fn the_palette_leaf_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_p, started) = on_a(cx, &rt);
    let entry = cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::VaultSwitch))
            .expect("the leaf is listed")
    });
    assert_eq!(entry.blocked(), None);
    assert_eq!(entry.label(), "+vault · switch vault");
    cx.update_window(started.handle, |_, window, cx| {
        started.workspace.update(cx, |w, cx| {
            w.run_palette_row(PaletteRun::Leaf(LeaderAction::VaultSwitch), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.shown_modal(cx), Some(ModalKind::WorkspaceSwitcher));
}

/// The rows are read when it opens: a workspace added to the settings
/// since shows the next time.
#[gpui_kit::test]
fn opening_reads_the_settings_again(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (p, started) = on_a(cx, &rt);
    press(&started, "f5", cx);
    assert_eq!(rows(&started, cx).len(), 2);
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    let extra = p.a.join("extra");
    std::fs::create_dir(&extra).unwrap();
    p.settings
        .write()
        .unwrap()
        .workspace_config
        .as_mut()
        .unwrap()
        .add_workspace("c".into(), extra)
        .unwrap();
    press(&started, "f5", cx);
    let names: Vec<String> = rows(&started, cx).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["a", "b", "c"]);
}

// ── Picking ──────────────────────────────────────────────────────────────

/// Down then Enter on B: the dialog closes, the window reopens on B (its
/// note, FILES, the title) and `b` is the current workspace, in memory and
/// on disk.
#[gpui_kit::test]
fn enter_on_the_other_workspace_switches_to_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (p, started) = on_a(cx, &rt);
    press(&started, "f5", cx);
    press(&started, "down", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    landed_on_b(&started, &rt, cx);
    drive(&rt, cx, |_| {
        on_disk_current(&p.config).as_deref() == Some("b")
    });
    assert!(started.sw_is_on("beta.md", cx));
    assert_eq!(current(&p.settings).as_deref(), Some("b"));
    drive(&rt, cx, |cx| {
        files_rows(&started, cx).contains(&"beta.md".to_string())
    });
    let files = files_rows(&started, cx);
    assert!(!files.contains(&"alpha.md".to_string()), "{files:?}");
    assert_eq!(started.sw_title(cx).as_deref(), Some("b - kimün"));
    // B is the current one now.
    press(&started, "f5", cx);
    assert_eq!(
        rows(&started, cx),
        [("a".to_string(), false), ("b".to_string(), true)]
    );
    assert_eq!(selected(&started, cx), Some(1));
}

/// Enter on the current workspace only closes: no reopen, nothing written.
#[gpui_kit::test]
fn picking_the_current_workspace_only_closes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (p, started) = on_a(cx, &rt);
    let editor = cx.read(|cx| started.workspace.read(cx).active_state().editor.entity_id());
    press(&started, "f5", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(!started.sw_reopening(cx), "no vault change");
    drive(&rt, cx, |_| true);
    assert!(started.sw_is_on("alpha.md", cx));
    assert_eq!(started.text(cx), ALPHA);
    assert_eq!(current(&p.settings).as_deref(), Some("a"));
    assert_eq!(on_disk_current(&p.config).as_deref(), Some("a"));
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).active_state().editor.entity_id()),
        editor
    );
    // A click on the current row does the same.
    press(&started, "f5", cx);
    click(&started, workspace_row_id("a"), cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(!started.sw_reopening(cx));
}

/// Up/Down (and `k`/`j`) move the selection and stop at the ends, as the
/// shared list does (the TUI wraps).
#[gpui_kit::test]
fn the_keys_move_the_selection_and_clamp(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_p, started) = on_a(cx, &rt);
    press(&started, "f5", cx);
    press(&started, "up", cx);
    assert_eq!(selected(&started, cx), Some(0), "clamped at the top");
    press(&started, "down", cx);
    assert_eq!(selected(&started, cx), Some(1));
    press(&started, "down", cx);
    assert_eq!(selected(&started, cx), Some(1), "clamped at the bottom");
    press(&started, "k", cx);
    assert_eq!(selected(&started, cx), Some(0));
    press(&started, "j", cx);
    assert_eq!(selected(&started, cx), Some(1));
    // A chord is swallowed, never read as the bare key.
    press(&started, "ctrl-k", cx);
    assert_eq!(selected(&started, cx), Some(1));
    assert_eq!(started.shown_modal(cx), Some(ModalKind::WorkspaceSwitcher));
}

/// One click on another workspace switches to it (C1: not a preview list).
#[gpui_kit::test]
fn a_click_switches(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (p, started) = on_a(cx, &rt);
    press(&started, "f5", cx);
    click(&started, workspace_row_id("b"), cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    landed_on_b(&started, &rt, cx);
    assert_eq!(current(&p.settings).as_deref(), Some("b"));
}

/// The `Switch` button switches to the selected workspace.
#[gpui_kit::test]
fn the_switch_button_switches(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_p, started) = on_a(cx, &rt);
    press(&started, "f5", cx);
    press(&started, "down", cx);
    click(&started, SWITCHER_SWITCH_ID, cx);
    started.settle_dialog(cx);
    landed_on_b(&started, &rt, cx);
}

// ── Window states ────────────────────────────────────────────────────────

/// A workspace that would not open: F5 and leader `v s` still open the
/// switcher, and picking another workspace opens it.
#[gpui_kit::test]
fn it_works_from_a_workspace_that_would_not_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_started(
        cx,
        Start::OpenFailed("a: database is locked".into()),
        p.settings.clone(),
    );
    cx.run_until_parked();
    assert!(!onboarding_open(&started, cx));
    press(&started, "ctrl-shift-b", cx);
    started.sw_typed("vs", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::WorkspaceSwitcher));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);

    press(&started, "f5", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::WorkspaceSwitcher));
    press(&started, "down", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    landed_on_b(&started, &rt, cx);
    assert_eq!(current(&p.settings).as_deref(), Some("b"));
}

/// Never over another modal (F5 and the open call both refused, the
/// palette stays) nor under the setup assistant.
#[gpui_kit::test]
fn refused_over_a_modal_or_the_setup_assistant(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_p, started) = on_a(cx, &rt);
    press(&started, "ctrl-shift-p", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Palette));
    press(&started, "f5", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Palette));
    cx.update_window(started.handle, |_, window, cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_workspace_switcher(window, cx))
    })
    .unwrap();
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Palette));
    assert!(cx.read(|cx| started.workspace.read(cx).switcher.is_none()));
    press(&started, "escape", cx);
    started.settle_dialog(cx);

    cx.update_window(started.handle, |_, window, cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_onboarding(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(onboarding_open(&started, cx));
    press(&started, "f5", cx);
    cx.update_window(started.handle, |_, window, cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_workspace_switcher(window, cx))
    })
    .unwrap();
    assert_eq!(started.shown_modal(cx), None);
    assert!(cx.read(|cx| started.workspace.read(cx).switcher.is_none()));
    assert!(started.shows(ONBOARDING_ID, cx));
}

// ── No workspace ─────────────────────────────────────────────────────────

/// A window on settings with no workspace and no assistant over it.
fn no_workspaces(cx: &mut TestAppContext) -> Started {
    let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
    let started = open_started(cx, Start::OpenFailed("no vault".into()), settings);
    cx.run_until_parked();
    started
}

/// No workspace configured: it says so, and `Run guided setup…` closes it
/// and opens the setup assistant.
#[gpui_kit::test]
fn no_workspace_offers_the_guided_setup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let started = no_workspaces(cx);
    press(&started, "f5", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::WorkspaceSwitcher));
    assert!(rows(&started, cx).is_empty());
    assert_eq!(selected(&started, cx), None);
    assert!(started.shows(SWITCHER_EMPTY_ID, cx));
    assert_eq!(EMPTY_TEXT, "No workspaces configured.");
    click(&started, SWITCHER_SETUP_ID, cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(onboarding_open(&started, cx));
    assert!(started.shows(ONBOARDING_ID, cx));
}

/// Enter does the same with no workspace.
#[gpui_kit::test]
fn enter_with_no_workspace_runs_the_setup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let started = no_workspaces(cx);
    press(&started, "f5", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(onboarding_open(&started, cx));
}

// ── Refused switch ───────────────────────────────────────────────────────

/// The open note cannot be written (a readonly file): the switch is
/// refused with a flash, the window stays on A and keeps working, and the
/// settings still name `a`, in memory and on disk.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_flush_keeps_the_old_workspace_and_its_setting(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (p, started) = on_a(cx, &rt);
    let file = p.a.join("alpha.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    started.sw_typed("X", cx);
    press(&started, "f5", cx);
    press(&started, "down", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    drive(&rt, cx, |cx| !started.sw_reopening(cx));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save alpha.md:")
            && flash.ends_with("The workspace was not changed."),
        "{flash}"
    );
    assert_eq!(started.shown_modal(cx), None, "the dialog stays closed");
    assert!(started.sw_is_on("alpha.md", cx));
    assert_eq!(started.text(cx), format!("X{ALPHA}"));
    assert_eq!(started.sw_title(cx).as_deref(), Some("a - kimün"));
    drive(&rt, cx, |_| true);
    assert_eq!(current(&p.settings).as_deref(), Some("a"));
    assert_eq!(on_disk_current(&p.config).as_deref(), Some("a"));
    // Still working: typing goes in and saves; the switcher still marks A.
    started.sw_typed("Y", cx);
    press(&started, "ctrl-s", cx);
    drive(&rt, cx, |_| {
        std::fs::read_to_string(&file).unwrap() == format!("XY{ALPHA}")
    });
    press(&started, "f5", cx);
    assert_eq!(
        rows(&started, cx),
        [("a".to_string(), true), ("b".to_string(), false)]
    );
}
