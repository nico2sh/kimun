//! The **Facet view** over a **Collection** ([`super`], CONTEXT.md): the
//! notes one query returned, seen as one read-only document.
//!
//! The list of notes is fixed when the view opens; the host changes it only
//! by [`FacetView::replace_members`] (a refresh), [`FacetView::remove_members`]
//! (deleted notes) and [`FacetView::set_member_path`] (a rename or move).
//! A member's new text ([`FacetView::set_member_text`]) re-derives that
//! member only. Each change prunes the picks against the merged facets.
//!
//! **Drawing.** Each piece's breadcrumb row starts with its note's title as
//! plain text (`Title › Day › Section`), so a piece is attributable even
//! far from its note's other pieces. A member's first and last gap are its
//! own: between two notes a gap row of each may stand next to each other.
//! With no member the view says "No notes match this query." with a
//! `Refresh` button. When the query hit the loader's cap, a muted line under
//! the bar says how many notes it matched.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, MAX_RANGES};
use kimun_notes::logic::query::COLLECTION_NOTE_CAP;
use kimun_notes::settings::themes::Theme;

use super::keys::Stop;
use super::members::{MemberInit, Members};
use super::{FIRST_NOTES_ID, FacetEvent, FacetView, REFRESH_ID, note_title_id};
use crate::theme;
use crate::workspace::facet_ui::{OUTER_CRUMB_SHRINK, text_button};

/// What a collection view keeps beyond its members.
pub(super) struct Collection {
    /// The resolved query the notes came from.
    query: String,
    /// How many notes the query matched, before the loader's cap.
    total: usize,
    /// The last load hit the loader's cap ([`COLLECTION_NOTE_CAP`]): the
    /// "first N of M" line shows only then. A note that could not be read
    /// also makes `total` exceed the members, but is no cap.
    capped: bool,
}

impl FacetView {
    /// A collection view of `members` (list order kept), the resolved
    /// `query`, the `total` hits (for the "first 500 of N" line) and the
    /// picks (pruned against the merged set).
    pub fn new_collection(
        query: String,
        members: Vec<MemberInit>,
        total: usize,
        picks: Vec<Facet>,
        cx: &mut Context<Self>,
    ) -> Self {
        let collection = Collection {
            query,
            total,
            capped: total > COLLECTION_NOTE_CAP,
        };
        Self::build(Members::from_inits(members), Some(collection), picks, cx)
    }

    /// The resolved query of a collection; `None` for a single-note view.
    pub fn query(&self) -> Option<&str> {
        self.collection.as_ref().map(|c| c.query.as_str())
    }

    pub fn is_collection(&self) -> bool {
        self.collection.is_some()
    }

    /// Replace the member list (a manual refresh): prune picks, reset
    /// reveals and the `Show more` cap. Nothing for a single-note view.
    pub fn replace_members(
        &mut self,
        members: Vec<MemberInit>,
        total: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(collection) = &mut self.collection else {
            return;
        };
        collection.total = total;
        collection.capped = total > COLLECTION_NOTE_CAP;
        self.members = Members::from_inits(members);
        self.limit = MAX_RANGES;
        self.members_changed(cx);
    }

    /// A member's new text (a save or reload): only that member
    /// re-derives. No-op for an unknown path.
    pub fn set_member_text(&mut self, path: &VaultPath, text: String, cx: &mut Context<Self>) {
        if let Some(idx) = self.members.position(path) {
            self.member_text(idx, text, cx);
        }
    }

    /// Rename or move: re-key a member (no re-read; the caller re-reads).
    /// Its facets are derived again from the new place at once, but with no
    /// `cx` this neither prunes, plans again nor notifies: the caller must
    /// follow with [`Self::set_member_text`] after re-reading the note.
    pub fn set_member_path(&mut self, from: &VaultPath, to: VaultPath) {
        if let Some(idx) = self.members.position(from) {
            self.members.set_path(idx, to);
        }
    }

    /// Deleted members leave the list, all at once: the merged facets, the
    /// picks and the plan follow once, however many went (a folder delete).
    /// Returns how many were removed. The other members keep what they
    /// reveal unless a pick went.
    pub fn remove_members(&mut self, paths: &[VaultPath], cx: &mut Context<Self>) -> usize {
        let removed = self.members.remove_paths(paths);
        if removed == 0 {
            return 0;
        }
        // A deleted note is no hit any more.
        if let Some(collection) = &mut self.collection {
            collection.total = collection.total.saturating_sub(removed);
        }
        if self.bar.prune(&self.members.merged) {
            self.select(cx);
            cx.emit(FacetEvent::PicksChanged);
        } else {
            self.plan(cx);
        }
        cx.notify();
        removed
    }

    /// The member at `path` (compared as the vault compares paths), as the
    /// view keys it; `None` when `path` is no member.
    pub fn member_path(&self, path: &VaultPath) -> Option<VaultPath> {
        let idx = self.members.position(path)?;
        Some(self.members.list[idx].path.clone())
    }

    /// The text member `path` shows (tests).
    #[cfg(test)]
    pub(crate) fn member_text_of(&self, path: &VaultPath) -> Option<String> {
        let idx = self.members.position(path)?;
        Some(self.members.list[idx].text.clone())
    }

    /// The members' paths, in list order.
    pub fn member_paths(&self) -> Vec<VaultPath> {
        self.members.list.iter().map(|m| m.path.clone()).collect()
    }

    /// The merged count of `facet` over every member (tests).
    #[cfg(test)]
    pub(crate) fn merged_count(&self, facet: &Facet) -> Option<usize> {
        self.members
            .merged
            .entries
            .iter()
            .find(|e| &e.facet == facet)
            .map(|e| e.count)
    }

    /// `r`: a collection asks for its query again. True when taken.
    pub(super) fn refresh_key(&mut self, cx: &mut Context<Self>) -> bool {
        if self.collection.is_none() {
            return false;
        }
        cx.emit(FacetEvent::Refresh);
        true
    }

    /// The title segment that leads chunk `i`'s breadcrumb row in a
    /// collection: plain text, bright, shrinking with an ellipsis down to
    /// 32 px. `None` for a single-note view.
    pub(super) fn note_title(&self, i: usize, t: &Theme) -> Option<AnyElement> {
        self.collection.as_ref()?;
        let title = SharedString::from(self.members.member_of(i)?.title.clone());
        let mut el = div()
            .id(note_title_id(i))
            .role(Role::Label)
            .aria_label(title.clone())
            .flex()
            .h_6()
            .min_w_8()
            .items_center()
            .px_1()
            .whitespace_nowrap()
            .text_xs()
            .text_color(theme::bright(t))
            .child(div().min_w_0().truncate().child(title))
            .test_support();
        // Shrinks with the outer crumbs, before the innermost one.
        el.style().flex_shrink = Some(OUTER_CRUMB_SHRINK);
        Some(el.into_any_element())
    }

    /// The empty state's `Refresh`, in a collection with no member.
    pub(super) fn render_refresh(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.collection.is_none() || !self.members.list.is_empty() {
            return None;
        }
        let el = text_button(REFRESH_ID.into(), "Refresh", t);
        Some(self.button(Stop::Refresh, el, t, cx).into_any_element())
    }

    /// "Showing the first N of M notes", muted, under the bar: when the
    /// last load hit the loader's cap and the query matched more notes than
    /// the view holds.
    pub(super) fn render_first_notes(&self, t: &Theme) -> Option<Div> {
        let collection = self.collection.as_ref()?;
        let total = collection.total;
        let held = self.members.list.len();
        if !collection.capped || total <= held {
            return None;
        }
        let text = SharedString::from(format!("Showing the first {held} of {total} notes"));
        Some(
            div().flex_none().px_4().pt_2().child(
                div()
                    .id(FIRST_NOTES_ID)
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child(text)
                    .test_support(),
            ),
        )
    }
}
