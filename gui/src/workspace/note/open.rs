//! Where an open goes: a group, a new tab, a split, or the queue while a
//! stored session record is restoring.
//! It does not read or flush the note. The switch does that
//! ([`Workspace::switch`]).

use gpui_kit::Context;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::{Axis, GroupId};

use crate::workspace::Workspace;
use crate::workspace::session::QueuedOpen;

impl Workspace {
    /// The group holding `path` ([`super::NoteHost::holds`]), in any tab. A note
    /// is open in at most one group (ADR-0058). A facet group holds no note
    /// (its session let it go, [`crate::workspace::facet_group`]), so it never
    /// matches.
    pub(crate) fn locate_note(&self, path: &VaultPath) -> Option<GroupId> {
        self.group_where(|host| host.holds(path))
    }

    /// Open the note at `path` in the active group's editor: save the open
    /// one, then read and load this one (the attachment view, if one shows,
    /// gives way once it has loaded). A note another group holds is not
    /// opened again: that group's tab shows and its editor takes the
    /// keyboard. A directory (the root, when the history is empty) or an
    /// attachment is not a note: nothing happens.
    pub(crate) fn open_note(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        self.open_note_with(path, None, cx);
    }

    /// Open the note at `path` in a new tab, after the last one: its one
    /// group starts empty, becomes the active group with the keyboard, and
    /// the note opens into it. A note already open somewhere is focused
    /// there instead ([`Self::focus_group`]); no tab is made.
    pub(crate) fn open_note_in_new_tab(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        // A stored record still restoring would replace the new tab: the
        // open waits for it ([`crate::workspace::session`]).
        if self.session_restoring() && self.may_open(&path) {
            self.queue_open(path, None, QueuedOpen::NewTab);
            return;
        }
        if let Some(group) = self.locate_note(&path) {
            self.focus_group(group, cx);
            return;
        }
        if !self.may_open(&path) {
            return;
        }
        let Some(state) = self.new_group_state(cx) else {
            return;
        };
        let (_, group) = self.layout.region_mut().new_tab();
        // Another tab shows: a zoom ends with the one it showed.
        self.layout.zoom(None);
        self.insert_group(group, state, cx);
        self.focus_group(group, cx);
        self.open_note_in(group, path, cx);
    }

    /// Open the note at `path` with the caret at the start of source line
    /// `line` (0-based), scrolled into view: a facet view's `Open note`. A
    /// note open somewhere shows there ([`Self::focus_group`]) and jumps at
    /// once, or once it has loaded when it is still on its way. Otherwise it
    /// opens in a new tab ([`Self::open_note_in_new_tab`]) and jumps once
    /// it has loaded. An open held for a session restore opens at the top.
    pub(crate) fn open_note_at(&mut self, path: VaultPath, line: usize, cx: &mut Context<Self>) {
        let holder = self.locate_note(&path);
        self.open_note_in_new_tab(path.clone(), cx);
        let group = holder.or_else(|| {
            let active = self.layout.active_group();
            self.host_in(active)
                .is_some_and(|host| host.holds(&path))
                .then_some(active)
        });
        if let Some(group) = group {
            self.jump_in(group, &path, line, cx);
        }
    }

    /// Put `group`'s caret on source line `line` of `path`: now when the
    /// group shows that note, else once it has loaded. Nothing when the
    /// group is on its way to another note.
    pub(super) fn jump_in(
        &mut self,
        group: GroupId,
        path: &VaultPath,
        line: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(host) = state.notes.as_mut() else {
            return;
        };
        let like = |p: Option<&VaultPath>| p.is_some_and(|p| p.is_like(path));
        if like(host.wanted.as_ref()) {
            host.wanted_line = Some(line);
        } else if like(host.session.pending_open()) {
            host.reading_line = Some(line);
        } else if like(host.note.as_ref()) && host.wanted.is_none() {
            state.editor.update(cx, |view, cx| {
                view.editor.jump_to_row(line);
                cx.notify();
            });
        }
    }

    /// Open the note at `path` in a new group split off the active one
    /// along `axis`, right after it: the group starts empty, becomes the
    /// active group with the keyboard, and the note opens into it. A note
    /// already open somewhere is focused there instead; no group is made.
    /// Refused with a flash while the shown tab's close writes its notes.
    pub(crate) fn open_note_in_split(
        &mut self,
        path: VaultPath,
        axis: Axis,
        cx: &mut Context<Self>,
    ) {
        // As for a new tab: the split waits for a restore on its way.
        if self.session_restoring() && self.may_open(&path) {
            self.queue_open(path, None, QueuedOpen::Split(axis));
            return;
        }
        if let Some(group) = self.locate_note(&path) {
            self.focus_group(group, cx);
            return;
        }
        if !self.may_open(&path) {
            return;
        }
        if self.tab_mid_close(self.layout.region().shown()) {
            self.flash_error(crate::workspace::tab_strip::TAB_CLOSING, cx);
            return;
        }
        let Some(state) = self.new_group_state(cx) else {
            return;
        };
        let active = self.layout.active_group();
        let Some(group) = self.layout.region_mut().split_group(active, axis) else {
            return;
        };
        self.insert_group(group, state, cx);
        self.focus_group(group, cx);
        self.open_note_in(group, path, cx);
    }

    /// Whether an open of `path` may start: it is a note and the window is
    /// not closing.
    pub(super) fn may_open(&self, path: &VaultPath) -> bool {
        path.is_note() && !self.window_closing()
    }

    /// [`Self::open_note`] in the editor of `group`, active or not, with no
    /// one-copy check: the caller made sure no other group holds `path`.
    pub(crate) fn open_note_in(&mut self, group: GroupId, path: VaultPath, cx: &mut Context<Self>) {
        self.open_note_in_with(group, path, None, cx);
    }

    /// [`Self::open_note`], the note's text then emphasising `emphasis` (the
    /// search needles of the query it was found by) until the first edit.
    pub(crate) fn open_note_with(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        let group = self.layout.active_group();
        // One copy: a note open in another group is shown there. The
        // active group's own note opens again, as it always did.
        if let Some(holder) = self.locate_note(&path)
            && holder != group
        {
            self.focus_group(holder, cx);
            return;
        }
        self.open_note_in_with(group, path, emphasis, cx);
    }

    pub(super) fn open_note_in_with(
        &mut self,
        group: GroupId,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if !self.may_open(&path) {
            return;
        }
        // The session record is still being restored: it wins, and this
        // open goes into its active group once it landed (the newest
        // request kept, [`crate::workspace::session`]).
        if self.session_restoring() {
            self.queue_open(path, emphasis, QueuedOpen::Plain);
            return;
        }
        // The newest request wins: an attachment still being read is dropped.
        self.forget_attachment_request(group);
        self.forget_facet_request(group);
        let Some(host) = self.host_in_mut(group) else {
            return;
        };
        host.wanted = Some(path);
        host.wanted_emphasis = emphasis;
        host.wanted_line = None;
        // A file operation or a property write holds the open note (flushed,
        // its tick stopped): the open waits for it to end
        // ([`Self::file_op_ended`]).
        let held = self.note_held();
        let Some(host) = self.host_in_mut(group) else {
            return;
        };
        if !host.is_switching() && !held {
            self.switch(group, cx);
        }
    }
}
