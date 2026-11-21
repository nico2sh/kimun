//! The **Facet view** (ADR-0061): a read-only view of one note narrowed to
//! the **facet scope** of the picked facets (CONTEXT.md, **Facet view**).
//!
//! **Layout.** A pinned bar over a scrolling column. The bar never scrolls
//! away and takes at most 40% of the view's height, scrolling inside itself
//! past that. Its first two rows, the kind chips and the picked pills, are
//! as [`mod@line`] describes. The summary line with `Clear` comes
//! last, while there are picks. Just after a clear it says `Picks cleared`
//! and offers `Undo` instead, for a few seconds or until the next pick. A
//! kind chip opens the list of that kind's
//! facets. The search chip and `/` open the list of every facet, by kind.
//! A list floats under its chip ([`super::facet_list`]) as one row per
//! facet, picked rows first with a tick, each with the pieces it gives in
//! the current view. It stays open while picks are made in it, even once
//! every row is ticked. It closes on Escape or a press outside, or when new
//! text leaves no facet at all in its scope. The bar
//! keeps clear of buttons its host floats over the view's top-right corner
//! ([`FacetView::set_corner_room`]).
//!
//! Under the bar a collection draws its collection line ([`mod@collection`]),
//! then the first-use hint shows until the first pick: what picks do, and
//! that `/` finds, `j`/`k` step and a click on a tag or link opens it.
//!
//! The column holds the shown chunks with a gap row between them, then
//! `Show N more` with `100 of 340 pieces shown` beside it when more chunks
//! match than the view draws. Each chunk is
//! a read-only [`EditorView`] sized to its wrapped text, measured in the frame that draws it, with no frame, under a
//! slim row: the headings that contain it as clickable crumbs, outer ones
//! shortened first with an ellipsis when the row is narrow, and `Open
//! note`. A gap row counts the hidden blocks (`3 blocks hidden`) and
//! reveals one at a time next to the chunk above (`Show next block`) or
//! below (`Show previous block`), or all of them with `Show all` when few
//! are hidden.
//!
//! **Colour.** Buttons (chips, `Clear`, `Undo`, `Show all`, `Show more`,
//! crumbs, `Open note`) have their labels in the text colour. Meta text
//! (counts, the hint, the summary, dateline meta, the collection line) is
//! muted ([`crate::theme::muted`], 4.5:1 against the page, less on a theme with
//! faint text so it stays quieter than the text).
//!
//! **Picking.** A row in a list toggles its facet, a picked pill unpicks
//! it, and a breadcrumb button toggles its heading. A press in a piece never
//! changes the filter: the piece is read-only text (a caret, a drag or a
//! double press on text selects, the copy chord copies). A click on a link
//! or tag there asks the host to follow it at the release. A drag that
//! moves more than a few pixels or leaves the target selects instead.
//! Ctrl+click (Cmd+click on macOS) follows at the press. The pointer is
//! the hand over every link and tag. `Clear` and Escape put the picks they
//! remove on a small undo stack, and Ctrl+Z (Cmd+Z) or `Undo` brings the
//! last set back, less the facets the notes lost since.
//!
//! **Keyboard.** The tab stops and the keys are as [`mod@keys`] describes.
//!
//! **Content.** The view shows a list of member notes ([`mod@members`]); a
//! single-note view is a list of one. Core plans everything: the merged
//! facets, the selection per member, one cap over the members' chunks, and
//! reveals inside a member. A change of picks or text plans again from the
//! picks (revealed blocks reset); new text also drops picks the notes no
//! longer hold. Each piece answers for its own note: a follow resolves links
//! against it, `Open note` and a follow name it. The view takes plain
//! data in and emits [`FacetEvent`]s; it knows no workspace.
//!
//! **Collections** ([`mod@collection`]): a view of the notes one query
//! returned. Each note starts with a dateline (its title, a hairline, its
//! folder and drawn piece count), so a piece says which note it comes from.
//! The summary keeps the picks in words and the pieces; the note numbers
//! are on the collection line.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{Context, Entity, EventEmitter, FocusHandle, Rems, ScrollHandle, Subscription};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, FacetKind, MAX_RANGES};
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::facet_bar::FacetBar;

use self::bar_motion::BarMotion;
use self::collection::Collection;
use self::keys::{Stop, Stops};
use self::members::{Member, Members};
use self::more::More;
use self::undo::Undo;
use crate::editor_view::EditorView;

mod bar_motion;
mod body;
mod chunks;
mod collection;
mod collection_text;
mod ids;
mod keys;
mod line;
#[cfg(test)]
mod list_kind_tests;
mod members;
mod more;
mod render;
mod reveal;
pub(super) mod rows;
#[cfg(test)]
mod test_seams;
mod timed;
#[cfg(test)]
mod timed_tests;
mod undo;
pub(super) mod window;

pub(crate) use self::ids::*;
pub(crate) use self::keys::init as init_keys;
#[cfg(test)]
pub(crate) use self::line::LIST_CAP;
pub use self::members::MemberInit;

/// What the view asks of its host.
#[derive(Debug, Clone, PartialEq)]
pub enum FacetEvent {
    /// Jump to `note`, the piece's own note, at the piece's first source
    /// line (0-based).
    OpenNote { note: VaultPath, line: usize },
    /// A click on a link or tag in a piece of `from`, the note a relative
    /// link resolves against.
    Follow {
        from: VaultPath,
        target: FollowTarget,
    },
    /// The picks changed; the workspace saves the session.
    PicksChanged,
    /// A collection asks for its query to run again (`r`, `Refresh`).
    Refresh,
}

/// Which list is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ListKind {
    /// The tags: the `Tags N` chip.
    Tags,
    /// The note links: the `Links N` chip.
    Links,
    /// The headings: the `Sections N` chip.
    Sections,
    /// Every facet, by kind: the search chip and `/`.
    All,
}

impl ListKind {
    /// The kind lists in chip order. Row 1 draws their chips in this order.
    pub(crate) fn chip_order() -> [ListKind; 3] {
        [ListKind::Tags, ListKind::Links, ListKind::Sections]
    }

    /// The name of the list. The chip, the list and the group label of
    /// the all-kinds list use it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            ListKind::Tags => "Tags",
            ListKind::Links => "Links",
            ListKind::Sections => "Sections",
            ListKind::All => "Find",
        }
    }

    /// The facet kinds the list holds. The all-kinds list holds every kind.
    pub(crate) fn facet_kinds(self) -> &'static [FacetKind] {
        match self {
            ListKind::Tags => &[FacetKind::Tag],
            ListKind::Links => &[FacetKind::NoteLink],
            ListKind::Sections => &[FacetKind::Heading],
            ListKind::All => &[FacetKind::Tag, FacetKind::NoteLink, FacetKind::Heading],
        }
    }

    /// The kind list that holds facets of `kind`.
    pub(crate) fn of(kind: FacetKind) -> ListKind {
        Self::chip_order()
            .into_iter()
            .find(|list| list.facet_kinds().contains(&kind))
            .expect("every facet kind has a chip list")
    }
}

const HINT_TEXT: &str = "Pick a tag, link or section to narrow this note. \
    / finds, j and k step, a click on a tag or link opens it.";
const HINT_TEXT_NOTES: &str = "Pick a tag, link or section to narrow these notes. \
    / finds, j and k step, a click on a tag or link opens it.";

/// The facet view. See the module docs.
pub struct FacetView {
    /// The notes shown, their merged facets and the plan of drawn chunks.
    members: Members,
    /// The query and hit count of a collection; `None` for a single note.
    collection: Option<Collection>,
    bar: FacetBar,
    /// The most chunks drawn over all members; `Show N more` doubles it.
    limit: usize,
    /// One read-only editor per drawn chunk, reused in place.
    chunks: Vec<Entity<EditorView>>,
    _chunk_events: Vec<Subscription>,
    more: Option<More>,
    focus: FocusHandle,
    pub(crate) scroll: ScrollHandle,
    /// The bar's scroll inside its height cap.
    pub(crate) bar_scroll: ScrollHandle,
    /// The bar's height as it eases after a change ([`mod@bar_motion`]).
    bar_motion: BarMotion,
    /// The view's tab stops ([`mod@keys`]).
    stops: RefCell<Stops>,
    /// A pick was made in this view (or it began with picks): the hint under
    /// the bar is gone for good. Not saved to the session.
    hint_done: bool,
    /// Room the bar keeps clear at its right end for what its host floats
    /// over the view's top-right corner ([`Self::set_corner_room`]).
    corner_room: Option<Rems>,
    /// The pick sets that clears removed, and the timed `Undo` offer
    /// ([`Undo`]). Not saved to the session.
    undo: Undo,
    /// The stop the next frame gives the keyboard to, once drawn: `Undo`
    /// after Enter or Space on `Clear`, a column's first or last stop after
    /// Tab into the column.
    focus_after: Option<Stop>,
    /// The `focus_after` stop that already waited one frame to be drawn,
    /// and where the keyboard was then.
    focus_retry: Option<(Stop, Option<FocusHandle>)>,
    /// A column stop that arrived while the column had no height: the
    /// next frame with a height reveals it.
    held_reveal: RefCell<Option<Stop>>,
    /// The focus handle of a list's field that [`Self::plan`] closed: the
    /// next frame gives the keyboard to the view if the field still had it.
    stranded: Option<FocusHandle>,
    /// The rows of each piece at the layout width ([`mod@window`]).
    heights: Rc<RefCell<window::Heights>>,
    /// The pieces near the viewport the last frame drew; spacers stood in
    /// for the rest, pinned pieces aside ([`mod@body`]) (tests).
    #[cfg(test)]
    drawn: std::ops::Range<usize>,
    /// Draw every piece, as before the window existed (tests).
    #[cfg(test)]
    draw_all: bool,
    /// A debug run warned of the table mismatch that goes on now.
    #[cfg(not(test))]
    warned: Rc<std::cell::Cell<bool>>,
    /// How many drawn pieces the last frame checked against the table
    /// (tests).
    #[cfg(test)]
    checked: Rc<std::cell::Cell<usize>>,
    /// How many piece editors were asked for the keyboard (tests).
    #[cfg(test)]
    focus_scans: std::cell::Cell<usize>,
    /// Added to the crumbs row the table counts, to forge a wrong table
    /// (tests).
    #[cfg(test)]
    forged: gpui_kit::Pixels,
    /// How many times [`Self::plan`] ran (tests).
    #[cfg(test)]
    plans_run: usize,
    /// The chip the last frame drew in the selected style (tests).
    #[cfg(test)]
    open_chip: std::cell::Cell<Option<ListKind>>,
    /// The chips the last frame drew dimmed (tests).
    #[cfg(test)]
    dimmed_chips: RefCell<Vec<ListKind>>,
}

impl EventEmitter<FacetEvent> for FacetView {}

impl FacetView {
    /// A view of `text`, the note at `source`, with `picks` (those the note
    /// does not hold are dropped: a restore can name facets the note lost).
    pub fn new(source: VaultPath, text: String, picks: Vec<Facet>, cx: &mut Context<Self>) -> Self {
        let title = source.get_clean_name();
        let members = Members::new(vec![Member::new(source, title, text)]);
        Self::build(members, None, picks, cx)
    }

    /// A view of `members` (a single note, or a collection), with `picks`
    /// pruned against their merged facets.
    fn build(
        members: Members,
        collection: Option<Collection>,
        picks: Vec<Facet>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut bar = FacetBar::default();
        bar.set_picks(picks);
        bar.prune(&members.merged);
        let hint_done = !bar.picks().is_empty();
        let mut view = Self {
            members,
            collection,
            bar,
            limit: MAX_RANGES,
            chunks: Vec::new(),
            _chunk_events: Vec::new(),
            more: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            bar_scroll: ScrollHandle::new(),
            bar_motion: BarMotion::default(),
            stops: RefCell::default(),
            hint_done,
            corner_room: None,
            undo: Undo::default(),
            focus_after: None,
            focus_retry: None,
            held_reveal: RefCell::default(),
            stranded: None,
            heights: Rc::default(),
            #[cfg(test)]
            drawn: 0..0,
            #[cfg(test)]
            draw_all: false,
            #[cfg(not(test))]
            warned: Rc::default(),
            #[cfg(test)]
            checked: Rc::default(),
            #[cfg(test)]
            focus_scans: Default::default(),
            #[cfg(test)]
            forged: gpui_kit::px(0.),
            #[cfg(test)]
            plans_run: 0,
            #[cfg(test)]
            open_chip: Default::default(),
            #[cfg(test)]
            dimmed_chips: RefCell::default(),
        };
        view.select(cx);
        view
    }

    /// The note of a single-note view (its one member); `None` for a
    /// collection.
    pub fn source(&self) -> Option<&VaultPath> {
        self.single().map(|member| &member.path)
    }

    /// The one member of a single-note view; `None` for a collection.
    fn single(&self) -> Option<&Member> {
        self.collection
            .is_none()
            .then(|| self.members.list.first())
            .flatten()
    }

    pub fn picks(&self) -> &[Facet] {
        self.bar.picks()
    }

    /// New content of a single-note view: re-derive, prune picks, reset
    /// reveals. Nothing for a collection ([`Self::set_member_text`]).
    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        if self.single().is_some() {
            self.member_text(0, text, cx);
        }
    }

    /// Member `idx` has new text: only it re-derives; picks are pruned
    /// against the merged facets, reveals reset.
    fn member_text(&mut self, idx: usize, text: String, cx: &mut Context<Self>) {
        self.members.set_text(idx, text);
        self.members_changed(cx);
    }

    /// The members changed (text, list): prune the picks, select again
    /// (reveals reset), and say so when a pick went.
    fn members_changed(&mut self, cx: &mut Context<Self>) {
        let pruned = self.bar.prune(&self.members.merged);
        self.select(cx);
        // New text can add or drop rows: an open list sorts again.
        self.sort_list(cx);
        if pruned {
            cx.emit(FacetEvent::PicksChanged);
        }
        cx.notify();
    }

    /// The source note of a single-note view moved or was renamed. Its
    /// facets are derived again at once, so relative note links resolve
    /// against the new place; the shown pieces are kept until the next
    /// [`Self::set_text`]. Nothing for a collection
    /// ([`Self::set_member_path`]).
    pub fn set_source(&mut self, source: VaultPath) {
        if self.single().is_some() {
            self.members.set_path(0, source);
        }
    }

    /// Keep `room` clear at the bar's right end (`None`: no room), for
    /// buttons the host floats over the view's top-right corner. The host
    /// calls it each time it draws the view.
    pub(crate) fn set_corner_room(&mut self, room: Option<Rems>, cx: &mut Context<Self>) {
        if self.corner_room != room {
            self.corner_room = room;
            cx.notify();
        }
    }

    /// A click on `target` in chunk `i`: ask the host to follow it,
    /// from the chunk's own note (a relative link resolves against it).
    fn follow(&mut self, i: usize, target: FollowTarget, cx: &mut Context<Self>) {
        if let Some(note) = self.members.member_of(i).map(|m| m.path.clone()) {
            cx.emit(FacetEvent::Follow { from: note, target });
        }
    }

    /// Pick `facet`, or unpick it. Nothing for a facet the note does not hold.
    pub(super) fn toggle(&mut self, facet: Facet, cx: &mut Context<Self>) {
        if !self.members.merged.entries.iter().any(|e| e.facet == facet) {
            return;
        }
        self.bar.toggle(facet);
        self.after_pick(cx);
    }

    /// The tail of a pick change: the hint goes for good, the `Undo` offer
    /// ends, the chunks are selected again and the host hears of it.
    fn after_pick(&mut self, cx: &mut Context<Self>) {
        self.hint_done = true;
        self.undo.hide();
        self.end_refresh_report();
        self.select(cx);
        cx.emit(FacetEvent::PicksChanged);
    }

    /// Unpick the latest pick: what its pill does. The empty message's button.
    pub(super) fn remove_last_pick(&mut self, cx: &mut Context<Self>) {
        if let Some(facet) = self.bar.last_pick().cloned() {
            self.toggle(facet, cx);
        }
    }

    /// The message that stands in for the pieces when none are drawn.
    pub(crate) fn empty_text(&self) -> String {
        if self.collection.is_some() && self.members.list.is_empty() {
            return "No notes match this query.".into();
        }
        if self.bar.picks().is_empty() {
            return match self.collection {
                Some(_) => "These notes are empty.".into(),
                None => "This note is empty.".into(),
            };
        }
        self.bar
            .empty_reason(&self.members.merged, self.members.len() > 0)
            .unwrap_or_else(|| "Nothing matches the picks.".into())
    }

    /// Whether the hint under the bar shows: no pick made yet in this view,
    /// and the note has something to pick.
    fn hint_shows(&self) -> bool {
        !self.hint_done && self.bar.picks().is_empty() && !self.members.merged.entries.is_empty()
    }

    /// What the picks show, while they show something: the picks in words
    /// and the pieces, counting those the cap leaves out. The note numbers
    /// of a collection are on its collection line, not here.
    fn summary(&self) -> Option<String> {
        let drawn = self.members.len();
        if drawn == 0 {
            return None;
        }
        self.bar
            .summary(&self.members.merged, drawn + self.members.cut, None)
    }

    /// Chunk `i`'s note and its first source line there, 0-based.
    fn chunk_source(&self, i: usize) -> Option<(VaultPath, usize)> {
        let member = self.members.member_of(i)?;
        Some((member.path.clone(), self.members.first_line(i)))
    }
}
