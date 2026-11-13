//! What one editor group holds: its editor, its note session and the
//! attachment it may show in place of the editor. The [`Workspace`] keeps
//! one per group of the layout tree (`Workspace::groups`).

use gpui_kit::{Entity, Subscription, Task};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::GroupId;

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
        }
    }
}

impl Workspace {
    /// The state of the active group.
    pub(crate) fn active_state(&self) -> &GroupState {
        self.groups
            .get(&self.layout.active_group())
            .expect("the active group has a state")
    }

    /// The state of the active group, mutable.
    pub(crate) fn active_state_mut(&mut self) -> &mut GroupState {
        self.groups
            .get_mut(&self.layout.active_group())
            .expect("the active group has a state")
    }

    /// The state of `group`, if it has one.
    #[allow(dead_code, reason = "read by the per-group tasks that follow")]
    pub(crate) fn state(&self, group: GroupId) -> Option<&GroupState> {
        self.groups.get(&group)
    }
}
