//! The workspace's side of a **Collection** (CONTEXT.md, ADR-0061): an
//! editor group that shows a [`FacetView`] over the notes one query
//! returned. A sibling of [`super::facet_group`], which it mirrors.
//!
//! **Opening.** [`Workspace::open_collection`] runs the resolved query
//! through [`load_collection`] on tokio, then leaves the group's note as a
//! single-note facet view does ([`Workspace::leave_note`]) and shows the
//! view. The group's host keeps **no note**, so autosave, FILES and the
//! one-copy rule never see the members. A query with no hit still shows
//! the view, in its empty state. A newer request for the group (another
//! collection, a note, an attachment) replaces one still loading; a
//! refresh does nothing while one is. Until the view shows, the group's
//! session record is the collection it opens (`collection_pending`).
//!
//! **Live content.** The list of notes is fixed when the view opens. When a
//! member is written or reloaded, only that member is read again
//! ([`Workspace::refresh_members`], one read per member path). A rename or
//! move re-keys the members it touches (a folder move every member under
//! it) and reads them again; a delete takes them out of the list, down to
//! the empty state. [`Workspace::refresh_collection`] runs the query again.
//! While a load for the group runs, the view's collection line says
//! `Loading...` over the old content ([`FacetView::set_loading`], set each
//! time the group is drawn). When a refresh lands, the line says for a few
//! seconds what changed.
//!
//! **What it is not.** It shows no one note: the toggle refuses (no editor
//! to go back to), the Note menu offers only the layout items, and the
//! header's funnel is a Refresh button.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, InteractiveElement as _,
    IntoElement as _, ParentElement as _, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::Facet;
use kimun_notes::logic::file_ops::relocated;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::FileOp;
use kimun_notes::logic::query::{CollectionLoad, load_collection};
use tokio::task::JoinHandle;

use super::Workspace;
use super::facet::{FacetView, MemberInit};
use super::file_ops::path_at_or_below;
use super::group_header::REFRESH_LABEL;
use super::note::{Closing, LeavingFor, NoteLeft};
use crate::{tasks, theme};

/// Run `resolved` against the vault on tokio.
fn spawn_load(
    vault: std::sync::Arc<kimun_core::NoteVault>,
    resolved: String,
) -> JoinHandle<Result<CollectionLoad, String>> {
    tokio::spawn(async move {
        load_collection(&vault, &resolved)
            .await
            .map_err(|e| e.to_string())
    })
}

/// The view's members, from what the loader read.
fn member_inits(load: &mut CollectionLoad) -> Vec<MemberInit> {
    std::mem::take(&mut load.notes)
        .into_iter()
        .map(|note| MemberInit {
            path: note.path,
            title: note.title,
            text: note.text,
        })
        .collect()
}

/// The flash for notes the loader could not read.
fn unreadable_text(n: usize) -> String {
    match n {
        1 => "Could not read 1 note".into(),
        n => format!("Could not read {n} notes"),
    }
}

impl Workspace {
    /// The resolved query of the collection `group` shows, if it shows one.
    pub(super) fn collection_query_in(&self, group: GroupId, cx: &App) -> Option<String> {
        self.state(group)?
            .facet
            .as_ref()
            .and_then(|view| view.read(cx).query().map(str::to_owned))
    }

    /// Load the notes of `resolved` (see [`load_collection`]) and show them
    /// as a collection in `group`, replacing what the group shows, with
    /// `picks` picked (pruned against the merged facets). Replaces an
    /// earlier load for that group. Flushes the group's own note first, as
    /// [`Self::open_facet_view`] does.
    pub fn open_collection(
        &mut self,
        group: GroupId,
        resolved: String,
        picks: Vec<Facet>,
        cx: &mut Context<Self>,
    ) {
        if self.closing != Closing::Open || self.state(group).is_none() {
            return;
        }
        let Some(vault) = self.vault() else {
            return;
        };
        // The newest request wins.
        self.forget_attachment_request(group);
        let load = spawn_load(vault, resolved.clone());
        let (resolved_kept, picks_kept) = (resolved.clone(), picks.clone());
        // Leaving the note for it needs the window (a flush runs in it).
        let window = self.window;
        let task = cx.spawn(async move |this, cx| {
            let loaded = tasks::joined(load).await;
            let Some(window) = window else {
                return;
            };
            window
                .update(cx, |_, window, cx| {
                    this.update(cx, |workspace, cx| {
                        workspace.collection_loaded(group, resolved, picks, loaded, window, cx)
                    })
                })
                .ok();
        });
        if let Some(state) = self.state_mut(group) {
            state.facet_loading = Some(task);
            // What the session records until the view shows.
            state.collection_pending = Some((resolved_kept, picks_kept));
        }
    }

    /// The query's notes were read: leave `group`'s note for the view, then
    /// show it; or say why not.
    fn collection_loaded(
        &mut self,
        group: GroupId,
        query: String,
        picks: Vec<Facet>,
        loaded: Result<CollectionLoad, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state_mut(group) else {
            return;
        };
        state.facet_loading = None;
        let load = match loaded {
            Ok(load) => load,
            Err(e) => {
                state.collection_pending = None;
                self.flash_error(format!("Couldn't run {query}: {e}"), cx);
                return;
            }
        };
        let name: SharedString = query.clone().into();
        self.leave_note(
            group,
            LeavingFor::Facet,
            name,
            move |workspace, left, _, cx| match left {
                NoteLeft::Now | NoteLeft::Flushed => {
                    workspace.show_collection(group, query, load, picks, cx)
                }
                NoteLeft::Stays => workspace.forget_collection_pending(group),
            },
            window,
            cx,
        );
        // No flush under way: the view showed, or the leave was refused
        // (flashed) and the group keeps its note.
        if self
            .state(group)
            .is_some_and(|state| state.facet_flushing.is_none())
        {
            self.forget_collection_pending(group);
        }
    }

    /// `group` is no longer opening a collection.
    fn forget_collection_pending(&mut self, group: GroupId) {
        if let Some(state) = self.state_mut(group) {
            state.collection_pending = None;
        }
    }

    /// Put a collection view of `load` in `group`, its note (already
    /// flushed) let go. FILES marks nothing and the keyboard follows when
    /// `group` is the active one.
    fn show_collection(
        &mut self,
        group: GroupId,
        query: String,
        mut load: CollectionLoad,
        picks: Vec<Facet>,
        cx: &mut Context<Self>,
    ) {
        if self.state(group).is_none() {
            return;
        }
        // Any view shown before goes with the note ([`Self::empty_group`]).
        self.release_note_in(group, cx);
        self.drop_attachment_in(group);
        let asked = picks.clone();
        let members = member_inits(&mut load);
        let total = load.total;
        let view = cx.new(|cx| FacetView::new_collection(query, members, total, picks, cx));
        let events = cx.subscribe(&view, Self::on_facet_event);
        // The view drops picks no member holds without an event.
        let pruned = view.read(cx).picks() != asked.as_slice();
        if let Some(state) = self.state_mut(group) {
            state._facet_events = Some(events);
            state.facet = Some(view);
            state.collection_pending = None;
        }
        if pruned {
            self.facet_picks_changed(cx);
        }
        if load.unreadable > 0 {
            self.flash_error(unreadable_text(load.unreadable), cx);
        }
        if group == self.layout.active_group() {
            if let Some(files) = self.files.as_ref() {
                files.update(cx, |pane, cx| pane.set_open_note(None, cx));
            }
            // LINKS and OUTLINE say "No note open".
            self.refresh_views(cx);
            self.focus_shown_content(cx);
        }
        cx.notify();
    }

    /// Run the stored query again for the collection in `group`: the member
    /// list is replaced, picks pruned, reveals reset. Nothing (no flash)
    /// while a load is already under way for the group: a newer open, or a
    /// refresh, is left to land.
    pub fn refresh_collection(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self
            .state(group)
            .is_none_or(|state| state.facet_loading.is_some())
        {
            return;
        }
        let Some(query) = self.collection_query_in(group, cx) else {
            return;
        };
        let Some(vault) = self.vault() else {
            return;
        };
        let load = spawn_load(vault, query.clone());
        let task = cx.spawn(async move |this, cx| {
            let loaded = tasks::joined(load).await;
            this.update(cx, |workspace, cx| {
                workspace.collection_refreshed(group, &query, loaded, cx)
            })
            .ok();
        });
        if let Some(state) = self.state_mut(group) {
            state.facet_loading = Some(task);
        }
        // The view's line says `Loading...` from the next frame.
        cx.notify();
    }

    /// The query ran again: `group`'s view takes the new list, when it
    /// still shows that query.
    pub(super) fn collection_refreshed(
        &mut self,
        group: GroupId,
        query: &str,
        loaded: Result<CollectionLoad, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state_mut(group) else {
            return;
        };
        state.facet_loading = None;
        // The line stops saying `Loading...` on the next frame, also when
        // the result is not for the view any more.
        cx.notify();
        let Some(view) = state.facet.clone() else {
            return;
        };
        if view.read(cx).query() != Some(query) {
            return;
        }
        let mut load = match loaded {
            Ok(load) => load,
            Err(e) => {
                self.flash_error(format!("Couldn't run {query}: {e}"), cx);
                return;
            }
        };
        let members = member_inits(&mut load);
        view.update(cx, |view, cx| view.replace_members(members, load.total, cx));
        if load.unreadable > 0 {
            self.flash_error(unreadable_text(load.unreadable), cx);
        }
        cx.notify();
    }

    /// `path` changed on disk (saved, flushed, reloaded): every collection
    /// that holds it reads that one note again. Groups that do not hold it
    /// read nothing.
    pub(super) fn refresh_members(&mut self, path: &VaultPath, cx: &mut Context<Self>) {
        for group in self.group_ids() {
            let member = self
                .state(group)
                .and_then(|state| state.facet.as_ref())
                .and_then(|view| view.read(cx).member_path(path));
            if let Some(member) = member
                && self.collection_query_in(group, cx).is_some()
            {
                self.read_member(group, member, cx);
            }
        }
    }

    /// Read member `path` of `group`'s collection; a newer read of the same
    /// path replaces one still under way.
    fn read_member(&mut self, group: GroupId, path: VaultPath, cx: &mut Context<Self>) {
        let Some(vault) = self.vault() else {
            return;
        };
        let read = {
            let path = path.clone();
            tokio::spawn(async move { vault.get_note_text(&path).await.map_err(|e| e.to_string()) })
        };
        let key = path.clone();
        let task = cx.spawn(async move |this, cx| {
            let text = tasks::joined(read).await;
            this.update(cx, |workspace, cx| {
                workspace.member_read(group, &path, text, cx)
            })
            .ok();
        });
        if let Some(state) = self.state_mut(group) {
            state.collection_reading.insert(key, task);
        }
    }

    /// Member `path` was read again: `group`'s collection takes the text,
    /// when it still holds that member.
    fn member_read(
        &mut self,
        group: GroupId,
        path: &VaultPath,
        text: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state_mut(group) else {
            return;
        };
        state.collection_reading.remove(path);
        let Some(view) = state.facet.clone() else {
            return;
        };
        match text {
            Ok(text) => view.update(cx, |view, cx| view.set_member_text(path, text, cx)),
            Err(e) => tracing::warn!("collection: couldn't read {path} again: {e}"),
        }
    }

    /// A file operation landed on `group`'s collection `view`: members
    /// renamed or moved (one note, or every note under a folder) are
    /// re-keyed and read again, deleted ones leave the list. The group
    /// keeps the view, in its empty state when no member is left.
    pub(super) fn collection_file_op_in(
        &mut self,
        group: GroupId,
        view: &gpui_kit::Entity<FacetView>,
        op: &FileOp,
        cx: &mut Context<Self>,
    ) {
        match op {
            FileOp::Renamed { from, to } | FileOp::Moved { from, to } => {
                let moved: Vec<(VaultPath, VaultPath)> = view
                    .read(cx)
                    .member_paths()
                    .into_iter()
                    .filter_map(|member| relocated(&member, from, to).map(|new| (member, new)))
                    .collect();
                for (old, new) in moved {
                    view.update(cx, |view, _| view.set_member_path(&old, new.clone()));
                    if let Some(state) = self.state_mut(group) {
                        state.collection_reading.remove(&old);
                    }
                    // `set_member_path` neither plans nor redraws: the text
                    // again does both.
                    self.read_member(group, new, cx);
                }
            }
            FileOp::Deleted(gone) => {
                let deleted: Vec<VaultPath> = view
                    .read(cx)
                    .member_paths()
                    .into_iter()
                    .filter(|member| path_at_or_below(member, gone))
                    .collect();
                // One remove for them all: a folder delete plans once.
                view.update(cx, |view, cx| view.remove_members(&deleted, cx));
                if let Some(state) = self.state_mut(group) {
                    for member in &deleted {
                        state.collection_reading.remove(member);
                    }
                }
            }
            _ => return,
        }
        cx.notify();
    }

    /// The Refresh button that stands for the funnel in a collection group
    /// (its header, or the corner of a lone group): it runs the query again
    /// ([`Self::refresh_collection`]). Its press stops here, so it does not
    /// start the header's drag.
    pub(super) fn refresh_button(
        &self,
        id: impl Into<ElementId>,
        group: GroupId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let hover = theme::hover_background(&theme);
        div()
            .id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(REFRESH_LABEL)
            .flex()
            .flex_none()
            .size_6()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .text_color(theme::muted(&theme))
            .hover(move |style| style.bg(hover))
            .tooltip(move |window, cx| Tooltip::new(REFRESH_LABEL).build(window, cx))
            .child(Icon::new(super::chrome::REFRESH_ICON).size_3())
            .on_mouse_down(gpui_kit::MouseButton::Left, super::chrome::stop_press)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if this.layout.active_group() != group {
                    this.focus_group(group, cx);
                }
                this.refresh_collection(group, cx);
            }))
            .into_any_element()
    }
}
