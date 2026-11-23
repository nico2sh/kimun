//! The workspace's side of the **Facet view** (ADR-0061, CONTEXT.md): an
//! editor group that shows a [`FacetView`] of a note in place of its editor.
//!
//! **Opening.** [`Workspace::open_facet_view`] reads the source through the
//! vault, then leaves the group's note the way an attachment does
//! ([`Workspace::leave_note`]: flushed first, refused when that fails) and
//! shows the view. While it shows, the group's note session holds **no
//! note** (`NoteHost.note` is `None`, the buffer empty): autosave and the
//! editor never see the source, and a note may be open in an editor group
//! and shown in any number of facet groups at once (the one-copy rule,
//! ADR-0058, is about editor groups). Opening a note in the group (any
//! door, `Edit note` included) takes the view down.
//!
//! **Live content.** When the source is written (a save in another group, a
//! flush) or reloaded from disk (a property write), every facet group on it
//! reads it again through the vault and takes the new text
//! ([`FacetView::set_text`]). A rename or move follows the source
//! ([`FacetView::set_source`], then the text again, so note-link facets
//! resolve from the new place); a delete empties the group.
//!
//! **Leaving it.** A follow or a jump to source that starts in a facet view
//! opens in a new tab (ADR-0061, decision 5); every other open keeps the
//! plain-open rule.

use gpui_kit::{App, AppContext as _, Context, Entity, SharedString, Window};
use kimun_core::nfs::VaultPath;
use kimun_core::note::Facet;
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::file_ops::{NoteOutcome, ShownPathEffect, effect_on};
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::{AppMessage, FileOp};

use super::Workspace;
use super::facet::{FacetEvent, FacetView};
use super::link_menu::LinkMenuPick;
use super::note::{Closing, LeavingFor, NoteLeft, OpenAt};
use crate::tasks;

impl Workspace {
    /// The source of the single-note facet view `group` shows, if it shows
    /// one (a collection has no one source).
    pub(crate) fn facet_source_in(&self, group: GroupId, cx: &App) -> Option<VaultPath> {
        self.state(group)?
            .facet
            .as_ref()
            .and_then(|view| view.read(cx).source().cloned())
    }

    /// Whether `group` shows a facet view: of one note, or a collection.
    pub(crate) fn facet_group_in(&self, group: GroupId, _cx: &App) -> bool {
        self.state(group).is_some_and(|state| state.facet.is_some())
    }

    /// The note `group` shows: its editor's, else its facet view's source.
    pub(super) fn shown_note_in(&self, group: GroupId, cx: &App) -> Option<VaultPath> {
        self.host_in(group)
            .and_then(|host| host.note.clone())
            .or_else(|| self.facet_source_in(group, cx))
    }

    /// Show a facet view of `note` in `group`, replacing what the group
    /// shows, with `picks` picked (those the note does not hold are
    /// dropped). See the module docs.
    pub fn open_facet_view(
        &mut self,
        group: GroupId,
        note: VaultPath,
        picks: Vec<Facet>,
        cx: &mut Context<Self>,
    ) {
        if self.closing != Closing::Open || !note.is_note() || self.state(group).is_none() {
            return;
        }
        let Some(vault) = self.vault() else {
            return;
        };
        // The newest request wins.
        self.forget_attachment_request(group);
        let read = {
            let note = note.clone();
            tokio::spawn(async move { vault.get_note_text(&note).await.map_err(|e| e.to_string()) })
        };
        // Leaving the note for it needs the window (a flush runs in it).
        let window = self.window;
        let task = cx.spawn(async move |this, cx| {
            let text = tasks::joined(read).await;
            let Some(window) = window else {
                return;
            };
            window
                .update(cx, |_, window, cx| {
                    this.update(cx, |workspace, cx| {
                        workspace.facet_read(group, note, picks, text, window, cx)
                    })
                })
                .ok();
        });
        if let Some(state) = self.state_mut(group) {
            state.facet_loading = Some(task);
            // A collection still loading gives way.
            state.collection_pending = None;
        }
    }

    /// The source was read: leave `group`'s note for the view, then show
    /// it; or say why not.
    fn facet_read(
        &mut self,
        group: GroupId,
        note: VaultPath,
        picks: Vec<Facet>,
        text: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state_mut(group) else {
            return;
        };
        state.facet_loading = None;
        let text = match text {
            Ok(text) => text,
            Err(e) => {
                self.flash_error(format!("Couldn't open {note}: {e}"), cx);
                return;
            }
        };
        let name: SharedString = note.get_name().into();
        self.leave_note(
            group,
            LeavingFor::Facet,
            name,
            move |workspace, left, _, cx| match left {
                NoteLeft::Now | NoteLeft::Flushed => {
                    workspace.show_facet(group, note, text, picks, cx)
                }
                NoteLeft::Stays => {}
            },
            window,
            cx,
        );
    }

    /// Put a facet view of `note` in `group`, its note (already flushed)
    /// let go. The Footer, FILES and the keyboard follow when `group` is
    /// the active one.
    fn show_facet(
        &mut self,
        group: GroupId,
        note: VaultPath,
        text: String,
        picks: Vec<Facet>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state(group) else {
            return;
        };
        // The group's own note, just written: its clean buffer is at least
        // as new as the read.
        let editor = state.editor.read(cx);
        let own = state
            .notes
            .as_ref()
            .and_then(|host| host.note.as_ref())
            .is_some_and(|open| open.is_like(&note));
        let text = if own && !editor.editor.is_dirty() {
            editor.editor.get_text()
        } else {
            text
        };
        // Any view shown before goes with the note ([`Self::empty_group`]).
        self.release_note_in(group, cx);
        self.drop_attachment_in(group);
        let asked = picks.clone();
        let vault = self.group_vault(group);
        let view = cx.new(|cx| {
            let mut view = FacetView::new(note.clone(), text, picks, cx);
            view.set_image_vault(vault, cx);
            view
        });
        let events = cx.subscribe(&view, Self::on_facet_event);
        // The view drops picks the note no longer holds without an event:
        // the same change as one it reports.
        let pruned = view.read(cx).picks() != asked.as_slice();
        if let Some(state) = self.state_mut(group) {
            state._facet_events = Some(events);
            state.facet = Some(view);
        }
        if pruned {
            self.facet_picks_changed(cx);
        }
        if group == self.layout.active_group() {
            if let Some(files) = self.files.as_ref() {
                files.update(cx, |pane, cx| pane.set_open_note(Some(note), cx));
            }
            // LINKS and OUTLINE say "No note open".
            self.refresh_views(cx);
            self.focus_shown_content(cx);
        }
        cx.notify();
    }

    /// What a facet view asks of its group.
    pub(super) fn on_facet_event(
        &mut self,
        view: Entity<FacetView>,
        event: &FacetEvent,
        cx: &mut Context<Self>,
    ) {
        let shown = self
            .groups
            .values()
            .any(|state| state.facet.as_ref() == Some(&view));
        if !shown {
            return;
        }
        match event {
            FacetEvent::OpenNote { note, line } => self.open_note_at_line(note.clone(), *line, cx),
            FacetEvent::Follow {
                from,
                target: FollowTarget::Link { target, kind },
            } => self.follow_link_from(from, target.clone(), *kind, OpenAt::NewTab, cx),
            FacetEvent::Follow {
                target: FollowTarget::Label(name),
                ..
            } => self.follow_label(name, cx),
            // A right-click on a link in a piece: the Link menu, from the
            // piece's own note, for the group that shows the view.
            FacetEvent::LinkMenu { from, at, target } => {
                let group = self
                    .group_ids()
                    .into_iter()
                    .find(|g| self.state(*g).and_then(|s| s.facet.as_ref()) == Some(&view));
                if let Some(group) = group {
                    let pick = LinkMenuPick {
                        group,
                        from: from.clone(),
                        target: target.clone(),
                        facet: true,
                    };
                    let at = *at;
                    self.with_window(cx, move |workspace, window, cx| {
                        workspace.open_link_menu(pick, at, window, cx);
                    });
                }
            }
            FacetEvent::ToggleTask { note, line, seen } => {
                self.toggle_facet_task(note.clone(), *line, seen.clone(), cx)
            }
            FacetEvent::PicksChanged => self.facet_picks_changed(cx),
            // A single-note view never emits it.
            FacetEvent::Refresh => {
                let group = self
                    .group_ids()
                    .into_iter()
                    .find(|g| self.state(*g).and_then(|s| s.facet.as_ref()) == Some(&view));
                if let Some(group) = group {
                    self.refresh_collection(group, cx);
                }
            }
        }
    }

    /// The picks of a shown facet view changed (picked, or dropped as the
    /// note lost them): the session record follows the window's notify.
    pub(super) fn facet_picks_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// What the facet toggle does for `group`: `Some(true)` while it shows
    /// a facet view (the toggle leaves it), `Some(false)` while it shows a
    /// note in its editor (the toggle opens a view), `None` for an empty
    /// group or an attachment (nothing to toggle).
    pub(crate) fn facet_toggle_state(&self, group: GroupId) -> Option<bool> {
        let state = self.state(group)?;
        if state.facet.is_some() {
            return Some(true);
        }
        let has_note = self.host_in(group).is_some_and(|host| host.note.is_some());
        (has_note && state.attachment.is_none()).then_some(false)
    }

    /// Why the facet toggle cannot run in `group`, if it cannot: a
    /// collection has no editor to go back to, a group with no note has
    /// nothing to show.
    pub(crate) fn facet_toggle_blocked(&self, group: GroupId, cx: &App) -> Option<&'static str> {
        if self.collection_query_in(group, cx).is_some() {
            return Some(super::route::NO_EDITOR_TO_RETURN_TO);
        }
        self.facet_toggle_state(group)
            .is_none()
            .then_some(super::route::NO_NOTE)
    }

    /// The facet toggle (the header button, the `ToggleFacetView` action):
    /// a group showing a note opens a facet view of it with no picks, a
    /// group showing a single-note facet view returns to the editor, a
    /// collection refuses (no editor to go back to), any other group says
    /// there is no note.
    pub(crate) fn toggle_facet_view(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if let Some(reason) = self.facet_toggle_blocked(group, cx) {
            self.flash(reason, cx);
            return;
        }
        // A group with no toggle state was refused above (`NO_NOTE`).
        let Some(shows_facet) = self.facet_toggle_state(group) else {
            return;
        };
        if shows_facet {
            self.close_facet_view(group, cx);
        } else if let Some(note) = self.shown_note_in(group, cx) {
            self.open_facet_view(group, note, Vec::new(), cx);
        }
    }

    /// `Edit note`: back to the editor on the source of `group`'s facet
    /// view. A source another group holds shows there instead (the
    /// plain-open rule), and the facet view stays.
    pub fn close_facet_view(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let Some(source) = self.facet_source_in(group, cx) else {
            return;
        };
        if let Some(holder) = self.locate_note(&source)
            && holder != group
        {
            self.focus_group(holder, cx);
            return;
        }
        // The view goes once the note has loaded ([`Self::clear_facet_in`]).
        self.open_note_in(group, source, cx);
    }

    /// Let `group`'s facet view go (one note's or a collection's), with its
    /// subscription and its reads. True when one was shown. The caller
    /// refocuses and redraws.
    pub(super) fn drop_facet_in(&mut self, group: GroupId) -> bool {
        let Some(state) = self.state_mut(group) else {
            return false;
        };
        state._facet_events = None;
        state.facet_reading = None;
        state.collection_reading.clear();
        state.facet.take().is_some()
    }

    /// A note opened in `group`: its facet view goes (the editor is back).
    pub(super) fn clear_facet_in(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self.drop_facet_in(group) && group == self.layout.active_group() {
            self.focus_shown_content(cx);
            cx.notify();
        }
    }

    /// Something else was asked of `group`: a facet view or a collection
    /// still being read gives way.
    pub(super) fn forget_facet_request(&mut self, group: GroupId) {
        if let Some(state) = self.state_mut(group) {
            state.facet_loading = None;
            state.collection_pending = None;
        }
    }

    /// `path` changed on disk (saved, flushed, reloaded): every facet view
    /// of it takes its text, read again through the vault, and so does
    /// every collection that holds it ([`Self::refresh_members`]).
    pub(super) fn refresh_facets(&mut self, path: &VaultPath, cx: &mut Context<Self>) {
        self.refresh_members(path, cx);
        for group in self.group_ids() {
            if self
                .facet_source_in(group, cx)
                .is_some_and(|source| source.is_like(path))
            {
                self.refresh_facet_in(group, path.clone(), cx);
            }
        }
    }

    /// Read `path` for `group`'s facet view; a newer read replaces one
    /// still under way.
    fn refresh_facet_in(&mut self, group: GroupId, path: VaultPath, cx: &mut Context<Self>) {
        let Some(vault) = self.vault() else {
            return;
        };
        let read = {
            let path = path.clone();
            tokio::spawn(async move { vault.get_note_text(&path).await.map_err(|e| e.to_string()) })
        };
        let task = cx.spawn(async move |this, cx| {
            let text = tasks::joined(read).await;
            this.update(cx, |workspace, cx| {
                workspace.facet_text_read(group, &path, text, cx)
            })
            .ok();
        });
        if let Some(state) = self.state_mut(group) {
            state.facet_reading = Some(task);
        }
    }

    /// The source was read again: `group`'s view takes the text, when it
    /// still shows that note.
    fn facet_text_read(
        &mut self,
        group: GroupId,
        path: &VaultPath,
        text: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state_mut(group) else {
            return;
        };
        state.facet_reading = None;
        let Some(view) = state.facet.clone() else {
            return;
        };
        if !view
            .read(cx)
            .source()
            .is_some_and(|source| source.is_like(path))
        {
            return;
        }
        match text {
            Ok(text) => view.update(cx, |view, cx| view.set_text(text, cx)),
            Err(e) => tracing::warn!("facet view: couldn't read {path} again: {e}"),
        }
    }

    /// A file operation landed: every facet view follows its source renamed
    /// or moved, and its group empties when the source was deleted. Only
    /// the active group's says so in the editor Panel. A collection follows
    /// its members instead ([`Self::collection_file_op_in`]).
    pub(super) fn facet_file_op(&mut self, op: &FileOp, cx: &mut Context<Self>) {
        for group in self.group_ids_active_first() {
            self.facet_file_op_in(group, op, cx);
        }
    }

    /// [`Self::facet_file_op`] for the view of `group`.
    fn facet_file_op_in(&mut self, group: GroupId, op: &FileOp, cx: &mut Context<Self>) {
        let Some(view) = self.state(group).and_then(|s| s.facet.clone()) else {
            return;
        };
        if view.read(cx).is_collection() {
            self.collection_file_op_in(group, &view, op, cx);
            return;
        }
        let Some(shown) = view.read(cx).source().cloned() else {
            return;
        };
        match effect_on(&shown, op) {
            ShownPathEffect::Retarget(new) => {
                view.update(cx, |view, _| view.set_source(new.clone()));
                // `set_source` keeps the facets: the text again rebuilds
                // the note links from the new place.
                self.refresh_facet_in(group, new, cx);
                cx.notify();
            }
            ShownPathEffect::Close => {
                self.empty_group(group, cx);
                self.say_shown_gone(group, &shown, cx);
            }
            ShownPathEffect::Unchanged => {}
        }
    }

    /// Open the note a create made where `at` says (a follow from a facet
    /// view, or a Link menu pick): [`FileOp::Created`] reported first when it is new, as
    /// [`NoteOutcome::announce_and_open`] does for a plain open.
    pub(super) fn open_made_at(&mut self, made: NoteOutcome, at: OpenAt, cx: &mut Context<Self>) {
        if made.created
            && let Some(tx) = self.tx.as_ref()
        {
            tx.send(AppMessage::FileOp(FileOp::Created(made.path.clone())))
                .ok();
        }
        self.open_note_at(made.path, at, cx);
    }

    /// Open the attachment at `path` in a new tab (a follow from a facet
    /// view): its one group becomes the active group with the keyboard.
    pub(super) fn open_attachment_in_new_tab(&mut self, path: &VaultPath, cx: &mut Context<Self>) {
        if self.window_closing() {
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
        self.open_attachment_in(group, path, cx);
    }
}
