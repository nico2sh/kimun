//! The GUI's **editor region** (ADR-0058): a row of **tabs**, each a
//! workspace of **editor groups** laid out as a `Node` tree. Toolkit-free.
//!
//! Rules: a tab always holds at least one group and the region at least one
//! tab. Closing the last group of a tab closes the tab; closing the last
//! group of the last tab keeps that group and reports
//! [`CloseOutcome::Emptied`]. Group ids come from one monotonic counter
//! shared by all tabs and are never reused.

use crate::logic::layout::node::Node;
use crate::logic::layout::{Axis, GroupId, UnitRect};

/// Identifies one tab. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u32);

/// What [`EditorRegion::close_group`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The group closed; its tab remains.
    Group,
    /// It was the last group of its tab, so the tab closed too.
    Tab(TabId),
    /// It was the last group of the last tab: the group stays, to be shown
    /// empty.
    Emptied,
}

#[derive(Debug, Clone, PartialEq)]
struct Tab {
    id: TabId,
    root: Node<GroupId>,
    active: GroupId,
}

/// The tabs, which one is shown, and the groups of each.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorRegion {
    tabs: Vec<Tab>,
    shown: TabId,
    next_tab: u32,
    next_group: u32,
}

impl Default for EditorRegion {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorRegion {
    /// One tab holding one group.
    pub fn new() -> Self {
        let first = Tab {
            id: TabId(0),
            root: Node::leaf(GroupId::new(0)),
            active: GroupId::new(0),
        };
        Self {
            shown: first.id,
            tabs: vec![first],
            next_tab: 1,
            next_group: 1,
        }
    }

    fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn position(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn shown_tab(&self) -> &Tab {
        self.tab(self.shown).expect("the shown tab exists")
    }

    fn fresh_group(&mut self) -> GroupId {
        let g = GroupId::new(self.next_group);
        self.next_group += 1;
        g
    }

    fn fresh_tab(&mut self) -> TabId {
        let t = TabId(self.next_tab);
        self.next_tab += 1;
        t
    }

    // ── Tabs ──────────────────────────────────────────────────────────

    /// Every tab in order.
    pub fn tabs(&self) -> Vec<TabId> {
        self.tabs.iter().map(|t| t.id).collect()
    }

    /// The tab on display.
    pub fn shown(&self) -> TabId {
        self.shown
    }

    /// Show `tab`. False (nothing changes) when it does not exist.
    pub fn show(&mut self, tab: TabId) -> bool {
        if self.position(tab).is_none() {
            return false;
        }
        self.shown = tab;
        true
    }

    fn step(&mut self, delta: usize) {
        let at = self.position(self.shown).expect("the shown tab exists");
        self.shown = self.tabs[(at + delta) % self.tabs.len()].id;
    }

    /// Show the next tab, wrapping to the first.
    pub fn next_tab(&mut self) {
        self.step(1);
    }

    /// Show the previous tab, wrapping to the last.
    pub fn prev_tab(&mut self) {
        self.step(self.tabs.len() - 1);
    }

    /// Open a tab with one new group after the last tab, and show it.
    pub fn new_tab(&mut self) -> (TabId, GroupId) {
        let (id, group) = (self.fresh_tab(), self.fresh_group());
        self.tabs.push(Tab {
            id,
            root: Node::leaf(group),
            active: group,
        });
        self.shown = id;
        (id, group)
    }

    /// Close `tab`, returning the groups it held in reading order. `None`
    /// (nothing changes) for the last tab or an unknown one. Closing the
    /// shown tab shows the one that takes its place (the next, else the
    /// previous).
    pub fn close_tab(&mut self, tab: TabId) -> Option<Vec<GroupId>> {
        if self.tabs.len() == 1 {
            return None;
        }
        let at = self.position(tab)?;
        let closed = self.tabs.remove(at);
        if self.shown == tab {
            self.shown = self.tabs[at.min(self.tabs.len() - 1)].id;
        }
        Some(closed.root.leaves().into_iter().copied().collect())
    }

    /// Move `tab` to position `to` (clamped to the last). The shown tab is
    /// unchanged. Ignored for an unknown tab.
    pub fn move_tab(&mut self, tab: TabId, to: usize) {
        let Some(at) = self.position(tab) else {
            return;
        };
        let moved = self.tabs.remove(at);
        self.tabs.insert(to.min(self.tabs.len()), moved);
    }

    // ── Groups ────────────────────────────────────────────────────────

    /// The groups of `tab` in reading order; empty for an unknown tab.
    pub fn groups(&self, tab: TabId) -> Vec<GroupId> {
        self.tab(tab)
            .map(|t| t.root.leaves().into_iter().copied().collect())
            .unwrap_or_default()
    }

    /// Every group of every tab, tab by tab.
    pub fn all_groups(&self) -> Vec<GroupId> {
        self.tabs
            .iter()
            .flat_map(|t| t.root.leaves().into_iter().copied())
            .collect()
    }

    /// The shown tab's active group: the one owning save and the footer.
    pub fn active_group(&self) -> GroupId {
        self.shown_tab().active
    }

    /// The tab holding `group`.
    pub fn group_tab(&self, group: GroupId) -> Option<TabId> {
        self.tabs
            .iter()
            .find(|t| t.root.contains(&group))
            .map(|t| t.id)
    }

    /// Make `group` the active group of its tab and show that tab. False
    /// (nothing changes) for an unknown group.
    pub fn set_active_group(&mut self, group: GroupId) -> bool {
        let Some(tab) = self.group_tab(group) else {
            return false;
        };
        let at = self.position(tab).expect("the tab exists");
        self.tabs[at].active = group;
        self.shown = tab;
        true
    }

    /// Split `group` along `axis`; the new group comes right after it, in
    /// the same tab. The active group is unchanged. `None` for an unknown
    /// group.
    pub fn split_group(&mut self, group: GroupId, axis: Axis) -> Option<GroupId> {
        let tab = self.group_tab(group)?;
        let new = self.fresh_group();
        let at = self.position(tab).expect("the tab exists");
        self.tabs[at].root.split_leaf(&group, axis, new);
        Some(new)
    }

    /// Close `group`. `None` for an unknown group. When the active group of
    /// its tab closes, the group before it (or the first) becomes active.
    pub fn close_group(&mut self, group: GroupId) -> Option<CloseOutcome> {
        let tab = self.group_tab(group)?;
        let at = self.position(tab).expect("the tab exists");
        if self.tabs[at].root.leaves().len() == 1 {
            return Some(if self.close_tab(tab).is_some() {
                CloseOutcome::Tab(tab)
            } else {
                CloseOutcome::Emptied
            });
        }
        self.remove_from(at, group);
        Some(CloseOutcome::Group)
    }

    /// Take `group` out of the tab at index `at` (which holds others),
    /// moving that tab's active group off it if needed.
    fn remove_from(&mut self, at: usize, group: GroupId) {
        let t = &mut self.tabs[at];
        let pos = t
            .root
            .leaves()
            .into_iter()
            .position(|g| *g == group)
            .expect("the group is in the tab");
        t.root.remove_leaf(&group);
        if t.active == group {
            let left = t.root.leaves();
            t.active = *left[pos.saturating_sub(1).min(left.len() - 1)];
        }
    }

    // ── Geometry ──────────────────────────────────────────────────────

    /// The shown tab's groups with their rectangles in the unit square.
    pub fn regions(&self) -> Vec<(GroupId, UnitRect)> {
        self.shown_tab()
            .root
            .layout(UnitRect::new(0.0, 0.0, 1.0, 1.0))
    }

    /// Move the boundary after `group`'s branch so it takes `share` of the
    /// pair (see `Node::resize_after`). False when there is none.
    pub fn resize_after(&mut self, group: GroupId, share: f64) -> bool {
        let Some(tab) = self.group_tab(group) else {
            return false;
        };
        let at = self.position(tab).expect("the tab exists");
        self.tabs[at].root.resize_after(&group, share)
    }

    // ── Moving groups ─────────────────────────────────────────────────

    /// Move `group` (keeping its id) into `target` as a new split on the
    /// right of the target tab's active group, where it becomes active. If
    /// it was alone in its tab, that tab closes; and if that was the shown
    /// tab, `target` is shown. False (nothing changes) for an unknown group
    /// or tab, or when `group` is already in `target`.
    pub fn move_group_to_tab(&mut self, group: GroupId, target: TabId) -> bool {
        let Some(from) = self.group_tab(group) else {
            return false;
        };
        if from == target || self.position(target).is_none() {
            return false;
        }
        let from_at = self.position(from).expect("the tab exists");
        let was_shown = self.shown == from;
        if self.tabs[from_at].root.leaves().len() == 1 {
            self.tabs.remove(from_at);
            if was_shown {
                self.shown = target;
            }
        } else {
            self.remove_from(from_at, group);
        }
        let at = self.position(target).expect("checked above");
        let t = &mut self.tabs[at];
        let after = t.active;
        t.root.split_leaf(&after, Axis::Horizontal, group);
        t.active = group;
        true
    }
}

#[cfg(test)]
mod tests;
