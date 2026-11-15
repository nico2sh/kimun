//! The one rule for every focus signal (ADR-0060, decision 11). A thing that
//! can look focused is a [`FocusTarget`]. [`Workspace::focus_targets`]
//! lists them all, [`Workspace::is_focused`] says when the model counts one
//! as focused, and [`Workspace::focus_amount`] is how lit it is now, from 0
//! to 1. A signal draws its color with [`Workspace::focus_color`]: its
//! resting color mixed toward its lit color by that amount.
//!
//! A signal that is always drawn mixes through `focus_color`. One that is
//! drawn only while it shows takes `focus_accent`. Nothing else mixes.
//!
//! A new focus signal is one entry in the list, one line in the table, and
//! one call to one of the two helpers.

use gpui_kit::Rgba;
use kimun_notes::logic::layout_tree::{Focus, GroupId, Leaf};
use kimun_notes::logic::nav::DrawerView;

use super::MotionKey;
use crate::theme;
use crate::workspace::Workspace;

/// A thing that can look focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::workspace) enum FocusTarget {
    /// The rail, the editor, or a slot: the border of a surface, the title
    /// of a slot, the bar of the rail.
    Surface(Focus),
    /// The title of an editor group's header.
    Title(GroupId),
    /// The target marker over an editor group.
    Marker(GroupId),
    /// The keyboard cursor over a view's cell in the rail.
    RailCursor(DrawerView),
}

impl Workspace {
    /// Everything that can look focused, now: each one has a focus amount in
    /// the store.
    pub(in crate::workspace) fn focus_targets(&self) -> Vec<FocusTarget> {
        let surfaces = [Focus::Rail, Focus::Leaf(Leaf::Editor)]
            .into_iter()
            .chain(
                self.layout
                    .slots()
                    .into_iter()
                    .map(|slot| Focus::Leaf(Leaf::Slot(slot))),
            )
            .map(FocusTarget::Surface);
        // The groups of every tab, not only the shown one: a switch of the
        // shown tab must not fade anything.
        let groups = self.layout.region().all_groups();
        let titles = groups.iter().copied().map(FocusTarget::Title);
        let markers = groups.iter().copied().map(FocusTarget::Marker);
        let cursors = self
            .rail_views()
            .iter()
            .copied()
            .map(FocusTarget::RailCursor);
        surfaces
            .chain(titles)
            .chain(markers)
            .chain(cursors)
            .collect()
    }

    /// Whether the layout model counts `target` as focused. The one table.
    pub(in crate::workspace) fn is_focused(&self, target: FocusTarget) -> bool {
        let editor_has_the_keyboard = self.layout.focused() == Focus::Leaf(Leaf::Editor);
        match target {
            FocusTarget::Surface(focus) => self.layout.focused() == focus,
            FocusTarget::Title(group) => {
                editor_has_the_keyboard && self.is_active_in_its_tab(group)
            }
            FocusTarget::Marker(group) => {
                let region = self.layout.region();
                !editor_has_the_keyboard
                    && self.layout.zoomed().is_none()
                    && self.is_active_in_its_tab(group)
                    && region
                        .group_tab(group)
                        .is_some_and(|tab| region.groups(tab).len() > 1)
            }
            FocusTarget::RailCursor(view) => {
                self.layout.focused() == Focus::Rail && self.rail_cursor_view() == Some(view)
            }
        }
    }

    /// Whether `group` is the active group of the tab that holds it. A hidden
    /// tab counts too, so the shown tab can change without anything fading.
    fn is_active_in_its_tab(&self, group: GroupId) -> bool {
        let region = self.layout.region();
        region
            .group_tab(group)
            .and_then(|tab| region.active_group_of(tab))
            == Some(group)
    }

    /// How lit `target` is, from 0 to 1, as the last frame drew it. A target
    /// the store does not hold yet is lit fully if the model focuses it, and
    /// not at all if not.
    pub(in crate::workspace) fn focus_amount(&self, target: FocusTarget) -> f32 {
        match self.motion.borrow().try_presence(MotionKey::Focus(target)) {
            Some(presence) => presence.frame,
            None if self.is_focused(target) => 1.0,
            None => 0.0,
        }
    }

    /// The accent at a part of its alpha, by how lit `target` is: `None`
    /// while the target shows no focus at all. A signal that is drawn only
    /// while it shows (a bar, a ring, a marker) takes its color from here.
    pub(in crate::workspace) fn focus_accent(
        &self,
        target: FocusTarget,
        cx: &gpui_kit::App,
    ) -> Option<Rgba> {
        let amount = self.focus_amount(target);
        (amount > 0.0).then(|| {
            let accent = theme::focus_border(&theme::current(cx));
            theme::fade_between(Rgba { a: 0.0, ..accent }, accent, amount)
        })
    }

    /// `rest` mixed toward `lit` by how lit `target` is. Every focus signal
    /// draws its color through this.
    pub(in crate::workspace) fn focus_color(
        &self,
        rest: Rgba,
        lit: Rgba,
        target: FocusTarget,
    ) -> Rgba {
        theme::fade_between(rest, lit, self.focus_amount(target))
    }
}
