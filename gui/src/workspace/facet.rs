//! The **Facet view** (ADR-0061): a read-only view of one note narrowed to
//! the **facet scope** of the picked facets (CONTEXT.md, **Facet view**).
//!
//! **Layout.** One column that scrolls as a whole: the pill rows (one per
//! facet kind, `+N more` folding the rest into a fuzzy list), then the shown
//! chunks with a gap row between them, then `Show N more` when more chunks
//! match than the view draws. Each chunk is a read-only [`EditorView`] sized
//! to its lines, with an `Open source` button; a gap row counts the hidden
//! blocks and reveals one at a time next to the chunk above (down) or below
//! (up).
//!
//! **Picking.** A pill toggles its facet. A plain press in a chunk toggles
//! the facet under it: a tag, a note link, or the heading of a heading line.
//! A press on anything else (a URL, an attachment, plain text) toggles
//! nothing. The editor reports one press per press, so a double press
//! toggles twice. A primary press only asks to follow.
//!
//! **Content.** Core plans everything: [`note_facets`], [`select_blocks`],
//! [`plan_chunks`] and [`reveal`]. A change of picks or text plans again
//! from the picks (revealed blocks reset); new text also drops picks the
//! note no longer holds. The view takes plain data in and emits
//! [`FacetEvent`]s; it knows no workspace.

use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, canvas, div, px,
};
use kimun_core::nfs::VaultPath;
use kimun_core::note::scan::{LinkSpanKind, heading_display_text};
use kimun_core::note::{
    ChunkPlan, Facet, FacetKind, MAX_RANGES, NoteFacets, NoteTarget, Reveal, note_facets,
    plan_chunks, reveal, select_blocks,
};
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::facet_bar::{FacetBar, Pill};
use kimun_notes::settings::themes::Theme;

use super::query_field::{focus_input, query_field};
use crate::editor_element::LINE_HEIGHT;
use crate::editor_font;
use crate::editor_view::{EditorEvent, EditorView};
use crate::theme;

/// Element ids, for tests.
pub(crate) const FACET_VIEW_ID: &str = "facet-view";
pub(crate) const SHOW_MORE_ID: &str = "facet-show-more";
pub(crate) const MORE_QUERY_ID: &str = "facet-more-query";

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

/// The open `+N more` list: its kind and its fuzzy query field.
struct More {
    kind: FacetKind,
    input: Entity<InputState>,
    _events: Subscription,
}

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
        cx.notify();
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
        self.select(cx);
        cx.emit(FacetEvent::PicksChanged);
    }

    /// Reveal one hidden block next to chunk `chunk`.
    fn reveal(&mut self, chunk: usize, dir: Reveal, cx: &mut Context<Self>) {
        self.shown = reveal(&self.shown, chunk, dir, self.facets.blocks.len());
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

    /// Chunk `i`'s first source line, 0-based.
    fn first_line(&self, i: usize) -> usize {
        let start = self
            .plans
            .get(i)
            .and_then(|plan| self.facets.blocks.get(plan.blocks.start))
            .map_or(0, |block| block.range.start);
        self.text[..start].matches('\n').count()
    }

    /// Open `kind`'s `+N more` list, or close it when it is the open one.
    fn toggle_more(&mut self, kind: FacetKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.more.as_ref().is_some_and(|more| more.kind == kind) {
            self.more = None;
            cx.notify();
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let events = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        focus_input(&input, window, cx);
        self.more = Some(More {
            kind,
            input,
            _events: events,
        });
        cx.notify();
    }

    /// A pick from the `+N more` list: toggle it, close the list.
    fn pick_more(&mut self, facet: Facet, cx: &mut Context<Self>) {
        self.more = None;
        self.toggle(facet, cx);
        cx.notify();
    }

    // ---- Drawing -------------------------------------------------------------

    fn render_rows(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let rows = self.bar.rows(&self.facets);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .children(rows.into_iter().map(|row| {
                let kind = row.kind;
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .children(row.pills.into_iter().map(|p| {
                        let facet = p.facet.clone();
                        pill(pill_id(&p.facet), &p, t)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.toggle(facet.clone(), cx)
                            }))
                            .test_support()
                    }))
                    .when(row.more > 0, |row_div| {
                        row_div.child(
                            chip(more_button_id(kind), format!("+{} more", row.more), t)
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.toggle_more(kind, window, cx)
                                }))
                                .test_support(),
                        )
                    })
            }))
    }

    fn render_more(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let more = self.more.as_ref()?;
        let query = more.input.read(cx).value().to_string();
        let pills = self.bar.more_list(&self.facets, more.kind, &query);
        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_1()
                .py_2()
                .border_1()
                .rounded_md()
                .border_color(theme::divider(t))
                .child(query_field(&more.input, MORE_QUERY_ID, "Filter facets", t))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .px_2()
                        .children(pills.into_iter().map(|p| {
                            let facet = p.facet.clone();
                            pill(more_item_id(&p.facet), &p, t)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.pick_more(facet.clone(), cx)
                                }))
                                .test_support()
                        })),
                ),
        )
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
        let noun = if hidden == 1 { "block" } else { "blocks" };
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(theme::muted(t))
                .when(g > 0, |row| {
                    row.child(
                        icon_chip(
                            gap_down_id(g),
                            IconName::ChevronDown,
                            "Show the next block",
                            t,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.reveal(g - 1, Reveal::Below, cx)
                        }))
                        .test_support(),
                    )
                })
                .when(g < self.plans.len(), |row| {
                    row.child(
                        icon_chip(
                            gap_up_id(g),
                            IconName::ChevronUp,
                            "Show the block before",
                            t,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.reveal(g, Reveal::Above, cx)
                        }))
                        .test_support(),
                    )
                })
                .child(format!("{hidden} hidden {noun}")),
        )
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
            .border_1()
            .rounded_md()
            .border_color(theme::divider(t))
            .overflow_hidden()
            .child(
                div().flex().flex_none().justify_end().px_1().pt_1().child(
                    icon_chip(open_id(i), IconName::ExternalLink, "Open source", t)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let line = this.first_line(i);
                            cx.emit(FacetEvent::OpenSource { line });
                        }))
                        .test_support(),
                ),
            )
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

/// A small bordered button, in the style of the sort chip.
fn chip(id: SharedString, label: impl Into<SharedString>, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .whitespace_nowrap()
        .px_1p5()
        .rounded_md()
        .border_1()
        .border_color(theme::divider(t))
        .text_xs()
        .text_color(theme::muted(t))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label)
}

/// A [`chip`] that shows an icon, named `label`.
fn icon_chip(id: SharedString, icon: IconName, label: &'static str, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .flex_none()
        .items_center()
        .p_0p5()
        .rounded_md()
        .border_1()
        .border_color(theme::divider(t))
        .text_color(theme::muted(t))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size_3())
}

/// A facet's pill: its label in its kind's colour, its count when more
/// than one, filled when picked.
fn pill(id: SharedString, p: &Pill, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let (label, color) = match p.facet.kind {
        FacetKind::Tag => (format!("#{}", p.label), theme::tag(t)),
        FacetKind::Heading => (p.label.clone(), theme::bright(t)),
        FacetKind::NoteLink => (p.label.clone(), theme::accent(t)),
    };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(SharedString::from(label.clone()))
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .whitespace_nowrap()
        .px_1p5()
        .rounded_md()
        .border_1()
        .text_xs()
        .cursor_pointer()
        .map(|pill| {
            if p.picked {
                pill.border_color(theme::accent(t))
                    .bg(theme::selected_background(t))
            } else {
                pill.border_color(theme::divider(t))
                    .hover(move |style| style.bg(hover))
            }
        })
        .child(div().text_color(color).child(label))
        .when(p.count > 1, |pill| {
            pill.child(div().text_color(theme::muted(t)).child(p.count.to_string()))
        })
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

        let mut body: Vec<AnyElement> = Vec::new();
        for i in 0..=self.plans.len() {
            if let Some(gap) = self.render_gap(i, t, cx) {
                body.push(gap.into_any_element());
            }
            if i < self.plans.len() {
                body.push(self.render_chunk(i, line_height, rem, t, cx));
            }
        }
        if self.plans.is_empty() {
            let empty = if self.bar.picks().is_empty() {
                "This note is empty."
            } else {
                "Nothing matches the picked facets."
            };
            body.push(
                div()
                    .text_sm()
                    .text_color(theme::muted(t))
                    .child(empty)
                    .into_any_element(),
            );
        }
        let show_more = self.show_more_label().map(|label| {
            chip(SHOW_MORE_ID.into(), label, t)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_more(cx)))
                .test_support()
        });

        div()
            .id(FACET_VIEW_ID)
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
            .child(
                div()
                    .id("facet-column")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(self.render_rows(t, cx))
                    .children(self.render_more(t, cx))
                    .children(body)
                    .when_some(show_more, |column, button| {
                        column.child(div().flex().flex_none().child(button))
                    })
                    .child(resize_check(cx.entity().downgrade())),
            )
    }
}
