//! UI integration tests for the open → edit → save loop (M5b-2 G2): a real
//! `Workspace` on a real temp vault, typed into through the window, saved
//! through the note session, checked on disk.
//!
//! Like the startup tests, each test owns a current-thread tokio runtime and
//! drives it between GPUI's `run_until_parked` (see `start_tests::drive`).
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, VisualTestContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::message::AppMessage;
use tokio::runtime::Runtime;

use super::Closing;
use super::note::DISCARD_WINDOW;
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use crate::startup::FirstNote;
use crate::status_row::{GIT_ID, PATH_ID, SAVE_ID};
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// The note's text on disk (`None`: not there).
fn disk(rt: &Runtime, v: &TestVault, p: &str) -> Option<String> {
    rt.block_on(v.vault.get_note_text(&path(p))).ok()
}

impl Started {
    fn dirty(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .active_state()
                .editor
                .read(cx)
                .editor
                .is_dirty()
        })
    }

    /// The note the Footer names.
    fn note(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).status.note.clone())
    }

    fn footer_dirty(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).status.dirty)
    }

    fn open_pending(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .active_state()
                .notes
                .as_ref()
                .is_some_and(|h| h.session.is_open_pending())
        })
    }

    fn session_path(&self, cx: &TestAppContext) -> VaultPath {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .active_state()
                .notes
                .as_ref()
                .unwrap()
                .session
                .path()
                .clone()
        })
    }

    /// Type `text` at the cursor (the editor has the keyboard).
    fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
    }

    /// The user closes the window; whether it closed right away.
    fn close(&self, cx: &mut TestAppContext) -> bool {
        VisualTestContext::from_window(self.handle, cx).simulate_close()
    }

    /// Pretend to be macOS (`true`: the app stays in the dock when its window
    /// closes) or any other system.
    fn stays_in_dock(&self, stays: bool, cx: &mut TestAppContext) {
        cx.update(|cx| self.workspace.update(cx, |w, _| w.stays_in_dock = stays));
    }

    /// How many times the app was asked to end.
    fn quits(&self, cx: &TestAppContext) -> usize {
        cx.read(|cx| self.workspace.read(cx).quits)
    }

    /// The user quits the app (the Quit chord, the palette, the leader).
    fn quit(&self, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            self.workspace
                .update(cx, |w, cx| w.request_quit(window, cx))
        })
        .unwrap();
    }

    fn is_closed(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, _, _| ()).is_err()
    }
}

/// A vault holding `notes`, started on `first` and driven until its text
/// shows.
fn started_on(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    first: &str,
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(notes, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path(first))),
        v.settings.clone(),
    );
    let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
    drive(rt, cx, |cx| started.text(cx) == want);
    (v, started)
}

// ── saving ───────────────────────────────────────────────────────────────

/// Edit, primary+S, the background save reports, the buffer is clean, and
/// the file holds the edit. The open was pushed to the history.
#[gpui_kit::test]
fn the_save_chord_writes_the_open_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    assert_eq!(started.note(cx), Some(path("a.md")));
    assert!(started.shows(PATH_ID, cx));
    assert!(started.shows(SAVE_ID, cx));
    assert_eq!(v.history(), vec![path("a.md")]);

    started.type_text("X", cx);
    assert!(started.dirty(cx));
    assert!(started.footer_dirty(cx), "the Footer follows the editor");
    started.press("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.dirty(cx));
    assert!(!started.footer_dirty(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
}

/// The restored note (the history's newest) is the one saved to.
#[gpui_kit::test]
fn the_restored_note_saves_to_its_own_path(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(
        &[("a.md", "alpha"), ("b.md", "beta")],
        &["b.md"],
    ));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    drive(&rt, cx, |cx| started.text(cx) == "beta");
    started.type_text("X", cx);
    started.press("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.dirty(cx));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("Xbeta"));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));
}

/// The periodic tick writes without being asked, at the settings' interval.
#[gpui_kit::test]
fn the_autosave_tick_writes_on_its_own(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha")], &[]));
    v.settings.write().unwrap().autosave_interval_secs = 1;
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    started.type_text("X", cx);
    drive(&rt, cx, |cx| !started.dirty(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
}

/// A failed background save says why, with core's error, and leaves the
/// buffer dirty.
#[gpui_kit::test]
fn a_failed_save_is_flashed_with_its_error(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.type_text("X", cx);
    send(
        &started,
        AppMessage::AutosaveCompleted {
            path: path("a.md"),
            saved_revision: None,
            title: None,
            error: Some("disk full".into()),
        },
        cx,
    );
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Couldn't save a.md: disk full")
    );
    assert!(started.dirty(cx));
}

/// Nothing is written while the vault is read-only (a case conflict): not
/// by the chord, which says so, nor by the tick.
#[gpui_kit::test]
fn a_read_only_vault_writes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    send(
        &started,
        AppMessage::VaultConflict("conflicts:\na.md / A.md".into()),
        cx,
    );
    // Preferences > Workspaces show the conflict over the window: close
    // them (nothing changed, Esc closes) to get back to the note.
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            started
                .workspace
                .read(cx)
                .preferences
                .as_ref()
                .is_some_and(|p| p.read(cx).model().is_some())
        })
    });
    started.press("escape", cx);
    assert!(cx.read(|cx| started.workspace.read(cx).preferences.is_none()));
    started.type_text("X", cx);
    started.press("ctrl-s", cx);
    send(&started, AppMessage::Autosave, cx);
    drive(&rt, cx, |_| true);
    assert!(started.flash(cx).unwrap().starts_with("Read-only: a.md"));
    assert!(started.dirty(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));

    // Closing says the edits can't be kept; closing again discards them.
    assert!(!started.close(cx));
    assert!(started.flash(cx).unwrap().contains("Close again"));
    assert!(started.close(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));
}

// ── switching notes ──────────────────────────────────────────────────────

/// Opening another note saves the one left first.
#[gpui_kit::test]
fn switching_notes_saves_the_one_left(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    started.type_text("X", cx);
    send(&started, AppMessage::open(path("b.md")), cx);
    drive(&rt, cx, |cx| started.text(cx) == "beta");
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
    assert_eq!(started.note(cx), Some(path("b.md")));
    assert!(!started.dirty(cx));
    assert_eq!(v.history(), vec![path("b.md"), path("a.md")]);
}

/// F1 through the GUI: while the new note is being read, a tick, the Save
/// chord and more typing in the old buffer must not write the old text into
/// the new note (nor anywhere else).
#[gpui_kit::test]
fn nothing_is_written_while_an_open_is_pending(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    started.type_text("X", cx);
    send(&started, AppMessage::open(path("b.md")), cx);
    // The flush wrote a.md and the read of b.md is queued, not yet run.
    drive(&rt, cx, |cx| started.open_pending(cx));
    assert_eq!(started.session_path(cx), path("a.md"));
    started.type_text("Y", cx);
    // Not taken at all (L1): the open would drop it.
    assert_eq!(started.text(cx), "Xalpha");
    send(&started, AppMessage::Autosave, cx);
    started.press("ctrl-s", cx);

    drive(&rt, cx, |cx| started.text(cx) == "beta");
    drive(&rt, cx, |_| true);
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
    assert!(!started.dirty(cx));
}

/// A note that is not there is offered for creation, and the editor stays
/// on the open note: declined, its edits still save to its own path, never
/// to the missing one.
#[gpui_kit::test]
fn a_missing_note_leaves_the_open_one_in_place(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    send(&started, AppMessage::open(path("missing.md")), cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(super::modal::ModalKind::CreateNote)
    });
    started.press("escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert_eq!(started.text(cx), "alpha");
    assert_eq!(started.note(cx), Some(path("a.md")));
    assert_eq!(started.session_path(cx), path("a.md"));

    started.type_text("X", cx);
    started.press("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.dirty(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
    assert_eq!(disk(&rt, &v, "missing.md"), None);
    assert_eq!(v.history(), vec![path("a.md")]);
}

// ── closing ──────────────────────────────────────────────────────────────

/// Nothing to save: the window closes at once.
#[gpui_kit::test]
fn a_clean_note_closes_at_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    assert!(started.close(cx));
}

/// Unsaved edits: the close waits for the flush, then the window goes.
#[gpui_kit::test]
fn closing_with_unsaved_edits_saves_them_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.type_text("X", cx);
    assert!(!started.close(cx), "the close waits for the save");
    assert_eq!(started.flash(cx).as_deref(), Some("Saving a.md…"));
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).closing.clone()),
        Closing::Flushing
    );
    drive(&rt, cx, |cx| started.is_closed(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
}

/// A close asked mid-switch waits for the switch, then flushes and closes.
#[gpui_kit::test]
fn closing_during_a_switch_saves_and_closes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    started.type_text("X", cx);
    send(&started, AppMessage::open(path("b.md")), cx);
    assert!(!started.close(cx));
    drive(&rt, cx, |cx| started.is_closed(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
}

// ── quit and the dock ─────────────────────────────────────────────────────

/// On macOS the close button closes the window and the app stays in the dock.
#[gpui_kit::test]
fn the_close_button_leaves_the_app_in_the_dock(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    assert!(
        started.close(cx),
        "nothing to save: the window closes at once"
    );
    assert_eq!(started.quits(cx), 0, "the app was not asked to end");
}

/// Quitting inside the app ends it, dock or not.
#[gpui_kit::test]
fn quitting_ends_the_app_even_in_the_dock(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    started.quit(cx);
    assert_eq!(started.quits(cx), 1);
    assert!(
        !started.is_closed(cx),
        "the app ends, the window is not removed first"
    );
}

/// A quit with unsaved edits saves them, then ends the app: once.
#[gpui_kit::test]
fn quitting_with_unsaved_edits_saves_then_ends_the_app(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    started.type_text("X", cx);
    started.quit(cx);
    assert_eq!(started.quits(cx), 0, "the app waits for the save");
    drive(&rt, cx, |cx| started.quits(cx) > 0);
    assert_eq!(started.quits(cx), 1);
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
}

/// The menu bar's Quit (and Cmd+Q with no focus in the window) goes through the
/// workspace, so the open note is saved before the app ends.
#[gpui_kit::test]
fn the_app_level_quit_saves_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.type_text("X", cx);
    cx.update(|cx| {
        crate::dock::set_main(cx, started.handle, started.workspace.downgrade());
        // As the menu does: from inside the active window's own update.
        started
            .handle
            .update(cx, |_, _, cx| crate::dock::quit_app(cx))
            .unwrap();
    });
    assert_eq!(started.quits(cx), 0, "the app waits for the save");
    drive(&rt, cx, |cx| started.quits(cx) > 0);
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
}

/// Off macOS the app ends with its window.
#[gpui_kit::test]
fn off_macos_the_close_button_ends_the_app(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(false, cx);
    assert!(started.close(cx));
    assert_eq!(started.quits(cx), 1);
}

/// A quit that did not go through (the flag stays set) must not turn the next
/// close-button click into a quit.
#[gpui_kit::test]
fn a_refused_quit_does_not_make_the_close_button_quit(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    cx.update(|cx| started.workspace.update(cx, |w, _| w.quit_on_close = true));
    assert!(started.close(cx));
    assert_eq!(started.quits(cx), 0);
}

// ── fix wave (review-g1-g2) ──────────────────────────────────────────────

/// Makes a note's file unwritable while alive, so its saves really fail;
/// the permissions come back on drop (the temp dir must stay removable).
#[cfg(unix)]
struct Unwritable(std::path::PathBuf);

#[cfg(unix)]
impl Unwritable {
    fn new(v: &TestVault, p: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let file = v.workspace.as_path().join(p);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        Self(file)
    }
}

#[cfg(unix)]
impl Drop for Unwritable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644)).ok();
    }
}

impl Started {
    fn closing(&self, cx: &TestAppContext) -> Closing {
        cx.read(|cx| self.workspace.read(cx).closing.clone())
    }

    fn flash_is_error(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .last_flash()
                .is_some_and(|(_, error)| error)
        })
    }

    fn switching(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .active_state()
                .notes
                .as_ref()
                .is_some_and(|h| h.is_switching())
        })
    }
}

/// H1: before the restore lands there is no editor to type into, so
/// nothing typed then can be lost under the restored note.
#[gpui_kit::test]
fn nothing_can_be_typed_before_the_first_note_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("b.md", "beta")], &["b.md"]));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    started.type_text("X", cx);
    assert_eq!(started.text(cx), "");
    assert!(!started.dirty(cx));
    drive(&rt, cx, |cx| started.text(cx) == "beta");
    assert!(!started.dirty(cx));
    assert_eq!(started.flash(cx), None, "no \"not kept\" warning");
    started.type_text("X", cx);
    assert_eq!(started.text(cx), "Xbeta", "typing works once it is open");
}

/// A switch whose flush really fails stays on the open note, edits kept,
/// and says why in the error colour.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_switch_flush_keeps_the_open_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    let _locked = Unwritable::new(&v, "a.md");
    started.type_text("X", cx);
    send(&started, AppMessage::open(path("b.md")), cx);
    drive(&rt, cx, |cx| {
        started.flash(cx).is_some() && !started.switching(cx)
    });
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save a.md:") && flash.ends_with("It stays open."),
        "{flash}"
    );
    assert!(started.flash_is_error(cx));
    assert_eq!(started.text(cx), "Xalpha");
    assert_eq!(started.note(cx), Some(path("a.md")));
    assert!(started.dirty(cx));
}

/// H2: a close that waited on a failed switch flush does not arm "discard":
/// the next close tries the flush again and warns before anything is lost.
#[cfg(unix)]
#[gpui_kit::test]
fn a_close_after_a_failed_switch_flush_tries_again(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    let _locked = Unwritable::new(&v, "a.md");
    started.type_text("X", cx);
    send(&started, AppMessage::open(path("b.md")), cx);
    assert!(!started.close(cx));
    assert_eq!(started.closing(cx), Closing::Requested);
    drive(&rt, cx, |cx| !started.switching(cx));
    assert_eq!(started.closing(cx), Closing::Open);
    assert!(!started.close(cx), "the second close retries the flush");
    drive(&rt, cx, |cx| {
        matches!(started.closing(cx), Closing::Discard(_))
    });
    assert!(started.flash(cx).unwrap().contains("Close again"));
    assert!(started.close(cx), "warned: now it discards");
}

/// A close flush that really fails keeps the window open and warns; the
/// next close discards.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_close_flush_warns_then_discards(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    let locked = Unwritable::new(&v, "a.md");
    started.type_text("X", cx);
    assert!(!started.close(cx));
    drive(&rt, cx, |cx| {
        matches!(started.closing(cx), Closing::Discard(_))
    });
    assert!(!started.is_closed(cx));
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save a.md:")
            && flash.ends_with("Close again to discard your edits."),
        "{flash}"
    );
    assert!(started.flash_is_error(cx));
    assert!(started.close(cx));
    drop(locked);
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));
}

/// M2: the "close again discards" state ends with a new edit, and when its
/// warning's time is up.
#[cfg(unix)]
#[gpui_kit::test]
fn discarding_expires_on_new_edits_and_with_time(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    let _locked = Unwritable::new(&v, "a.md");
    started.type_text("X", cx);
    assert!(!started.close(cx));
    drive(&rt, cx, |cx| {
        matches!(started.closing(cx), Closing::Discard(_))
    });
    started.type_text("Y", cx);
    assert!(!started.close(cx), "new edits: the flush is tried again");
    drive(&rt, cx, |cx| {
        matches!(started.closing(cx), Closing::Discard(_))
    });

    cx.executor()
        .advance_clock(DISCARD_WINDOW + std::time::Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(started.closing(cx), Closing::Open);
    assert!(
        !started.close(cx),
        "the warning is gone: try again, warn again"
    );
    drive(&rt, cx, |cx| {
        matches!(started.closing(cx), Closing::Discard(_))
    });
}

/// The "close again discards" window keeps its own clock: closing its
/// warning toast early (its X, or a toast layer that dropped it) neither
/// ends nor extends it.
#[cfg(unix)]
#[gpui_kit::test]
fn the_discard_window_does_not_follow_the_toast(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    let locked = Unwritable::new(&v, "a.md");
    started.type_text("X", cx);
    assert!(!started.close(cx));
    drive(&rt, cx, |cx| {
        matches!(started.closing(cx), Closing::Discard(_))
    });
    cx.update_window(started.handle, |_, window, cx| {
        window.clear_notifications(cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(DISCARD_WINDOW - std::time::Duration::from_millis(100));
    cx.run_until_parked();
    assert!(
        matches!(started.closing(cx), Closing::Discard(_)),
        "the toast went, the window holds"
    );
    assert!(started.close(cx), "inside the window: the edits go");
    drop(locked);
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));
}

/// M1: a read that never comes back (its task cancelled) gives up the
/// pending open, so saves and the close work again, with no spinning.
#[gpui_kit::test]
fn a_lost_read_lets_saves_and_the_close_resume(cx: &mut TestAppContext) {
    use kimun_notes::logic::session::NoteRead;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    started.type_text("X", cx);
    let lost = tokio::spawn(std::future::pending::<NoteRead>());
    lost.abort();
    let Err(error) = rt.block_on(lost) else {
        panic!("the aborted read reports a JoinError");
    };
    started.workspace.update(cx, |w, cx| {
        let _read = w
            .active_state_mut()
            .notes
            .as_mut()
            .unwrap()
            .session
            .begin_open(path("b.md"));
        w.active_state_mut()
            .editor
            .update(cx, |view, _| view.input_blocked = true);
        w.switch_read(w.layout.active_group(), Err(error), cx);
    });
    assert!(!started.open_pending(cx));
    assert!(started.flash_is_error(cx));
    started.type_text("Y", cx);
    assert_eq!(started.text(cx), "XYalpha", "input is taken again");
    assert!(!started.close(cx));
    drive(&rt, cx, |cx| started.is_closed(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("XYalpha"));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
}

/// L8, as toasts (N1): a later notice no longer displaces an error flash;
/// both show, each its own toast, the error with its error kind.
#[gpui_kit::test]
fn an_error_flash_stays_beside_a_later_notice(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    send(
        &started,
        AppMessage::AutosaveCompleted {
            path: path("a.md"),
            saved_revision: None,
            title: None,
            error: Some("disk full".into()),
        },
        cx,
    );
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Couldn't save a.md: disk full")
    );
    assert!(started.flash_is_error(cx));
    send(&started, AppMessage::FlashMessage("copied".into()), cx);
    cx.run_until_parked();
    assert_eq!(started.flash(cx).as_deref(), Some("copied"));
    assert!(!started.flash_is_error(cx));
    let flashes = cx.read(|cx| started.workspace.read(cx).flashes.clone());
    assert_eq!(
        flashes,
        [
            ("Couldn't save a.md: disk full".into(), true),
            ("copied".into(), false)
        ]
    );
    assert_eq!(started.toasts(cx), 2, "the error toast was not displaced");
}

/// The window's editor completes `[[` from the open vault: the popup lists
/// the vault's notes, Tab writes the link, and the note saves with it.
#[gpui_kit::test]
fn the_workspace_editor_completes_wikilinks_from_its_vault(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    // The suggestions come from the index: wait for the startup indexing.
    drive(&rt, cx, |cx| {
        cx.read(|cx| !started.workspace.read(cx).status.indexing)
    });
    started.type_text("[[b", cx);
    let listed = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            started
                .workspace
                .read(cx)
                .active_state()
                .editor
                .read(cx)
                .autocomplete_state()
                .map(|s| {
                    s.items
                        .iter()
                        .map(|i| i.display.clone())
                        .collect::<Vec<_>>()
                })
        })
    };
    drive(&rt, cx, |cx| listed(cx).is_some());
    assert_eq!(listed(cx), Some(vec!["b".to_string()]));
    started.press("tab", cx);
    assert_eq!(started.text(cx), "[[b]]alpha");
    started.press("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.dirty(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("[[b]]alpha"));
}

// ── the git summary ──────────────────────────────────────────────────────

/// Whether a usable `git` is on the machine (the summary shells out to it).
fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn flashes(started: &Started, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .flashes
            .iter()
            .map(|(text, _)| text.to_string())
            .collect()
    })
}

fn git_segment(started: &Started, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| started.workspace.read(cx).status.git.map(|g| g.label()))
}

/// A workspace that is a git repository: the Footer shows `git ●N` for its
/// changed paths, and leader `g s` says the same in a toast.
#[gpui_kit::test]
fn a_git_workspace_shows_its_summary_and_leader_g_s_flashes_it(cx: &mut TestAppContext) {
    if !git_available() {
        return;
    }
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha"), ("b.md", "beta")], &[]));
    let init = std::process::Command::new("git")
        .arg("-C")
        .arg(v.workspace.as_path())
        .args(["init", "-q"])
        .status()
        .unwrap();
    assert!(init.success());
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| git_segment(&started, cx).is_some());
    let shown = git_segment(&started, cx).unwrap();
    assert!(
        shown.starts_with("git ●"),
        "untracked notes are changes: {shown}"
    );
    assert!(started.shows(GIT_ID, cx), "the Footer has the segment");
    started.press("ctrl-shift-b", cx);
    started.type_text("gs", cx);
    // The toast says more than the footer: the branch and the kinds.
    drive(&rt, cx, |cx| {
        flashes(&started, cx)
            .iter()
            .any(|t| t.starts_with("On ") && t.contains("untracked"))
    });
}

/// Not a repository: no segment, and `g s` says so.
#[gpui_kit::test]
fn without_a_repository_there_is_no_segment_and_g_s_says_so(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    cx.run_until_parked();
    assert_eq!(git_segment(&started, cx), None);
    assert!(!started.shows(GIT_ID, cx));
    started.press("ctrl-shift-b", cx);
    started.type_text("gs", cx);
    drive(&rt, cx, |cx| {
        flashes(&started, cx).contains(&"Not a git repository".to_string())
    });
}

// ── the server's update notice ───────────────────────────────────────────

/// The server reporting a newer release of itself: one toast per news.
#[gpui_kit::test]
fn a_newer_server_release_is_toasted_once(cx: &mut TestAppContext) {
    use kimun_notes::server_client::sync::ServerUpdate;
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    let toasts = |cx: &TestAppContext| {
        flashes(&started, cx)
            .into_iter()
            .filter(|t| t.contains("Kimün server"))
            .collect::<Vec<_>>()
    };
    send(
        &started,
        AppMessage::ServerUpdate(Some(ServerUpdate::Newer("9.9.9".into()))),
        cx,
    );
    cx.run_until_parked();
    assert_eq!(
        toasts(cx),
        ["Kimün server 9.9.9 is available. Update it the way you installed it."]
    );
    // The same news again: nothing new.
    send(
        &started,
        AppMessage::ServerUpdate(Some(ServerUpdate::Newer("9.9.9".into()))),
        cx,
    );
    // Gone, then back: the news is news again.
    send(&started, AppMessage::ServerUpdate(None), cx);
    send(
        &started,
        AppMessage::ServerUpdate(Some(ServerUpdate::Legacy)),
        cx,
    );
    cx.run_until_parked();
    assert_eq!(toasts(cx).len(), 2);
    assert!(toasts(cx)[1].starts_with("A newer Kimün server is available"));
}

/// A window that closes while the app stays in the dock waits for the layout's last
/// write: the window opened next reads the config file and must find it.
#[gpui_kit::test]
fn closing_a_window_in_the_dock_flushes_the_layout(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    assert!(started.close(cx));
    assert_eq!(cx.read(|cx| started.workspace.read(cx).layout_flushes), 1);
}

/// The same when the close has to save the note first.
#[gpui_kit::test]
fn closing_a_dirty_window_in_the_dock_flushes_the_layout(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    started.type_text("X", cx);
    assert!(!started.close(cx), "the note is saved first");
    drive(&rt, cx, |cx| started.is_closed(cx));
    assert_eq!(cx.read(|cx| started.workspace.read(cx).layout_flushes), 1);
}

/// Quit with edits to save, then the red button while the save runs: the app still
/// ends with the save, it is not left in the dock.
#[gpui_kit::test]
fn the_close_button_during_a_quit_flush_does_not_cancel_the_quit(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    started.type_text("X", cx);
    started.quit(cx);
    assert!(!started.close(cx), "the flush is still running");
    drive(&rt, cx, |cx| started.quits(cx) > 0);
    assert_eq!(started.quits(cx), 1, "the quit went through");
}

/// A quit that was refused leaves no intent behind: Preferences closing the window
/// later is a window close, not a quit.
#[gpui_kit::test]
fn a_refused_quit_does_not_make_a_later_window_close_a_quit(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    cx.update(|cx| started.workspace.update(cx, |w, _| w.quit_on_close = true));
    cx.update_window(started.handle, |_, window, cx| {
        started
            .workspace
            .update(cx, |w, cx| w.request_close(window, cx))
    })
    .unwrap();
    assert_eq!(started.quits(cx), 0, "a window close, not a quit");
}

/// The menu's Close Window (Cmd+W) closes the window as the red button does: the
/// open note is saved, and the app stays in the dock.
#[gpui_kit::test]
fn the_app_level_close_window_saves_and_leaves_the_app(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.stays_in_dock(true, cx);
    started.type_text("X", cx);
    cx.update(|cx| {
        crate::dock::set_main(cx, started.handle, started.workspace.downgrade());
        started
            .handle
            .update(cx, |_, _, cx| crate::dock::close_window(cx))
            .unwrap();
    });
    drive(&rt, cx, |cx| started.is_closed(cx));
    assert_eq!(started.quits(cx), 0, "the app stays");
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
}

// ── groups ───────────────────────────────────────────────────────────────

/// A switch remembers the group that started it: the flush it ran lands in
/// that group even when another one became active meanwhile.
#[gpui_kit::test]
fn a_flush_result_applies_to_the_group_that_started_it(cx: &mut TestAppContext) {
    use kimun_notes::logic::layout_tree::{Axis, Focus, Leaf};

    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    started.type_text("X", cx);
    assert!(started.dirty(cx));

    let (first, second) = cx.update(|cx| {
        started.workspace.update(cx, |w, cx| {
            let first = w.layout.active_group();
            let second = w
                .layout
                .region_mut()
                .split_group(first, Axis::Horizontal)
                .expect("the first group splits");
            let settings = v.settings.clone();
            w.add_group_for_test(second, Some(settings), cx);
            // The switch starts in the first group, its flush still running.
            w.open_note_in(first, path("b.md"), cx);
            // Another group becomes the active one before the flush lands.
            assert!(w.layout.region_mut().set_active_group(second));
            assert!(w.layout.focus(Focus::Leaf(Leaf::Editor)));
            (first, second)
        })
    });

    let first_note = |cx: &TestAppContext| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.state(first)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.note.clone())
        })
    };
    drive(&rt, cx, |cx| first_note(cx) == Some(path("b.md")));

    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert_eq!(w.layout.active_group(), second);
        let other = w.state(second).expect("the second group has a state");
        assert_eq!(other.notes.as_ref().and_then(|h| h.note.clone()), None);
        assert_eq!(other.editor.read(cx).editor.get_text(), "");
        let own = w.state(first).expect("the first group has a state");
        assert_eq!(own.editor.read(cx).editor.get_text(), "beta");
    });
}

/// The "close again discards" guard compares each group's text with the
/// warning's, not one sum of revisions: a group taken away after the
/// warning (its tab's close was already writing it) lowers a sum that new
/// edits elsewhere can raise back to the same number. Those new edits are
/// not discarded: the close warns again.
#[gpui_kit::test]
fn the_discard_guard_is_not_fooled_by_revisions_that_sum_equal(cx: &mut TestAppContext) {
    use super::file_ops::FileOpRequest;
    use kimun_notes::logic::layout_tree::GroupId;

    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    let first = cx.read(|cx| started.workspace.read(cx).layout.active_group());
    // The first group's note is deleted: an empty group, whose text can
    // never be written.
    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| {
            w.request_file_op(FileOpRequest::Delete(path("a.md")), window, cx)
        })
    })
    .unwrap();
    drive(&rt, cx, |cx| {
        cx.read(|cx| started.workspace.read(cx).file_task.is_none())
    });
    // A second tab on b.md, shown.
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_note_in_new_tab(path("b.md"), cx))
    });
    drive(&rt, cx, |cx| started.text(cx) == "beta");
    let second = cx.read(|cx| started.workspace.read(cx).layout.active_group());
    let editor_of = |group: GroupId, cx: &TestAppContext| {
        cx.read(|cx| {
            started
                .workspace
                .read(cx)
                .state(group)
                .unwrap()
                .editor
                .clone()
        })
    };
    let revision = |group: GroupId, cx: &TestAppContext| {
        let editor = editor_of(group, cx);
        cx.read(|cx| editor.read(cx).editor.content_revision().get())
    };
    let type_in = |group: GroupId, text: &str, cx: &mut TestAppContext| {
        let editor = editor_of(group, cx);
        cx.update(|cx| {
            editor.update(cx, |view, cx| {
                view.editor.insert_at_cursor(text);
                cx.notify();
            })
        });
    };
    type_in(first, "scratch", cx);

    // The second tab's close starts writing it (its flush waits on the
    // runtime); the window's close then warns about the first group.
    started.press("ctrl-w", cx);
    assert!(cx.read(|cx| started.workspace.read(cx).groups_closing.is_some()));
    assert!(!started.close(cx));
    assert!(matches!(started.closing(cx), Closing::Discard(_)));
    let warned = revision(first, cx) + revision(second, cx);

    // The tab goes; new text in the first group brings the sum back.
    drive(&rt, cx, |cx| {
        cx.read(|cx| started.workspace.read(cx).layout.region().tabs().len()) == 1
    });
    assert!(matches!(started.closing(cx), Closing::Discard(_)));
    while revision(first, cx) < warned {
        type_in(first, "x", cx);
    }
    assert_eq!(revision(first, cx), warned, "the sums are equal");

    assert!(
        !started.close(cx),
        "new edits since the warning: warn again"
    );
    assert!(!started.is_closed(cx));
}
