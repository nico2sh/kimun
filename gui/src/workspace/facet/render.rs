//! How the facet view draws.
//!
//! It owns how the facet view draws, and the order of its tab stops.
//! It holds no state of its own.

use gpui_kit::assets::IconName;
use gpui_kit::base::{ObservedElement, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px, relative,
};
use kimun_core::note::GAP_SHOW_ALL_MAX;
use kimun_notes::settings::themes::Theme;

use super::bar_motion::HintDraw;
use super::chunks::resize_check;
use super::keys::{Stop, keys};
use super::{
    CLEAR_ID, COLUMN_ID, EMPTY_ID, FACET_BAR_ID, FACET_VIEW_ID, FacetView, HINT_ID, HINT_TEXT,
    HINT_TEXT_NOTES, REMOVE_LAST_ID, SHOW_MORE_COUNT_ID, SHOW_MORE_ID, SUMMARY_ID, chunk_id,
    collection, crumb_id, gap_count_id, gap_down_id, gap_up_id, open_id, show_all_id,
};
use crate::editor_element::LINE_HEIGHT;
use crate::editor_font;
use crate::theme;
use crate::workspace::facet_ui::{chip, crumb as crumb_button, icon_chip, ring_room, text_button};

impl FacetView {
    /// The pinned bar above the column: the chips, the picks
    /// ([`mod@super::line`]), then the summary line. `None` when it would be empty
    /// (a note with no facet).
    pub(super) fn render_bar(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let counts = self
            .bar
            .menus(&self.members.merged, &|f| self.members.in_view_of(f));
        // Drawn in this order, so Tab walks the chips, the picks, `Clear`.
        let chips = self.render_chips(counts, t, cx);
        let picks = self.render_picks(t, cx);
        let summary = self.render_summary(t, cx);
        if chips.is_none() && picks.is_empty() && summary.is_none() {
            self.bar_hidden();
            return None;
        }
        self.reveal_arrived(chips.is_some(), picks.len());
        // While it moves, the bar lays out at its full height inside a frame
        // that clips it ([`mod@super::bar_motion`]). At rest it has no frame.
        let moving = self.bar_frame(window, cx);
        // One wrapping row: row 1 and the summary line take the full width,
        // so the picks wrap between them as their own row.
        let bar = div()
            .id(FACET_BAR_ID)
            .flex()
            .flex_wrap()
            .items_center()
            .flex_none()
            .gap_1()
            .when(moving.is_none(), |bar| bar.max_h(relative(0.4)))
            .overflow_y_scroll()
            .track_scroll(&self.bar_scroll)
            .px_4()
            .when_some(self.corner_room, |bar, room| bar.pr(room))
            .py_3()
            .border_b_1()
            .border_color(theme::divider(t))
            .children(chips)
            .children(picks)
            .children(summary)
            .test_support();
        Some(match moving {
            Some(height) => self.framed_bar(bar, height),
            None => bar.into_any_element(),
        })
    }

    /// `el`, a button that does what `stop` does, as that tab stop.
    pub(super) fn button(
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
    pub(super) fn render_summary(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        if self.undo.offered() && self.bar.picks().is_empty() {
            return Some(self.render_cleared(t, cx));
        }
        if self.bar.picks().is_empty() {
            return None;
        }
        let text = self
            .summary()
            .map(|summary| muted_note(SUMMARY_ID, Role::Status, summary.into(), t));
        Some(
            div()
                .flex()
                .flex_none()
                .w_full()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w_0().children(text))
                .child(self.button(Stop::Clear, chip(CLEAR_ID.into(), "Clear", false, t), t, cx)),
        )
    }

    /// Gap `g` ([`mod@super::members`]), or `None` when it hides nothing.
    pub(super) fn render_gap(&self, g: usize, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let gap = *self.members.gaps.get(g)?;
        let hidden = self.members.gap_hidden(g);
        if hidden == 0 {
            return None;
        }
        let count = SharedString::from(match hidden {
            1 => "1 block hidden".to_string(),
            n => format!("{n} blocks hidden"),
        });
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
                    let el = icon_chip(down, IconName::ChevronDown, "Show next block", t);
                    row.child(self.button(Stop::GapDown(g), el, t, cx))
                })
                .when(gap.below.is_some(), |row| {
                    let el = icon_chip(gap_up_id(g), IconName::ChevronUp, "Show previous block", t);
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
    /// note` at the right end. Drawn with no crumbs too, so pieces line up.
    pub(super) fn render_crumbs(&self, i: usize, t: &Theme, cx: &mut Context<Self>) -> Div {
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
        // The piece clips, and so does the crumbs' own box: both keep room
        // for a focus ring at their edge. The crumbs' box reaches into the
        // row's room, so the crumbs sit where the row's other parts do.
        let room = ring_room();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .p(room)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_0p5()
                    .p(room)
                    .m(-room)
                    .overflow_hidden()
                    .children(crumbs),
            )
            .child(self.button(
                Stop::Open(i),
                text_button(open_id(i), "Open note", t),
                t,
                cx,
            ))
    }

    pub(super) fn render_chunk(
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

    /// The column's items: each gap, then the chunk below it, and the
    /// empty message while no chunk is drawn. It records where each piece
    /// starts for `j`/`k`.
    fn render_body(
        &mut self,
        line_height: f32,
        rem: f32,
        title_size: Pixels,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut body: Vec<AnyElement> = Vec::new();
        let mut pieces = Vec::with_capacity(self.members.len());
        // Each gap, then the chunk below it if any: every chunk once, in
        // order, each member's gaps its own. In a collection a member's
        // dateline stands above its first gap; `j`/`k` onto the member's
        // first chunk bring the dateline to the top.
        let mut lead = None;
        for g in 0..self.members.gaps.len() {
            let spaced = !body.is_empty();
            if let Some(line) = self.render_dateline(self.members.gaps[g], spaced, title_size, t) {
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
            body.push(self.render_empty(t, cx));
        }
        body
    }

    /// The message that stands where the pieces would be, with `Remove
    /// last pick` or a collection's `Refresh` under it.
    fn render_empty(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
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
            .into_any_element()
    }

    /// The first-use hint under the bar, while it shows, and while it
    /// collapses after the first pick.
    fn render_hint(&mut self, t: &Theme, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let text = match self.collection {
            Some(_) => HINT_TEXT_NOTES,
            None => HINT_TEXT,
        };
        let hint = || {
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
        };
        match self.hint_draw(self.hint_shows(), window, cx) {
            HintDraw::Gone => None,
            HintDraw::Plain => Some(hint().into_any_element()),
            HintDraw::Framed(height) => Some(self.framed_hint(hint(), height)),
        }
    }

    /// The row with `Show N more` and the count of pieces shown beside it.
    /// `None` while no chunk is left out.
    fn render_show_more(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let show_more = self.show_more_label().map(|label| {
            self.button(
                Stop::ShowMore,
                chip(SHOW_MORE_ID.into(), label, false, t),
                t,
                cx,
            )
        });
        let show_more_count = self
            .show_more_count()
            .map(|text| muted_note(SHOW_MORE_COUNT_ID, Role::Label, text.into(), t));
        show_more.map(|button| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(button)
                .children(show_more_count)
        })
    }

    /// After the stops are drawn: give the keyboard to the stop that asked
    /// for it, or to the view when its stop or its list field went away.
    fn settle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A stop asked for the keyboard (`Undo`, after Enter on `Clear`)
        // takes it once drawn. Else, when the stop with the keyboard went
        // away (`Undo` once used), the view takes it.
        let gone = self.stops.get_mut().end();
        let asked = self
            .focus_after
            .take()
            .and_then(|stop| self.stops.get_mut().handle(&stop));
        if let Some(handle) = asked {
            window.focus(&handle, cx);
        } else if gone {
            window.focus(&self.focus, cx);
        }
        // A list closed by new text took its field with it (see `plan`).
        if let Some(field) = self.stranded.take()
            && field.is_focused(window)
        {
            window.focus(&self.focus, cx);
        }
    }
}

/// A line of small muted text with `id` and `role`: the summary, `Picks
/// cleared` and the count beside `Show N more`.
pub(super) fn muted_note(
    id: impl Into<ElementId>,
    role: Role,
    text: SharedString,
    t: &Theme,
) -> ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .role(role)
        .aria_label(text.clone())
        .text_xs()
        .text_color(theme::muted(t))
        .child(text)
        .test_support()
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
        let title_size = collection::dateline_title_size(cx);
        let rem = f32::from(window.rem_size());
        self.sized_rows = (0..self.members.len())
            .map(|i| self.rows_of(i, cx))
            .collect();
        self.stops.get_mut().begin(window);

        // Drawn in this order: the tab stops follow it.
        let bar = self.render_bar(t, window, cx);
        let body = self.render_body(line_height, rem, title_size, t, cx);
        let hint = self.render_hint(t, window, cx);
        let collection_line = self.render_collection_line(t);
        let show_more = self.render_show_more(t, cx);
        self.settle_focus(window, cx);

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
            .children(collection_line)
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
                    .children(show_more)
                    .child(resize_check(cx.entity().downgrade())),
            )
    }
}
