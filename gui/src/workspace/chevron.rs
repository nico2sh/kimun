//! The **Expand chevron** (CONTEXT.md): the one control that steps a
//! list's preview by pointer. `›` while the Right key grows the preview
//! (Collapsed, Context), `‹` in Full, where the Left key shrinks it. A
//! drawer list draws it at the right end of its selected row and of the
//! hovered row ([`DrawerList::set_chevron`](super::drawer_list::DrawerList::set_chevron));
//! the open preview carries it on its one-line header ([`preview_header`]).
//! It is a pointer shortcut only: the keys stay as they are.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Div, InteractiveElement as _, ParentElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::settings::themes::Theme;

use super::drawer_list::chevron_id;
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

/// The chevron itself, `id` its element id: quiet at rest, bright under the
/// pointer, its tooltip naming the step. The caller places it, takes its
/// click and adds `test_support`.
pub(crate) fn chevron(id: SharedString, reach: Reach, t: &Theme) -> Stateful<Div> {
    let hover = theme::foreground(t);
    let tip = reach.tooltip();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(tip)
        .flex()
        .flex_none()
        .items_center()
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(theme::secondary(t))
        .hover(move |style| style.text_color(hover))
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
                .text_sm()
                .group_hover(HEADER_GROUP, move |style| style.text_color(hover))
                .test_support(),
        )
}
