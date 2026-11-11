//! The **Expand chevron** (CONTEXT.md): the one control that steps a
//! list's preview by pointer. `›` while the Right key grows the preview
//! (Collapsed, Context), `‹` in Full, where the Left key shrinks it. A
//! drawer list draws it over the right end of its selected row and of the
//! hovered row ([`DrawerList::set_chevron`](super::drawer_list::DrawerList::set_chevron));
//! the open preview carries it on its one-line header ([`preview_header`]).
//! Both draw the one zone of [`chevron`]: a strip at the right end, as tall
//! as the row (or the header line), set off by a thin divider on its left
//! edge, the glyph centred in it; the whole zone takes the click. As the
//! zone would cover the right end of every line, each text line of a row
//! stays clear of it while it is there ([`clear_of_zone_while_shown`]), so a
//! score and the `…` of a cut line stay in sight; the header's title stays
//! [`clear_of_zone`]. It is a pointer shortcut only: the keys stay as they
//! are.
//!
//! A view with such a preview (FIND, SEM, the Sources view) takes its steps
//! from [`PreviewSteps`], keeps its rows' chevron in step with
//! [`sync_chevron`] and draws its header with [`step_header`].

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Rgba, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Window, div, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::settings::themes::Theme;

use super::drawer_list::{DrawerList, DrawerRow, ROW_GROUP, chevron_id};
use super::files::ellipsis;
use crate::theme;

/// The group a preview header carries, for its chevron's hover.
const HEADER_GROUP: &str = "preview-header";

/// Which way a click on the chevron steps the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Collapsed or Context: Right grows it.
    Grow,
    /// Full: Left shrinks it.
    Shrink,
}

impl Reach {
    /// The step `model` takes next: [`Reach::Shrink`] in Full, else
    /// [`Reach::Grow`].
    pub(crate) fn of(model: &PreviewModel) -> Self {
        if model.is_full() {
            Self::Shrink
        } else {
            Self::Grow
        }
    }

    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Grow => "›",
            Self::Shrink => "‹",
        }
    }

    /// What the step does and its key. Also the chevron's accessible name.
    pub(crate) fn tooltip(self) -> &'static str {
        match self {
            Self::Grow => "Expand the preview (→)",
            Self::Shrink => "Collapse the preview (←)",
        }
    }
}

/// The zone's width, in rems. It runs the full height of what carries it
/// (a row, the preview header).
pub(crate) const ZONE_REMS: f32 = 1.5;

/// What the zone lies over: its look depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZoneOn {
    /// The selected row.
    SelectedRow,
    /// A row that is not selected (the zone shows only while it is
    /// hovered).
    HoveredRow,
    /// The open preview's header, which has no fill of its own.
    Header,
}

impl ZoneOn {
    /// The row a list draws: selected or not.
    pub(crate) fn row(selected: bool) -> Self {
        if selected {
            Self::SelectedRow
        } else {
            Self::HoveredRow
        }
    }

    /// The fill under the zone, as the row frame draws it
    /// ([`row_frame`](super::files::row_frame)): `None` on the header.
    pub(crate) fn fill(self, t: &Theme) -> Option<Rgba> {
        match self {
            Self::SelectedRow => Some(theme::selected_background(t)),
            Self::HoveredRow => Some(theme::hover_background(t)),
            Self::Header => None,
        }
    }
}

/// The colours the zone draws with: a thin divider on its left edge and
/// the glyph in the dim text colour at rest; a faint fill and the text
/// colour for the glyph under the pointer.
///
/// On a row, the zone is opaque in the row's own fill ([`ZoneOn::fill`]):
/// it looks unfilled, yet masks the row's text that runs on beneath it (a
/// context line nearly always does). Its hover fill is that fill tinted
/// toward the text, opaque as well. The header has no fill and nothing
/// under its zone (its title stops short, [`clear_of_zone`]), so there the
/// zone has no rest fill and a see-through hover tint.
///
/// The divider is the text colour, see-through, over the zone's fill. The
/// theme's own divider colour cannot do the job: in nine built-in themes it
/// is the very colour of a hovered row. The strengths below keep the
/// divider and the hover fill clear of the row fill they are drawn on in
/// every built-in theme (`zone_reads_on_every_row`) and no stronger: a
/// quiet line, a faint fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChevronLook {
    pub(crate) fill: Option<Rgba>,
    pub(crate) divider: Rgba,
    pub(crate) glyph: Rgba,
    pub(crate) hover_fill: Rgba,
    pub(crate) hover_glyph: Rgba,
}

/// How opaque the text colour is in the zone's divider.
const DIVIDER_ALPHA: f32 = 0.2;
/// ...in its fill under the pointer.
const HOVER_FILL_ALPHA: f32 = 0.1;

impl ChevronLook {
    pub(crate) fn of(t: &Theme, on: ZoneOn) -> Self {
        let text = theme::foreground(t);
        let tint = |a: f32| Rgba {
            a: text.a * a,
            ..text
        };
        let fill = on.fill(t);
        Self {
            fill,
            divider: tint(DIVIDER_ALPHA),
            glyph: theme::secondary(t),
            hover_fill: match fill {
                Some(under) => theme::mix_rgba(under, text, HOVER_FILL_ALPHA * text.a),
                None => tint(HOVER_FILL_ALPHA),
            },
            hover_glyph: text,
        }
    }
}

/// The chevron itself, `id` its element id, over `on`: a zone [`ZONE_REMS`]
/// wide laid over the right end of what carries it, flush with that end
/// and as tall as it, a thin divider on its left edge, the glyph centred in
/// it ([`ChevronLook`]). On a row its right corners round as the row's do.
/// The whole zone is the click target; under the pointer it fills faintly,
/// and its tooltip names the step. The carrier is `relative`; the caller
/// takes the click and adds `test_support`.
pub(crate) fn chevron(id: SharedString, reach: Reach, on: ZoneOn, t: &Theme) -> Stateful<Div> {
    let look = ChevronLook::of(t, on);
    let tip = reach.tooltip();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(tip)
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .w(rems(ZONE_REMS))
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(look.divider)
        .when(on != ZoneOn::Header, |zone| zone.rounded_r_md())
        .when_some(look.fill, |zone, fill| zone.bg(fill))
        .text_sm()
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(look.glyph)
        .hover(move |style| chevron_hover(look, style))
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        .child(reach.glyph())
}

/// The zone under the pointer: `style` with the faint fill and the text
/// colour of `look`.
pub(crate) fn chevron_hover(look: ChevronLook, style: StyleRefinement) -> StyleRefinement {
    style.bg(look.hover_fill).text_color(look.hover_glyph)
}

/// `line` kept clear of the zone: its right end stops short of where the
/// zone lies, so what it ends with (a score) is never covered.
pub(crate) fn clear_of_zone<E: Styled>(line: E) -> E {
    line.pr(rems(ZONE_REMS))
}

/// `line` of a list row kept clear of the zone while the zone is there: a
/// margin as wide as the zone on the selected row (`selected`) and, through
/// the row's hover group ([`ROW_GROUP`]), on the hovered one. An idle row's
/// lines keep their whole width. The margin (not padding) puts the line's
/// own edge, where it clips and ends in an ellipsis, left of the zone, so a
/// line that is cut short keeps its `…` in sight.
pub(crate) fn clear_of_zone_while_shown<E: Styled + gpui_kit::InteractiveElement>(
    line: E,
    selected: bool,
) -> E {
    let line = if selected {
        line.mr(rems(ZONE_REMS))
    } else {
        line
    };
    line.group_hover(ROW_GROUP, |style| style.mr(rems(ZONE_REMS)))
}

/// The open preview's one-line header: `title` on the left, cut short with
/// an ellipsis before the zone, the chevron's zone at the right end (its
/// id [`chevron_id`] of `id`). The caller adds `on_click` (the whole line
/// steps the preview), then `test_support`.
pub(crate) fn preview_header(
    id: SharedString,
    title: SharedString,
    reach: Reach,
    t: &Theme,
) -> Stateful<Div> {
    let chevron_id = chevron_id(&id);
    let hover = theme::foreground(t);
    clear_of_zone(
        div()
            .id(id)
            .aria_label(title.clone())
            .group(HEADER_GROUP)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .pl_2()
            .py_0p5()
            .text_xs()
            .cursor_pointer(),
    )
    .child(ellipsis(title).flex_1().text_color(theme::muted(t)))
    .child(
        chevron(chevron_id, reach, ZoneOn::Header, t)
            .group_hover(HEADER_GROUP, move |style| style.text_color(hover))
            .test_support(),
    )
}

/// A view whose preview of its selected row grows Collapsed → Context →
/// Full: the steps the keys and the chevron take, written once. The view
/// keeps its [`PreviewModel`] and draws its preview.
pub(crate) trait PreviewSteps: Sized + 'static {
    /// How much of the selected row shows.
    fn steps_model(&mut self) -> &mut PreviewModel;
    /// The selected row's note.
    fn steps_path(&self, cx: &App) -> Option<VaultPath>;
    /// The model changed: show what it says.
    fn steps_sync(&mut self, cx: &mut Context<Self>);
    /// Give the view's list the keyboard.
    fn steps_focus(&self, window: &mut Window, cx: &mut Context<Self>);

    /// Right: Collapsed → Context → Full (Full stays).
    fn grow(&mut self, cx: &mut Context<Self>) {
        if !self.steps_model().is_full() {
            let path = self.steps_path(cx);
            self.steps_model().toggle(path);
        }
        self.steps_sync(cx);
        cx.notify();
    }

    /// Left: Full → Context → Collapsed.
    fn shrink(&mut self, cx: &mut Context<Self>) {
        let path = self.steps_path(cx);
        self.steps_model().collapse_step(path);
        self.steps_sync(cx);
        cx.notify();
    }

    /// Space: Collapsed → Context → Full → Collapsed.
    fn cycle(&mut self, cx: &mut Context<Self>) {
        let path = self.steps_path(cx);
        self.steps_model().toggle(path);
        self.steps_sync(cx);
        cx.notify();
    }

    /// A click on a chevron (a row's or the header's): the step its
    /// [`Reach`] names, Full shrinking as Left does, else growing as Right
    /// does; the list has the keyboard after, so the arrows the tooltip
    /// names keep working.
    fn chevron_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match Reach::of(self.steps_model()) {
            Reach::Shrink => self.shrink(cx),
            Reach::Grow => self.grow(cx),
        }
        self.steps_focus(window, cx);
    }
}

/// The rows' chevron of `list` follows `model`: `›` until Full, `‹` there.
pub(crate) fn sync_chevron<R: DrawerRow>(
    list: &Entity<DrawerList<R>>,
    model: &PreviewModel,
    cx: &mut App,
) {
    let reach = Reach::of(model);
    list.update(cx, |list, cx| list.set_chevron(Some(reach), cx));
}

/// The [`preview_header`] of a [`PreviewSteps`] view, `id` its element id
/// and `title` its text: a click on it steps the preview
/// ([`PreviewSteps::chevron_step`]).
pub(crate) fn step_header<V: PreviewSteps>(
    id: &'static str,
    title: SharedString,
    model: &PreviewModel,
    t: &Theme,
    cx: &Context<V>,
) -> impl IntoElement + use<V> {
    preview_header(id.into(), title, Reach::of(model), t)
        .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
            // A double click steps once, with its first press, as the
            // row's chevron does.
            if event.standard_click() && event.click_count() == 1 {
                this.chevron_step(window, cx);
            }
        }))
        .test_support()
}
