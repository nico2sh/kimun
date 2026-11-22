//! One switch at a time in a group. A switch flushes the open note, then
//! reads the wanted note and loads it.
//! It does not choose the group. [`mod@super::open`] does that.

use gpui_kit::Context;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::session::{FlushResult, NoteRead, OpenOutcome, SaveOutcome};
use tokio::task::JoinError;

use super::{Closing, flush_problem, rewritable};
use crate::workspace::Workspace;

impl Workspace {
    /// Start the switch to the wanted note: flush the open one first.
    pub(in crate::workspace) fn switch(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let can_save = self.can_save_in(group);
        let dirty = self.editor_dirty_in(group, cx);
        let Some((editor, host)) = self.editor_and_host_in(group) else {
            return;
        };
        if host.wanted.is_none() {
            return;
        }
        let flush = if can_save {
            match host.session.prepare_flush(Some(&editor.read(cx).editor)) {
                Ok(flush) => Some(flush),
                // One switch or close at a time holds the only flush.
                Err(_) => {
                    tracing::warn!("note switch: a flush is still outstanding");
                    return;
                }
            }
        } else {
            None
        };
        host.switching = Some(cx.spawn(async move |this, cx| {
            let flushed = match flush {
                Some(flush) => Some(tokio::spawn(flush.run()).await),
                None => None,
            };
            this.update(cx, |workspace, cx| {
                workspace.switch_flushed(group, flushed, cx)
            })
            .ok();
        }));
        if !can_save && dirty {
            let text = format!("{} They were not kept.", self.unsaveable_in(group));
            self.flash_error(text, cx);
        }
    }

    /// Apply a finished flush to the session and the buffer.
    pub(in crate::workspace) fn apply_flush(
        &mut self,
        group: GroupId,
        result: Result<FlushResult, JoinError>,
        cx: &mut Context<Self>,
    ) -> SaveOutcome {
        let Some((editor, host)) = self.editor_and_host_in(group) else {
            return SaveOutcome::Clean;
        };
        let outcome = match result {
            Ok(result) => editor.update(cx, |view, cx| {
                let outcome = host.session.apply_flush(result, Some(&mut view.editor));
                cx.notify();
                outcome
            }),
            Err(e) => SaveOutcome::Failed(e.to_string()),
        };
        if matches!(outcome, SaveOutcome::Saved(_)) && matches!(self.closing, Closing::Discard(_)) {
            self.closing = Closing::Open;
        }
        if let SaveOutcome::Saved(saved) = &outcome {
            self.files_note_saved(saved, cx);
            self.refresh_facets(&saved.path, cx);
            if self.host_in(group).and_then(|h| h.note.as_ref()) == Some(&saved.path) {
                self.refresh_note_properties_in(group);
            }
        }
        outcome
    }

    /// The switch's flush ended: read the wanted note, unless the flush
    /// failed (stay, edits kept) or the window is closing.
    pub(super) fn switch_flushed(
        &mut self,
        group: GroupId,
        flushed: Option<Result<FlushResult, JoinError>>,
        cx: &mut Context<Self>,
    ) {
        let outcome = flushed.map(|result| self.apply_flush(group, result, cx));
        let dirty = self.editor_dirty_in(group, cx);
        let can_save = self.can_save_in(group);
        let Some(host) = self.groups.get_mut(&group).and_then(|s| s.notes.as_mut()) else {
            return;
        };
        host.switching = None;
        if let Some(problem) = outcome.as_ref().and_then(flush_problem) {
            host.drop_wanted();
            let note = host.note.clone().unwrap_or_else(VaultPath::root);
            self.flash_error(
                format!("Couldn't save {note}: {problem}. It stays open."),
                cx,
            );
            self.continue_close(cx);
            return;
        }
        if self.closing == Closing::Requested {
            host.drop_wanted();
            self.close_flush(cx);
            return;
        }
        // Typed while the flush ran: write that too before leaving. Only
        // when that flush could capture the buffer: never on a pending open
        // (it captures nothing, and the loop would never end).
        if can_save && dirty && rewritable(outcome.as_ref()) && !host.session.is_open_pending() {
            self.switch(group, cx);
            return;
        }
        let Some(path) = host.wanted.take() else {
            return;
        };
        host.reading_emphasis = host.wanted_emphasis.take();
        host.reading_line = host.wanted_line.take();
        let read = host.session.begin_open(path);
        host.switching = Some(cx.spawn(async move |this, cx| {
            let read = tokio::spawn(read.run()).await;
            this.update(cx, |workspace, cx| workspace.switch_read(group, read, cx))
                .ok();
        }));
        self.block_input_in(group, true, cx);
    }

    /// The wanted note was read: load it (history, autosave tick), or stay
    /// on the open note and say why not.
    pub(crate) fn switch_read(
        &mut self,
        group: GroupId,
        read: Result<NoteRead, JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(tx) = self.tx.clone() else {
            return;
        };
        let read_only = self.status.read_only;
        let Some((_, host)) = self.editor_and_host_in(group) else {
            return;
        };
        host.switching = None;
        let emphasis = host.reading_emphasis.take();
        let line = host.reading_line.take();
        let read = match read {
            Ok(read) => read,
            Err(e) => {
                // The read never came back: drop the pending open, so the
                // note the buffer holds saves again.
                tracing::error!("note read failed to finish: {e}");
                host.session.cancel_open();
                self.block_input_in(group, false, cx);
                self.flash_error(format!("Couldn't open the note: {e}"), cx);
                self.after_switch(group, cx);
                return;
            }
        };
        self.block_input_in(group, false, cx);
        let Some((editor, host)) = self.editor_and_host_in(group) else {
            return;
        };
        let previous = host.note.clone();
        let outcome = editor.update(cx, |view, cx| {
            let outcome = host.session.finish_open(read, Some(&mut view.editor), &tx);
            if matches!(outcome, OpenOutcome::Opened(_)) {
                // A composition in progress belonged to the old text.
                view.preedit = None;
                // After the load, so the new text's revision owns them.
                if let Some(needles) = emphasis {
                    view.editor.set_search_needles(needles);
                }
                // The buffer holds the note as written, frontmatter too:
                // a source line is a buffer row.
                if let Some(line) = line {
                    view.editor.jump_to_row(line);
                }
            }
            cx.notify();
            outcome
        });
        match outcome {
            OpenOutcome::Opened(path) => {
                if read_only {
                    host.session.stop_autosave();
                }
                host.note = Some(path.clone());
                host.meta.load_note_counts(&path, &tx);
                self.forget_note_gone(group);
                // A menu on the group names the note it showed before.
                if previous.as_ref() != Some(&path) {
                    self.forget_note_menu_of(group, cx);
                    self.forget_link_menu_of(group, cx);
                }
                if group == self.layout.active_group() {
                    self.status.note = Some(path.clone());
                    self.files_note_opened(&path, cx);
                    // The note takes the editor Panel back from an attachment.
                    self.clear_attachment(cx);
                } else {
                    // Not on display: its attachment goes quietly.
                    self.drop_attachment_in(group);
                }
                // A facet view gives the group back to the editor.
                self.clear_facet_in(group, cx);
            }
            // The editor still holds the note it had: point the session back
            // at it, so nothing is saved under the missing path.
            // The create dialog offers to make it (the TUI's rule).
            OpenOutcome::NotFound(path) => {
                host.session
                    .set_path(previous.unwrap_or_else(VaultPath::root));
                self.offer_create(path, cx);
            }
            OpenOutcome::ReadFailed { path, .. } => {
                host.session
                    .set_path(previous.unwrap_or_else(VaultPath::root));
                self.flash_error(format!("Couldn't open {path} (see the log)"), cx);
            }
            OpenOutcome::Superseded => {}
        }
        self.after_switch(group, cx);
    }

    /// A switch ended: hand over to a waiting close, or to the next open.
    pub(super) fn after_switch(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self.closing == Closing::Requested {
            self.close_flush(cx);
        } else if self.host_in(group).is_some_and(|h| h.wanted.is_some()) {
            self.switch(group, cx);
        }
        // The first open settled (or failed): FILES may pick its folder;
        // LINKS and OUTLINE follow the note while shown.
        self.ensure_files_listing(cx);
        self.refresh_views(cx);
        cx.notify();
    }
}
