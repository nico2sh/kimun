//! UI integration tests for the pinned-notes dialog (M7b G8) on a real
//! `Workspace` over a real, indexed temp vault whose pins are core's.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the list, the reorder
//! and the unpin land only when tokio runs, which is also the proof that
//! none of them runs on the UI thread.
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, Focusable as _, ScrollDelta, TestAppContext, point, px};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::Panel;
use tokio::runtime::Runtime;

use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::pinned::{
    EMPTY_TEXT, PINNED_EMPTY_ID, PINNED_ID, PINNED_LOWER_ID, PINNED_OPEN_ID, PINNED_RAISE_ID,
    PinnedNotesDialog, WHEEL_STEP_REMS, pinned_row_id,
};
use super::route::NO_WORKSPACE;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use super::tests::open_default;
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

const NOTES: &[(&str, &str)] = &[
    ("alpha.md", "# Alpha\nthe alpha note"),
    ("beta.md", "# Beta\nthe beta note"),
    ("gamma.md", "# Gamma\nthe gamma note"),
];

/// A vault holding [`NOTES`] with `pins` pinned (in order, through core),
/// started on alpha and indexed.
fn started_with(cx: &mut TestAppContext, rt: &Runtime, pins: &[&str]) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    for p in pins {
        rt.block_on(v.vault.toggle_pinned_note(&path(p))).unwrap();
    }
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("alpha.md"))),
        v.settings.clone(),
    );
    let want = NOTES[0].1;
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

/// Leader `f p`.
fn open_pins(s: &Started, cx: &mut TestAppContext) {
    press(s, "ctrl-shift-b", cx);
    type_in(s, "fp", cx);
}

fn dialog(s: &Started, cx: &TestAppContext) -> Entity<PinnedNotesDialog> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .pinned
            .as_ref()
            .and_then(|open| open.dialog.upgrade())
            .expect("the pinned notes are open")
    })
}

/// The rows once the latest read landed: (path, missing).
fn rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<(VaultPath, bool)> {
    let view = dialog(s, cx);
    drive(rt, cx, |cx| {
        cx.read(|cx| view.read(cx).is_loaded() && !view.read(cx).is_pending())
    });
    shown_rows(&view, cx)
}

/// The rows shown now, without waiting.
fn shown_rows(view: &Entity<PinnedNotesDialog>, cx: &TestAppContext) -> Vec<(VaultPath, bool)> {
    cx.read(|cx| {
        view.read(cx)
            .rows()
            .iter()
            .map(|r| (r.path.clone(), r.missing))
            .collect()
    })
}

fn names(rows: &[(VaultPath, bool)]) -> Vec<String> {
    rows.iter().map(|(p, _)| p.get_clean_name()).collect()
}

fn selected(s: &Started, cx: &TestAppContext) -> usize {
    let view = dialog(s, cx);
    cx.read(|cx| view.read(cx).selected())
}

/// The pins core keeps, by name.
fn stored(rt: &Runtime, v: &TestVault) -> Vec<String> {
    rt.block_on(v.vault.list_pinned_notes())
        .unwrap()
        .iter()
        .map(VaultPath::get_clean_name)
        .collect()
}

fn editor_focused(s: &Started, cx: &mut TestAppContext) -> bool {
    let editor = cx.read(|cx| s.workspace.read(cx).active_state().editor.clone());
    cx.update_window(s.handle, |_, window, cx| {
        editor.read(cx).focus_handle(cx).is_focused(window)
    })
    .unwrap()
}

// ── Listing ──────────────────────────────────────────────────────────────

/// Leader `f p` opens it; nothing is listed until tokio has read the pins
/// (no empty-state either); then the pins in the user's order, numbered.
/// Escape closes it and the editor has the keyboard back.
#[gpui_kit::test]
fn leader_f_p_lists_the_pins_in_order(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &["gamma.md", "alpha.md", "beta.md"]);
    open_pins(&started, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));
    let view = dialog(&started, cx);
    assert!(
        shown_rows(&view, cx).is_empty() && !cx.read(|cx| view.read(cx).is_loaded()),
        "the pins are read off the UI thread, not yet here"
    );
    assert!(!started.shows(PINNED_EMPTY_ID, cx), "blank, not empty");
    let listed = rows(&started, &rt, cx);
    assert_eq!(names(&listed), ["gamma", "alpha", "beta"]);
    assert!(listed.iter().all(|(_, missing)| !missing));
    let first = pinned_row_id(&listed[0].0);
    let shown = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(first).is_some_and(|e| e.visible())
        })
        .unwrap();
    assert!(shown, "the first row is drawn");
    assert!(!started.shows(PINNED_EMPTY_ID, cx));
    assert_eq!(selected(&started, cx), 0);

    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(editor_focused(&started, cx), "the keyboard returns");
}

/// With no pins the dialog says how to make one, once the read landed.
#[gpui_kit::test]
fn no_pins_shows_the_empty_state(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &[]);
    open_pins(&started, cx);
    assert!(!started.shows(PINNED_EMPTY_ID, cx));
    assert!(rows(&started, &rt, cx).is_empty());
    assert!(started.shows(PINNED_EMPTY_ID, cx));
    assert!(EMPTY_TEXT.contains("leader m p"));
    assert_eq!(disabled(&started, PINNED_OPEN_ID, cx), Some(true));
}

// ── Opening ──────────────────────────────────────────────────────────────

/// A digit opens that pin: the dialog closes and the note opens in the
/// editor. A digit past the end flashes and the dialog stays.
#[gpui_kit::test]
fn a_digit_opens_that_note_and_closes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &["alpha.md", "beta.md"]);
    open_pins(&started, cx);
    rows(&started, &rt, cx);
    press(&started, "5", cx);
    assert_eq!(started.flash(cx).as_deref(), Some("no pinned note 5"));
    assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));

    press(&started, "2", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    drive(&rt, cx, |cx| started.text(cx) == NOTES[1].1);
    assert!(editor_focused(&started, cx));
}

/// A click opens a row at once (C1): the dialog closes, the note opens.
#[gpui_kit::test]
fn a_click_opens_a_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &["alpha.md", "gamma.md"]);
    open_pins(&started, cx);
    let listed = rows(&started, &rt, cx);
    let gamma = pinned_row_id(&listed[1].0);
    click(&started, gamma, cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    drive(&rt, cx, |cx| started.text(cx) == NOTES[2].1);
}

/// A click on a missing pin is inert: it selects the row, opens nothing,
/// flashes nothing, and the dialog stays.
#[gpui_kit::test]
fn a_click_on_a_missing_pin_only_selects_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &["beta.md", "gamma.md"]);
    std::fs::remove_file(v.workspace.as_path().join("gamma.md")).unwrap();
    open_pins(&started, cx);
    let listed = rows(&started, &rt, cx);
    assert!(listed[1].1, "gamma is missing");
    let before = started.flash(cx);
    click(&started, pinned_row_id(&listed[1].0), cx);
    assert_eq!(selected(&started, cx), 1);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));
    assert_eq!(started.flash(cx), before, "no flash");
    assert_eq!(started.text(cx), NOTES[0].1, "alpha stays open");
}

/// A pin whose note is gone from disk is kept and flagged; neither its
/// digit nor Enter opens it (a flash says why) and the dialog stays.
#[gpui_kit::test]
fn a_missing_pin_is_flagged_and_refuses_to_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &["beta.md", "gamma.md"]);
    // Gone behind core's back (a delete through core would unpin it).
    std::fs::remove_file(v.workspace.as_path().join("gamma.md")).unwrap();
    open_pins(&started, cx);
    let listed = rows(&started, &rt, cx);
    assert_eq!(
        listed.iter().map(|(_, m)| *m).collect::<Vec<_>>(),
        [false, true]
    );
    let flash = format!("pinned note not found: {}", listed[1].0);
    press(&started, "2", cx);
    assert_eq!(started.flash(cx), Some(flash.clone()));
    assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));
    press(&started, "down", cx);
    assert_eq!(disabled(&started, PINNED_OPEN_ID, cx), Some(true));
    press(&started, "enter", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));
    assert_eq!(started.text(cx), NOTES[0].1, "alpha stays open");
    assert_eq!(stored(&rt, &v), ["beta", "gamma"], "the pin is kept");
}

/// The wheel over the list moves the selection, at most a row per event.
fn wheel(s: &Started, delta: ScrollDelta, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.scroll(PINNED_ID, delta, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

/// A notch moves the selection one row; a horizontal wheel (Shift+wheel on
/// Linux) moves nothing; a trackpad reversing direction does not carry the
/// travel it made the other way.
#[gpui_kit::test]
fn the_wheel_moves_the_selection_vertically_only(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &["alpha.md", "beta.md", "gamma.md"]);
    open_pins(&started, cx);
    rows(&started, &rt, cx);
    // Positive y moves content down: scrolling up.
    wheel(&started, ScrollDelta::Lines(point(0., -3.)), cx);
    assert_eq!(selected(&started, cx), 1, "a notch is one row");
    wheel(&started, ScrollDelta::Lines(point(1., 0.)), cx);
    assert_eq!(
        selected(&started, cx),
        1,
        "a horizontal wheel moves nothing"
    );
    wheel(&started, ScrollDelta::Pixels(point(px(3.), px(0.))), cx);
    assert_eq!(selected(&started, cx), 1);

    let step = cx
        .update_window(started.handle, |_, window, _| {
            f32::from(window.rem_size()) * WHEEL_STEP_REMS
        })
        .unwrap();
    // Most of a row up, then the trackpad turns: a row's travel down is a
    // row down, not eaten by the travel up.
    wheel(
        &started,
        ScrollDelta::Pixels(point(px(0.), px(step * 0.8))),
        cx,
    );
    assert_eq!(selected(&started, cx), 1);
    wheel(
        &started,
        ScrollDelta::Pixels(point(px(0.), px(-step * 0.2))),
        cx,
    );
    wheel(
        &started,
        ScrollDelta::Pixels(point(px(0.), px(-step * 0.8))),
        cx,
    );
    assert_eq!(
        selected(&started, cx),
        2,
        "the reversal dropped the travel up"
    );
}

// ── Managing ─────────────────────────────────────────────────────────────

/// Alt+Up raises the selected pin and Shift+J lowers it: the rows swap at
/// once, the cursor follows, and core keeps the new order.
#[gpui_kit::test]
fn raise_and_lower_persist_and_the_cursor_follows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &["alpha.md", "beta.md", "gamma.md"]);
    open_pins(&started, cx);
    rows(&started, &rt, cx);
    assert_eq!(disabled(&started, PINNED_RAISE_ID, cx), Some(true), "top");
    press(&started, "down", cx);
    press(&started, "down", cx);
    press(&started, "alt-up", cx);
    let view = dialog(&started, cx);
    assert_eq!(
        names(&shown_rows(&view, cx)),
        ["alpha", "gamma", "beta"],
        "the rows change at once"
    );
    assert_eq!(selected(&started, cx), 1);
    assert_eq!(names(&rows(&started, &rt, cx)), ["alpha", "gamma", "beta"]);
    assert_eq!(stored(&rt, &v), ["alpha", "gamma", "beta"]);

    press(&started, "up", cx);
    press(&started, "shift-j", cx);
    assert_eq!(names(&rows(&started, &rt, cx)), ["gamma", "alpha", "beta"]);
    assert_eq!(selected(&started, cx), 1);
    assert_eq!(stored(&rt, &v), ["gamma", "alpha", "beta"]);

    // The buttons do the same.
    click(&started, PINNED_LOWER_ID, cx);
    assert_eq!(names(&rows(&started, &rt, cx)), ["gamma", "beta", "alpha"]);
    assert_eq!(selected(&started, cx), 2);
    assert_eq!(stored(&rt, &v), ["gamma", "beta", "alpha"]);
    assert_eq!(
        disabled(&started, PINNED_LOWER_ID, cx),
        Some(true),
        "bottom"
    );
}

/// One write at a time: a second move while the first is under way is
/// ignored, and only the first reaches core.
#[gpui_kit::test]
fn a_second_move_while_one_runs_is_ignored(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &["alpha.md", "beta.md", "gamma.md"]);
    open_pins(&started, cx);
    rows(&started, &rt, cx);
    press(&started, "alt-down", cx);
    let view = dialog(&started, cx);
    assert!(cx.read(|cx| view.read(cx).is_pending()));
    press(&started, "alt-down", cx);
    assert_eq!(selected(&started, cx), 1, "the second press did nothing");
    assert_eq!(names(&rows(&started, &rt, cx)), ["beta", "alpha", "gamma"]);
    assert_eq!(stored(&rt, &v), ["beta", "alpha", "gamma"]);
}

/// Delete unpins the selected pin (the row goes at once, the cursor clamps)
/// and `d` does too; unpinning the last shows the empty state.
#[gpui_kit::test]
fn unpin_removes_the_pin_and_the_cursor_clamps(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &["alpha.md", "beta.md"]);
    open_pins(&started, cx);
    rows(&started, &rt, cx);
    press(&started, "down", cx);
    press(&started, "delete", cx);
    let view = dialog(&started, cx);
    assert_eq!(names(&shown_rows(&view, cx)), ["alpha"], "gone at once");
    assert_eq!(selected(&started, cx), 0, "clamped");
    assert_eq!(names(&rows(&started, &rt, cx)), ["alpha"]);
    assert_eq!(stored(&rt, &v), ["alpha"]);
    // The note itself is untouched.
    assert!(rt.block_on(v.vault.exists(&path("beta.md"))));

    press(&started, "d", cx);
    assert!(rows(&started, &rt, cx).is_empty());
    assert!(stored(&rt, &v).is_empty());
    assert!(started.shows(PINNED_EMPTY_ID, cx));
}

/// A chord that is not the dialog's never unpins or reorders, even with
/// the dialog's own letters (the TUI's rule).
#[gpui_kit::test]
fn chorded_keys_never_unpin_or_reorder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &["alpha.md", "beta.md"]);
    open_pins(&started, cx);
    rows(&started, &rt, cx);
    for key in [
        "alt-d",
        "ctrl-d",
        "alt-delete",
        "ctrl-delete",
        "ctrl-shift-j",
        "alt-shift-k",
        "ctrl-up",
        "ctrl-1",
    ] {
        press(&started, key, cx);
    }
    let view = dialog(&started, cx);
    assert!(
        !cx.read(|cx| view.read(cx).is_pending()),
        "no write started"
    );
    assert_eq!(names(&shown_rows(&view, cx)), ["alpha", "beta"]);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));
    assert_eq!(stored(&rt, &v), ["alpha", "beta"]);
}

/// A read landing after the dialog closed is dropped.
#[gpui_kit::test]
fn a_read_landing_after_close_is_dropped(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &["alpha.md"]);
    open_pins(&started, cx);
    let view = dialog(&started, cx);
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(cx.read(|cx| started.workspace.read(cx).pinned.is_none()));
    // Let the read land.
    for _ in 0..20 {
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
        cx.run_until_parked();
    }
    assert!(!cx.read(|cx| view.read(cx).is_loaded()), "dropped");
    assert!(shown_rows(&view, cx).is_empty());
}

// ── Doors ────────────────────────────────────────────────────────────────

/// The palette's `+find · pinned notes` runs (not blocked) with a
/// workspace and opens the dialog.
#[gpui_kit::test]
fn the_palette_leaf_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &["alpha.md"]);
    let entry = cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::FindPinned))
            .expect("the leaf is listed")
    });
    assert_eq!(entry.blocked(), None);
    cx.update_window(started.handle, |_, window, cx| {
        started.workspace.update(cx, |w, cx| {
            w.run_palette_row(PaletteRun::Leaf(LeaderAction::FindPinned), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));
    assert_eq!(names(&rows(&started, &rt, cx)), ["alpha"]);
}

/// Without a workspace the palette row is blocked.
#[gpui_kit::test]
fn the_palette_leaf_is_blocked_without_a_workspace(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    let blocked = cx.read(|cx| {
        shell
            .workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::FindPinned))
            .and_then(|e| e.blocked())
    });
    assert_eq!(blocked, Some(NO_WORKSPACE));
}

// ── The Files pinned section ─────────────────────────────────────────────

mod in_files {
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::super::files::{
        FILES_PINNED_ID, FILES_PINNED_MANAGE_ID, FILES_PINNED_TOGGLE_ID, files_pinned_row_id,
    };
    use super::super::modal::ModalKind;
    use super::super::start_tests::{drive, runtime};
    use super::{click, open_pins, press, rows, started_with, type_in};

    /// The pin rows the Files pane shows, by name.
    fn shown(s: &super::Started, cx: &TestAppContext) -> Vec<String> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .files
                .as_ref()
                .map(|f| {
                    f.read(cx)
                        .pins()
                        .iter()
                        .map(|p| p.path.get_clean_name())
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn drawn(
        s: &super::Started,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> bool {
        use gpui_kit::test::TestWindowExt as _;
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    /// The pins are listed above the folder's rows in the user's order,
    /// once core has read them off the UI thread; a click opens the note.
    #[gpui_kit::test]
    fn the_section_lists_the_pins_in_order_and_a_click_opens_one(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, started) = started_with(cx, &rt, &["gamma.md", "alpha.md"]);
        drive(&rt, cx, |cx| shown(&started, cx).len() == 2);
        assert_eq!(shown(&started, cx), ["gamma", "alpha"]);
        assert!(drawn(&started, FILES_PINNED_ID, cx));
        click(&started, FILES_PINNED_TOGGLE_ID, cx);
        // The pins core keeps are absolute: the row's id names that path.
        let gamma = cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone().unwrap();
            files.read(cx).pins()[0].path.clone()
        });
        assert!(drawn(&started, files_pinned_row_id(&gamma), cx));
        click(&started, files_pinned_row_id(&gamma), cx);
        drive(&rt, cx, |cx| started.text(cx) == "# Gamma\nthe gamma note");
    }

    /// No pins, no section: users who never pin see no change.
    #[gpui_kit::test]
    fn without_pins_there_is_no_section(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, started) = started_with(cx, &rt, &[]);
        cx.run_until_parked();
        assert!(!drawn(&started, FILES_PINNED_ID, cx));
    }

    /// Pinning with the leader makes the section appear, and unpinning with
    /// it takes the section away again.
    #[gpui_kit::test]
    fn the_section_follows_a_leader_pin_and_unpin(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, started) = started_with(cx, &rt, &[]);
        press(&started, "ctrl-shift-b", cx);
        type_in(&started, "mp", cx);
        drive(&rt, cx, |cx| shown(&started, cx) == ["alpha"]);
        assert!(drawn(&started, FILES_PINNED_ID, cx));
        assert!(
            !is_open(&started, cx),
            "a new pin does not open the section"
        );
        press(&started, "ctrl-shift-b", cx);
        type_in(&started, "mp", cx);
        drive(&rt, cx, |cx| shown(&started, cx).is_empty());
        assert!(!drawn(&started, FILES_PINNED_ID, cx));
    }

    /// An unpin made in the dialog shows in the section at once.
    #[gpui_kit::test]
    fn the_section_follows_the_dialog(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, started) = started_with(cx, &rt, &["alpha.md", "beta.md"]);
        drive(&rt, cx, |cx| shown(&started, cx).len() == 2);
        open_pins(&started, cx);
        rows(&started, &rt, cx);
        press(&started, "delete", cx);
        drive(&rt, cx, |cx| shown(&started, cx) == ["beta"]);
    }

    fn is_open(s: &super::Started, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let files = s.workspace.read(cx).files.clone().unwrap();
            files.read(cx).pins_open()
        })
    }

    /// The section starts closed in a run, with only its heading (and the
    /// count) drawn. The heading opens it and closes it again; the rows and
    /// the `Manage` link are drawn only while it is open. The state is not
    /// saved: it is a field of the pane, so a new run starts closed.
    #[gpui_kit::test]
    fn the_section_starts_closed_and_the_heading_toggles_it(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, started) = started_with(cx, &rt, &["gamma.md", "alpha.md"]);
        drive(&rt, cx, |cx| shown(&started, cx).len() == 2);
        let gamma = cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone().unwrap();
            files.read(cx).pins()[0].path.clone()
        });
        assert!(!is_open(&started, cx));
        assert!(drawn(&started, FILES_PINNED_TOGGLE_ID, cx));
        assert!(!drawn(&started, files_pinned_row_id(&gamma), cx));
        assert!(!drawn(&started, FILES_PINNED_MANAGE_ID, cx));

        click(&started, FILES_PINNED_TOGGLE_ID, cx);
        assert!(is_open(&started, cx));
        assert!(drawn(&started, files_pinned_row_id(&gamma), cx));
        assert!(drawn(&started, FILES_PINNED_MANAGE_ID, cx));

        click(&started, FILES_PINNED_TOGGLE_ID, cx);
        assert!(!is_open(&started, cx));
        assert!(!drawn(&started, files_pinned_row_id(&gamma), cx));
    }

    /// The chevrons of the heading are bundled: a missing SVG paints an
    /// empty button.
    #[test]
    fn the_heading_chevrons_are_bundled() {
        use crate::assets::AppAssets;
        use gpui_kit::AssetSource as _;
        for icon in [
            gpui_kit::assets::IconName::ChevronDown,
            gpui_kit::assets::IconName::ChevronRight,
        ] {
            let path = icon.path();
            assert!(
                AppAssets.load(&path).unwrap().is_some(),
                "{path} is not bundled"
            );
        }
    }

    /// The rows ease open and shut: part way, the frame is between nothing
    /// and its full height; at rest it is exactly the rows, none cut off.
    /// Closed, no row is drawn.
    #[gpui_kit::test]
    fn the_rows_ease_open_and_shut(cx: &mut TestAppContext) {
        use std::time::Duration;

        use gpui_kit::test::TestWindowExt as _;

        use super::super::files::FILES_PINNED_ROWS_ID;
        use super::super::start_tests::{open_started_motion, vault_start};
        use super::{FirstNote, NOTES, path, test_vault};
        let rt = runtime();
        let _guard = rt.enter();
        let v = rt.block_on(test_vault(NOTES, &[]));
        for p in ["gamma.md", "alpha.md"] {
            rt.block_on(v.vault.toggle_pinned_note(&path(p))).unwrap();
        }
        let started = open_started_motion(
            cx,
            vault_start(&v, FirstNote::Note(path("alpha.md"))),
            v.settings.clone(),
            false,
            false,
        );
        drive(&rt, cx, |cx| {
            shown(&started, cx).len() == 2
                && started.text(cx) == NOTES[0].1
                && !started.indexing(cx)
        });
        let frame = |cx: &mut TestAppContext| -> f32 {
            cx.update_window(started.handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .find(FILES_PINNED_ROWS_ID)
                    .bounds()
                    .size
                    .height
                    .into()
            })
            .unwrap()
        };
        let step = |ms: u64, cx: &mut TestAppContext| {
            cx.executor().advance_clock(Duration::from_millis(ms));
            cx.run_until_parked();
        };
        assert_eq!(frame(cx), 0.0, "closed at the start");

        click(&started, FILES_PINNED_TOGGLE_ID, cx);
        step(60, cx);
        let part = frame(cx);
        step(1000, cx);
        let full = frame(cx);
        assert!(full > 0.0, "open at rest");
        assert!(0.0 < part && part < full, "part way: {part} of {full}");
        let last = cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone().unwrap();
            files.read(cx).pins()[1].path.clone()
        });
        let last_bottom: f32 = cx
            .update_window(started.handle, |_, window, cx| {
                window.render_frame(cx);
                let b = window.find(files_pinned_row_id(&last)).bounds();
                (b.origin.y + b.size.height).into()
            })
            .unwrap();
        let frame_bottom: f32 = cx
            .update_window(started.handle, |_, window, _| {
                let b = window.find(FILES_PINNED_ROWS_ID).bounds();
                (b.origin.y + b.size.height).into()
            })
            .unwrap();
        assert!(last_bottom <= frame_bottom + 0.5, "no row is cut off");

        click(&started, FILES_PINNED_TOGGLE_ID, cx);
        step(60, cx);
        let closing = frame(cx);
        assert!(0.0 < closing && closing < full, "closing: {closing}");
        step(1000, cx);
        assert_eq!(frame(cx), 0.0);
        assert!(!drawn(&started, files_pinned_row_id(&last), cx));
    }

    /// `Manage` opens the dialog.
    #[gpui_kit::test]
    fn manage_opens_the_dialog(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, started) = started_with(cx, &rt, &["alpha.md"]);
        drive(&rt, cx, |cx| shown(&started, cx).len() == 1);
        assert!(
            !drawn(&started, FILES_PINNED_MANAGE_ID, cx),
            "closed: no link"
        );
        click(&started, FILES_PINNED_TOGGLE_ID, cx);
        click(&started, FILES_PINNED_MANAGE_ID, cx);
        cx.run_until_parked();
        assert_eq!(started.shown_modal(cx), Some(ModalKind::PinnedNotes));
    }
}
