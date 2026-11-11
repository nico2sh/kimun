//! The **Expand chevron** (CONTEXT.md): the one control that steps a
//! list's preview by pointer. `›` while the Right key grows the preview
//! (Collapsed, Context), `‹` in Full, where the Left key shrinks it. A
//! drawer list draws it over the right end of its selected row and of the
//! hovered row ([`DrawerList::set_chevron`](super::drawer_list::DrawerList::set_chevron));
//! the open preview carries it on its one-line header ([`preview_header`]).
//! Both draw the one compact bordered button of [`chevron`], so it reads as
//! clickable. It is a pointer shortcut only: the keys stay as they are.
//!
//! A view with such a preview (FIND, SEM, the Sources view) takes its steps
//! from [`PreviewSteps`], keeps its rows' chevron in step with
//! [`sync_chevron`] and draws its header with [`step_header`].

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, ClickEvent, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Rgba, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, div, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::settings::themes::Theme;

use super::drawer_list::{DrawerList, DrawerRow, chevron_id};
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

/// The chevron button's side, in rems: a compact square. Centred on a
/// three-line row (3.75 rem) it stays clear of the first line, where SEM
/// shows its score.
pub(crate) const CHEVRON_REMS: f32 = 1.25;

/// The colours the chevron button draws with, so it reads as clickable: a
/// subtle fill inside a hairline border at rest; a stronger fill and the
/// text colour for the glyph under the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChevronLook {
    pub(crate) fill: Rgba,
    pub(crate) border: Rgba,
    pub(crate) glyph: Rgba,
    pub(crate) hover_fill: Rgba,
    pub(crate) hover_glyph: Rgba,
}

impl ChevronLook {
    pub(crate) fn of(t: &Theme) -> Self {
        Self {
            fill: theme::hover_background(t),
            border: theme::divider(t),
            glyph: theme::secondary(t),
            hover_fill: theme::selected_background(t),
            hover_glyph: theme::foreground(t),
        }
    }
}

/// The chevron itself, `id` its element id: a compact bordered button
/// ([`CHEVRON_REMS`] square, [`ChevronLook`]), quiet at rest, stronger
/// under the pointer, its tooltip naming the step. The caller places it,
/// takes its click and adds `test_support`.
pub(crate) fn chevron(id: SharedString, reach: Reach, t: &Theme) -> Stateful<Div> {
    let look = ChevronLook::of(t);
    let tip = reach.tooltip();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(tip)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rems(CHEVRON_REMS))
        .rounded_md()
        .border_1()
        .border_color(look.border)
        .bg(look.fill)
        .text_sm()
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(look.glyph)
        .hover(move |style| style.bg(look.hover_fill).text_color(look.hover_glyph))
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        .child(reach.glyph())
}

/// The open preview's one-line header: `title` on the left, cut short with
/// an ellipsis, the chevron on the right (its id [`chevron_id`] of `id`).
/// The caller adds `on_click` (the whole line steps the preview), then
/// `test_support`.
pub(crate) fn preview_header(
    id: SharedString,
    title: SharedString,
    reach: Reach,
    t: &Theme,
) -> Stateful<Div> {
    let chevron_id = chevron_id(&id);
    let hover = theme::foreground(t);
    div()
        .id(id)
        .aria_label(title.clone())
        .group(HEADER_GROUP)
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .px_2()
        .py_0p5()
        .text_xs()
        .cursor_pointer()
        .child(ellipsis(title).flex_1().text_color(theme::muted(t)))
        .child(
            chevron(chevron_id, reach, t)
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
            if event.standard_click() {
                this.chevron_step(window, cx);
            }
        }))
        .test_support()
}
