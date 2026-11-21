//! The gap rows of the facet view.
//!
//! A gap with hidden blocks draws a row above the crumbs of the piece under
//! it, or after the last piece of a note. Its up row sits under the crumbs,
//! right above the text. Both draw their stops in paint order.

use gpui_kit::assets::IconName;
use gpui_kit::base::{ObservedElement, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_core::note::GAP_SHOW_ALL_MAX;
use kimun_notes::settings::themes::Theme;

use super::body::count_in_up_row;
use super::keys::Stop;
use super::rows;
use super::{FacetView, gap_count_id, gap_down_id, gap_up_id, show_all_id};
use crate::theme;
use crate::workspace::facet_ui::{icon_chip, ring_room, text_button};

/// The icon of the down button: it shows the next block under the piece
/// above. The app's asset source carries it ([`crate::assets`]).
pub(crate) const GAP_DOWN_ICON: IconName = IconName::ArrowDownToLine;

/// The icon of the up button: it shows the block above the piece under it.
/// The two icons point in opposite ways, so the shape says which way each
/// button goes, not only the tooltip.
pub(crate) const GAP_UP_ICON: IconName = IconName::ArrowUpToLine;

impl FacetView {
    /// The row of gap `g` ([`mod@super::members`]) that is its own item of
    /// the column: above the crumbs of the piece under it, or after the
    /// last piece of a note. It holds the down button, the count and `Show
    /// all`. `None` when the gap hides nothing or has no block above it to
    /// extend. A `pulled` row moves up toward the piece above it by
    /// [`rows::gap_pull`]: it adds text to that piece. The height table
    /// counts the same pull ([`mod@super::window`]).
    pub(super) fn render_gap(
        &self,
        g: usize,
        pulled: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let gap = *self.members.gaps.get(g)?;
        let hidden = self.members.gap_hidden(g);
        if hidden == 0 || gap.above.is_none() {
            return None;
        }
        // Built in the order they are painted, so the stops track in Tab
        // order.
        let el = icon_chip(gap_down_id(g), GAP_DOWN_ICON, "Show next block", t);
        let down = self.button(Stop::GapDown(g), el, t, cx);
        let count = gap_count(g, hidden);
        let show_all = self.show_all(g, hidden, t, cx);
        Some(
            gap_row(t)
                .when(pulled, |row| row.mt(-rows::gap_pull()))
                .child(down)
                .child(count)
                .children(show_all),
        )
    }

    /// The up row of gap `g`, between the crumbs and the text of the piece
    /// under it. It holds the up button. When the gap has no block above it,
    /// the count and `Show all` join it ([`count_in_up_row`]). `None` when
    /// the gap hides nothing.
    pub(super) fn render_up_row(&self, g: usize, t: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        let gap = *self.members.gaps.get(g)?;
        let hidden = self.members.gap_hidden(g);
        if hidden == 0 || gap.below.is_none() {
            return None;
        }
        let el = icon_chip(gap_up_id(g), GAP_UP_ICON, "Show previous block", t);
        let up = self.button(Stop::GapUp(g), el, t, cx);
        let (count, show_all) = match count_in_up_row(gap.above) {
            true => (Some(gap_count(g, hidden)), self.show_all(g, hidden, t, cx)),
            false => (None, None),
        };
        // The piece clips: the row keeps room for a focus ring at its edge,
        // as the crumbs row does.
        Some(
            gap_row(t)
                .p(ring_room())
                .pl(rows::spine())
                .child(up)
                .children(count)
                .children(show_all),
        )
    }

    /// Gap `g`'s `Show all`, when it hides few enough blocks.
    fn show_all(
        &self,
        g: usize,
        hidden: usize,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        (hidden <= GAP_SHOW_ALL_MAX).then(|| {
            self.button(
                Stop::ShowAll(g),
                text_button(show_all_id(g), "Show all", t),
                t,
                cx,
            )
        })
    }
}

/// A gap row's frame: one line of small muted text and buttons. Its first
/// button's edge starts at the column's one left edge ([`rows::spine`]).
fn gap_row(t: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .pl(rows::spine())
        .gap_2()
        .text_size(rows::small_text())
        .text_color(theme::muted(t))
}

/// Gap `g`'s count of `hidden` blocks.
fn gap_count(g: usize, hidden: usize) -> ObservedElement<Stateful<Div>> {
    let count = SharedString::from(match hidden {
        1 => "1 block hidden".to_string(),
        n => format!("{n} blocks hidden"),
    });
    div()
        .id(gap_count_id(g))
        .role(Role::Label)
        .aria_label(count.clone())
        .child(count)
        .test_support()
}
