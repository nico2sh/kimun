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
//! no button. It stands above the note's first piece and that piece's up
//! row, so the hidden count of the note's first gap reads as the note's own; the note above keeps its last gap above
//! the dateline. Every dateline but the column's first row has 24 px above
//! it. A piece's breadcrumb row holds only its heading crumbs and `Open
//! note`. A member's first and last gap are its own: between two notes a
//! gap row of each may stand next to each other.
//! The title is drawn at the pieces' size, semibold, in the text colour.
//! With no member the view says "No notes match this query." with a
//! `Refresh` button.
//!
//! **The view title.** The row at the top of the view says what it shows:
//! a small muted `Collection` and, under it, the query ([`mod@super::heading`]).
//!
//! **The collection line.** One muted line under the bar, always there in
//! a collection and never in a single-note view. It holds the note
//! numbers: `23 notes` with no picks, `12 pieces in 5 of 23 notes` with
//! picks. Then, joined by ` · `, `first 500 of 600` when the query hit the
//! loader's cap, and the list's age, `list from 10:42` (the local time its
//! load completed, 24-hour). When the cap cuts the column, the `Show more`
//! row says how many pieces show. While a load runs it says `Loading…`
//! over the old content. For a few seconds after a refresh it
//! says what changed: `Refreshed: 2 notes added, 1 removed`, or
//! `Refreshed: no changes`, and ` · 1 pick dropped` when the new list
//! dropped picks. A pick, an unpick, a clear or an undo ends that report
//! at once. A screen reader hears the line as a status only while it
//! reports a load or a refresh.

use std::time::Duration;

use chrono::NaiveTime;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Role, SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, MAX_RANGES};
use kimun_notes::logic::query::COLLECTION_NOTE_CAP;
use kimun_notes::settings::themes::Theme;

pub(crate) use super::collection_text::{dateline_meta, note_folder, piece_count, refresh_text};
use super::collection_text::{local_now, note_count};
use super::heading::DATELINE_LEVEL;
use super::keys::Stop;
use super::members::{GapRef, MemberInit, Members};
use super::rows;
use super::timed::Timed;
use super::{
    FacetEvent, FacetView, REFRESH_ID, dateline_folder_id, dateline_id, dateline_meta_id,
    dateline_rule_id, dateline_title_id,
};
use crate::workspace::facet_ui::text_button;
use crate::{editor_font, theme};

/// How much faster a dateline's folder shrinks than its title.
const META_FOLDER_SHRINK: f32 = 1000.;

/// The size of a dateline's title: the size the pieces under it are drawn
/// at (the editor's font size), never smaller. The title stays quiet by the
/// room around it, not by its size.
pub(crate) fn dateline_title_size(cx: &App) -> Pixels {
    editor_font::font_size(cx)
}

/// What the collection line says while a load runs.
const LOADING: &str = "Loading\u{2026}";

/// How long the collection line reports a refresh.
const REFRESH_SHOWN: Duration = Duration::from_secs(6);

/// What tells a collection the local time a load completed.
pub(crate) type Clock = fn() -> NaiveTime;

/// What a collection view keeps beyond its members.
pub(super) struct Collection {
    /// The resolved query the notes came from.
    query: String,
    /// How many notes the query matched, before the loader's cap.
    total: usize,
    /// The last load hit the loader's cap ([`COLLECTION_NOTE_CAP`]): the
    /// line says "first N of M" only then. A note that could not be read
    /// also makes `total` exceed the members, but is no cap.
    capped: bool,
    /// The local time the last load completed (the view opened, or a
    /// refresh landed).
    loaded_at: NaiveTime,
    /// Where `loaded_at` comes from: the local time, or a test's clock.
    clock: Clock,
    /// A load for the view is under way: the line says `Loading…`.
    loading: bool,
    /// What the line says just after a refresh. A new load or refresh
    /// drops it.
    refreshed: Option<String>,
    /// The timer that ends `refreshed` after [`REFRESH_SHOWN`].
    report: Timed,
}

impl Collection {
    /// End the report of the last refresh and its timer.
    fn end_report(&mut self) {
        self.refreshed = None;
        self.report.cancel();
    }
}

impl FacetView {
    /// A collection view of `members` (list order kept), the resolved
    /// `query`, the `total` hits (for "first 500 of N") and the picks
    /// (pruned against the merged set). The list's age is the local time
    /// now: the view opens when its load completes.
    pub fn new_collection(
        query: String,
        members: Vec<MemberInit>,
        total: usize,
        picks: Vec<Facet>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_collection_at(query, members, total, picks, local_now, cx)
    }

    /// [`Self::new_collection`] with the times taken from `clock`.
    pub(crate) fn new_collection_at(
        query: String,
        members: Vec<MemberInit>,
        total: usize,
        picks: Vec<Facet>,
        clock: Clock,
        cx: &mut Context<Self>,
    ) -> Self {
        let collection = Collection {
            query,
            total,
            capped: total > COLLECTION_NOTE_CAP,
            loaded_at: clock(),
            clock,
            loading: false,
            refreshed: None,
            report: Timed::default(),
        };
        Self::build(Members::from_inits(members), Some(collection), picks, cx)
    }

    /// Whether a load for the view is under way. While one is, the
    /// collection line says `Loading…` over the old content. A new load
    /// ends the report of the last refresh. Nothing for a single-note view.
    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        let Some(collection) = &mut self.collection else {
            return;
        };
        if collection.loading == loading {
            return;
        }
        collection.loading = loading;
        if loading {
            collection.end_report();
        }
        cx.notify();
    }

    /// End the report of the last refresh: a pick, an unpick, a clear or
    /// an undo changes what the collection line has to say.
    pub(super) fn end_refresh_report(&mut self) {
        if let Some(collection) = &mut self.collection {
            collection.end_report();
        }
    }

    /// Whether the timer of a refresh report runs (tests).
    #[cfg(test)]
    pub(super) fn refresh_report_timer(&self) -> bool {
        self.collection
            .as_ref()
            .is_some_and(|c| c.report.is_active())
    }

    /// Whether the line says `Loading…` now (tests).
    #[cfg(test)]
    pub(crate) fn is_loading(&self) -> bool {
        self.collection.as_ref().is_some_and(|c| c.loading)
    }

    /// The resolved query of a collection; `None` for a single-note view.
    pub fn query(&self) -> Option<&str> {
        self.collection.as_ref().map(|c| c.query.as_str())
    }

    pub fn is_collection(&self) -> bool {
        self.collection.is_some()
    }

    /// Replace the member list (a manual refresh): prune picks, reset
    /// reveals and the `Show more` cap. The list's age is now. For
    /// [`REFRESH_SHOWN`] the collection line reports the notes added and
    /// removed and the picks dropped. Nothing for a single-note view.
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
        collection.loaded_at = (collection.clock)();
        let old = std::mem::replace(&mut self.members, Members::from_inits(members));
        let added = self
            .members
            .list
            .iter()
            .filter(|m| old.position(&m.path).is_none())
            .count();
        let removed = old
            .list
            .iter()
            .filter(|m| self.members.position(&m.path).is_none())
            .count();
        let picks_before = self.bar.picks().len();
        self.limit = MAX_RANGES;
        self.members_changed(cx);
        let dropped = picks_before - self.bar.picks().len();
        let text = refresh_text(added, removed, dropped);
        if let Some(collection) = &mut self.collection {
            collection.refreshed = Some(text);
            // The collection line is outside the bar, so the end of the
            // report changes no bar shape and needs no bar mark.
            collection.report.start(REFRESH_SHOWN, cx, |this, _| {
                if let Some(collection) = &mut this.collection {
                    collection.end_report();
                }
            });
        }
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
        title_size: Pixels,
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
                .aria_level(DATELINE_LEVEL)
                .aria_label(title.clone())
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .when(spaced, |row| row.mt(rows::dateline_margin()))
                // The title starts at the column's edge, left of the spine
                // the piece text and the crumbs share ([`rows::spine`]). The
                // outdent is on purpose: it marks where a note begins.
                .child(
                    div()
                        .id(dateline_title_id(m))
                        // A shrunk title keeps a letter and its ellipsis.
                        .min_w_8()
                        .truncate()
                        .text_size(title_size)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::foreground(t))
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
                // About a space's width to the dot, not the row's gap. A rem
                // value, so it follows the font size and the zoom.
                .mr(-rows::meta_join())
                .truncate()
                .text_size(rows::small_text())
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
            .text_size(rows::small_text())
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

    /// Whether the collection line reports something new: a load that runs
    /// or the end of a refresh. Only then is it a status a screen reader
    /// says ([`Self::render_collection_line`]).
    pub(super) fn collection_line_reports(&self) -> bool {
        self.collection
            .as_ref()
            .is_some_and(|c| c.loading || c.refreshed.is_some())
    }

    /// What the collection line says (see the module docs); `None` for a
    /// single-note view.
    pub(super) fn collection_line(&self) -> Option<String> {
        let collection = self.collection.as_ref()?;
        if collection.loading {
            return Some(LOADING.into());
        }
        if let Some(text) = &collection.refreshed {
            return Some(text.clone());
        }
        let held = self.members.list.len();
        let drawn = self.members.len();
        let cut = self.members.cut;
        let mut parts = Vec::new();
        parts.push(match self.bar.picks().is_empty() {
            true => note_count(held),
            false => format!(
                "{} in {} of {}",
                piece_count(drawn + cut),
                self.members.notes_shown(),
                note_count(held)
            ),
        });
        if collection.capped && collection.total > held {
            parts.push(format!("first {held} of {}", collection.total));
        }
        parts.push(format!(
            "list from {}",
            collection.loaded_at.format("%H:%M")
        ));
        Some(parts.join(" \u{b7} "))
    }
}
