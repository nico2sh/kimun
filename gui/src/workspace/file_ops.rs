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
//! - **Followed.** A rename or move of the note, or of a folder holding it:
//!   the session retargets to where the note went (the same place under the
//!   new folder, `file_ops::relocated`) and reloads it clean from there (a
//!   note rename may rewrite its self-links). The editor stays; the Footer,
//!   the FILES marker and the counts follow. Every group holding the note
//!   follows it, in any tab (ADR-0058).
//! - **Gone.** A delete of the note or of a folder holding it, or a reload
//!   that fails after a rename or move: the group stays where it is, empty
//!   (ADR-0058). The tick and any
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

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AppContext as _, ClickEvent, Context, ParentElement as _, Window};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops::{self, relocated};
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::FileOp;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::session::{
    FlushRequest, FlushResult, NoteRead, ReadRequest, RenameOutcome,
};
use tokio::task::JoinError;

use super::Workspace;
use super::file_dialogs::{
    DeleteBody, FileOpKind, FileOpsMenu, MoveDialog, OpStatus, RenameDialog, confirm_delete,
};
use super::modal::{self, ModalKind, Slot};
use super::note_flush::FlushBlock;
use super::route::NO_NOTE;
use crate::startup::EmptyState;
use crate::tasks;

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
#[cfg(test)]
pub(crate) enum FileDialogView {
    Menu,
    Rename(gpui_kit::WeakEntity<RenameDialog>),
    Move(gpui_kit::WeakEntity<MoveDialog>),
    Delete,
}

/// Run `flushes` side by side; each result with its group.
async fn run_all(
    flushes: Vec<(GroupId, FlushRequest)>,
) -> Vec<(GroupId, Result<FlushResult, JoinError>)> {
    let running: Vec<_> = flushes
        .into_iter()
        .map(|(group, flush)| (group, tokio::spawn(flush.run())))
        .collect();
    let mut results = Vec::with_capacity(running.len());
    for (group, run) in running {
        results.push((group, run.await));
    }
    results
}

/// Run `reads` side by side; each result with its group.
async fn run_all_reads(
    reads: Vec<(GroupId, ReadRequest)>,
) -> Vec<(GroupId, Result<NoteRead, JoinError>)> {
    let running: Vec<_> = reads
        .into_iter()
        .map(|(group, read)| (group, tokio::spawn(read.run())))
        .collect();
    let mut results = Vec::with_capacity(running.len());
    for (group, run) in running {
        results.push((group, run.await));
    }
    results
}

/// An entry's name as the dialogs and the editor Panel say it.
fn name_of(path: &VaultPath) -> String {
    path.get_parent_path().1
}

impl Workspace {
    /// The `FileOperations` chord: the note menu for the open note, or say
    /// there is none.
    pub(crate) fn file_ops_chord(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self
            .active_state()
            .notes
            .as_ref()
            .and_then(|h| h.note.clone())
        {
            Some(note) => self.open_file_ops_menu(note, false, window, cx),
            None => self.flash(NO_NOTE, cx),
        }
    }

    /// Open the note menu for `target` in the modal slot, with
    /// `Properties…` first when `with_properties` (the editor's right-click).
    /// Refused while a modal is open, and without a vault.
    pub(crate) fn open_file_ops_menu(
        &mut self,
        target: VaultPath,
        with_properties: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) || self.active_state().notes.is_none() {
            return;
        }
        let workspace = cx.entity().downgrade();
        let title = name_of(&target);
        let menu = cx.new(|cx| FileOpsMenu::new(target, with_properties, workspace, cx));
        #[cfg(test)]
        {
            self.file_dialog = Some(FileDialogView::Menu);
        }
        self.show_modal(
            Slot::Own(ModalKind::FileOps),
            MENU_REMS,
            modal::focus_view(menu.clone()),
            move |dialog, _, _| {
                let chooser = menu.clone();
                dialog
                    .title(title.clone())
                    // Enter picks the selected item; the item's dialog
                    // replaces this one, so the kit must not close anything
                    // itself.
                    .on_ok(move |_, window, cx| {
                        chooser.update(cx, |menu, cx| menu.choose_selected(window, cx));
                        false
                    })
                    .child(menu.clone())
            },
            window,
            cx,
        );
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
        // Not a file operation: the properties dialog has its own pipeline.
        if kind == FileOpKind::Properties {
            self.open_properties(target, window, cx);
            return;
        }
        let Some(vault) = self.vault() else {
            return;
        };
        self.cancel_leader(cx);
        let status = cx.new(|_| OpStatus::default());
        self.file_status = Some(status.clone());
        let workspace = cx.entity().downgrade();
        // Escape, the Cancel button and the backdrop leave, unless an
        // operation runs (it would finish with nothing to report into).
        let running = status.clone();
        let on_cancel = move |_: &ClickEvent, _: &mut Window, cx: &mut App| !running.read(cx).busy;
        match kind {
            FileOpKind::Rename => {
                let view =
                    cx.new(|cx| RenameDialog::new(target, vault, status, workspace, window, cx));
                #[cfg(test)]
                {
                    self.file_dialog = Some(FileDialogView::Rename(view.downgrade()));
                }
                let input = view.read(cx).input.clone();
                self.show_modal(
                    Slot::Own(ModalKind::Rename),
                    DIALOG_REMS,
                    move |window, cx| input.update(cx, |input, cx| input.focus(window, cx)),
                    move |dialog, _, _| {
                        let confirm = view.clone();
                        dialog
                            .title("Rename")
                            .on_ok(move |_, window, cx| {
                                confirm.update(cx, |dialog, cx| dialog.confirm(window, cx));
                                false
                            })
                            .on_cancel(on_cancel.clone())
                            .child(view.clone())
                    },
                    window,
                    cx,
                );
            }
            FileOpKind::Move => {
                let view =
                    cx.new(|cx| MoveDialog::new(target, vault, status, workspace, window, cx));
                #[cfg(test)]
                {
                    self.file_dialog = Some(FileDialogView::Move(view.downgrade()));
                }
                let filter = view.read(cx).filter.clone();
                self.show_modal(
                    Slot::Own(ModalKind::Move),
                    DIALOG_REMS,
                    move |window, cx| filter.update(cx, |filter, cx| filter.focus(window, cx)),
                    move |dialog, _, _| {
                        let confirm = view.clone();
                        dialog
                            .title("Move")
                            .on_ok(move |_, window, cx| {
                                confirm.update(cx, |dialog, cx| dialog.confirm(window, cx));
                                false
                            })
                            .on_cancel(on_cancel.clone())
                            .child(view.clone())
                    },
                    window,
                    cx,
                );
            }
            FileOpKind::Delete => {
                let body = cx.new(|cx| DeleteBody::new(status, cx));
                self.modal = Some(ModalKind::Delete);
                #[cfg(test)]
                {
                    self.file_dialog = Some(FileDialogView::Delete);
                }
                confirm_delete(
                    window,
                    cx,
                    format!("Delete \"{}\"?", name_of(&target)),
                    "This cannot be undone.",
                    move |alert| {
                        let workspace = workspace.clone();
                        let target = target.clone();
                        alert
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
                    },
                );
            }
            // Opened above.
            FileOpKind::Properties => {}
        }
        cx.notify();
    }

    /// A dialog's Cancel or Escape: close it, unless its operation runs.
    pub(crate) fn dismiss_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_task.is_some() {
            return;
        }
        self.close_modal(window, cx);
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
        // Another operation holds the open note (a property write or the
        // reload after it, a flush, a vault change): even an operation that
        // leaves the note alone waits.
        let held = format!(
            "Another operation on the note is still running. Nothing was {}.",
            request.past()
        );
        if self.note_held() {
            self.set_file_status(false, Some(held), cx);
            return;
        }
        let Some(host) = self.active_state().notes.as_ref() else {
            return;
        };
        let vault = host.session.vault().clone();
        // Every group whose note the operation names, or a folder holding it
        // (the open notes it touches), the active one first.
        let touched: Vec<GroupId> = self
            .group_ids_active_first()
            .into_iter()
            .filter(|group| {
                self.state(*group)
                    .and_then(|s| s.notes.as_ref())
                    .is_some_and(|h| {
                        h.note.is_some() && h.session.affects_open_note(request.from())
                    })
            })
            .collect();
        let mut flushes = Vec::new();
        if let Some(refusal) = touched.iter().find_map(|group| {
            self.flush_block_in(*group, cx).map(|block| match block {
                FlushBlock::Opening => "The note is still opening. Try again in a moment.".into(),
                FlushBlock::Closing => "The window is closing.".to_string(),
                // Checked above; kept so the match is whole.
                FlushBlock::Held => held.clone(),
                FlushBlock::Unsaveable => {
                    format!(
                        "{} Nothing was {}.",
                        self.unsaveable_in(*group),
                        request.past()
                    )
                }
            })
        }) {
            self.set_file_status(false, Some(refusal), cx);
            return;
        }
        for group in &touched {
            // No tick from here on: the next write would bake in a path the
            // operation is about to rename or delete.
            match self.begin_note_flush_in(*group, cx) {
                Some(request) => flushes.push((*group, request)),
                None => {
                    // The tick is back already (`begin_note_flush_in`); the
                    // groups begun before it get theirs and their input
                    // back, their flushes dropped unrun.
                    for (begun, _) in flushes {
                        self.restart_tick_unless_read_only_in(begun);
                        self.block_input_in(begun, false, cx);
                    }
                    let text = format!(
                        "Another save is still running. Nothing was {}.",
                        request.past()
                    );
                    self.set_file_status(false, Some(text), cx);
                    return;
                }
            }
        }
        self.set_file_status(true, None, cx);
        self.file_task = Some(cx.spawn_in(window, async move |this, cx| {
            if !flushes.is_empty() {
                let flushed = run_all(flushes).await;
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
            let result = tasks::joined(tokio::spawn(async move { run.run(&vault).await })).await;
            let reads = this
                .update_in(cx, |workspace, window, cx| {
                    workspace.file_op_finished(result, touched, window, cx)
                })
                .unwrap_or_default();
            if !reads.is_empty() {
                let read = run_all_reads(reads).await;
                this.update_in(cx, |workspace, window, cx| {
                    workspace.file_op_retargeted(read, window, cx)
                })
                .ok();
            }
        }));
        cx.notify();
    }

    /// The flushes before an operation on open notes ended: go on when
    /// everything is written, else refuse the operation (each note keeps
    /// its edits, its tick and the keyboard).
    fn file_op_flushed(
        &mut self,
        flushed: Vec<(GroupId, Result<FlushResult, JoinError>)>,
        past: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let touched: Vec<GroupId> = flushed.iter().map(|(group, _)| *group).collect();
        let mut failure = None;
        for (group, result) in flushed {
            let outcome = self.apply_flush(group, result, cx);
            if failure.is_none()
                && let Some(problem) = self.flush_failure_in(group, &outcome, cx)
            {
                failure = Some((group, problem));
            }
        }
        let Some((group, problem)) = failure else {
            return true;
        };
        let note = self
            .state(group)
            .and_then(|s| s.notes.as_ref())
            .and_then(|h| h.note.clone())
            .unwrap_or_else(VaultPath::root);
        self.file_op_failed(
            format!("Couldn't save {note}: {problem}. Nothing was {past}."),
            &touched,
            window,
            cx,
        );
        false
    }

    /// The operation was refused or failed: nothing changed on disk. The
    /// open notes it `touched` get their tick and their keyboard back, the
    /// dialog says why (a flash when it is gone).
    fn file_op_failed(
        &mut self,
        error: String,
        touched: &[GroupId],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for group in touched {
            self.restart_tick_unless_read_only_in(*group);
            self.block_input_in(*group, false, cx);
        }
        self.file_task = None;
        if modal::dialog_open(window, cx) {
            self.set_file_status(false, Some(error), cx);
        } else {
            self.flash_error(error, cx);
        }
        self.file_op_ended(cx);
    }

    /// Core ran the operation. On success the dialog closes, the history
    /// and FILES follow, and every open note is followed, discarded or left
    /// alone (module docs). Returns the reads of the notes followed to new
    /// paths: the operation is not over until they land.
    fn file_op_finished(
        &mut self,
        result: Result<FileOp, String>,
        touched: Vec<GroupId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(GroupId, ReadRequest)> {
        let op = match result {
            Ok(op) => op,
            Err(error) => {
                self.file_op_failed(error, &touched, window, cx);
                return Vec::new();
            }
        };
        self.history_follows(&op);
        if modal::dialog_open(window, cx) {
            window.close_all_dialogs(cx);
        }
        self.reset_modal_state(cx);
        self.file_status = None;
        self.files_file_op(&op, cx);
        self.attachment_file_op(&op, cx);
        let active = self.layout.active_group();
        if !touched.contains(&active) {
            self.refresh_note_counts();
        }
        if touched.is_empty() {
            self.file_task = None;
            self.file_op_ended(cx);
            return Vec::new();
        }
        let open = self
            .active_state()
            .notes
            .as_ref()
            .and_then(|h| h.note.clone())
            .filter(|_| touched.contains(&active));
        let (reads, active_gone) = match &op {
            // The note renamed or moved, or a folder holding it: it follows,
            // in every group holding it.
            FileOp::Renamed { from, to } | FileOp::Moved { from, to } => {
                (self.note_moved(from, to), false)
            }
            FileOp::Deleted(path) => (Vec::new(), self.note_deleted(path, cx)),
            _ => (Vec::new(), false),
        };
        if let Some(open) = open.filter(|_| active_gone) {
            let (title, detail, dir) = match &op {
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
            self.say_note_gone(title, detail, dir, window, cx);
        }
        // A touched note neither followed nor gone (the operation did not
        // reach it after all) gets its tick and its keyboard back.
        for group in &touched {
            let following = reads.iter().any(|(g, _)| g == group);
            if !following
                && self
                    .state(*group)
                    .and_then(|s| s.notes.as_ref())
                    .is_some_and(|h| h.note.is_some())
            {
                self.restart_tick_unless_read_only_in(*group);
                self.block_input_in(*group, false, cx);
            }
        }
        if reads.is_empty() {
            self.file_task = None;
            self.file_op_ended(cx);
        }
        reads
    }

    /// The notes at or below `from` were renamed or moved to `to`: every
    /// group holding one follows it to where it went ([`relocated`]) once
    /// its read lands ([`Self::file_op_retargeted`]). Returns those reads;
    /// no group changes until then. A background save still aimed at the
    /// old path is aborted.
    pub(crate) fn note_moved(
        &mut self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Vec<(GroupId, ReadRequest)> {
        let mut reads = Vec::new();
        for group in self.group_ids() {
            let Some(host) = self.state_mut(group).and_then(|s| s.notes.as_mut()) else {
                continue;
            };
            let Some(new) = host
                .note
                .as_ref()
                .and_then(|note| relocated(note, from, to))
            else {
                continue;
            };
            let at = host.session.path().clone();
            if let Some(read) = host.session.begin_rename(&at, &new) {
                reads.push((group, read));
            }
        }
        reads
    }

    /// The note at `path`, or every note below it, was deleted: each group
    /// holding one becomes an empty group in its place (no save: the file
    /// is gone). Whether the active group was one of them; the caller
    /// says so in the editor Panel.
    pub(crate) fn note_deleted(&mut self, path: &VaultPath, cx: &mut Context<Self>) -> bool {
        self.empty_groups_where(|note| relocated(note, path, path).is_some(), cx)
    }

    /// Empty every group whose note matches `gone` ([`Self::empty_group`]);
    /// whether the active group was one of them.
    fn empty_groups_where(
        &mut self,
        gone: impl Fn(&VaultPath) -> bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let active = self.layout.active_group();
        let mut active_gone = false;
        for group in self.group_ids() {
            let holds = self
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.note.as_ref())
                .is_some_and(&gone);
            if holds {
                self.empty_group(group, cx);
                active_gone |= group == active;
            }
        }
        active_gone
    }

    /// The followed notes were read at their new paths: each loads clean
    /// there, or is given up when it would not read.
    fn file_op_retargeted(
        &mut self,
        reads: Vec<(GroupId, Result<NoteRead, JoinError>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (group, read) in reads {
            self.retarget(group, read, window, cx);
        }
        self.file_task = None;
        self.file_op_ended(cx);
        cx.notify();
    }

    /// [`Self::file_op_retargeted`] for one group.
    fn retarget(
        &mut self,
        group: GroupId,
        read: Result<NoteRead, JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let read_only = self.status.read_only;
        let active = group == self.layout.active_group();
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        let (Some(tx), Some(host)) = (self.tx.clone(), state.notes.as_mut()) else {
            return;
        };
        let old = host.note.clone().unwrap_or_else(VaultPath::root);
        let outcome = match read {
            Ok(read) => state.editor.update(cx, |view, cx| {
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
                self.block_input_in(group, false, cx);
                if active {
                    self.status.note = Some(path.clone());
                    self.files_note_opened(&path, cx);
                    self.refresh_views(cx);
                }
            }
            RenameOutcome::ReloadFailed { parent } => {
                self.empty_group(group, cx);
                if active {
                    self.say_note_gone(
                        format!("{} couldn't be reopened", name_of(&old)),
                        "It was renamed, but would not read from its new place.".to_string(),
                        parent,
                        window,
                        cx,
                    );
                }
            }
            // `begin_rename` matched the open note.
            RenameOutcome::NotOpen => {}
        }
    }

    /// The active group's note went away under an operation (its group is
    /// already empty, [`Self::empty_group`]): say so in the editor Panel,
    /// and give FILES the keyboard on `dir`.
    fn say_note_gone(
        &mut self,
        title: String,
        detail: String,
        dir: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            self.open_view_focused(DrawerView::Files);
            self.place_rail_cursor(Some(DrawerView::Files));
            self.apply_focus(window, cx);
            self.save_layout(cx);
        }
        // LINKS and OUTLINE say "No note open".
        self.refresh_views(cx);
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

    /// An operation ended (a file operation, a property write, the reload
    /// after it): an open that waited for it goes now, in whichever group
    /// asked for it, unless another one still holds the notes.
    pub(super) fn file_op_ended(&mut self, cx: &mut Context<Self>) {
        if self.note_held() {
            cx.notify();
            return;
        }
        // Every group's: an open parked in a group that is not active
        // would otherwise hold its note for good.
        for group in self.group_ids() {
            if self
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .is_some_and(|h| h.is_open_wanted() && !h.is_switching())
            {
                self.switch(group, cx);
            }
        }
        cx.notify();
    }
}
