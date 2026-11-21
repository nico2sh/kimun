//! Background writes of a note: the autosave tick, the Save chord and the
//! report that a write finished.
//! It does not own the close flush ([`Workspace::close_flush`]).

use gpui_kit::Context;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::TickId;
use kimun_notes::logic::session::SaveCompleted;

use super::Closing;
use crate::workspace::Workspace;

impl Workspace {
    /// A bare autosave: write the note of every group in the background if
    /// dirty (a group with nothing new to write declines). The GUI's own
    /// ticks are per group ([`Self::autosave_tick`]).
    pub(crate) fn autosave(&mut self, cx: &mut Context<Self>) {
        for group in self.group_ids() {
            self.autosave_in(group, cx);
        }
    }

    /// The periodic tick of one note session: write the note of the group
    /// that session belongs to, if dirty. Every group's session runs its own
    /// tick, so each group is looked at once per interval. A tick whose
    /// session is gone (its group closed meanwhile) writes nothing.
    pub(crate) fn autosave_tick(&mut self, tick: TickId, cx: &mut Context<Self>) {
        if let Some(group) = self.group_where(|h| h.session.autosave_tick() == tick) {
            self.autosave_in(group, cx);
        }
    }

    /// Write `group`'s note in the background if dirty.
    pub(super) fn autosave_in(&mut self, group: GroupId, cx: &mut Context<Self>) {
        // Never while a file operation holds the note: it was flushed, and the
        // path the save would bake in may be renamed or deleted under it.
        // Nor while a property write or its reload does: the buffer's
        // frontmatter is about to be replaced by the disk's.
        if self.note_held() {
            return;
        }
        let Some(tx) = self.tx.clone() else {
            return;
        };
        #[cfg(test)]
        if let Some(host) = self.host_in_mut(group) {
            host.tick_checks += 1;
        }
        if !self.can_save_in(group) {
            return;
        }
        if let Some((editor, host)) = self.editor_and_host_in(group) {
            let _began = host
                .session
                .spawn_autosave(Some(&editor.read(cx).editor), &tx);
            #[cfg(test)]
            if _began {
                host.tick_writes += 1;
            }
        }
    }

    /// The Save chord: write the open note now (in the background, as the
    /// tick does). Already saved: nothing to do, the Footer says "Saved".
    pub(crate) fn save_note(&mut self, cx: &mut Context<Self>) {
        self.save_note_in(self.layout.active_group(), cx);
    }

    /// [`Self::save_note`] for the note `group` holds.
    pub(super) fn save_note_in(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let Some(note) = self.host_in(group).and_then(|h| h.note.clone()) else {
            return;
        };
        if self.status.read_only {
            let text = format!(
                "Read-only: {note} can't be saved until the names that differ only by case are fixed."
            );
            self.flash_error(text, cx);
            return;
        }
        if !self.editor_dirty_in(group, cx) || self.note_held() {
            return;
        }
        let Some(tx) = self.tx.clone() else {
            return;
        };
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        if let Some(host) = state.notes.as_mut()
            && !host
                .session
                .spawn_autosave(Some(&state.editor.read(cx).editor), &tx)
        {
            // A save is in flight (or a switch's flush, which saves anyway):
            // go again when it reports.
            host.save_again = true;
        }
    }

    /// The group whose note session a save of `path` was made for: the one
    /// whose session names it (a note is open in one group), else the
    /// active one (a stale report, which marks nothing).
    pub(super) fn group_saving(&self, path: &VaultPath) -> GroupId {
        self.group_where(|h| h.session.path() == path)
            .unwrap_or_else(|| self.layout.active_group())
    }

    /// A background save reported: mark the buffer of the group that saved
    /// it saved (if it still holds what was written), flash a failure.
    pub(crate) fn autosave_completed(&mut self, done: SaveCompleted, cx: &mut Context<Self>) {
        let group = self.group_saving(&done.path);
        let active = group == self.layout.active_group();
        let Some((editor, host)) = self.editor_and_host_in(group) else {
            return;
        };
        let path = done.path.clone();
        let completion = editor.update(cx, |view, cx| {
            let completion = host
                .session
                .on_autosave_completed(done, Some(&mut view.editor));
            cx.notify();
            completion
        });
        let again = std::mem::take(&mut host.save_again);
        if let Some(saved) = &completion.saved {
            self.files_note_saved(saved, cx);
            self.refresh_facets(&saved.path, cx);
        }
        // The Footer's property count follows what was written (the TUI's
        // rule: the open note's completion, written or not).
        if completion.is_open_note {
            self.refresh_note_properties_in(group);
        }
        // A write changed the working tree.
        if completion.wrote {
            self.refresh_git();
        }
        if completion.wrote && matches!(self.closing, Closing::Discard(_)) {
            self.closing = Closing::Open;
        }
        // OUTLINE follows the headings the open note was saved with.
        if active && completion.wrote && completion.is_open_note && completion.error.is_none() {
            self.outline_saved(cx);
        }
        if let Some(error) = completion.error {
            self.flash_error(format!("Couldn't save {path}: {error}"), cx);
        } else if again {
            self.save_note_in(group, cx);
        }
    }
}
