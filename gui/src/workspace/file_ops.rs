//! **File operations** in the window (M6 G3): rename, move and delete a
//! note, folder or attachment, from the note menu (the `FileOperations`
//! chord, the editor's right-click without a selection), a FILES row's
//! context menu, or the leader and the palette. The dialogs are
//! [`super::file_dialogs`]; every change on disk is core's, through
//! `logic::file_ops`. This file owns what happens around the open note.
//!
//! **The open note is never lost** (the session's hazards, adopted as
//! rules). An operation *touches* the open note when it names it or a
//! folder holding it (`NoteSession::affects_open_note`). Before such an
//! operation the autosave tick stops, the editor takes no input, and the
//! buffer is flushed; a flush that fails (or finds another one outstanding,
//! or edits that cannot be saved) refuses the operation: nothing changes on
//! disk, the dialog stays open saying why, and the tick and the input come
//! back. Only then does core run the operation. After it:
//!
//! - **Followed.** A rename of the note, or a rename or move of a folder
//!   holding it: the session retargets to where the note went (the same
//!   place under the new folder, `file_ops::relocated`) and reloads it clean
//!   from there (a note rename may rewrite its self-links). The editor stays;
//!   the Footer, the FILES marker and the counts follow.
//! - **Gone.** A delete of the note or of a folder holding it, a move of the
//!   note itself, or a reload that fails after a rename: the tick and any
//!   background save are stopped, the buffer is discarded *without* saving
//!   (the file is gone or elsewhere; a save would recreate it), the editor
//!   Panel says what happened, and FILES shows where it was (or went).
//! - **Untouched.** Anything else leaves the open note, its edits and its
//!   tick alone; FILES refreshes.
//!
//! **One at a time.** While an operation runs its dialog's confirm is
//! disabled and another request is ignored; saves, the tick and opening
//! another note wait for it. A failure stays in the dialog; success closes
//! it. The history follows renames, moves and deletes, as the TUI's does.

use std::cell::Cell;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Focusable as _, ParentElement as _, Window,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops::{self, relocated};
use kimun_notes::logic::layout::Region;
use kimun_notes::logic::message::FileOp;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::session::{FlushResult, NoteRead, ReadRequest, RenameOutcome};
use tokio::task::JoinError;

use super::Workspace;
use super::file_dialogs::{
    DeleteBody, FileOpKind, FileOpsMenu, MoveDialog, OpStatus, RenameDialog,
};
use super::modal::{self, ModalKind};
use super::note::{Closing, flush_problem};
use super::route::NO_NOTE;
use crate::shell_layout::RAIL_VIEWS;
use crate::startup::EmptyState;

/// The note menu's width, in rems.
const MENU_REMS: f32 = 18.;
/// The rename and move dialogs' width, in rems.
const DIALOG_REMS: f32 = 30.;

/// A file operation asked for by a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileOpRequest {
    Delete(VaultPath),
    Rename { from: VaultPath, to: VaultPath },
    Move { from: VaultPath, to: VaultPath },
}

impl FileOpRequest {
    /// The entry the operation acts on.
    fn from(&self) -> &VaultPath {
        match self {
            FileOpRequest::Delete(path) => path,
            FileOpRequest::Rename { from, .. } | FileOpRequest::Move { from, .. } => from,
        }
    }

    /// "Nothing was …", for a refusal.
    fn past(&self) -> &'static str {
        match self {
            FileOpRequest::Delete(_) => "deleted",
            FileOpRequest::Rename { .. } => "renamed",
            FileOpRequest::Move { .. } => "moved",
        }
    }

    /// Run it through core.
    async fn run(self, vault: &kimun_core::NoteVault) -> Result<FileOp, String> {
        match self {
            FileOpRequest::Delete(path) => file_ops::delete(vault, &path).await,
            FileOpRequest::Rename { from, to } => file_ops::rename(vault, &from, &to).await,
            FileOpRequest::Move { from, to } => file_ops::move_entry(vault, &from, &to).await,
        }
    }
}

/// The open file dialog's view (tests read it through these).
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum FileDialogView {
    Menu,
    Rename(gpui_kit::WeakEntity<RenameDialog>),
    Move(gpui_kit::WeakEntity<MoveDialog>),
    Delete,
}

/// An entry's name as the dialogs and the editor Panel say it.
fn name_of(path: &VaultPath) -> String {
    path.get_parent_path().1
}

impl Workspace {
    /// The `FileOperations` chord: the note menu for the open note, or say
    /// there is none.
    pub(crate) fn file_ops_chord(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.notes.as_ref().and_then(|h| h.note.clone()) {
            Some(note) => self.open_file_ops_menu(note, window, cx),
            None => self.flash(NO_NOTE, cx),
        }
    }

    /// Open the note menu for `target` in the modal slot. Refused while a
    /// modal is open, and without a vault.
    pub(crate) fn open_file_ops_menu(
        &mut self,
        target: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) || self.notes.is_none() {
            return;
        }
        self.cancel_leader(cx);
        let workspace = cx.entity().downgrade();
        let theme = self.theme.clone();
        let title = name_of(&target);
        let menu = cx.new(|cx| FileOpsMenu::new(target, workspace, theme, cx));
        let width = window.rem_size() * MENU_REMS;
        let first = Cell::new(true);
        self.modal = Some(ModalKind::FileOps);
        self.file_dialog = Some(FileDialogView::Menu);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                let handle = menu.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            let chooser = menu.clone();
            dialog
                .title(title.clone())
                .w(width)
                .close_button(false)
                // Enter picks the selected item; the item's dialog replaces
                // this one, so the kit must not close anything itself.
                .on_ok(move |_, window, cx| {
                    chooser.update(cx, |menu, cx| menu.choose_selected(window, cx));
                    false
                })
                .child(menu.clone())
        });
        cx.notify();
    }

    /// An item of the note menu was picked: the menu gives way to its
    /// dialog.
    pub(crate) fn choose_file_op(
        &mut self,
        kind: FileOpKind,
        target: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_modal(window, cx);
        self.file_dialog = None;
        self.open_file_dialog(kind, target, window, cx);
    }

    /// Open the rename, move or delete dialog for `target`. Refused while a
    /// modal is open, and without a vault.
    pub(crate) fn open_file_dialog(
        &mut self,
        kind: FileOpKind,
        target: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(vault) = self.notes.as_ref().map(|h| h.session.vault().clone()) else {
            return;
        };
        self.cancel_leader(cx);
        let status = cx.new(|_| OpStatus::default());
        self.file_status = Some(status.clone());
        let workspace = cx.entity().downgrade();
        let theme = self.theme.clone();
        let width = window.rem_size() * DIALOG_REMS;
        // Escape, the Cancel button and the backdrop leave, unless an
        // operation runs (it would finish with nothing to report into).
        let running = status.clone();
        let on_cancel = move |_: &ClickEvent, _: &mut Window, cx: &mut App| !running.read(cx).busy;
        let first = Cell::new(true);
        match kind {
            FileOpKind::Rename => {
                let view = cx.new(|cx| {
                    RenameDialog::new(target, vault, status, workspace, theme, window, cx)
                });
                self.modal = Some(ModalKind::Rename);
                self.file_dialog = Some(FileDialogView::Rename(view.downgrade()));
                window.open_dialog(cx, move |dialog, window, cx| {
                    if first.replace(false) {
                        let input = view.read(cx).input.clone();
                        input.update(cx, |input, cx| input.focus(window, cx));
                    }
                    let confirm = view.clone();
                    dialog
                        .title("Rename")
                        .w(width)
                        .close_button(false)
                        .on_ok(move |_, window, cx| {
                            confirm.update(cx, |dialog, cx| dialog.confirm(window, cx));
                            false
                        })
                        .on_cancel(on_cancel.clone())
                        .child(view.clone())
                });
            }
            FileOpKind::Move => {
                let view = cx
                    .new(|cx| MoveDialog::new(target, vault, status, workspace, theme, window, cx));
                self.modal = Some(ModalKind::Move);
                self.file_dialog = Some(FileDialogView::Move(view.downgrade()));
                window.open_dialog(cx, move |dialog, window, cx| {
                    if first.replace(false) {
                        let filter = view.read(cx).filter.clone();
                        filter.update(cx, |filter, cx| filter.focus(window, cx));
                    }
                    let confirm = view.clone();
                    dialog
                        .title("Move")
                        .w(width)
                        .close_button(false)
                        .on_ok(move |_, window, cx| {
                            confirm.update(cx, |dialog, cx| dialog.confirm(window, cx));
                            false
                        })
                        .on_cancel(on_cancel.clone())
                        .child(view.clone())
                });
            }
            FileOpKind::Delete => {
                let body = cx.new(|cx| DeleteBody::new(status, theme, cx));
                let title = format!("Delete \"{}\"?", name_of(&target));
                self.modal = Some(ModalKind::Delete);
                self.file_dialog = Some(FileDialogView::Delete);
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let workspace = workspace.clone();
                    let target = target.clone();
                    alert
                        .confirm()
                        .title(title.clone())
                        .description("This cannot be undone.")
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("Cancel")
                        .on_ok(move |_, window, cx| {
                            let request = FileOpRequest::Delete(target.clone());
                            workspace
                                .update(cx, |workspace, cx| {
                                    workspace.request_file_op(request, window, cx)
                                })
                                .ok();
                            false
                        })
                        .on_cancel(on_cancel.clone())
                        .child(body.clone())
                });
            }
        }
        cx.notify();
    }

    /// A dialog's Cancel or Escape: close it, unless its operation runs.
    pub(crate) fn dismiss_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_task.is_some() {
            return;
        }
        self.close_modal(window, cx);
        self.file_dialog = None;
        self.file_status = None;
    }

    fn set_file_status(&self, busy: bool, error: Option<String>, cx: &mut Context<Self>) {
        if let Some(status) = self.file_status.as_ref() {
            status.update(cx, |status, cx| {
                status.busy = busy;
                status.error = error.map(Into::into);
                cx.notify();
            });
        }
    }

    // ---- Running one ---------------------------------------------------------

    /// Run `request` (a dialog confirmed it), keeping the open note safe
    /// (see the module docs). Ignored while another operation runs.
    pub(crate) fn request_file_op(
        &mut self,
        request: FileOpRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_task.is_some() {
            return;
        }
        let dirty = self.editor_dirty(cx);
        let can_save = self.can_save();
        let unsaveable = self.unsaveable();
        let closing = self.closing != Closing::Open;
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        let vault = host.session.vault().clone();
        let touches = host.note.is_some() && host.session.affects_open_note(request.from());
        let mut flush = None;
        if touches {
            let refusal = if host.is_switching() || host.is_open_wanted() {
                Some("The note is still opening. Try again in a moment.".to_string())
            } else if closing {
                Some("The window is closing.".to_string())
            } else if dirty && !can_save {
                Some(format!("{unsaveable} Nothing was {}.", request.past()))
            } else {
                None
            };
            if let Some(refusal) = refusal {
                self.set_file_status(false, Some(refusal), cx);
                return;
            }
            // No tick from here on: the next write would bake in a path the
            // operation is about to rename or delete.
            host.session.stop_autosave();
            match host
                .session
                .prepare_flush(Some(&self.editor.read(cx).editor))
            {
                Ok(request) => flush = Some(request),
                Err(_) => {
                    self.resume_note();
                    let text = format!(
                        "Another save is still running. Nothing was {}.",
                        request.past()
                    );
                    self.set_file_status(false, Some(text), cx);
                    return;
                }
            }
            self.block_input(true, cx);
        }
        self.set_file_status(true, None, cx);
        self.file_task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(flush) = flush {
                let flushed = tokio::spawn(flush.run()).await;
                let past = request.past();
                let go = this
                    .update_in(cx, |workspace, window, cx| {
                        workspace.file_op_flushed(flushed, past, window, cx)
                    })
                    .unwrap_or(false);
                if !go {
                    return;
                }
            }
            let run = request.clone();
            let result = tokio::spawn(async move { run.run(&vault).await })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            let read = this
                .update_in(cx, |workspace, window, cx| {
                    workspace.file_op_finished(result, touches, window, cx)
                })
                .ok()
                .flatten();
            if let Some(read) = read {
                let read = tokio::spawn(read.run()).await;
                this.update_in(cx, |workspace, window, cx| {
                    workspace.file_op_retargeted(read, window, cx)
                })
                .ok();
            }
        }));
        cx.notify();
    }

    /// The flush before an operation on the open note ended: go on when
    /// everything is written, else refuse the operation (the note keeps its
    /// edits, its tick and the keyboard).
    fn file_op_flushed(
        &mut self,
        flushed: Result<FlushResult, JoinError>,
        past: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let outcome = self.apply_flush(flushed, cx);
        let problem = flush_problem(&outcome).or_else(|| {
            // Nothing can type while the flush runs; should the buffer still
            // hold unwritten text, it is not run over either.
            self.editor_dirty(cx)
                .then(|| "the editor still has unsaved edits".to_string())
        });
        let Some(problem) = problem else {
            return true;
        };
        let note = self
            .notes
            .as_ref()
            .and_then(|h| h.note.clone())
            .unwrap_or_else(VaultPath::root);
        self.file_op_failed(
            format!("Couldn't save {note}: {problem}. Nothing was {past}."),
            true,
            window,
            cx,
        );
        false
    }

    /// The operation was refused or failed: nothing changed on disk. The
    /// open note gets its tick and its keyboard back (`touched`), the
    /// dialog says why (a flash when it is gone).
    fn file_op_failed(
        &mut self,
        error: String,
        touched: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if touched {
            self.resume_note();
            self.block_input(false, cx);
        }
        self.file_task = None;
        if modal::dialog_open(window, cx) {
            self.set_file_status(false, Some(error), cx);
        } else {
            self.flash_error(error, cx);
        }
        self.file_op_ended(cx);
    }

    /// The tick runs again over the open note (not in a read-only vault,
    /// where it never runs).
    fn resume_note(&mut self) {
        if !self.status.read_only {
            self.restart_tick();
        }
    }

    /// Core ran the operation. On success the dialog closes, the history
    /// and FILES follow, and the open note is followed, discarded or left
    /// alone (module docs). Returns the read of the note's new path when it
    /// is followed: the operation is not over until it lands.
    fn file_op_finished(
        &mut self,
        result: Result<FileOp, String>,
        touched: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ReadRequest> {
        let op = match result {
            Ok(op) => op,
            Err(error) => {
                self.file_op_failed(error, touched, window, cx);
                return None;
            }
        };
        self.history_follows(&op);
        if modal::dialog_open(window, cx) {
            window.close_all_dialogs(cx);
        }
        self.modal = None;
        self.file_dialog = None;
        self.file_status = None;
        self.files_file_op(&op, cx);
        let open = self.notes.as_ref().and_then(|h| h.note.clone());
        let Some(open) = open.filter(|_| touched) else {
            self.refresh_note_counts();
            self.file_task = None;
            self.file_op_ended(cx);
            return None;
        };
        let followed = match &op {
            // The note renamed, or a folder holding it renamed or moved.
            FileOp::Renamed { from, to } => relocated(&open, from, to),
            FileOp::Moved { from, to } if !from.is_like(&open) => relocated(&open, from, to),
            _ => None,
        };
        if let Some(new) = followed
            && let Some(host) = self.notes.as_mut()
        {
            let at = host.session.path().clone();
            if let Some(read) = host.session.begin_rename(&at, &new) {
                return Some(read);
            }
        }
        let (title, detail, dir) = match &op {
            // FILES stays in the folder the note left, as the TUI does.
            FileOp::Moved { from, to } => (
                format!("{} was moved", name_of(&open)),
                format!("It is now {to}."),
                from.get_parent_path().0,
            ),
            FileOp::Deleted(path) => (
                format!("{} was deleted", name_of(&open)),
                "FILES shows the folder it was in.".to_string(),
                path.get_parent_path().0,
            ),
            _ => (
                format!("{} is no longer there", name_of(&open)),
                String::new(),
                open.get_parent_path().0,
            ),
        };
        self.note_gone(title, detail, dir, window, cx);
        self.file_task = None;
        self.file_op_ended(cx);
        None
    }

    /// The followed note was read at its new path: load it clean there, or
    /// give it up when it would not read.
    fn file_op_retargeted(
        &mut self,
        read: Result<NoteRead, JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let read_only = self.status.read_only;
        let (Some(tx), Some(host)) = (self.tx.clone(), self.notes.as_mut()) else {
            return;
        };
        let old = host.note.clone().unwrap_or_else(VaultPath::root);
        let outcome = match read {
            Ok(read) => self.editor.update(cx, |view, cx| {
                let outcome = host
                    .session
                    .finish_rename(read, Some(&mut view.editor), &tx);
                view.preedit = None;
                cx.notify();
                outcome
            }),
            Err(e) => {
                tracing::error!("reading the renamed note failed to finish: {e}");
                RenameOutcome::ReloadFailed {
                    parent: old.get_parent_path().0,
                }
            }
        };
        match outcome {
            RenameOutcome::Retargeted(path) => {
                if read_only {
                    host.session.stop_autosave();
                }
                host.note = Some(path.clone());
                host.meta.load_note_counts(&path, &tx);
                self.status.note = Some(path.clone());
                self.block_input(false, cx);
                self.files_note_opened(&path, cx);
            }
            RenameOutcome::ReloadFailed { parent } => {
                self.note_gone(
                    format!("{} couldn't be reopened", name_of(&old)),
                    "It was renamed, but would not read from its new place.".to_string(),
                    parent,
                    window,
                    cx,
                );
            }
            // `begin_rename` matched the open note.
            RenameOutcome::NotOpen => {}
        }
        self.file_task = None;
        self.file_op_ended(cx);
        cx.notify();
    }

    /// The open note went away under an operation: stop every writer,
    /// discard the buffer without saving it (the file is gone or elsewhere),
    /// say so in the editor Panel, and give FILES the keyboard on `dir`.
    fn note_gone(
        &mut self,
        title: String,
        detail: String,
        dir: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(host) = self.notes.as_mut() {
            host.session.stop_autosave();
            host.session.abort_autosave();
            host.session.set_path(VaultPath::root());
            host.note = None;
        }
        self.editor.update(cx, |view, cx| {
            view.editor.set_text(String::new());
            view.editor.mark_saved(String::new());
            view.preedit = None;
            view.input_blocked = false;
            cx.notify();
        });
        self.status.note = None;
        self.note_gone = Some(EmptyState {
            title,
            detail,
            failed: false,
        });
        if let Some(files) = self.files.clone() {
            files.update(cx, |pane, cx| {
                pane.set_open_note(None, cx);
                pane.enter_dir(dir, window, cx);
            });
            let (side, index) = self.layout.open_view(DrawerView::Files);
            if let Some(i) = RAIL_VIEWS.iter().position(|v| *v == DrawerView::Files) {
                self.rail_cursor = i;
            }
            self.layout.focus_region(Region::Slot { side, index });
            self.apply_focus(window, cx);
            self.store.save(&self.layout);
        }
        cx.notify();
    }

    /// The history keeps pointing at the notes (the TUI app loop's rule).
    fn history_follows(&self, op: &FileOp) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let settings = settings.read().unwrap_or_else(|e| e.into_inner());
        match op {
            FileOp::Renamed { from, to } | FileOp::Moved { from, to } => {
                settings.follow_rename_in_history(from, to);
            }
            FileOp::Deleted(path) => settings.follow_delete_in_history(path),
            _ => {}
        }
    }

    /// An operation ended: an open that waited for it goes now.
    fn file_op_ended(&mut self, cx: &mut Context<Self>) {
        if let Some(host) = self.notes.as_ref()
            && host.is_open_wanted()
            && !host.is_switching()
        {
            self.switch(cx);
        }
        cx.notify();
    }
}
