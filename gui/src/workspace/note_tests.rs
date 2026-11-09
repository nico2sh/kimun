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
use crate::status_row::{PATH_ID, SAVE_ID};
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
        cx.read(|cx| self.workspace.read(cx).editor.read(cx).editor.is_dirty())
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
                .notes
                .as_ref()
                .is_some_and(|h| h.session.is_open_pending())
        })
    }

    fn session_path(&self, cx: &TestAppContext) -> VaultPath {
        cx.read(|cx| {
            self.workspace
                .read(cx)
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
        cx.read(|cx| started.workspace.read(cx).closing),
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
        cx.read(|cx| self.workspace.read(cx).closing)
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
        let _read = w.notes.as_mut().unwrap().session.begin_open(path("b.md"));
        w.editor.update(cx, |view, _| view.input_blocked = true);
        w.switch_read(Err(error), cx);
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
