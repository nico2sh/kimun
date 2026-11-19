//! The **Facet view** (ADR-0061): a read-only view of one note narrowed to
//! the **facet scope** of the picked facets (CONTEXT.md, **Facet view**).
//!
//! **Layout.** A pinned bar over a scrolling column. The bar never scrolls
//! away and takes at most 40% of the view's height, scrolling inside itself
//! past that: a summary line with `Clear` while picks show something, then
//! one row per facet kind (tag, note link, heading), each led by its rail
//! icon, `+N more` folding the rest into a fuzzy list. A long note's heading
//! row collapses to its picked headings and a `Sections N` chip that opens
//! the same list. The search button at the bar's top-right opens the
//! search over every kind. The list floats under its chip
//! ([`super::facet_list`]), stays open while picks are made in it, and
//! closes on Escape or a press outside, or when new text takes away what it
//! hangs from. The bar keeps clear of buttons its host floats over the
//! view's top-right corner ([`FacetView::set_corner_room`]). The column
//! holds the shown chunks with a gap row between them, then `Show N more`
//! when more chunks match than the view draws. Each chunk is a read-only
//! [`EditorView`] sized to its lines, with no frame, under a slim row: the
//! headings that contain it as clickable crumbs, outer ones shortened first
//! with an ellipsis when the row is narrow, and `Open source`. A gap row counts the hidden blocks (`3
//! hidden`) and reveals one at a time next to the chunk above (down) or
//! below (up), or all of them with `Show all` when few are hidden.
//!
//! **Picking.** A pill toggles its facet. A plain press in a chunk toggles
//! the facet under it: a tag, a note link, or the heading of a heading line.
//! A press on anything else (a URL, an attachment, plain text) toggles
//! nothing. The editor reports one press per press, so a double press
//! toggles twice. A primary press only asks to follow.
//!
//! **Keyboard** ([`mod@keys`]): every chip, pill and text button is a tab stop
//! with a focus ring, and Enter or Space does what a click does. `/` opens
//! the search, `j`/`k` step between pieces, Escape clears the picks (from
//! inside a piece it first hands the keyboard to the view).
//!
//! **Content.** Core plans everything: [`note_facets`], [`select_blocks`],
//! [`plan_chunks`] and [`reveal`]. A change of picks or text plans again
//! from the picks (revealed blocks reset); new text also drops picks the
//! note no longer holds. The view takes plain data in and emits
//! [`FacetEvent`]s; it knows no workspace.

use std::cell::RefCell;
use std::ops::Range;

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
use kimun_core::note::scan::{LinkSpanKind, heading_display_text};
use kimun_core::note::{
    ChunkPlan, Facet, FacetKind, GAP_SHOW_ALL_MAX, MAX_RANGES, NoteFacets, NoteTarget, Reveal,
    note_facets, plan_chunks, reveal, reveal_gap, select_blocks,
};
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::facet_bar::{FacetBar, Pill, Row};
use kimun_notes::settings::themes::Theme;

use self::keys::{Stop, Stops, keys};
use super::facet_list::{OpenList, list_popover};
use super::facet_ui::{chip, crumb as crumb_button, icon_chip, leader, pill, text_button};
use super::query_field::focus_input;
use crate::editor_element::LINE_HEIGHT;
use crate::editor_font;
use crate::editor_view::{EditorEvent, EditorView};
use crate::theme;

mod keys;

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
/// The `Sections N` chip of a collapsed heading row.
pub(crate) const SECTIONS_ID: &str = "facet-sections";
/// The bar's search button: the trigger of the search over every kind.
pub(crate) const SEARCH_ID: &str = "facet-search";
/// The scrolling column of pieces.
pub(crate) const COLUMN_ID: &str = "facet-column";

fn kind_slug(kind: FacetKind) -> &'static str {
    match kind {
        FacetKind::Heading => "heading",
        FacetKind::Tag => "tag",
        FacetKind::NoteLink => "link",
    }
}

/// The pill of `facet` in its row.
pub(crate) fn pill_id(facet: &Facet) -> SharedString {
    format!("facet-pill:{}:{}", kind_slug(facet.kind), facet.key).into()
}

/// The pill of `facet` in the `+N more` list.
pub(crate) fn more_item_id(facet: &Facet) -> SharedString {
    format!("facet-more-item:{}:{}", kind_slug(facet.kind), facet.key).into()
}

/// The `+N more` button of `kind`'s row.
pub(crate) fn more_button_id(kind: FacetKind) -> SharedString {
    format!("facet-more:{}", kind_slug(kind)).into()
}

/// The leading icon of `kind`'s row.
pub(crate) fn row_leader_id(kind: FacetKind) -> SharedString {
    format!("facet-leader:{}", kind_slug(kind)).into()
}

/// Chunk `i`.
pub(crate) fn chunk_id(i: usize) -> SharedString {
    format!("facet-chunk:{i}").into()
}

/// Chunk `i`'s `Open source` button.
pub(crate) fn open_id(i: usize) -> SharedString {
    format!("facet-open:{i}").into()
}

/// Gap `g`'s button that reveals the block below the chunk above it. Gap
/// `g` sits before chunk `g`; the last gap follows the last chunk.
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

/// The facet a plain press toggles, in a note at `source`: the tag or note
/// link under the press, else the heading of `line` (the pressed line).
/// `None` for a link that is no note link (a URL, an attachment, an image)
/// and for a line that is no heading.
pub(crate) fn pressed_facet(
    target: Option<&FollowTarget>,
    line: &str,
    source: &VaultPath,
) -> Option<Facet> {
    match target {
        Some(FollowTarget::Label(name)) => Some(Facet::new(FacetKind::Tag, name)),
        Some(FollowTarget::Link { target, kind }) => {
            let note = match kind {
                LinkSpanKind::WikiLink => NoteTarget::from_wikilink(target, source),
                LinkSpanKind::Markdown => NoteTarget::from_markdown(target, source),
                LinkSpanKind::Image => None,
            }?;
            Some(Facet::new(
                FacetKind::NoteLink,
                &note.path().get_clean_name(),
            ))
        }
        None => heading_display_text(line).map(|text| Facet::new(FacetKind::Heading, &text)),
    }
}

/// What the view asks of its host.
#[derive(Debug, Clone, PartialEq)]
pub enum FacetEvent {
    /// Jump to the source note, at the chunk's first source line (0-based).
    OpenSource { line: usize },
    /// Primary+click on a link or tag.
    Follow(FollowTarget),
    /// The picks changed; the workspace saves the session.
    PicksChanged,
}

/// The open list: its kind (`None`: the search over every kind), its
/// fuzzy query field, and the result Enter toggles (lit; a new query lights
/// the first).
struct More {
    kind: Option<FacetKind>,
    input: Entity<InputState>,
    highlight: usize,
    _events: Subscription,
}

const HINT_TEXT: &str = "Pick a tag, link or section to narrow this note.";

/// The facet view. See the module docs.
pub struct FacetView {
    source: VaultPath,
    text: String,
    facets: NoteFacets,
    bar: FacetBar,
    /// Block ranges shown: the picks' selection, plus revealed blocks.
    shown: Vec<Range<usize>>,
    /// The most chunks drawn; `Show N more` doubles it.
    limit: usize,
    /// One plan per drawn chunk; `plans[i]` is `shown[i]`.
    plans: Vec<ChunkPlan>,
    /// Shown ranges past `limit`, not drawn.
    cut: usize,
    /// One read-only editor per plan, reused in place.
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
    /// The focus handle of a list's field that [`Self::plan`] closed: the
    /// next frame gives the keyboard to the view if the field still had it.
    stranded: Option<FocusHandle>,
}

impl EventEmitter<FacetEvent> for FacetView {}

impl FacetView {
    /// A view of `text`, the note at `source`, with `picks` (those the note
    /// does not hold are dropped: a restore can name facets the note lost).
    pub fn new(source: VaultPath, text: String, picks: Vec<Facet>, cx: &mut Context<Self>) -> Self {
        let facets = note_facets(&text, &source);
        let mut bar = FacetBar::default();
        bar.set_picks(picks);
        bar.prune(&facets);
        let hint_done = !bar.picks().is_empty();
        let mut view = Self {
            source,
            text,
            facets,
            bar,
            shown: Vec::new(),
            limit: MAX_RANGES,
            plans: Vec::new(),
            cut: 0,
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
            stranded: None,
        };
        view.select(cx);
        view
    }

    pub fn source(&self) -> &VaultPath {
        &self.source
    }

    pub fn picks(&self) -> &[Facet] {
        self.bar.picks()
    }

    /// The note text the view shows (tests).
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// New content: re-derive, prune picks, reset reveals.
    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.facets = note_facets(&text, &self.source);
        self.text = text;
        let pruned = self.bar.prune(&self.facets);
        self.select(cx);
        if pruned {
            cx.emit(FacetEvent::PicksChanged);
        }
        cx.notify();
    }

    /// The source note moved or was renamed. Relative note links resolve
    /// against the new place from the next [`Self::set_text`] on.
    pub fn set_source(&mut self, source: VaultPath) {
        self.source = source;
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

    /// The open `+N more` list's query field (tests).
    #[cfg(test)]
    pub(crate) fn more_input(&self) -> Option<Entity<InputState>> {
        self.more.as_ref().map(|more| more.input.clone())
    }

    /// The open list's results, in order (tests).
    #[cfg(test)]
    pub(crate) fn list_results(&self, cx: &App) -> Vec<Facet> {
        self.list_pills(cx).into_iter().map(|p| p.facet).collect()
    }

    /// The element drawn with the focus ring in the last frame (tests).
    #[cfg(test)]
    pub(crate) fn ringed(&self) -> Option<SharedString> {
        self.stops.borrow().ringed.as_ref().map(Stop::id)
    }

    /// `Show N more`, while chunks are left out: N is what the next click
    /// shows, the limit or what is left when that is less.
    pub(crate) fn show_more_label(&self) -> Option<String> {
        (self.cut > 0).then(|| format!("Show {} more", self.cut.min(self.limit)))
    }

    /// Show what the picks select, nothing revealed.
    fn select(&mut self, cx: &mut Context<Self>) {
        self.shown = select_blocks(&self.facets, self.bar.picks());
        self.plan(cx);
    }

    /// Plan the chunks for `shown` and bring the editors in step: one per
    /// plan, an editor whose text is unchanged left alone.
    fn plan(&mut self, cx: &mut Context<Self>) {
        let (plans, cut) = plan_chunks(&self.facets, &self.text, &self.shown, self.limit);
        self.chunks.truncate(plans.len());
        self._chunk_events.truncate(plans.len());
        self.laid_out.truncate(plans.len());
        for (i, plan) in plans.iter().enumerate() {
            match self.chunks.get(i) {
                Some(editor) => {
                    if self.plans.get(i).is_none_or(|old| old.text != plan.text) {
                        editor.update(cx, |editor, cx| editor.set_text_read_only(&plan.text, cx));
                        self.laid_out[i] = false;
                    }
                }
                None => {
                    let editor = cx.new(|cx| EditorView::new_read_only(&plan.text, cx));
                    let events = cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
                        if let EditorEvent::ReadOnlyPress {
                            target,
                            line,
                            primary,
                        } = event
                        {
                            this.pressed(target.as_ref(), line, *primary, cx);
                        }
                    });
                    self.chunks.push(editor);
                    self._chunk_events.push(events);
                    self.laid_out.push(false);
                }
            }
        }
        self.plans = plans;
        self.cut = cut;
        // New text can take away what the open list hangs from (its kind's
        // row, or the whole bar for the search): it closes, and the next
        // frame hands the keyboard from its field to the view.
        let gone = self
            .more
            .as_ref()
            .is_some_and(|more| !self.list_drawable(more.kind));
        if gone && let Some(more) = self.more.take() {
            self.stranded = Some(more.input.read(cx).focus_handle(cx));
        }
        cx.notify();
    }

    /// Whether the bar draws what `kind`'s list hangs from: a row of that
    /// kind, or for the search (`None`) the bar itself (a note with a facet).
    fn list_drawable(&self, kind: Option<FacetKind>) -> bool {
        self.facets
            .entries
            .iter()
            .any(|entry| kind.is_none_or(|kind| entry.facet.kind == kind))
    }

    /// A press in a chunk: a primary press asks to follow, a plain one
    /// toggles the facet under it.
    fn pressed(
        &mut self,
        target: Option<&FollowTarget>,
        line: &str,
        primary: bool,
        cx: &mut Context<Self>,
    ) {
        if primary {
            if let Some(target) = target {
                cx.emit(FacetEvent::Follow(target.clone()));
            }
            return;
        }
        if let Some(facet) = pressed_facet(target, line, &self.source) {
            self.toggle(facet, cx);
        }
    }

    /// Pick `facet`, or unpick it. Nothing for a facet the note does not hold.
    pub(super) fn toggle(&mut self, facet: Facet, cx: &mut Context<Self>) {
        if !self.facets.entries.iter().any(|e| e.facet == facet) {
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
        if self.bar.picks().is_empty() {
            return "This note is empty.".into();
        }
        self.bar
            .empty_reason(&self.facets)
            .unwrap_or_else(|| "Nothing matches the picked facets.".into())
    }

    /// Whether the hint under the bar shows: no pick made yet in this view,
    /// and the note has something to pick.
    fn hint_shows(&self) -> bool {
        !self.hint_done && self.bar.picks().is_empty() && !self.facets.entries.is_empty()
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
        if self.plans.is_empty() {
            return None;
        }
        self.bar.summary(&self.facets, self.plans.len() + self.cut)
    }

    /// Reveal one hidden block next to chunk `chunk`.
    fn reveal(&mut self, chunk: usize, dir: Reveal, cx: &mut Context<Self>) {
        self.shown = reveal(&self.shown, chunk, dir, self.facets.blocks.len());
        self.plan(cx);
    }

    /// Reveal every block hidden in gap `g` (before chunk `g`): `Show all`.
    fn reveal_all(&mut self, g: usize, cx: &mut Context<Self>) {
        self.shown = reveal_gap(&self.shown, g, self.facets.blocks.len());
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
        let written = self.plans[i].text.split('\n').count();
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
        self.sized_rows.len() != self.plans.len()
            || (0..self.plans.len()).any(|i| self.rows_of(i, cx) != self.sized_rows[i])
    }

    /// After a paint (the editors laid out in its prepaint): every chunk's
    /// layout is current. True when a chunk was sized for other rows and no
    /// correcting frame is asked for yet; the caller asks for one.
    fn painted(&mut self, cx: &App) -> bool {
        self.laid_out.iter_mut().for_each(|done| *done = true);
        if self.frame_asked || !self.rows_changed(cx) {
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

    /// Whether the last paint asked for a correcting frame (tests).
    #[cfg(test)]
    pub(crate) fn resize_asked(&self) -> bool {
        self.frame_asked
    }

    /// Chunk `i`'s first source line, 0-based.
    fn first_line(&self, i: usize) -> usize {
        let start = self
            .plans
            .get(i)
            .and_then(|plan| self.facets.blocks.get(plan.blocks.start))
            .map_or(0, |block| block.range.start);
        self.text[..start].matches('\n').count()
    }

    /// Open `kind`'s list (`None`: the search over every kind), its filter
    /// field empty and holding the keyboard. It replaces a list already open.
    pub(crate) fn open_list(
        &mut self,
        kind: Option<FacetKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Nothing to hang it from: a note with no facet draws no bar.
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
    /// it (the search button for the search), or to the view when that chip
    /// is not drawn (a row with every facet picked has none).
    pub(crate) fn close_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(more) = self.more.take() else {
            return;
        };
        let chip = match more.kind {
            Some(kind) => Stop::Opener(kind),
            None => Stop::Search,
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
        kind: Option<FacetKind>,
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

    /// The pinned bar above the column: the summary line, then one row per
    /// kind, with the search button at its top-right. `None` when it would
    /// be empty (a note with no facet).
    fn render_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let summary = self.render_summary(t, cx);
        let rows: Vec<Div> = self
            .bar
            .rows(&self.facets)
            .into_iter()
            .map(|row| self.render_row(row, t, cx))
            .collect();
        if summary.is_none() && rows.is_empty() {
            return None;
        }
        Some(
            div()
                .id(FACET_BAR_ID)
                .flex()
                .flex_none()
                .items_start()
                .gap_2()
                .max_h(relative(0.4))
                .overflow_y_scroll()
                .px_4()
                .when_some(self.corner_room, |bar, room| bar.pr(room))
                .py_3()
                .border_b_1()
                .border_color(theme::divider(t))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .children(summary)
                        .children(rows),
                )
                .child(self.render_search(t, cx))
                .test_support(),
        )
    }

    /// The search button: the search over every kind (`/`) hangs from it.
    fn render_search(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let button = icon_chip(SEARCH_ID.into(), IconName::Search, "Search facets (/)", t);
        let button = self.track(Stop::Search, button.test_support(), t, cx);
        self.render_list(None, button.into_any_element(), t, cx)
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

    /// One kind's row: its icon, its pills, then `+N more`, or for a
    /// collapsed heading row the picked headings and `Sections N`. Both
    /// chips open the kind's full list.
    fn render_row(&self, row: Row, t: &Theme, cx: &mut Context<Self>) -> Div {
        let kind = row.kind;
        let total = row.pills.len() + row.more;
        let (id, label) = if row.collapsed {
            (SECTIONS_ID.into(), format!("Sections {total}"))
        } else {
            (more_button_id(kind), format!("+{} more", row.more))
        };
        let has_opener = row.collapsed || row.more > 0;
        let opener = has_opener.then(|| {
            let chip = self.track(Stop::Opener(kind), chip(id, label, t).test_support(), t, cx);
            self.render_list(Some(kind), chip.into_any_element(), t, cx)
        });
        // Picks can leave a row with no chip (every facet of the kind picked)
        // while its list is open: the list then hangs from the row's icon.
        let leader = if !has_opener && self.list_open(Some(kind)) {
            self.render_list(Some(kind), leader(kind, t).into_any_element(), t, cx)
        } else {
            leader(kind, t).into_any_element()
        };
        div()
            .flex()
            .flex_none()
            .items_start()
            .gap_2()
            .child(leader)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .children(row.pills.into_iter().map(|p| {
                        let el = pill(pill_id(&p.facet), &p, t);
                        self.button(Stop::Pill(p.facet), el, t, cx)
                    }))
                    .children(opener),
            )
    }

    fn list_open(&self, kind: Option<FacetKind>) -> bool {
        self.more.as_ref().is_some_and(|more| more.kind == kind)
    }

    /// `trigger` with `kind`'s list ([`list_popover`]) hung under it, open
    /// while the view holds it open: the field, then the pills of
    /// [`FacetBar::more_list`] (of [`FacetBar::search`] for `None`).
    fn render_list(
        &self,
        kind: Option<FacetKind>,
        trigger: AnyElement,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let list = self
            .more
            .as_ref()
            .filter(|more| more.kind == kind)
            .map(|more| {
                let pills = self.list_pills(cx);
                let lit = more.highlight.min(pills.len().saturating_sub(1));
                let pills = pills
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let facet = p.facet.clone();
                        pill(more_item_id(&p.facet), &p, t)
                            .aria_selected(i == lit)
                            .when(i == lit, |p| p.border_color(theme::focus_border(t)))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.pick_more(facet.clone(), window, cx)
                            }))
                            .test_support()
                            .into_any_element()
                    })
                    .collect();
                OpenList {
                    input: more.input.clone(),
                    pills,
                }
            });
        let changed = cx.listener(move |this, open: &bool, window, cx| {
            this.list_changed(kind, *open, window, cx)
        });
        list_popover(kind, trigger, list, changed, t, cx)
    }

    /// The open list's results: [`FacetBar::more_list`] of its kind, or
    /// [`FacetBar::search`] over every kind, filtered by its query.
    fn list_pills(&self, cx: &App) -> Vec<Pill> {
        let Some(more) = &self.more else {
            return Vec::new();
        };
        let query = more.input.read(cx).value().to_string();
        match more.kind {
            Some(kind) => self.bar.more_list(&self.facets, kind, &query),
            None => self.bar.search(&self.facets, &query),
        }
    }

    /// Gap `g` (before chunk `g`), or `None` when it hides nothing.
    fn render_gap(&self, g: usize, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let hidden = match g {
            0 => self.plans.first()?.hidden_above,
            _ => self.plans.get(g - 1)?.hidden_below,
        };
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
                .when(g > 0, |row| {
                    let down = gap_down_id(g);
                    let el = icon_chip(down, IconName::ChevronDown, "Show one more below", t);
                    row.child(self.button(Stop::GapDown(g), el, t, cx))
                })
                .when(g < self.plans.len(), |row| {
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
        for (index, crumb) in self.plans[i].crumbs.iter().enumerate() {
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
            let last = index + 1 == self.plans[i].crumbs.len();
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
        self.sized_rows = (0..self.plans.len()).map(|i| self.rows_of(i, cx)).collect();
        self.stops.get_mut().begin(window);

        let bar = self.render_bar(t, cx);
        let mut body: Vec<AnyElement> = Vec::new();
        let mut pieces = Vec::with_capacity(self.plans.len());
        for i in 0..=self.plans.len() {
            if let Some(gap) = self.render_gap(i, t, cx) {
                body.push(gap.into_any_element());
            }
            if i < self.plans.len() {
                pieces.push(body.len());
                body.push(self.render_chunk(i, line_height, rem, t, cx));
            }
        }
        self.piece_items = pieces;
        if self.plans.is_empty() {
            let text = SharedString::from(self.empty_text());
            let remove = (!self.bar.picks().is_empty()).then(|| {
                self.button(
                    Stop::RemoveLast,
                    text_button(REMOVE_LAST_ID.into(), "Remove last pick", t),
                    t,
                    cx,
                )
            });
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
            div()
                .id(HINT_ID)
                .role(Role::Status)
                .aria_label(HINT_TEXT)
                .flex_none()
                .px_4()
                .pt_2()
                .text_xs()
                .text_color(theme::muted(t))
                .child(HINT_TEXT)
                .test_support()
        });
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
