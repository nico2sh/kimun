use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;
use crate::logic::editor::Editor;
use crate::logic::message::AppMessage;
use crate::settings::{AppSettings, SharedSettings};

struct Fixture {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    session: NoteSession,
    ed: Editor,
    tx: AppTx,
    rx: UnboundedReceiver<AppMessage>,
    _history: tempfile::TempDir,
}

/// A session over a temp vault holding `n.md` ("hi"), with history wired to a
/// scratch dir. Nothing is open yet.
async fn fixture(prefix: &str) -> Fixture {
    let vault = crate::test_support::temp_vault(prefix).await;
    vault
        .create_note(&VaultPath::new("n.md"), "hi")
        .await
        .unwrap();
    let history = tempfile::TempDir::new().unwrap();
    let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::for_test_workspace(
        "ws",
        vault.workspace_path(),
        crate::test_support::sys(history.path()),
    )));
    let session = NoteSession::new(vault.clone(), settings.clone(), VaultPath::root());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    Fixture {
        vault,
        settings,
        session,
        ed: Editor::new(false),
        tx,
        rx,
        _history: history,
    }
}

/// `fixture` with `n.md` open in the editor.
async fn on_note(prefix: &str) -> Fixture {
    let mut f = fixture(prefix).await;
    let opening = f
        .session
        .open(VaultPath::new("n.md"), Some(&mut f.ed), &f.tx)
        .await;
    assert_eq!(
        opening.outcome,
        OpenResult::Note(OpenOutcome::Opened(VaultPath::new("n.md")))
    );
    f
}

/// Sets its flag when dropped: proves a spawned future was cancelled.
struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn type_x(ed: &mut Editor) {
    ed.insert_at_cursor("x");
    assert!(ed.is_dirty(), "precondition: typing dirties the buffer");
}

async fn disk(vault: &NoteVault, path: &str) -> String {
    vault.get_note_text(&VaultPath::new(path)).await.unwrap()
}

// ---- Saving ----------------------------------------------------------------

#[tokio::test]
async fn save_now_on_a_clean_buffer_writes_nothing() {
    let mut f = on_note("session-clean").await;
    assert_eq!(
        f.session.save_now(Some(&mut f.ed)).await,
        SaveOutcome::Clean
    );
    assert_eq!(f.session.save_now::<Editor>(None).await, SaveOutcome::Clean);
}

#[tokio::test]
async fn save_now_writes_a_dirty_buffer_and_marks_it_clean() {
    let mut f = on_note("session-dirty").await;
    type_x(&mut f.ed);
    let outcome = f.session.save_now(Some(&mut f.ed)).await;
    let SaveOutcome::Saved(saved) = outcome else {
        panic!("expected Saved, got {outcome:?}");
    };
    assert_eq!(saved.path, VaultPath::new("n.md"));
    assert!(!f.ed.is_dirty());
    assert_eq!(disk(&f.vault, "n.md").await, "xhi");
}

/// A background save that never finishes: the flush waits `SAVE_TIMEOUT`,
/// aborts it, and writes nothing — two writers on one path is the race the
/// slot prevents. The buffer stays dirty for the next try.
#[tokio::test]
async fn a_stuck_background_save_aborts_the_flush_without_writing() {
    let mut f = on_note("session-timeout").await;
    type_x(&mut f.ed);
    let cancelled = Arc::new(AtomicBool::new(false));
    let guard = SetOnDrop(cancelled.clone());
    f.session.autosave_task_mut().spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await
    });
    f.session
        .set_save_timeout(std::time::Duration::from_millis(50));
    let outcome = f.session.save_now(Some(&mut f.ed)).await;
    assert_eq!(outcome, SaveOutcome::InFlightTimedOut);
    // abort() cancels at the task's next poll: give the runtime a moment.
    for _ in 0..100 {
        if cancelled.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        cancelled.load(Ordering::SeqCst),
        "the stuck save is aborted (its future was dropped)"
    );
    assert!(f.ed.is_dirty(), "nothing was written");
    assert_eq!(disk(&f.vault, "n.md").await, "hi");
}

#[tokio::test]
async fn spawn_autosave_reports_a_completion_that_marks_the_buffer_clean() {
    let mut f = on_note("session-autosave").await;
    type_x(&mut f.ed);
    assert!(f.session.spawn_autosave(Some(&f.ed), &f.tx));
    let Some(AppMessage::AutosaveCompleted {
        path,
        saved_revision,
        title,
        error,
    }) = f.rx.recv().await
    else {
        panic!("expected AutosaveCompleted");
    };
    let done = f.session.on_autosave_completed(
        SaveCompleted {
            path,
            saved_revision,
            title,
            error,
        },
        Some(&mut f.ed),
    );
    assert!(done.is_open_note && done.wrote);
    assert_eq!(done.saved.unwrap().path, VaultPath::new("n.md"));
    assert!(!f.ed.is_dirty());
    assert_eq!(disk(&f.vault, "n.md").await, "xhi");
}

#[tokio::test]
async fn spawn_autosave_declines_when_clean_or_in_flight() {
    let mut f = on_note("session-decline").await;
    assert!(!f.session.spawn_autosave(Some(&f.ed), &f.tx), "clean");
    type_x(&mut f.ed);
    f.session
        .autosave_task_mut()
        .spawn(std::future::pending::<()>());
    assert!(!f.session.spawn_autosave(Some(&f.ed), &f.tx), "in flight");
    f.session.abort_autosave();
}

/// A flush prepared and not yet applied (a GUI switch awaiting its save)
/// keeps background saves off the path; dropping or applying it lifts that.
#[tokio::test]
async fn a_pending_flush_blocks_background_saves() {
    let mut f = on_note("session-flushguard").await;
    type_x(&mut f.ed);
    let req = f.session.prepare_flush(Some(&f.ed)).unwrap();
    assert!(!f.session.spawn_autosave(Some(&f.ed), &f.tx));
    let result = req.run().await;
    assert!(
        !f.session.spawn_autosave(Some(&f.ed), &f.tx),
        "result unapplied"
    );
    assert!(matches!(
        f.session.apply_flush(result, Some(&mut f.ed)),
        SaveOutcome::Saved(_)
    ));
    type_x(&mut f.ed);
    assert!(f.session.spawn_autosave(Some(&f.ed), &f.tx));
}

#[tokio::test]
async fn a_completion_for_another_path_marks_nothing() {
    let mut f = on_note("session-stalepath").await;
    type_x(&mut f.ed);
    let done = f.session.on_autosave_completed(
        SaveCompleted {
            path: VaultPath::new("other.md"),
            saved_revision: Some(f.ed.content_revision()),
            title: Some("other".into()),
            error: None,
        },
        Some(&mut f.ed),
    );
    assert!(f.ed.is_dirty());
    assert!(!done.is_open_note);
    assert_eq!(
        done.saved,
        Some(Saved {
            path: VaultPath::new("other.md"),
            title: "other".into()
        }),
        "the other note's row is still retitled"
    );
}

#[tokio::test]
async fn a_completion_for_an_older_revision_marks_nothing() {
    let mut f = on_note("session-stalerev").await;
    type_x(&mut f.ed);
    let old = f.ed.content_revision();
    type_x(&mut f.ed);
    f.session.on_autosave_completed(
        SaveCompleted {
            path: VaultPath::new("n.md"),
            saved_revision: Some(old),
            title: Some("hi".into()),
            error: None,
        },
        Some(&mut f.ed),
    );
    assert!(f.ed.is_dirty(), "typing after the snapshot stays unsaved");
}

#[tokio::test]
async fn close_flushes_the_buffer() {
    let mut f = on_note("session-close").await;
    type_x(&mut f.ed);
    assert!(matches!(
        f.session.close(Some(&mut f.ed)).await,
        SaveOutcome::Saved(_)
    ));
    assert_eq!(disk(&f.vault, "n.md").await, "xhi");
}

// ---- Opening -----------------------------------------------------------------

#[tokio::test]
async fn open_loads_the_buffer_and_records_history() {
    let f = on_note("session-open").await;
    assert_eq!(f.session.path(), &VaultPath::new("n.md"));
    assert_eq!(f.ed.get_text(), "hi");
    assert!(!f.ed.is_dirty());
    assert_eq!(
        f.settings.read().unwrap().current_last_paths(),
        vec![VaultPath::new("n.md")]
    );
}

#[tokio::test]
async fn open_saves_the_current_note_before_switching() {
    let mut f = on_note("session-switch").await;
    f.vault
        .create_note(&VaultPath::new("m.md"), "other")
        .await
        .unwrap();
    type_x(&mut f.ed);
    let opening = f
        .session
        .open(VaultPath::new("m.md"), Some(&mut f.ed), &f.tx)
        .await;
    assert!(matches!(opening.saved, Some(SaveOutcome::Saved(_))));
    assert_eq!(disk(&f.vault, "n.md").await, "xhi");
    assert_eq!(f.ed.get_text(), "other");
}

#[tokio::test]
async fn opening_a_missing_note_is_not_found_and_leaves_history_alone() {
    let mut f = on_note("session-missing").await;
    let opening = f
        .session
        .open(VaultPath::new("missing.md"), Some(&mut f.ed), &f.tx)
        .await;
    assert_eq!(
        opening.outcome,
        OpenResult::Note(OpenOutcome::NotFound(VaultPath::new("missing.md")))
    );
    assert_eq!(
        f.session.path(),
        &VaultPath::new("missing.md"),
        "a create lands on the asked-for path"
    );
    assert_eq!(
        f.settings.read().unwrap().current_last_paths(),
        vec![VaultPath::new("n.md")]
    );
}

#[tokio::test]
async fn opening_a_directory_changes_nothing() {
    let mut f = on_note("session-dir").await;
    f.vault
        .create_note(&VaultPath::new("sub").append(&VaultPath::new("a.md")), "a")
        .await
        .unwrap();
    type_x(&mut f.ed);
    let opening = f
        .session
        .open(VaultPath::new("sub"), Some(&mut f.ed), &f.tx)
        .await;
    assert_eq!(
        opening.outcome,
        OpenResult::Directory(VaultPath::new("sub"))
    );
    assert_eq!(opening.saved, None, "no switch, no save");
    assert_eq!(f.session.path(), &VaultPath::new("n.md"));
    assert!(f.ed.is_dirty());
}

#[tokio::test]
async fn a_read_for_a_superseded_open_is_ignored() {
    let mut f = on_note("session-superseded").await;
    let first = f.session.begin_open(VaultPath::new("n.md"));
    let _second = f.session.begin_open(VaultPath::new("m.md"));
    let read = first.run().await;
    assert_eq!(
        f.session.finish_open(read, Some(&mut f.ed), &f.tx),
        OpenOutcome::Superseded
    );
}

// ---- Changes on disk ---------------------------------------------------------

/// The background save still targets the old path: it is aborted before the
/// retarget, so it cannot recreate the renamed-away file.
#[tokio::test]
async fn renaming_the_open_note_aborts_the_autosave_and_retargets() {
    let mut f = on_note("session-rename").await;
    f.session
        .autosave_task_mut()
        .spawn(std::future::pending::<()>());
    let to = VaultPath::new("renamed.md");
    f.vault
        .rename_note(&VaultPath::new("n.md"), &to)
        .await
        .unwrap();
    let outcome = f
        .session
        .on_renamed(&VaultPath::new("n.md"), &to, Some(&mut f.ed), &f.tx)
        .await;
    assert_eq!(outcome, RenameOutcome::Retargeted(to.clone()));
    assert!(!f.session.is_autosave_in_flight());
    assert_eq!(f.session.path(), &to);
    assert!(!f.ed.is_dirty());
}

#[tokio::test]
async fn renaming_another_note_leaves_the_session_alone() {
    let mut f = on_note("session-rename-other").await;
    f.session
        .autosave_task_mut()
        .spawn(std::future::pending::<()>());
    let outcome = f
        .session
        .on_renamed(
            &VaultPath::new("a.md"),
            &VaultPath::new("b.md"),
            Some(&mut f.ed),
            &f.tx,
        )
        .await;
    assert_eq!(outcome, RenameOutcome::NotOpen);
    assert!(f.session.is_autosave_in_flight());
    f.session.abort_autosave();
}

#[tokio::test]
async fn a_rename_whose_target_cannot_be_read_falls_back_to_the_parent() {
    let mut f = on_note("session-rename-fail").await;
    let outcome = f
        .session
        .on_renamed(
            &VaultPath::new("n.md"),
            &VaultPath::new("sub").append(&VaultPath::new("gone.md")),
            Some(&mut f.ed),
            &f.tx,
        )
        .await;
    assert_eq!(
        outcome,
        RenameOutcome::ReloadFailed {
            parent: VaultPath::new("sub")
        }
    );
}

#[tokio::test]
async fn reload_keeps_a_dirty_buffer_and_its_autosave() {
    let mut f = on_note("session-reload-dirty").await;
    type_x(&mut f.ed);
    f.session
        .autosave_task_mut()
        .spawn(std::future::pending::<()>());
    f.vault
        .save_note(&VaultPath::new("n.md"), "+++\na = 1\n+++\nhi")
        .await
        .unwrap();
    let outcome = f
        .session
        .on_reload_from_disk(&VaultPath::new("n.md"), Some(&mut f.ed))
        .await;
    assert_eq!(outcome, ReloadOutcome::KeptDirty);
    assert_eq!(f.ed.get_text(), "xhi");
    assert!(f.session.is_autosave_in_flight());
    f.session.abort_autosave();
}

#[tokio::test]
async fn reload_replaces_a_clean_buffer() {
    let mut f = on_note("session-reload").await;
    f.vault
        .save_note(&VaultPath::new("n.md"), "+++\na = 1\n+++\nbody")
        .await
        .unwrap();
    let outcome = f
        .session
        .on_reload_from_disk(&VaultPath::new("n.md"), Some(&mut f.ed))
        .await;
    assert_eq!(outcome, ReloadOutcome::Reloaded);
    assert_eq!(f.ed.get_text(), "+++\na = 1\n+++\nbody");
    assert!(!f.ed.is_dirty());
}

#[tokio::test]
async fn reload_of_another_note_or_a_vanished_one() {
    let mut f = on_note("session-reload-other").await;
    assert_eq!(
        f.session
            .on_reload_from_disk(&VaultPath::new("x.md"), Some(&mut f.ed))
            .await,
        ReloadOutcome::NotOpen
    );
    f.vault.delete_note(&VaultPath::new("n.md")).await.unwrap();
    assert_eq!(
        f.session
            .on_reload_from_disk(&VaultPath::new("n.md"), Some(&mut f.ed))
            .await,
        ReloadOutcome::Failed
    );
}

#[tokio::test]
async fn removing_the_open_note_flushes_and_names_its_parent() {
    let mut f = fixture("session-removed").await;
    f.vault
        .create_note(&VaultPath::new("sub").append(&VaultPath::new("a.md")), "a")
        .await
        .unwrap();
    f.session
        .open(
            VaultPath::new("sub").append(&VaultPath::new("a.md")),
            Some(&mut f.ed),
            &f.tx,
        )
        .await;
    let outcome = f
        .session
        .on_removed(
            &VaultPath::new("sub").append(&VaultPath::new("a.md")),
            Some(&mut f.ed),
        )
        .await;
    assert_eq!(
        outcome,
        RemovedOutcome::OpenNoteRemoved {
            saved: SaveOutcome::Clean,
            parent: VaultPath::new("sub"),
        }
    );
    let outcome = f
        .session
        .on_removed(&VaultPath::new("n.md"), Some(&mut f.ed))
        .await;
    assert_eq!(
        outcome,
        RemovedOutcome::NotOpen {
            parent: VaultPath::new("n.md").get_parent_path().0
        }
    );
}

// ---- Fix wave: pending opens (F1) ---------------------------------------------

/// Between `begin_open` and `finish_open` the buffer still holds the old
/// note: a tick, a Save chord or a second switch must never write it into
/// the new path. The session keeps naming the old note and every save
/// declines until the open lands.
#[tokio::test]
async fn saves_decline_while_an_open_is_pending() {
    let mut f = on_note("session-pending").await;
    f.vault
        .create_note(&VaultPath::new("m.md"), "other")
        .await
        .unwrap();
    type_x(&mut f.ed);
    let read = f.session.begin_open(VaultPath::new("m.md"));
    assert!(f.session.is_open_pending());
    assert_eq!(
        f.session.path(),
        &VaultPath::new("n.md"),
        "not switched yet"
    );
    // Autosave tick and Save chord (both go through spawn_autosave).
    assert!(!f.session.spawn_autosave(Some(&f.ed), &f.tx));
    assert!(f.session.prepare_save(Some(&f.ed)).is_none());
    // A second switch's flush captures nothing.
    assert_eq!(
        f.session.save_now(Some(&mut f.ed)).await,
        SaveOutcome::Clean
    );
    let read = read.run().await;
    assert_eq!(
        f.session.finish_open(read, Some(&mut f.ed), &f.tx),
        OpenOutcome::Opened(VaultPath::new("m.md"))
    );
    assert!(!f.session.is_open_pending());
    assert_eq!(disk(&f.vault, "m.md").await, "other", "new note untouched");
    assert_eq!(disk(&f.vault, "n.md").await, "hi");
    assert_eq!(f.ed.get_text(), "other");
}

/// A cancelled open (its read never came back) lets saves resume on the
/// note the buffer holds; the lost read, if it shows up, changes nothing.
#[tokio::test]
async fn a_cancelled_open_lets_saves_resume() {
    let mut f = on_note("session-cancel-open").await;
    f.vault
        .create_note(&VaultPath::new("m.md"), "other")
        .await
        .unwrap();
    type_x(&mut f.ed);
    let read = f.session.begin_open(VaultPath::new("m.md"));
    f.session.cancel_open();
    assert!(!f.session.is_open_pending());
    assert_eq!(f.session.path(), &VaultPath::new("n.md"));
    assert!(matches!(
        f.session.save_now(Some(&mut f.ed)).await,
        SaveOutcome::Saved(_)
    ));
    let late = read.run().await;
    assert_eq!(
        f.session.finish_open(late, Some(&mut f.ed), &f.tx),
        OpenOutcome::Superseded
    );
    assert_eq!(disk(&f.vault, "m.md").await, "other");
    assert_ne!(f.ed.get_text(), "other");
}

/// A second switch supersedes the first: the first read changes nothing,
/// the second lands.
#[tokio::test]
async fn a_second_switch_supersedes_the_first() {
    let mut f = on_note("session-second-switch").await;
    f.vault
        .create_note(&VaultPath::new("m.md"), "other")
        .await
        .unwrap();
    let first = f.session.begin_open(VaultPath::new("missing.md"));
    let second = f.session.begin_open(VaultPath::new("m.md"));
    let first = first.run().await;
    assert_eq!(
        f.session.finish_open(first, Some(&mut f.ed), &f.tx),
        OpenOutcome::Superseded
    );
    assert_eq!(f.session.path(), &VaultPath::new("n.md"));
    let second = second.run().await;
    assert_eq!(
        f.session.finish_open(second, Some(&mut f.ed), &f.tx),
        OpenOutcome::Opened(VaultPath::new("m.md"))
    );
}

/// Pins HEAD's TUI behaviour (a documented hazard the GUI must avoid, not a
/// rule): after a `NotFound` open the session names the missing path while
/// the buffer still holds the old note and the old tick keeps running, so
/// typing autosaves the old text into the missing path.
#[tokio::test]
async fn not_found_then_typing_saves_the_old_text_to_the_missing_path() {
    let mut f = on_note("session-notfound-type").await;
    let opening = f
        .session
        .open(VaultPath::new("missing.md"), Some(&mut f.ed), &f.tx)
        .await;
    assert_eq!(
        opening.outcome,
        OpenResult::Note(OpenOutcome::NotFound(VaultPath::new("missing.md")))
    );
    assert_eq!(f.ed.get_text(), "hi", "the old note's text stays");
    assert!(f.session.is_ticking(), "the old tick keeps running");
    type_x(&mut f.ed);
    assert!(f.session.spawn_autosave(Some(&f.ed), &f.tx));
    loop {
        if let Some(AppMessage::AutosaveCompleted { saved_revision, .. }) = f.rx.recv().await {
            assert!(saved_revision.is_some());
            break;
        }
    }
    assert_eq!(disk(&f.vault, "missing.md").await, "xhi");
}

// ---- Fix wave: one flush at a time (F2) ----------------------------------------

#[tokio::test]
async fn a_second_flush_is_refused_while_one_is_outstanding() {
    let mut f = on_note("session-two-flushes").await;
    type_x(&mut f.ed);
    let first = f.session.prepare_flush(Some(&f.ed)).unwrap();
    assert_eq!(
        f.session.prepare_flush(Some(&f.ed)).err(),
        Some(FlushPending)
    );
    assert_eq!(
        f.session.save_now(Some(&mut f.ed)).await,
        SaveOutcome::FlushPending
    );
    let result = first.run().await;
    assert_eq!(
        f.session.prepare_flush(Some(&f.ed)).err(),
        Some(FlushPending),
        "an unapplied result still counts"
    );
    assert!(matches!(
        f.session.apply_flush(result, Some(&mut f.ed)),
        SaveOutcome::Saved(_)
    ));
    assert!(f.session.prepare_flush(Some(&f.ed)).is_ok());
}

#[tokio::test]
async fn dropping_an_unapplied_flush_lifts_the_block() {
    let mut f = on_note("session-drop-flush").await;
    type_x(&mut f.ed);
    let req = f.session.prepare_flush(Some(&f.ed)).unwrap();
    assert!(!f.session.spawn_autosave(Some(&f.ed), &f.tx));
    drop(req);
    let result = f.session.prepare_flush(Some(&f.ed)).unwrap().run().await;
    assert!(!f.session.spawn_autosave(Some(&f.ed), &f.tx));
    drop(result);
    assert!(f.session.spawn_autosave(Some(&f.ed), &f.tx));
}

// ---- Fix wave: missing pins -----------------------------------------------------

#[tokio::test]
async fn a_failed_rename_read_stops_the_tick() {
    let mut f = on_note("session-rename-tick").await;
    assert!(f.session.is_ticking(), "precondition: open starts the tick");
    f.session
        .on_renamed(
            &VaultPath::new("n.md"),
            &VaultPath::new("gone.md"),
            Some(&mut f.ed),
            &f.tx,
        )
        .await;
    assert!(!f.session.is_ticking());
}

#[tokio::test]
async fn removing_the_open_note_stops_the_tick() {
    let mut f = on_note("session-remove-tick").await;
    assert!(f.session.is_ticking());
    f.session
        .on_removed(&VaultPath::new("n.md"), Some(&mut f.ed))
        .await;
    assert!(!f.session.is_ticking());
}

/// An index row's `from` is absolute while the open path is relative.
#[tokio::test]
async fn an_absolute_from_matches_the_relative_open_path() {
    let mut f = on_note("session-absolute").await;
    let from = VaultPath::new("n.md").absolute();
    assert!(matches!(
        f.session.on_removed(&from, Some(&mut f.ed)).await,
        RemovedOutcome::OpenNoteRemoved { .. }
    ));
}

#[tokio::test]
async fn an_absolute_rename_from_retargets_the_relative_open_path() {
    let mut f = on_note("session-absolute-rename").await;
    let to = VaultPath::new("r.md");
    f.vault
        .rename_note(&VaultPath::new("n.md"), &to)
        .await
        .unwrap();
    let from = VaultPath::new("n.md").absolute();
    assert_eq!(
        f.session
            .on_renamed(&from, &to, Some(&mut f.ed), &f.tx)
            .await,
        RenameOutcome::Retargeted(to)
    );
}

/// The rename may rewrite self-links: the buffer takes the new file's text.
#[tokio::test]
async fn a_rename_reloads_the_buffer_text() {
    let mut f = on_note("session-rename-text").await;
    let to = VaultPath::new("r.md");
    f.vault
        .rename_note(&VaultPath::new("n.md"), &to)
        .await
        .unwrap();
    f.vault.save_note(&to, "rewritten").await.unwrap();
    f.session
        .on_renamed(&VaultPath::new("n.md"), &to, Some(&mut f.ed), &f.tx)
        .await;
    assert_eq!(f.ed.get_text(), "rewritten");
    assert!(!f.ed.is_dirty());
}

#[tokio::test]
async fn a_failed_background_save_hands_its_error_to_the_host() {
    let f = on_note("session-save-error").await;
    let failed = || SaveCompleted {
        path: VaultPath::new("n.md"),
        saved_revision: None,
        title: None,
        error: Some("disk full".into()),
    };
    let done = f.session.on_autosave_completed::<Editor>(failed(), None);
    assert!(!done.wrote);
    assert_eq!(done.error.as_deref(), Some("disk full"));
    let msg: AppMessage = failed().into();
    assert!(matches!(
        msg,
        AppMessage::AutosaveCompleted { error: Some(e), .. } if e == "disk full"
    ));
}

// ---- Fix wave: the run futures are Send + 'static (F3) ---------------------------

fn assert_send_static<T: Send + 'static>(_: &T) {}

#[tokio::test]
async fn the_run_futures_are_send_and_static() {
    let mut f = on_note("session-send").await;
    type_x(&mut f.ed);
    let save = f.session.prepare_save(Some(&f.ed)).unwrap().run();
    assert_send_static(&save);
    drop(save);
    let flush = f.session.prepare_flush(Some(&f.ed)).unwrap().run();
    assert_send_static(&flush);
    drop(flush);
    let read = f.session.begin_open(VaultPath::new("n.md")).run();
    assert_send_static(&read);
    drop(read);
    let kind = f.session.open_kind(&VaultPath::new("n.md"));
    assert_send_static(&kind);
    let kind = classify_open(f.vault.clone(), VaultPath::new("n.md"));
    assert_send_static(&kind);
}

#[tokio::test]
async fn affects_open_note_covers_the_note_and_every_directory_holding_it() {
    let mut f = fixture("session-affects").await;
    f.session.set_path(VaultPath::new("/Work/Projects/plan.md"));
    let s = &f.session;
    // The open note itself, relative or absolute, any case.
    assert!(s.affects_open_note(&VaultPath::new("work/projects/plan.md")));
    assert!(s.affects_open_note(&VaultPath::new("/WORK/Projects/Plan.md")));
    // Every ancestor directory, the root included.
    assert!(s.affects_open_note(&VaultPath::new("work/projects")));
    assert!(s.affects_open_note(&VaultPath::new("/work")));
    assert!(s.affects_open_note(&VaultPath::root()));
    // Unrelated siblings, including name-prefix look-alikes.
    assert!(!s.affects_open_note(&VaultPath::new("work/projects/other.md")));
    assert!(!s.affects_open_note(&VaultPath::new("work/proj")));
    assert!(!s.affects_open_note(&VaultPath::new("work/projects2")));
    assert!(!s.affects_open_note(&VaultPath::new("work/projects/plan.md/x")));
    assert!(!s.affects_open_note(&VaultPath::new("work/projects/plan")));
}

// ---- Preferences: the autosave period follows a save ---------------------------

#[tokio::test]
async fn retime_restarts_a_running_tick_on_the_new_period() {
    let mut f = on_note("session-retime").await;
    let configured = f.settings.read().unwrap().autosave_interval_secs;
    assert_eq!(f.session.autosave_interval(), Some(configured));
    assert!(!f.session.retime_autosave(&f.tx), "same period: left alone");
    f.settings.write().unwrap().autosave_interval_secs = configured + 5;
    assert!(f.session.retime_autosave(&f.tx));
    assert_eq!(f.session.autosave_interval(), Some(configured + 5));
    assert!(f.session.is_ticking());
}

#[tokio::test]
async fn retime_never_starts_a_stopped_tick() {
    let mut f = on_note("session-retime-stopped").await;
    f.session.stop_autosave();
    assert_eq!(f.session.autosave_interval(), None);
    f.settings.write().unwrap().autosave_interval_secs = 120;
    assert!(!f.session.retime_autosave(&f.tx));
    assert!(!f.session.is_ticking());
    f.session.restart_autosave(&f.tx);
    assert_eq!(f.session.autosave_interval(), Some(120));
}
