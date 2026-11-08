//! UI integration tests for file operations and the open note (M6 G3): a
//! real `Workspace` on a real temp vault, the dialogs driven through the
//! window's keys and pointer, every outcome checked on disk.
//!
//! Like the other vault tests, each test owns a current-thread tokio runtime
//! and drives it between GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops::ValidationState;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::message::{AppMessage, FileOp};
use tokio::runtime::Runtime;

use super::file_dialogs::{
    CONFIRM_ID, FileOpKind, MENU_ID, MoveDialog, RenameDialog, menu_item_id,
};
use super::file_ops::FileDialogView;
use super::files::row_id;
use super::modal::ModalKind;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// The note's text on disk (`None`: not there).
fn disk(rt: &Runtime, v: &TestVault, p: &str) -> Option<String> {
    rt.block_on(v.vault.get_note_text(&path(p))).ok()
}

fn exists(rt: &Runtime, v: &TestVault, p: &str) -> bool {
    rt.block_on(v.vault.exists(&path(p)))
}

/// A vault holding `notes`, started on `first`, driven until it shows.
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

impl Started {
    fn hit(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn is_dirty(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).editor.read(cx).editor.is_dirty())
    }

    fn note_open(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).notes.as_ref()?.note.clone())
    }

    fn session_at(&self, cx: &TestAppContext) -> VaultPath {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            w.notes.as_ref().unwrap().session.path().clone()
        })
    }

    fn footer_note(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).status.note.clone())
    }

    fn modal(&self, cx: &TestAppContext) -> Option<ModalKind> {
        cx.read(|cx| self.workspace.read(cx).modal)
    }

    fn running(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).file_task.is_some())
    }

    fn dialog_open(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.has_active_dialog(cx)
        })
        .unwrap()
    }

    /// The error the open file dialog shows.
    fn op_error(&self, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| {
            let status = self.workspace.read(cx).file_status.clone()?;
            status.read(cx).error.as_ref().map(|e| e.to_string())
        })
    }

    fn rename_view(&self, cx: &TestAppContext) -> Entity<RenameDialog> {
        cx.read(|cx| match self.workspace.read(cx).file_dialog.clone() {
            Some(FileDialogView::Rename(view)) => view.upgrade().expect("the rename dialog"),
            _ => panic!("no rename dialog"),
        })
    }

    /// What the open rename dialog renames.
    fn rename_target(&self, cx: &TestAppContext) -> VaultPath {
        let view = self.rename_view(cx);
        cx.read(|cx| view.read(cx).target().clone())
    }

    fn move_view(&self, cx: &TestAppContext) -> Entity<MoveDialog> {
        cx.read(|cx| match self.workspace.read(cx).file_dialog.clone() {
            Some(FileDialogView::Move(view)) => view.upgrade().expect("the move dialog"),
            _ => panic!("no move dialog"),
        })
    }

    /// Open `kind`'s dialog for `target`, as a menu item or a leaf does.
    fn open_op_dialog(&self, kind: FileOpKind, target: &str, cx: &mut TestAppContext) {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.open_file_dialog(kind, path(target), window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        assert!(self.dialog_open(cx), "the {kind:?} dialog opens");
    }

    /// Replace the rename input's text with `name` and wait for its check.
    fn rename_to(&self, rt: &Runtime, name: &str, cx: &mut TestAppContext) -> ValidationState {
        self.hit("ctrl-a", cx);
        self.typed(name, cx);
        let view = self.rename_view(cx);
        drive(rt, cx, |cx| {
            cx.read(|cx| view.read(cx).state()) != ValidationState::Pending
        });
        cx.read(|cx| view.read(cx).state())
    }

    /// Wait for the move dialog's folders, filter them by `query`, and wait
    /// for the selected row's check.
    fn move_to(&self, rt: &Runtime, query: &str, cx: &mut TestAppContext) -> Option<VaultPath> {
        let view = self.move_view(cx);
        drive(rt, cx, |cx| !cx.read(|cx| view.read(cx).is_loading()));
        self.typed(query, cx);
        drive(rt, cx, |cx| {
            cx.read(|cx| view.read(cx).state()) != ValidationState::Pending
        });
        cx.read(|cx| view.read(cx).selected_dir())
    }

    /// Wait until the operation under way has ended.
    fn finish(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| !self.running(cx));
    }

    fn files_dir(&self, cx: &TestAppContext) -> VaultPath {
        cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone().unwrap();
            files.read(cx).current_dir().clone()
        })
    }

    fn files_rows(&self, cx: &TestAppContext) -> Vec<String> {
        cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone().unwrap();
            files
                .read(cx)
                .model()
                .core()
                .map(|core| {
                    core.visible_rows()
                        .into_iter()
                        .map(|r| r.path().to_string())
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn region_focused(&self, cx: &TestAppContext) -> Panel {
        cx.read(|cx| self.workspace.read(cx).focused_panel())
    }

    fn editor_focused(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("editor").focused() == Some(true)
        })
        .unwrap()
    }
}

/// Let a closing dialog finish leaving (the kit restores focus then).
fn settle(started: &Started, cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(started.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Let more than one autosave tick (1 s) pass on the runtime's real clock.
fn past_a_tick(rt: &Runtime, cx: &mut TestAppContext) {
    for _ in 0..30 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(50)));
        cx.run_until_parked();
    }
}

// ── rename ───────────────────────────────────────────────────────────────

/// Renaming the open note while it has unsaved edits: the edits are on disk
/// under the new name, the buffer is clean there, the old name is gone, and
/// later saves go to the new name.
#[gpui_kit::test]
fn renaming_the_dirty_open_note_keeps_its_edits(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.typed("X", cx);
    assert!(started.is_dirty(cx));
    // F2, then R: the note menu, then its rename dialog.
    started.hit("f2", cx);
    assert_eq!(started.modal(cx), Some(ModalKind::FileOps));
    started.hit("r", cx);
    assert_eq!(started.modal(cx), Some(ModalKind::Rename));
    assert_eq!(started.rename_target(cx), path("a.md"));
    assert_eq!(started.rename_to(&rt, "b", cx), ValidationState::Available);
    started.hit("enter", cx);
    started.finish(&rt, cx);

    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("Xalpha"));
    assert!(!exists(&rt, &v, "a.md"));
    assert_eq!(started.text(cx), "Xalpha");
    assert!(!started.is_dirty(cx), "reloaded clean at the new path");
    assert_eq!(started.note_open(cx), Some(path("b.md")));
    assert_eq!(started.session_at(cx), path("b.md"));
    assert_eq!(started.footer_note(cx), Some(path("b.md")));
    settle(&started, cx);
    assert!(!started.dialog_open(cx), "success closes the dialog");
    assert_eq!(started.modal(cx), None);
    assert!(
        started.editor_focused(cx),
        "the keyboard is back in the note"
    );

    // Later saves land on the new name; the old one never comes back.
    started.typed("Y", cx);
    started.hit("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.is_dirty(cx));
    assert!(disk(&rt, &v, "b.md").unwrap().contains('Y'));
    assert!(!exists(&rt, &v, "a.md"));
}

/// A flush that fails refuses the rename: nothing is renamed, the dialog
/// stays open saying why, and the note keeps its edits and its keyboard.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_flush_refuses_the_rename(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    let file = v.workspace.as_path().join("a.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    started.typed("X", cx);
    started.open_op_dialog(FileOpKind::Rename, "a.md", cx);
    assert_eq!(started.rename_to(&rt, "b", cx), ValidationState::Available);
    started.hit("enter", cx);
    started.finish(&rt, cx);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let error = started.op_error(cx).expect("the dialog says why");
    assert!(
        error.starts_with("Couldn't save a.md:") && error.ends_with("Nothing was renamed."),
        "{error}"
    );
    assert!(started.dialog_open(cx), "the dialog stays open");
    assert_eq!(started.modal(cx), Some(ModalKind::Rename));
    assert!(exists(&rt, &v, "a.md"));
    assert!(!exists(&rt, &v, "b.md"));
    assert_eq!(started.note_open(cx), Some(path("a.md")));
    assert!(started.is_dirty(cx), "the edits are still in the buffer");
    assert_eq!(started.text(cx), "Xalpha");
    // The note takes input again once the dialog is gone.
    started.hit("escape", cx);
    settle(&started, cx);
    started.typed("Z", cx);
    assert!(started.text(cx).contains('Z'));
}

/// A name that is taken keeps the confirm disabled: Enter does nothing.
#[gpui_kit::test]
fn a_taken_name_cannot_be_confirmed(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    started.open_op_dialog(FileOpKind::Rename, "a.md", cx);
    assert_eq!(started.rename_to(&rt, "B", cx), ValidationState::Taken);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(CONFIRM_ID).disabled(), Some(true));
    })
    .unwrap();
    started.hit("enter", cx);
    cx.run_until_parked();
    assert!(!started.running(cx));
    assert!(started.dialog_open(cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
}

/// A file operation landing from elsewhere closes an open file dialog and
/// forgets it, as it does every other modal (R1: the reset is one list).
#[gpui_kit::test]
fn a_file_op_landing_forgets_the_file_dialog(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    started.open_op_dialog(FileOpKind::Rename, "a.md", cx);
    assert!(cx.read(|cx| started.workspace.read(cx).file_dialog.is_some()));
    started.workspace.update(cx, |w, cx| {
        w.on_app_message(AppMessage::FileOp(FileOp::Created(path("b.md"))), cx)
    });
    started.settle_dialog(cx);
    assert!(!started.dialog_open(cx));
    assert_eq!(started.shown_modal(cx), None);
    assert!(
        cx.read(|cx| started.workspace.read(cx).file_dialog.is_none()),
        "no stale file dialog"
    );
}

/// Core refusing the operation (the name was taken after the check): the
/// error stays in the dialog, the open note is untouched and saves again.
#[gpui_kit::test]
fn an_error_from_core_stays_in_the_dialog(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.typed("X", cx);
    started.open_op_dialog(FileOpKind::Rename, "a.md", cx);
    assert_eq!(started.rename_to(&rt, "b", cx), ValidationState::Available);
    rt.block_on(v.vault.create_note(&path("b.md"), "beta"))
        .unwrap();
    started.hit("enter", cx);
    started.finish(&rt, cx);
    assert!(started.op_error(cx).is_some(), "core's error is shown");
    assert!(started.dialog_open(cx));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
    assert_eq!(started.note_open(cx), Some(path("a.md")));
    // The flush before the operation wrote the edits to the note.
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("Xalpha"));
    assert!(!started.is_dirty(cx));
}

/// Renaming a folder holding the open note: the note follows to the same
/// place under the new name, edits kept.
#[gpui_kit::test]
fn renaming_a_folder_follows_the_open_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("d/sub/a.md", "alpha")], "d/sub/a.md");
    started.typed("X", cx);
    started.open_op_dialog(FileOpKind::Rename, "d", cx);
    assert_eq!(started.rename_to(&rt, "e", cx), ValidationState::Available);
    started.hit("enter", cx);
    started.finish(&rt, cx);
    assert_eq!(disk(&rt, &v, "e/sub/a.md").as_deref(), Some("Xalpha"));
    assert!(!exists(&rt, &v, "d"));
    assert_eq!(started.note_open(cx), Some(path("e/sub/a.md")));
    assert_eq!(started.session_at(cx), path("e/sub/a.md"));
    assert!(!started.is_dirty(cx));
    assert_eq!(started.text(cx), "Xalpha");
}

// ── delete ───────────────────────────────────────────────────────────────

/// Deleting the open note (confirmed in the alert dialog): its edits are
/// discarded, no autosave tick brings the file back, the editor Panel says
/// so and FILES shows its folder.
#[gpui_kit::test]
fn deleting_the_open_note_leaves_nothing_behind(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("d/a.md", "alpha"), ("d/b.md", "beta")], &[]));
    v.settings.write().unwrap().autosave_interval_secs = 1;
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("d/a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    started.typed("X", cx);
    started.hit("f2", cx);
    started.hit("d", cx);
    assert_eq!(started.modal(cx), Some(ModalKind::Delete));
    // The alert dialog holds the keyboard: Enter is its `Delete`.
    started.hit("enter", cx);
    started.finish(&rt, cx);

    assert!(!exists(&rt, &v, "d/a.md"));
    assert_eq!(started.note_open(cx), None);
    assert_eq!(started.placeholder(cx).as_deref(), Some("a.md was deleted"));
    assert!(!started.is_dirty(cx));
    assert!(started.files_dir(cx).is_like(&path("d")));
    assert_eq!(started.region_focused(cx), Panel::Drawer(Side::Left));
    past_a_tick(&rt, cx);
    assert!(!exists(&rt, &v, "d/a.md"), "no tick recreates it");
    // Typing goes to FILES (the filter), never to a hidden note.
    started.typed("zz", cx);
    past_a_tick(&rt, cx);
    assert!(!exists(&rt, &v, "d/a.md"));
    assert_eq!(disk(&rt, &v, "d/b.md").as_deref(), Some("beta"));
}

/// Deleting the folder holding the open note: the same as deleting the
/// note.
#[gpui_kit::test]
fn deleting_the_open_notes_folder_discards_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("d/a.md", "alpha"), ("top.md", "t")], &[]));
    v.settings.write().unwrap().autosave_interval_secs = 1;
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("d/a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    started.typed("X", cx);
    started.open_op_dialog(FileOpKind::Delete, "d", cx);
    started.hit("enter", cx);
    started.finish(&rt, cx);
    assert!(!exists(&rt, &v, "d"));
    assert_eq!(started.note_open(cx), None);
    assert_eq!(started.placeholder(cx).as_deref(), Some("a.md was deleted"));
    assert!(started.files_dir(cx).is_root_or_empty());
    past_a_tick(&rt, cx);
    assert!(!exists(&rt, &v, "d/a.md"));
    assert!(!exists(&rt, &v, "d"));
}

// ── move ─────────────────────────────────────────────────────────────────

/// Moving the open note: its edits go with it, the old place stays empty
/// through later ticks, and FILES lists where it went.
#[gpui_kit::test]
fn moving_the_open_note_flushes_it_and_lets_it_go(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha"), ("z/k.md", "k")], &[]));
    v.settings.write().unwrap().autosave_interval_secs = 1;
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    started.typed("X", cx);
    started.hit("f2", cx);
    started.hit("m", cx);
    assert_eq!(started.modal(cx), Some(ModalKind::Move));
    assert!(
        started
            .move_to(&rt, "z", cx)
            .is_some_and(|d| d.is_like(&path("z")))
    );
    started.hit("enter", cx);
    started.finish(&rt, cx);

    assert_eq!(disk(&rt, &v, "z/a.md").as_deref(), Some("Xalpha"));
    assert!(!exists(&rt, &v, "a.md"));
    assert_eq!(started.note_open(cx), None);
    assert_eq!(started.placeholder(cx).as_deref(), Some("a.md was moved"));
    assert!(
        started.files_dir(cx).is_like(&VaultPath::root()),
        "FILES stays in the folder the note left"
    );
    // The listing of the folder it left refreshes without it.
    drive(&rt, cx, |cx| {
        !started.files_rows(cx).iter().any(|r| r.ends_with("a.md"))
    });
    past_a_tick(&rt, cx);
    assert!(!exists(&rt, &v, "a.md"), "no tick recreates the old place");
}

/// Moving a folder holding the open note: the note follows.
#[gpui_kit::test]
fn moving_a_folder_follows_the_open_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("d/a.md", "alpha"), ("z/k.md", "k")], "d/a.md");
    started.typed("X", cx);
    started.open_op_dialog(FileOpKind::Move, "d", cx);
    assert!(
        started
            .move_to(&rt, "z", cx)
            .is_some_and(|d| d.is_like(&path("z")))
    );
    started.hit("enter", cx);
    started.finish(&rt, cx);
    assert_eq!(disk(&rt, &v, "z/d/a.md").as_deref(), Some("Xalpha"));
    assert!(!exists(&rt, &v, "d"));
    assert!(
        started
            .note_open(cx)
            .is_some_and(|n| n.is_like(&path("z/d/a.md")))
    );
    assert!(!started.is_dirty(cx));
    started.typed("Y", cx);
    started.hit("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.is_dirty(cx));
    assert!(disk(&rt, &v, "z/d/a.md").unwrap().contains('Y'));
    assert!(!exists(&rt, &v, "d"));
}

// ── other entries ───────────────────────────────────────────────────────

/// Renaming, moving and deleting other entries leaves the open note, its
/// unsaved edits and the file under it alone.
#[gpui_kit::test]
fn other_entries_leave_the_open_note_alone(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[
            ("a.md", "alpha"),
            ("b.md", "beta"),
            ("c.md", "gamma"),
            ("z/k.md", "k"),
        ],
        "a.md",
    );
    started.typed("X", cx);

    started.open_op_dialog(FileOpKind::Rename, "b.md", cx);
    assert_eq!(started.rename_to(&rt, "bb", cx), ValidationState::Available);
    started.hit("enter", cx);
    started.finish(&rt, cx);
    settle(&started, cx);
    assert!(exists(&rt, &v, "bb.md"));

    started.open_op_dialog(FileOpKind::Move, "c.md", cx);
    assert!(
        started
            .move_to(&rt, "z", cx)
            .is_some_and(|d| d.is_like(&path("z")))
    );
    started.hit("enter", cx);
    started.finish(&rt, cx);
    settle(&started, cx);
    assert!(exists(&rt, &v, "z/c.md"));

    started.open_op_dialog(FileOpKind::Delete, "bb.md", cx);
    started.hit("enter", cx);
    started.finish(&rt, cx);
    settle(&started, cx);
    assert!(!exists(&rt, &v, "bb.md"));

    assert_eq!(started.note_open(cx), Some(path("a.md")));
    assert!(started.is_dirty(cx), "the edits are still unsaved");
    assert_eq!(started.text(cx), "Xalpha");
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));
    assert!(started.editor_focused(cx));
}

// ── the menu and the modal slot ─────────────────────────────────────────

/// Escape closes the note menu and gives the keyboard back to the note;
/// only one modal is ever open.
#[gpui_kit::test]
fn escape_closes_and_only_one_modal_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.hit("f2", cx);
    assert_eq!(started.modal(cx), Some(ModalKind::FileOps));
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(MENU_ID).visible());
        assert!(window.find(menu_item_id(FileOpKind::Delete)).visible());
    })
    .unwrap();
    // Neither the palette nor another dialog opens over it.
    started.hit("ctrl-shift-p", cx);
    assert_eq!(started.modal(cx), Some(ModalKind::FileOps));
    started.open_op_dialog(FileOpKind::Rename, "a.md", cx);
    assert_eq!(started.modal(cx), Some(ModalKind::FileOps));
    started.hit("escape", cx);
    settle(&started, cx);
    assert!(!started.dialog_open(cx));
    assert!(started.editor_focused(cx));

    // The rename dialog: Escape closes it too, and nothing changed.
    started.open_op_dialog(FileOpKind::Rename, "a.md", cx);
    started.hit("escape", cx);
    settle(&started, cx);
    assert!(!started.dialog_open(cx));
    assert_eq!(started.modal(cx), None);
    assert!(started.editor_focused(cx));
}

/// A right-click in the editor opens the note menu, but not over a
/// selection (the editor copies it instead).
#[gpui_kit::test]
fn editor_right_click_opens_the_menu_without_a_selection(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.hit("shift-right", cx);
    started.hit("shift-right", cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.right_click("editor", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!started.dialog_open(cx), "a selection: no menu");
    started.hit("right", cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.right_click("editor", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(started.dialog_open(cx));
    assert_eq!(started.modal(cx), Some(ModalKind::FileOps));
}

/// A FILES row's context menu offers rename, move and delete for its
/// entry, a folder or a note, without the note being open.
#[gpui_kit::test]
fn a_files_row_has_a_context_menu(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("d/b.md", "beta")], "a.md");
    started.hit("ctrl-e", cx);
    drive(&rt, cx, |cx| started.files_rows(cx).len() >= 2);
    for (entry, kind) in [("d", "dir"), ("a.md", "note")] {
        let row = cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone().unwrap();
            let pane = files.read(cx);
            let core = pane.model().core().unwrap();
            core.visible_rows()
                .into_iter()
                .find(|r| r.path().is_like(&path(entry)))
                .map(|r| row_id(r).to_string())
                .unwrap_or_else(|| panic!("no row {entry}"))
        });
        cx.update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.right_click(gpui_kit::SharedString::from(row.clone()), cx);
        })
        .unwrap();
        cx.run_until_parked();
        // The menu has the keyboard: the first item is Rename….
        started.hit("down", cx);
        started.hit("enter", cx);
        settle(&started, cx);
        assert_eq!(started.modal(cx), Some(ModalKind::Rename), "{kind}");
        assert!(started.rename_target(cx).is_like(&path(entry)), "{kind}");
        started.hit("escape", cx);
        settle(&started, cx);
        assert!(!started.dialog_open(cx), "{kind}");
    }
}

/// The palette's and the leader's rename leaves open the same dialog for
/// the open note.
#[gpui_kit::test]
fn the_palette_and_the_leader_open_the_same_dialogs(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    started.hit("ctrl-shift-p", cx);
    started.typed("note rename", cx);
    started.hit("enter", cx);
    settle(&started, cx);
    assert_eq!(started.modal(cx), Some(ModalKind::Rename));
    assert_eq!(started.rename_target(cx), path("a.md"));
    started.hit("escape", cx);
    settle(&started, cx);

    started.hit("ctrl-shift-b", cx);
    started.hit("n", cx);
    started.hit("m", cx);
    settle(&started, cx);
    assert_eq!(started.modal(cx), Some(ModalKind::Move));
    started.hit("escape", cx);
    settle(&started, cx);
    assert!(started.editor_focused(cx));
}
