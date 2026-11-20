//! The **Facet view** over a **Collection** ([`super`], CONTEXT.md): the
//! notes one query returned, seen as one read-only document.
//!
//! The list of notes is fixed when the view opens; the host changes it only
//! by [`FacetView::replace_members`] (a refresh), [`FacetView::remove_members`]
//! (deleted notes) and [`FacetView::set_member_path`] (a rename or move).
//! A member's new text ([`FacetView::set_member_text`]) re-derives that
//! member only. Each change prunes the picks against the merged facets.
//!
//! **Drawing.** Each note with a drawn piece starts with a dateline: its
//! title, a hairline that fills the row, and a muted meta text, the note's
//! folder and how many of its pieces are drawn (`journal · 2 pieces`; a note
//! at the vault root shows the count only). It is plain text, no tab stop and
//! no button. It stands above the note's first gap, so that gap's hidden
//! count reads as the note's own; the note above keeps its last gap above
//! the dateline. Every dateline but the column's first row has 24 px above
//! it. A piece's breadcrumb row holds only its heading crumbs and `Open
//! source`. A member's first and last gap are its own: between two notes a
//! gap row of each may stand next to each other.
//! With no member the view says "No notes match this query." with a
//! `Refresh` button. When the query hit the loader's cap, a muted line under
//! the bar says how many notes it matched.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, MAX_RANGES};
use kimun_notes::logic::query::COLLECTION_NOTE_CAP;
use kimun_notes::settings::themes::Theme;

use super::keys::Stop;
use super::members::{GapRef, MemberInit, Members};
use super::{FIRST_NOTES_ID, FacetEvent, FacetView, REFRESH_ID};
use crate::theme;
use crate::workspace::facet_ui::text_button;

/// The dateline of member `member` (its index in the member list).
pub(crate) fn dateline_id(member: usize) -> SharedString {
    format!("facet-dateline:{member}").into()
}

/// The title on member `member`'s dateline.
pub(crate) fn dateline_title_id(member: usize) -> SharedString {
    format!("facet-dateline-title:{member}").into()
}

/// The hairline on member `member`'s dateline.
pub(crate) fn dateline_rule_id(member: usize) -> SharedString {
    format!("facet-dateline-rule:{member}").into()
}

/// The folder on member `member`'s dateline, when it has one.
pub(crate) fn dateline_folder_id(member: usize) -> SharedString {
    format!("facet-dateline-folder:{member}").into()
}

/// The piece count on member `member`'s dateline, at its right end; its
/// name is the whole meta text ([`dateline_meta`]).
pub(crate) fn dateline_meta_id(member: usize) -> SharedString {
    format!("facet-dateline-meta:{member}").into()
}

/// The folder a dateline names for the note at `path`: its parent folder,
/// written from the vault root without a leading separator
/// (`journal/2026`). `None` for a note at the vault root.
pub(crate) fn note_folder(path: &VaultPath) -> Option<String> {
    let (mut parent, _) = path.flatten().get_parent_path();
    if parent.is_root_or_empty() {
        return None;
    }
    parent.to_relative();
    Some(parent.to_string())
}

/// `1 piece`, `2 pieces`.
pub(crate) fn piece_count(pieces: usize) -> String {
    match pieces {
        1 => "1 piece".into(),
        n => format!("{n} pieces"),
    }
}

/// A dateline's meta text: the note's folder ([`note_folder`]) and its
/// drawn pieces ([`piece_count`]), joined by ` · `; the count alone at the
/// vault root.
pub(crate) fn dateline_meta(path: &VaultPath, pieces: usize) -> String {
    let count = piece_count(pieces);
    match note_folder(path) {
        Some(folder) => format!("{folder} \u{b7} {count}"),
        None => count,
    }
}

/// How much faster a dateline's folder shrinks than its title.
const META_FOLDER_SHRINK: f32 = 1000.;

/// The size of a dateline's title, in pixels.
const DATELINE_TITLE_PX: f32 = 13.0;

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

    /// The dateline above `gap` (see the module docs): only in a collection,
    /// and only for a member's first gap when that member has a drawn chunk
    /// below it. `spaced` adds the room that makes 24 px above it with the
    /// column's gap: false for the column's first row.
    pub(super) fn render_dateline(
        &self,
        gap: GapRef,
        spaced: bool,
        t: &Theme,
    ) -> Option<AnyElement> {
        self.collection.as_ref()?;
        let first = gap.below.filter(|_| gap.above.is_none())?;
        let member = self.members.list.get(gap.member)?;
        let m = gap.member;
        let title = SharedString::from(member.title.clone());
        let pieces = self.members.drawn_with(first);
        Some(
            div()
                .id(dateline_id(m))
                .role(Role::Heading)
                .aria_label(title.clone())
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .when(spaced, |row| row.mt_3())
                .child(
                    div()
                        .id(dateline_title_id(m))
                        // A shrunk title keeps a letter and its ellipsis.
                        .min_w_8()
                        .truncate()
                        .text_size(px(DATELINE_TITLE_PX))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::bright(t))
                        .child(title)
                        .test_support(),
                )
                .child(
                    div()
                        .id(dateline_rule_id(m))
                        .flex_1()
                        .h(px(1.))
                        .bg(theme::divider(t))
                        .test_support(),
                )
                .children(
                    self.render_meta(m, &member.path, pieces, t)
                        .into_iter()
                        .flatten(),
                )
                .test_support()
                .into_any_element(),
        )
    }

    /// Member `m`'s dateline meta text ([`dateline_meta`]), as two items of
    /// the row: the folder, which gives up its room first and is cut with
    /// an ellipsis, and the count, which stays whole and carries the whole
    /// text for screen readers. Two row items, not one box: a box would
    /// never shrink below its folder's full width.
    fn render_meta(
        &self,
        m: usize,
        path: &VaultPath,
        pieces: usize,
        t: &Theme,
    ) -> [Option<AnyElement>; 2] {
        let meta = SharedString::from(dateline_meta(path, pieces));
        let count = piece_count(pieces);
        let (folder, count) = match note_folder(path) {
            Some(folder) => (Some(folder), format!("\u{b7} {count}")),
            None => (None, count),
        };
        let muted = theme::muted(t);
        let folder = folder.map(|folder| {
            let mut el = div()
                .id(dateline_folder_id(m))
                .min_w_0()
                // A space's width to the dot, not the row's gap.
                .mr(px(-4.))
                .truncate()
                .text_xs()
                .text_color(muted)
                .child(folder)
                .test_support();
            el.style().flex_shrink = Some(META_FOLDER_SHRINK);
            el.into_any_element()
        });
        let count = div()
            .id(dateline_meta_id(m))
            .role(Role::Label)
            .aria_label(meta)
            .flex_none()
            .whitespace_nowrap()
            .text_xs()
            .text_color(muted)
            .child(count)
            .test_support()
            .into_any_element();
        [folder, Some(count)]
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
