//! Where a view goes: the one entry every mouse, chooser and keyboard
//! placement ends in (ADR-0054 §1, §4, §5, §8). Each caller keeps its own
//! guards (a drop checks [`Workspace::drop_target_allowed`], a chooser pick
//! that the layout did not change under it); what they share is
//! [`Workspace::place`], so a placement is dispatched in one spot and the
//! steps behind it ([`Workspace::move_into`], [`Workspace::spawn_beside`],
//! [`Workspace::spawn_at`]) are reached from nowhere else.

use gpui_kit::{Context, Window};
use kimun_notes::logic::layout::{Dir, Side};
use kimun_notes::logic::layout_tree::{Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;

use super::Workspace;

/// Where a view is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Shown in this slot. A view shown in another slot trades places with
    /// the view shown here; any other view is placed here, the view shown
    /// here staying placed, hidden ([`Workspace::move_into`]).
    Into(SlotId),
    /// Shown in this slot, the view shown there leaving the layout
    /// (unplaced, back to the rail only): a rail drag's centre.
    Replace(SlotId),
    /// In a new slot on this side of this slot.
    Beside(SlotId, Dir),
    /// At the window edge on this side: that side's first slot, else a new
    /// slot at the edge of the outermost editor group.
    AtEdge(Side),
}

impl Workspace {
    /// Place `view` at `placement`, with the focus, settling and saving of
    /// the step behind it. True when dealt with (a placement that found no
    /// room says so in a flash and counts); false, nothing changed, when
    /// the target is not in the layout or the move is pointless there
    /// (zoomed; a view alone in its slot placed beside it).
    pub(super) fn place(
        &mut self,
        view: DrawerView,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match placement {
            Placement::Into(slot) => self.move_into(view, slot, window, cx),
            Placement::Replace(slot) => self.replace_into(view, slot, window, cx),
            Placement::Beside(slot, dir) => {
                self.spawn_beside(view, slot, dir, window, cx).handled()
            }
            Placement::AtEdge(side) => match self.window_edge_slot(side) {
                Some(slot) => self.move_into(view, slot, window, cx),
                None => match self.layout.edge_group(side) {
                    Some(group) => self
                        .spawn_at(view, Leaf::Group(group), side.dir(), window, cx)
                        .handled(),
                    None => false,
                },
            },
        }
    }
}
