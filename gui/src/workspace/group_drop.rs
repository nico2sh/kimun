//! Dragging an editor group by its header (ADR-0058). The drag carries a
//! [`GroupDrag`] through the drop machinery of [`super::drop_zones`] (the
//! live drag, the outcomes worked out once per drag, the zone look, the
//! hover label, Esc and the modal ending it). Its targets:
//!
//! - the **inner layer**: the centre and four edges over the body of every
//!   other group of the shown tab ([`Workspace::group_zone_layer`]). An
//!   edge puts the dragged group on that side of the group
//!   ([`EditorRegion::move_group_beside`]); the centre swaps the two
//!   groups' places ([`EditorRegion::swap_groups`]);
//! - a tab's label: the group joins that tab, a split right of its active
//!   group ([`EditorRegion::move_group_to_tab`]);
//! - the plus button: the group goes alone to a new tab
//!   ([`EditorRegion::move_group_to_new_tab`]).
//!
//! [`GroupDrag`]: super::drop_zones::GroupDrag
//!
//! The group moves whole, keeping its id, so its state (`groups`), its note
//! and its unsaved text go with it: nothing is written, nothing is added or
//! dropped from `groups`. A group alone in its tab leaves no empty group
//! behind: its tab closes. The tab strip shows during the drag even with
//! one tab, so the plus button is there to aim at.
//!
//! [`EditorRegion::move_group_beside`]: kimun_notes::logic::editor_region::EditorRegion::move_group_beside
//! [`EditorRegion::swap_groups`]: kimun_notes::logic::editor_region::EditorRegion::swap_groups
//! [`EditorRegion::move_group_to_tab`]: kimun_notes::logic::editor_region::EditorRegion::move_group_to_tab
//! [`EditorRegion::move_group_to_new_tab`]: kimun_notes::logic::editor_region::EditorRegion::move_group_to_new_tab

use gpui_kit::{Context, Window};
use kimun_notes::logic::layout_tree::GroupId;

use super::Workspace;
use super::drop_zones::{Dragged, DropOutcome, DropTarget, Zone, group_zone_id};
use super::modal;

impl Workspace {
    /// Every target a group's drag may have, in drawing order: the five
    /// zones of each group of the shown tab, each tab's label, the plus
    /// button. Not filtered by what a drop would change
    /// ([`Self::group_drop_allowed`]).
    pub(super) fn group_drop_targets(&self) -> Vec<DropTarget> {
        let region = self.layout.region();
        let zones = region
            .groups(region.shown())
            .into_iter()
            .flat_map(|group| Zone::ALL.map(|zone| DropTarget::Group(group, zone)));
        let tabs = region.tabs().into_iter().map(DropTarget::Tab);
        zones
            .chain(tabs)
            .chain(std::iter::once(DropTarget::NewTab))
            .collect()
    }

    /// What dropping `group` on `target` would do; `None` where it would
    /// change nothing (the zone is then not drawn).
    pub(super) fn group_drop_outcome(
        &self,
        group: GroupId,
        target: DropTarget,
    ) -> Option<DropOutcome> {
        if !self.group_drop_allowed(group, target) {
            return None;
        }
        match target {
            DropTarget::Group(_, Zone::Centre) => Some(DropOutcome::SwapGroups),
            DropTarget::Group(_, Zone::Edge(dir)) => Some(DropOutcome::SplitGroup(dir)),
            DropTarget::Tab(_) => Some(DropOutcome::IntoTab),
            DropTarget::NewTab => Some(DropOutcome::IntoNewTab),
            DropTarget::Slot(..) | DropTarget::Window(_) => None,
        }
    }

    /// Whether dropping `group` on `target` changes anything: not while
    /// zoomed, while a tab's or a group's close writes its notes, or while
    /// the window closes; not on the group itself, nor on a group of a tab
    /// not shown; not on the label of the tab it is in; and not on the plus
    /// button for the only group of the only tab. A slot's zones and the
    /// window-edge strips are a view's.
    pub(crate) fn group_drop_allowed(&self, group: GroupId, target: DropTarget) -> bool {
        if self.layout.zoomed().is_some() || self.arrangement_locked() {
            return false;
        }
        let region = self.layout.region();
        let Some(from) = region.group_tab(group) else {
            return false;
        };
        match target {
            DropTarget::Group(other, _) => {
                other != group && region.group_tab(other) == Some(region.shown())
            }
            DropTarget::Tab(tab) => tab != from && region.tabs().contains(&tab),
            DropTarget::NewTab => region.tabs().len() > 1 || region.groups(from).len() > 1,
            DropTarget::Slot(..) | DropTarget::Window(_) => false,
        }
    }

    /// Drop `group` on `target` (see the module doc). After a drop on a
    /// group the dragged group is active with the keyboard; after a drop on
    /// the strip the shown tab stays (unless the group left it empty: then
    /// the tab it went to shows) and its active group takes the keyboard.
    /// False, nothing changed, when [`Self::group_drop_allowed`] says so.
    pub(crate) fn drop_group(
        &mut self,
        group: GroupId,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.group_drop_allowed(group, target) || modal::dialog_open(window, cx) {
            return false;
        }
        let region = self.layout.region_mut();
        let moved = match target {
            DropTarget::Group(other, Zone::Centre) => region.swap_groups(group, other),
            DropTarget::Group(other, Zone::Edge(dir)) => {
                region.move_group_beside(group, other, dir)
            }
            DropTarget::Tab(tab) => region.move_group_to_tab(group, tab),
            DropTarget::NewTab => region.move_group_to_new_tab(group).is_some(),
            DropTarget::Slot(..) | DropTarget::Window(_) => false,
        };
        if !moved {
            return false;
        }
        let focus = match target {
            DropTarget::Group(..) => group,
            _ => self.layout.active_group(),
        };
        self.focus_group(focus, cx);
        self.scroll_to_shown_tab();
        cx.notify();
        true
    }

    /// The inner layer over the body of the group at `at` (`group`) while a
    /// group is dragged: its centre and four edges, laid out as a slot's
    /// ([`Self::zone_grid`]). `None` while no group is dragged; a zone where
    /// the drop would change nothing (the dragged group's own) is left out.
    pub(super) fn group_zone_layer(
        &self,
        at: usize,
        group: GroupId,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::Div> {
        let Some(Dragged::Group(_)) = self.zones_for(cx) else {
            return None;
        };
        Some(self.zone_grid(
            |zone| DropTarget::Group(group, zone),
            |zone| group_zone_id(at, zone),
            cx,
        ))
    }
}
