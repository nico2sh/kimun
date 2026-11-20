//! The **Facet view** (ADR-0061): a read-only view of one note narrowed to
//! the **facet scope** of the picked facets (CONTEXT.md, **Facet view**).
//!
//! **Layout.** A pinned bar over a scrolling column. The bar never scrolls
//! away and takes at most 40% of the view's height, scrolling inside itself
//! past that: the filter line ([`mod@line`]), then a summary line with
//! `Clear` while there are picks. The filter line is one row that never
//! wraps: the picked pills first (each with a remove mark), then the tags
//! (`#name`) and note links (`[[name]]`) that fit, ranked by count, a `+N`
//! chip for those left out and a `Sections N` chip for the headings. `+N`
//! (and `/`) opens the list of every unpicked tag and link, `Sections N`
//! the list of headings (`/` too, when no tag or link is left unpicked). A
//! list floats under what opened it ([`super::facet_list`]), stays open
//! while picks are made in it, and closes on Escape or a press outside, or
//! when new text or a pick takes away all it holds. The bar keeps clear of
//! buttons its host floats over the view's top-right corner ([`FacetView::set_corner_room`]).
//!
//! The column holds the shown chunks with a gap row between them, then
//! `Show N more` when more chunks match than the view draws. Each chunk is
//! a read-only [`EditorView`] sized to its lines, with no frame, under a
//! slim row: the headings that contain it as clickable crumbs, outer ones
//! shortened first with an ellipsis when the row is narrow, and `Open
//! source`. A gap row counts the hidden blocks (`3 hidden`) and reveals one
//! at a time next to the chunk above (down) or below (up), or all of them
//! with `Show all` when few are hidden.
//!
//! **Picking.** A pill toggles its facet, as do the `+N` and `Sections N`
//! lists and the breadcrumb buttons. A press in a piece never changes the
//! filter, with or without a modifier: the piece is read-only text (a caret,
//! a drag or a double press selects, the copy chord copies). A primary press
//! (Ctrl, or Cmd on macOS) on a link or tag only asks to follow it.
//!
//! **Keyboard** ([`mod@keys`]): every chip, pill and text button is a tab stop
//! with a focus ring, and Enter or Space does what a click does. `/` opens
//! the list of every tag and link, `j`/`k` step between pieces, Escape clears the picks (from
//! inside a piece it first hands the keyboard to the view).
//!
//! **Content.** The view shows a list of member notes ([`mod@members`]); a
//! single-note view is a list of one. Core plans everything: the merged
//! facets, the selection per member, one cap over the members' chunks, and
//! reveals inside a member. A change of picks or text plans again from the
//! picks (revealed blocks reset); new text also drops picks the notes no
//! longer hold. Each piece answers for its own note: a follow resolves links
//! against it, `Open source` and a follow name it. The view takes plain
//! data in and emits [`FacetEvent`]s; it knows no workspace.
//!
//! **Collections** ([`mod@collection`]): a view of the notes one query
//! returned. Each note starts with a dateline (its title, a hairline, its
//! folder and drawn piece count), so a piece says which note it comes from.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, ParentElement as _, Rems, Render, Role,
    ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, canvas, div, px, relative,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{Facet, FacetKind, GAP_SHOW_ALL_MAX, MAX_RANGES, Reveal};
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::facet_bar::FacetBar;
use kimun_notes::settings::themes::Theme;

use self::collection::Collection;
use self::keys::{Stop, Stops, keys};
use self::line::PillWidths;
use self::members::{Member, Members, chunk_plan};
use super::facet_ui::{chip, crumb as crumb_button, icon_chip, text_button};
use super::query_field::focus_input;
use crate::editor_element::LINE_HEIGHT;
use crate::editor_font;
use crate::editor_view::{EditorEvent, EditorView};
use crate::theme;

mod collection;
mod keys;
mod line;
mod members;

#[cfg(test)]
pub(crate) use self::collection::{
    dateline_folder_id, dateline_id, dateline_meta_id, dateline_rule_id, dateline_title_id,
};
pub use self::members::MemberInit;

/// Element ids, for tests.
pub(crate) const FACET_VIEW_ID: &str = "facet-view";
pub(crate) const SHOW_MORE_ID: &str = "facet-show-more";
pub(crate) const MORE_QUERY_ID: &str = "facet-more-query";
/// The open list's frame (the popover under its chip).
pub(crate) const MORE_LIST_ID: &str = "facet-more-list";
pub(crate) const FACET_BAR_ID: &str = "facet-bar";
pub(crate) const SUMMARY_ID: &str = "facet-summary";
/// The message that stands where the pieces would be: the note is empty, or
/// the picks select nothing.
pub(crate) const EMPTY_ID: &str = "facet-empty";
/// The `Remove last pick` button of the empty message.
pub(crate) const REMOVE_LAST_ID: &str = "facet-remove-last";
/// The one-line hint under the bar, until the first pick.
pub(crate) const HINT_ID: &str = "facet-hint";
pub(crate) const CLEAR_ID: &str = "facet-clear";
/// The `Sections N` chip: opens the headings list.
pub(crate) const SECTIONS_ID: &str = "facet-sections";
/// The filter line: the pills that fit, `+N` and `Sections N`.
pub(crate) const LINE_ID: &str = "facet-line";
/// The `+N` chip, while the line leaves facets out: opens the list of every
/// tag and link.
pub(crate) const MORE_ID: &str = "facet-more";
/// The filter line's clipped container of pills.
pub(crate) const LINE_PILLS_ID: &str = "facet-line-pills";
/// The scrolling column of pieces.
pub(crate) const COLUMN_ID: &str = "facet-column";
/// A collection's `Refresh` text button, in its empty state.
pub(crate) const REFRESH_ID: &str = "facet-refresh";
/// A collection's "Showing the first N of M notes" line under the bar.
pub(crate) const FIRST_NOTES_ID: &str = "facet-first-notes";

fn kind_slug(kind: FacetKind) -> &'static str {
    match kind {
        FacetKind::Heading => "heading",
        FacetKind::Tag => "tag",
        FacetKind::NoteLink => "link",
    }
}

/// The pill of `facet` in the filter line.
pub(crate) fn pill_id(facet: &Facet) -> SharedString {
    format!("facet-pill:{}:{}", kind_slug(facet.kind), facet.key).into()
}

/// The remove mark on the picked pill of `facet` in the filter line.
#[cfg(test)]
pub(crate) fn remove_mark_id(facet: &Facet) -> SharedString {
    super::facet_ui::remove_mark(&pill_id(facet))
}

/// The outline icon in front of the picked heading pill of `facet`.
#[cfg(test)]
pub(crate) fn outline_mark_id(facet: &Facet) -> SharedString {
    super::facet_ui::outline_mark(&pill_id(facet))
}

/// The pill of `facet` in an open list (the `+N` list, the `Sections N`
/// list).
pub(crate) fn more_item_id(facet: &Facet) -> SharedString {
    format!("facet-more-item:{}:{}", kind_slug(facet.kind), facet.key).into()
}

/// The label over `kind`'s group in the `+N` list (`Tags`, `Links`).
pub(crate) fn list_group_id(kind: FacetKind) -> SharedString {
    format!("facet-list-group:{}", kind_slug(kind)).into()
}

/// Chunk `i`.
pub(crate) fn chunk_id(i: usize) -> SharedString {
    format!("facet-chunk:{i}").into()
}

/// Chunk `i`'s `Open source` button.
pub(crate) fn open_id(i: usize) -> SharedString {
    format!("facet-open:{i}").into()
}

/// Gap `g`'s button that reveals the block below the chunk above it. In a
/// single-note view gap `g` sits before chunk `g`, and the last gap follows
/// the last chunk ([`mod@members`] numbers the gaps of several notes).
pub(crate) fn gap_down_id(g: usize) -> SharedString {
    format!("facet-gap-down:{g}").into()
}

/// Gap `g`'s button that reveals the block above the chunk below it.
pub(crate) fn gap_up_id(g: usize) -> SharedString {
    format!("facet-gap-up:{g}").into()
}

/// Gap `g`'s `Show all` button, while it hides [`GAP_SHOW_ALL_MAX`] blocks
/// or fewer.
pub(crate) fn show_all_id(g: usize) -> SharedString {
    format!("facet-show-all:{g}").into()
}

/// Gap `g`'s count of hidden blocks (`3 hidden`).
pub(crate) fn gap_count_id(g: usize) -> SharedString {
    format!("facet-gap-count:{g}").into()
}

/// Crumb `index` (outermost first) of chunk `chunk`'s breadcrumb.
pub(crate) fn crumb_id(chunk: usize, index: usize) -> SharedString {
    format!("facet-crumb:{chunk}:{index}").into()
}

/// What the view asks of its host.
#[derive(Debug, Clone, PartialEq)]
pub enum FacetEvent {
    /// Jump to `note`, the piece's own note, at the piece's first source
    /// line (0-based).
    OpenSource { note: VaultPath, line: usize },
    /// Primary+click on a link or tag in a piece of `from`, the note a
    /// relative link resolves against.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListKind {
    /// Every unpicked tag and link, by kind: the `+N` chip and `/`.
    Facets,
    /// The headings: the `Sections N` chip.
    Sections,
}

/// The open list: its kind, its fuzzy query field, and the result Enter
/// toggles (lit, an index into the list's pills in order; a new query
/// lights the first).
struct More {
    kind: ListKind,
    input: Entity<InputState>,
    highlight: usize,
    _events: Subscription,
}

const HINT_TEXT: &str = "Pick a tag, link or section to narrow this note.";
const HINT_TEXT_NOTES: &str = "Pick a tag, link or section to narrow these notes.";

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
    /// The visual row count each chunk was last sized for ([`Self::rows_of`]).
    /// A frame that paints other counts draws the view again.
    sized_rows: Vec<usize>,
    /// Per chunk: its editor has been painted since it got its text, so its
    /// layout describes that text. A new or swapped text is not laid out
    /// until the next frame's prepaint.
    laid_out: Vec<bool>,
    /// A correcting frame is asked for and has not run yet.
    frame_asked: bool,
    more: Option<More>,
    focus: FocusHandle,
    pub(crate) scroll: ScrollHandle,
    /// The view's tab stops ([`mod@keys`]).
    stops: RefCell<Stops>,
    /// Per drawn chunk, its index among the column's children (`j`/`k`).
    piece_items: Vec<usize>,
    /// A pick was made in this view (or it began with picks): the hint under
    /// the bar is gone for good. Not saved to the session.
    hint_done: bool,
    /// Room the bar keeps clear at its right end for what its host floats
    /// over the view's top-right corner ([`Self::set_corner_room`]).
    corner_room: Option<Rems>,
    /// The filter line's room for pills and `+N`, as the last frame laid it
    /// out ([`mod@line`]); `None` before the first layout.
    line_room: Rc<Cell<Option<f32>>>,
    /// The room the last render fitted the line to.
    room_used: Option<f32>,
    /// The measured pill widths ([`mod@line`]).
    widths: RefCell<PillWidths>,
    /// The facets the last frame's line left out (its `+N`).
    more_count: Cell<usize>,
    /// The primary modifier was held at the last modifiers change: a piece's
    /// links and tags show the pointing hand only while it is ([`mod@keys`]).
    primary_held: bool,
    /// The focus handle of a list's field that [`Self::plan`] closed: the
    /// next frame gives the keyboard to the view if the field still had it.
    stranded: Option<FocusHandle>,
    /// How many times [`Self::plan`] ran (tests).
    #[cfg(test)]
    plans_run: usize,
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
            sized_rows: Vec::new(),
            laid_out: Vec::new(),
            frame_asked: false,
            more: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            stops: RefCell::default(),
            piece_items: Vec::new(),
            hint_done,
            corner_room: None,
            line_room: Rc::default(),
            room_used: None,
            widths: RefCell::default(),
            more_count: Cell::new(0),
            primary_held: false,
            stranded: None,
            #[cfg(test)]
            plans_run: 0,
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

    /// The note text a single-note view shows; empty for a collection
    /// (tests).
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        self.single().map_or("", |member| &member.text)
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

    /// The chunks' editors, in order (tests).
    #[cfg(test)]
    pub(crate) fn chunks(&self) -> &[Entity<EditorView>] {
        &self.chunks
    }

    /// The open list's query field (tests).
    #[cfg(test)]
    pub(crate) fn more_input(&self) -> Option<Entity<InputState>> {
        self.more.as_ref().map(|more| more.input.clone())
    }

    /// The open list's results, in order (tests).
    #[cfg(test)]
    pub(crate) fn list_results(&self, cx: &App) -> Vec<Facet> {
        self.list_pills(cx).into_iter().map(|p| p.facet).collect()
    }

    /// How many facets the last frame's line left out: the `+N` (tests).
    #[cfg(test)]
    pub(crate) fn more_count(&self) -> usize {
        self.more_count.get()
    }

    /// The width measured for `facet`'s pill as the line draws it now
    /// (tests).
    #[cfg(test)]
    pub(crate) fn measured_width(&self, facet: &Facet) -> Option<f32> {
        let line = self.bar.line(&self.members.merged);
        let pill = line.pills.iter().find(|p| p.facet == *facet)?;
        self.widths.borrow().get(pill)
    }

    /// The element drawn with the focus ring in the last frame (tests).
    #[cfg(test)]
    pub(crate) fn ringed(&self) -> Option<SharedString> {
        self.stops.borrow().ringed.as_ref().map(Stop::id)
    }

    /// `Show N more`, while chunks are left out: N is what the next click
    /// shows, the limit or what is left when that is less.
    pub(crate) fn show_more_label(&self) -> Option<String> {
        let cut = self.members.cut;
        (cut > 0).then(|| format!("Show {} more", cut.min(self.limit)))
    }

    /// Show what the picks select, nothing revealed.
    fn select(&mut self, cx: &mut Context<Self>) {
        self.members.select(self.bar.picks());
        self.plan(cx);
    }

    /// Plan the chunks for every member's `shown` and bring the editors in
    /// step: one per drawn chunk, an editor whose text is unchanged left
    /// alone.
    fn plan(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        {
            self.plans_run += 1;
        }
        let (old_plans, old_refs) = self.members.plan(self.limit);
        let len = self.members.len();
        self.chunks.truncate(len);
        self._chunk_events.truncate(len);
        self.laid_out.truncate(len);
        for i in 0..len {
            let plan = self
                .members
                .chunk(i)
                .expect("every chunk below `len` has a plan");
            match self.chunks.get(i) {
                Some(editor) => {
                    let old = chunk_plan(&old_plans, &old_refs, i);
                    if old.is_none_or(|old| old.text != plan.text) {
                        editor.update(cx, |editor, cx| editor.set_text_read_only(&plan.text, cx));
                        self.laid_out[i] = false;
                    }
                }
                None => {
                    let editor = cx.new(|cx| EditorView::new_read_only(&plan.text, cx));
                    let events = cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| {
                        let EditorEvent::ReadOnlyFollow { target } = event else {
                            return;
                        };
                        if let Some(i) = this.chunks.iter().position(|c| *c == editor) {
                            this.follow(i, target.clone(), cx);
                        }
                    });
                    self.chunks.push(editor);
                    self._chunk_events.push(events);
                    self.laid_out.push(false);
                }
            }
        }
        // New text, or a pick of the last unpicked tag or link, can take
        // away everything the open list holds (every tag and link, or every
        // heading): it closes, and the next frame hands the keyboard from
        // its field to the view.
        let gone = self
            .more
            .as_ref()
            .is_some_and(|more| !self.list_drawable(more.kind));
        if gone && let Some(more) = self.more.take() {
            self.stranded = Some(more.input.read(cx).focus_handle(cx));
        }
        cx.notify();
    }

    /// Whether `kind`'s list has anything to hold: an unpicked tag or note
    /// link for the `+N` list (picked ones lead the line, not the list), a
    /// heading for the `Sections N` list.
    fn list_drawable(&self, kind: ListKind) -> bool {
        let merged = &self.members.merged;
        match kind {
            ListKind::Facets => !self.bar.grouped(merged, "").is_empty(),
            ListKind::Sections => merged
                .entries
                .iter()
                .any(|entry| entry.facet.kind == FacetKind::Heading),
        }
    }

    /// A primary press on `target` in chunk `i`: ask the host to follow it,
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
        self.hint_done = true;
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
            .unwrap_or_else(|| "Nothing matches the picked facets.".into())
    }

    /// Whether the hint under the bar shows: no pick made yet in this view,
    /// and the note has something to pick.
    fn hint_shows(&self) -> bool {
        !self.hint_done && self.bar.picks().is_empty() && !self.members.merged.entries.is_empty()
    }

    /// Unpick everything: the whole note again, reveals reset.
    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.bar.picks().is_empty() {
            return;
        }
        self.bar.clear();
        self.select(cx);
        cx.emit(FacetEvent::PicksChanged);
    }

    /// What the picks show, while they show something: the picks in words
    /// and the pieces, counting those the cap leaves out.
    fn summary(&self) -> Option<String> {
        let drawn = self.members.len();
        if drawn == 0 {
            return None;
        }
        let notes = self.collection.as_ref().map(|_| self.members.notes_shown());
        self.bar
            .summary(&self.members.merged, drawn + self.members.cut, notes)
    }

    /// Reveal one block hidden in gap `g`: next to the chunk above it
    /// ([`Reveal::Below`]) or below it ([`Reveal::Above`]), inside that
    /// chunk's note. Nothing when the gap has no chunk on that side.
    fn reveal(&mut self, g: usize, dir: Reveal, cx: &mut Context<Self>) {
        let gap = self.members.gaps.get(g);
        let chunk = match dir {
            Reveal::Below => gap.and_then(|gap| gap.above),
            Reveal::Above => gap.and_then(|gap| gap.below),
        };
        if let Some(chunk) = chunk {
            self.members.reveal(chunk, dir);
            self.plan(cx);
        }
    }

    /// Reveal every block hidden in gap `g`: `Show all`.
    fn reveal_all(&mut self, g: usize, cx: &mut Context<Self>) {
        self.members.reveal_gap(g);
        self.plan(cx);
    }

    fn show_more(&mut self, cx: &mut Context<Self>) {
        self.limit = self.limit.saturating_mul(2);
        self.plan(cx);
    }

    /// The visual rows chunk `i`'s editor lays its text out in: soft-wrapped
    /// at the width it was last drawn at. Until the editor has laid out its
    /// current text (a new chunk, new text), its rows as written: the layout
    /// still describes the old text, even when that had as many rows.
    pub(crate) fn rows_of(&self, i: usize, cx: &App) -> usize {
        let written = self
            .members
            .chunk(i)
            .map_or(1, |plan| plan.text.split('\n').count());
        let layout = &self.chunks[i].read(cx).editor.session().display.layout;
        if self.laid_out.get(i) == Some(&true) && layout.row_count() == written {
            layout.visual_line_count().max(1)
        } else {
            written.max(1)
        }
    }

    /// Whether a chunk's editor now lays out in other rows than its chunk
    /// was sized for (a resize, a new layout, the background layout of a
    /// large chunk landing).
    fn rows_changed(&self, cx: &App) -> bool {
        self.sized_rows.len() != self.members.len()
            || (0..self.members.len()).any(|i| self.rows_of(i, cx) != self.sized_rows[i])
    }

    /// After a paint (the editors laid out in its prepaint): every chunk's
    /// layout is current. True when a chunk was sized for other rows, or the
    /// filter line was fitted to another room than it got, and no correcting
    /// frame is asked for yet; the caller asks for one.
    fn painted(&mut self, cx: &App) -> bool {
        self.laid_out.iter_mut().for_each(|done| *done = true);
        let room_changed = self.line_room.get() != self.room_used;
        if self.frame_asked || !(self.rows_changed(cx) || room_changed) {
            return false;
        }
        self.frame_asked = true;
        true
    }

    /// The rows each chunk was sized for by the last render (tests).
    #[cfg(test)]
    pub(crate) fn sized_rows(&self) -> &[usize] {
        &self.sized_rows
    }

    /// How many times the chunks were planned (tests).
    #[cfg(test)]
    pub(crate) fn plans_run(&self) -> usize {
        self.plans_run
    }

    /// Whether the last paint asked for a correcting frame (tests).
    #[cfg(test)]
    pub(crate) fn resize_asked(&self) -> bool {
        self.frame_asked
    }

    /// Chunk `i`'s note and its first source line there, 0-based.
    fn chunk_source(&self, i: usize) -> Option<(VaultPath, usize)> {
        let member = self.members.member_of(i)?;
        Some((member.path.clone(), self.members.first_line(i)))
    }

    /// Open `kind`'s list, its filter field empty and holding the keyboard.
    /// It replaces a list already open.
    pub(crate) fn open_list(
        &mut self,
        kind: ListKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Nothing to list: no unpicked tag or link, no heading.
        if !self.list_drawable(kind) {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let events = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(more) = &mut this.more {
                    more.highlight = 0;
                }
                cx.notify();
            }
        });
        focus_input(&input, window, cx);
        self.more = Some(More {
            kind,
            input,
            highlight: 0,
            _events: events,
        });
        cx.notify();
    }

    /// Close the open list. The keyboard goes back to the chip that opens
    /// it (`+N`, `Sections N`), or to the view when that chip is not drawn
    /// (every pill fits the line: no `+N`, `/` opened it).
    pub(crate) fn close_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(more) = self.more.take() else {
            return;
        };
        let chip = match more.kind {
            ListKind::Facets => Stop::More,
            ListKind::Sections => Stop::Sections,
        };
        let handle = self.stops.borrow().handle(&chip);
        window.focus(&handle.unwrap_or_else(|| self.focus.clone()), cx);
        cx.notify();
    }

    /// The kit opened or closed `kind`'s popover (a press on its chip,
    /// Escape, a press outside). A close of a list no longer open (another
    /// chip's list replaced it) changes nothing.
    fn list_changed(
        &mut self,
        kind: ListKind,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = self.more.as_ref().is_some_and(|more| more.kind == kind);
        if open && !is_open {
            self.open_list(kind, window, cx);
        } else if !open && is_open {
            self.close_list(window, cx);
        }
    }

    /// A pick from the open list: toggle it. The list stays open, its filter
    /// kept and holding the keyboard, for the next pick.
    fn pick_more(&mut self, facet: Facet, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(facet, cx);
        if let Some(more) = &self.more {
            focus_input(&more.input, window, cx);
        }
        cx.notify();
    }

    // ---- Drawing -------------------------------------------------------------

    /// The pinned bar above the column: the filter line ([`mod@line`]), then
    /// the summary line. `None` when it would be empty (a note with no
    /// facet).
    fn render_bar(
        &self,
        window: &Window,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let summary = self.render_summary(t, cx);
        let line = self.bar.line(&self.members.merged);
        let line = (!line.pills.is_empty() || line.sections > 0)
            .then(|| self.render_line(line, window, t, cx));
        if summary.is_none() && line.is_none() {
            return None;
        }
        Some(
            div()
                .id(FACET_BAR_ID)
                .flex()
                .flex_col()
                .flex_none()
                .gap_1()
                .max_h(relative(0.4))
                .overflow_y_scroll()
                .px_4()
                .when_some(self.corner_room, |bar, room| bar.pr(room))
                .py_3()
                .border_b_1()
                .border_color(theme::divider(t))
                .children(line)
                .children(summary)
                .test_support(),
        )
    }

    /// `el`, a button that does what `stop` does, as that tab stop.
    fn button(
        &self,
        stop: Stop,
        el: Stateful<Div>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let press = stop.clone();
        let el = el
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate(press.clone(), window, cx)
            }))
            .test_support();
        self.track(stop, el, t, cx)
    }

    /// While there are picks: the summary text when they show something
    /// ("Showing text tagged #a · 3 pieces"), and `Clear` at the line's right
    /// end either way.
    fn render_summary(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        if self.bar.picks().is_empty() {
            return None;
        }
        let text = self.summary().map(|summary| {
            let summary = SharedString::from(summary);
            div()
                .id(SUMMARY_ID)
                .role(Role::Status)
                .aria_label(summary.clone())
                .text_xs()
                .text_color(theme::muted(t))
                .child(summary)
                .test_support()
        });
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w_0().children(text))
                .child(self.button(Stop::Clear, chip(CLEAR_ID.into(), "Clear", t), t, cx)),
        )
    }

    fn list_open(&self, kind: ListKind) -> bool {
        self.more.as_ref().is_some_and(|more| more.kind == kind)
    }

    /// Gap `g` ([`mod@members`]), or `None` when it hides nothing.
    fn render_gap(&self, g: usize, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let gap = *self.members.gaps.get(g)?;
        let hidden = self.members.gap_hidden(g);
        if hidden == 0 {
            return None;
        }
        let count = SharedString::from(format!("{hidden} hidden"));
        let show_all = (hidden <= GAP_SHOW_ALL_MAX).then(|| {
            self.button(
                Stop::ShowAll(g),
                text_button(show_all_id(g), "Show all", t),
                t,
                cx,
            )
        });
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(theme::muted(t))
                .when(gap.above.is_some(), |row| {
                    let down = gap_down_id(g);
                    let el = icon_chip(down, IconName::ChevronDown, "Show one more below", t);
                    row.child(self.button(Stop::GapDown(g), el, t, cx))
                })
                .when(gap.below.is_some(), |row| {
                    let el = icon_chip(gap_up_id(g), IconName::ChevronUp, "Show one more above", t);
                    row.child(self.button(Stop::GapUp(g), el, t, cx))
                })
                .child(
                    div()
                        .id(gap_count_id(g))
                        .role(Role::Label)
                        .aria_label(count.clone())
                        .child(count)
                        .test_support(),
                )
                .children(show_all),
        )
    }

    /// The slim row above chunk `i`, outside its sized height: the headings
    /// that contain it as clickable crumbs joined by ` › `, and `Open
    /// source` at the right end. Drawn with no crumbs too, so pieces line up.
    fn render_crumbs(&self, i: usize, t: &Theme, cx: &mut Context<Self>) -> Div {
        let mut crumbs: Vec<AnyElement> = Vec::new();
        let plan_crumbs = self.members.chunk(i).map_or(&[][..], |plan| &plan.crumbs);
        for (index, crumb) in plan_crumbs.iter().enumerate() {
            if index > 0 {
                crumbs.push(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme::muted(t))
                        .child("\u{203a}")
                        .into_any_element(),
                );
            }
            let last = index + 1 == plan_crumbs.len();
            let el = crumb_button(crumb_id(i, index), crumb.label.clone(), last, t);
            crumbs.push(
                self.button(Stop::Crumb(i, index), el, t, cx)
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_0p5()
                    .overflow_hidden()
                    .children(crumbs),
            )
            .child(self.button(
                Stop::Open(i),
                text_button(open_id(i), "Open source", t),
                t,
                cx,
            ))
    }

    fn render_chunk(
        &self,
        i: usize,
        line_height: f32,
        rem: f32,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.sized_rows.get(i).copied().unwrap_or(1);
        // The editor's own vertical inset (`py_2`, twice), and a pixel so
        // the last row is never cut by rounding.
        let height = rows as f32 * line_height + rem + 1.0;
        div()
            .id(chunk_id(i))
            .test_support()
            .role(Role::Group)
            .flex()
            .flex_col()
            .flex_none()
            .overflow_hidden()
            .child(self.render_crumbs(i, t, cx))
            .child(
                div()
                    .flex_none()
                    .h(px(height))
                    .child(self.chunks[i].clone()),
            )
            .into_any_element()
    }
}

/// Painted after the chunks, whose editors lay out in prepaint. When an
/// editor's visual rows differ from what its chunk was sized for, it asks
/// for one more frame ([`Window::on_next_frame`]) that notifies the view: a
/// notify made while painting marks nothing dirty and draws no frame. A
/// frame with every chunk sized right asks for nothing, so it settles after
/// one correcting frame.
fn resize_check(view: WeakEntity<FacetView>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, cx| {
            let Some(entity) = view.upgrade() else {
                return;
            };
            if !entity.update(cx, |this, cx| this.painted(cx)) {
                return;
            }
            let view = view.clone();
            window.on_next_frame(move |_, cx| {
                if let Some(entity) = view.upgrade() {
                    entity.update(cx, |this, cx| {
                        this.frame_asked = false;
                        cx.notify();
                    });
                }
            });
        },
    )
    .absolute()
    .size_0()
}

impl Focusable for FacetView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FacetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let line_height = (f32::from(editor_font::font_size(cx)) * LINE_HEIGHT).round();
        let rem = f32::from(window.rem_size());
        self.sized_rows = (0..self.members.len())
            .map(|i| self.rows_of(i, cx))
            .collect();
        self.stops.get_mut().begin(window);
        self.room_used = self.line_room.get();

        let bar = self.render_bar(window, t, cx);
        let mut body: Vec<AnyElement> = Vec::new();
        let mut pieces = Vec::with_capacity(self.members.len());
        // Each gap, then the chunk below it if any: every chunk once, in
        // order, each member's gaps its own. In a collection a member's
        // dateline stands above its first gap; `j`/`k` onto the member's
        // first chunk bring the dateline to the top.
        let mut lead = None;
        for g in 0..self.members.gaps.len() {
            let spaced = !body.is_empty();
            if let Some(line) = self.render_dateline(self.members.gaps[g], spaced, t) {
                lead = Some(body.len());
                body.push(line);
            }
            if let Some(gap) = self.render_gap(g, t, cx) {
                body.push(gap.into_any_element());
            }
            if let Some(i) = self.members.gaps[g].below {
                pieces.push(lead.take().unwrap_or(body.len()));
                body.push(self.render_chunk(i, line_height, rem, t, cx));
            }
        }
        self.piece_items = pieces;
        if self.members.len() == 0 {
            let text = SharedString::from(self.empty_text());
            let remove = (!self.bar.picks().is_empty()).then(|| {
                self.button(
                    Stop::RemoveLast,
                    text_button(REMOVE_LAST_ID.into(), "Remove last pick", t),
                    t,
                    cx,
                )
                .into_any_element()
            });
            let remove = remove.or_else(|| self.render_refresh(t, cx));
            body.push(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1()
                    .child(
                        div()
                            .id(EMPTY_ID)
                            .role(Role::Status)
                            .aria_label(text.clone())
                            .text_sm()
                            .text_color(theme::muted(t))
                            .child(text)
                            .test_support(),
                    )
                    .children(remove)
                    .into_any_element(),
            );
        }
        let hint = self.hint_shows().then(|| {
            let text = match self.collection {
                Some(_) => HINT_TEXT_NOTES,
                None => HINT_TEXT,
            };
            div()
                .id(HINT_ID)
                .role(Role::Status)
                .aria_label(text)
                .flex_none()
                .px_4()
                .pt_2()
                .text_xs()
                .text_color(theme::muted(t))
                .child(text)
                .test_support()
        });
        let first_notes = self.render_first_notes(t);
        let show_more = self
            .show_more_label()
            .map(|label| self.button(Stop::ShowMore, chip(SHOW_MORE_ID.into(), label, t), t, cx));
        // The stop with the keyboard went away (`Clear` once cleared): the
        // view takes it.
        if self.stops.get_mut().end() {
            window.focus(&self.focus, cx);
        }
        // A list closed by new text took its field with it (see `plan`).
        if let Some(field) = self.stranded.take()
            && field.is_focused(window)
        {
            window.focus(&self.focus, cx);
        }

        keys(div().id(FACET_VIEW_ID), cx)
            .test_support()
            .track_focus(&self.focus)
            .role(Role::Group)
            .aria_label("Facet view")
            .flex()
            .flex_col()
            .flex_1()
            .size_full()
            .min_h_0()
            .bg(theme::background(t))
            .children(bar)
            .children(first_notes)
            .children(hint)
            .child(
                div()
                    .id(COLUMN_ID)
                    .test_support()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(body)
                    .when_some(show_more, |column, button| {
                        column.child(div().flex().flex_none().child(button))
                    })
                    .child(resize_check(cx.entity().downgrade())),
            )
    }
}
