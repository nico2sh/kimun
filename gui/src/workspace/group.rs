//! What one editor group holds: its editor, its note session and the
//! attachment it may show in place of the editor. The [`Workspace`] keeps
//! one per group of the layout tree (`Workspace::groups`).

use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, Entity, Subscription, Task};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::autocomplete::VaultSuggestions;
use kimun_notes::logic::layout_tree::{Focus, GroupId, Leaf};
use kimun_notes::logic::session::NoteSession;

use super::Workspace;
use super::attachment::AttachmentView;
use super::note::NoteHost;
use crate::editor_view::EditorView;

/// The active group's state as a borrow of `self.groups` alone, so the
/// caller can still use the workspace's other fields (`tx`, `closing`, ...)
/// while it holds the note session.
macro_rules! active_mut {
    ($ws:ident) => {
        $ws.groups
            .get_mut(&$ws.layout.active_group())
            .expect("the active group has a state")
    };
}
pub(super) use active_mut;

/// The per-group state: moved here from the workspace, field for field.
pub(crate) struct GroupState {
    pub(crate) editor: Entity<EditorView>,
    /// The note session over the vault, when the group has one (not in the
    /// empty states, nor in the tests that feed the editor text directly).
    pub(crate) notes: Option<NoteHost>,
    /// The attachment the editor Panel shows instead of the editor
    /// ([`super::attachment`]), and its events.
    pub(crate) attachment: Option<Entity<AttachmentView>>,
    pub(super) _attachment_events: Option<Subscription>,
    /// The read of an attachment asked for (a newer request replaces it).
    pub(super) attachment_loading: Option<Task<()>>,
    /// The flush of the open note before an attachment shows: it holds the
    /// note like a file operation does.
    pub(super) attachment_flushing: Option<Task<()>>,
    /// An attachment asked for while that flush ran: it opens next.
    pub(super) attachment_wanted: Option<VaultPath>,
    /// The Footer's dirty watch and the window requests of an editor made
    /// for a new group ([`Workspace::new_group_state`]); the first group's
    /// live on the workspace.
    _editor_watch: Option<Subscription>,
    _editor_events: Option<Subscription>,
}

impl GroupState {
    pub(crate) fn new(editor: Entity<EditorView>) -> Self {
        Self {
            editor,
            notes: None,
            attachment: None,
            _attachment_events: None,
            attachment_loading: None,
            attachment_flushing: None,
            attachment_wanted: None,
            _editor_watch: None,
            _editor_events: None,
        }
    }
}

impl Workspace {
    // Invariant: `groups` has a state for every group of `layout`, the active
    // one included. Code that replaces `layout` (a preset, an undo) must
    // keep them in step; the accessors below assert it in debug builds.

    /// The state of the active group.
    pub(crate) fn active_state(&self) -> &GroupState {
        debug_assert!(
            self.groups.contains_key(&self.layout.active_group()),
            "groups out of step with the layout"
        );
        self.groups
            .get(&self.layout.active_group())
            .expect("the active group has a state")
    }

    /// The state of the active group, mutable.
    pub(crate) fn active_state_mut(&mut self) -> &mut GroupState {
        debug_assert!(
            self.groups.contains_key(&self.layout.active_group()),
            "groups out of step with the layout"
        );
        self.groups
            .get_mut(&self.layout.active_group())
            .expect("the active group has a state")
    }

    /// Every group of the region, in every tab, in the region's order: what
    /// the note lifecycle (saves, the close, a vault change, file
    /// operations) walks, so no group is left out for not being active.
    pub(crate) fn group_ids(&self) -> Vec<GroupId> {
        self.layout.region().all_groups()
    }

    /// [`Self::group_ids`] with the active group first: where one of them
    /// is named in a message, the one the user looks at comes first.
    pub(crate) fn group_ids_active_first(&self) -> Vec<GroupId> {
        let active = self.layout.active_group();
        let mut groups = self.group_ids();
        groups.retain(|g| *g != active);
        groups.insert(0, active);
        groups
    }

    /// The first group, in the region's order ([`Self::group_ids`]), whose
    /// note host satisfies `pred`. `None` when none does: a caller that
    /// wants a fallback adds it.
    pub(crate) fn group_where(&self, pred: impl Fn(&NoteHost) -> bool) -> Option<GroupId> {
        self.first_group_where(self.group_ids(), pred)
    }

    /// [`Self::group_where`] trying the active group first
    /// ([`Self::group_ids_active_first`]).
    pub(crate) fn group_where_active_first(
        &self,
        pred: impl Fn(&NoteHost) -> bool,
    ) -> Option<GroupId> {
        self.first_group_where(self.group_ids_active_first(), pred)
    }

    fn first_group_where(
        &self,
        groups: Vec<GroupId>,
        pred: impl Fn(&NoteHost) -> bool,
    ) -> Option<GroupId> {
        groups.into_iter().find(|group| {
            self.state(*group)
                .and_then(|s| s.notes.as_ref())
                .is_some_and(&pred)
        })
    }

    /// The state of `group`, if it has one.
    pub(crate) fn state(&self, group: GroupId) -> Option<&GroupState> {
        self.groups.get(&group)
    }

    /// The state of `group`, mutable, if it has one. A background job finds
    /// its own group here, whichever one is active when it ends.
    pub(crate) fn state_mut(&mut self, group: GroupId) -> Option<&mut GroupState> {
        self.groups.get_mut(&group)
    }

    /// The open vault, read from whichever group has a note session (the
    /// active one may be a group still being made).
    fn any_vault(&self) -> Option<Arc<NoteVault>> {
        self.groups
            .values()
            .find_map(|s| s.notes.as_ref())
            .map(|h| h.session.vault().clone())
    }

    /// The state of a new group: an empty editor of the kind the window
    /// runs (vim or plain), on the window's channel and completing from the
    /// open vault, and a note session of its own over that vault, on no
    /// note yet. `None` without a vault (no workspace).
    pub(super) fn new_group_state(&self, cx: &mut Context<Self>) -> Option<GroupState> {
        let vault = self.any_vault()?;
        let settings = self.settings.clone()?;
        let tx = self.tx.clone()?;
        let vim = self.active_state().editor.read(cx).editor.is_vim();
        let editor = cx.new(|cx| EditorView::on_channel(vim, tx, cx));
        editor.update(cx, |view, _| {
            view.set_suggestions(Some(Arc::new(VaultSuggestions {
                vault: vault.clone(),
            })));
        });
        let watch = cx.observe(&editor, Self::editor_changed);
        let mut state = GroupState::new(editor);
        state._editor_watch = Some(watch);
        state.notes = Some(NoteHost::new(NoteSession::new(
            vault,
            settings,
            VaultPath::root(),
        )));
        Some(state)
    }

    /// Give `group`, just added to the region, its `state`; its editor's
    /// window requests are wired once the current update is done.
    pub(super) fn insert_group(
        &mut self,
        group: GroupId,
        state: GroupState,
        cx: &mut Context<Self>,
    ) {
        let editor = state.editor.clone();
        self.groups.insert(group, state);
        self.with_window(cx, move |workspace, window, cx| {
            // The group may have gone, or its id been given to another
            // group's state (a region replaced meanwhile): wire only this
            // editor.
            if workspace
                .groups
                .get(&group)
                .is_none_or(|state| state.editor != editor)
            {
                return;
            }
            let events = cx.subscribe_in(&editor, window, Self::on_editor_event);
            if let Some(state) = workspace.groups.get_mut(&group) {
                state._editor_events = Some(events);
            }
        });
    }

    /// An editor changed: the Footer's dirty state follows the active
    /// group's.
    pub(super) fn editor_changed(&mut self, editor: Entity<EditorView>, cx: &mut Context<Self>) {
        if self
            .state(self.layout.active_group())
            .is_some_and(|s| s.editor != editor)
        {
            return;
        }
        let dirty = editor.read(cx).editor.is_dirty();
        if self.status.dirty != dirty {
            self.status.dirty = dirty;
            cx.notify();
        }
    }

    /// Make `group` the active group, show its tab and give its editor the
    /// keyboard; a zoom on a slot ends, as the group is asked to show, and
    /// so does any zoom when another tab shows (a zoom shows one group of
    /// one tab). The Footer (its counts fetched again), FILES' mark and the
    /// views that follow the open note follow the group. Nothing happens
    /// for an unknown group.
    pub(crate) fn focus_group(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let region = self.layout.region();
        let other_tab = region.group_tab(group) != Some(region.shown());
        if !self.groups.contains_key(&group) || !self.layout.region_mut().set_active_group(group) {
            return;
        }
        if other_tab || self.layout.zoomed().is_some_and(|z| z != Leaf::Editor) {
            self.layout.zoom(None);
        }
        self.layout.focus(Focus::Leaf(Leaf::Editor));
        self.sync_active_status(cx);
        // Counts kept while the group was hidden may be stale.
        self.refresh_note_counts();
        self.focus_shown_content(cx);
        cx.notify();
    }

    /// The Footer, FILES' open-note mark and the views that follow the
    /// open note take the active group's note, attachment and dirty state.
    pub(super) fn sync_active_status(&mut self, cx: &mut Context<Self>) {
        let state = self.active_state();
        let note = state.notes.as_ref().and_then(|h| h.note.clone());
        let attachment = state
            .attachment
            .as_ref()
            .map(|view| view.read(cx).path().clone());
        let dirty = state.editor.read(cx).editor.is_dirty();
        self.status.note = note.clone();
        self.status.attachment = attachment.clone();
        self.status.dirty = dirty;
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| pane.set_open_note(note.or(attachment), cx));
        }
        self.refresh_views(cx);
    }

    /// Give `group` (already in the layout) an empty editor and, with
    /// `settings`, a note session over the open vault (read from any group
    /// that has one, so `group` may already be the active one).
    #[cfg(test)]
    pub(crate) fn add_group_for_test(
        &mut self,
        group: GroupId,
        settings: Option<kimun_notes::settings::SharedSettings>,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        let mut state = GroupState::new(gpui_kit::AppContext::new(cx, |cx| {
            EditorView::new(None, false, cx)
        }));
        if let (Some(settings), Some(vault)) = (settings, self.any_vault()) {
            state.notes = Some(NoteHost::new(NoteSession::new(
                vault,
                settings,
                VaultPath::root(),
            )));
        }
        self.groups.insert(group, state);
    }
}
