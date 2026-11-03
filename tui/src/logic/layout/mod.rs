//! The editor screen's layout model (ADR-0050): which **Panels** exist, which
//! are visible, how each **drawer** is split into **drawer slots**, how the
//! editor Panel is split into **editor groups**, and where focus is.
//! Toolkit-free — widths are abstract units the front end maps to columns or
//! pixels, and geometry is in [`UnitRect`]s, never terminal cells.
//!
//! Panel order is rail → left drawer → editor → right drawer. The right
//! drawer starts hidden and empty, so the default layout is the TUI's
//! original fixed line.

mod node;

use node::Node;
pub use node::{Axis, UnitRect};

/// The most drawer slots one drawer holds. The node type allows more; the
/// model enforces this cap.
pub const MAX_DRAWER_SLOTS: usize = 2;

/// Decimal places [`PanelLayout::split_ratio`] reports. Ratios and widths are
/// `f64` throughout; the rounding keeps persisted ratios short and exact.
pub const RATIO_DECIMALS: i32 = 4;

/// Identifies a persistent panel by kind, without the drawer's side — the
/// TUI's view of the screen, which has one drawer. Closed set, mirrors
/// `OverlayKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelKind {
    /// The fixed-width activity rail on the far left.
    Rail,
    /// The single drawer panel; renders whichever rail view is active.
    Drawer,
    /// The editor; always visible, fills the remaining width.
    Editor,
}

impl PanelKind {
    /// Footer label shown when this panel is focused. The drawer's label
    /// depends on its active view — `PanelSet::focused_label()` resolves it.
    pub fn label(&self) -> &'static str {
        match self {
            PanelKind::Rail => "RAIL",
            PanelKind::Drawer => "DRAWER",
            PanelKind::Editor => "EDITOR",
        }
    }
}

/// Which side of the editor a drawer sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// A persistent panel in the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    Rail,
    Drawer(Side),
    Editor,
}

impl From<Panel> for PanelKind {
    fn from(panel: Panel) -> Self {
        match panel {
            Panel::Rail => PanelKind::Rail,
            Panel::Drawer(_) => PanelKind::Drawer,
            Panel::Editor => PanelKind::Editor,
        }
    }
}

/// Panels in their fixed left → right order.
const PANEL_ORDER: [Panel; 4] = [
    Panel::Rail,
    Panel::Drawer(Side::Left),
    Panel::Editor,
    Panel::Drawer(Side::Right),
];

/// Identifies one editor group. Stable across splits and closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupId(u32);

/// A focusable region: the rail, one drawer slot, or one editor group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    Rail,
    Slot { side: Side, index: usize },
    Group(GroupId),
}

impl Region {
    /// The panel this region belongs to.
    pub fn panel(&self) -> Panel {
        match self {
            Region::Rail => Panel::Rail,
            Region::Slot { side, .. } => Panel::Drawer(*side),
            Region::Group(_) => Panel::Editor,
        }
    }
}

/// A direction for geometric focus movement. `Ord` so it can parameterise
/// the ordered action vocabulary (`ActionShortcuts::FocusRegion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// What [`PanelLayout::focus_dir`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMove {
    /// Focus moved to this region.
    Moved(Region),
    /// A neighbour could exist in that direction but focus is at the edge;
    /// focus is unchanged and the key is handled (no wrap).
    AtEdge,
    /// Up/Down with nothing stacked in the focused column: focus is
    /// unchanged and the caller should let the key through.
    NotHandled,
}

/// Why a drawer-slot operation was refused. The layout is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    /// The drawer already holds [`MAX_DRAWER_SLOTS`] slots.
    SlotCap,
    /// No slot at that index in that drawer.
    NoSuchSlot,
    /// The view is already in a slot of that drawer.
    AlreadyInDrawer,
}

/// A front end's sizing floor for drawer width clamping, in its own units
/// (columns, pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidthLimits {
    /// Narrowest a drawer may be.
    pub drawer_min: f64,
    /// Width that must remain for the editor.
    pub editor_min: f64,
    /// Width shared by the drawers and the editor (the window minus the rail).
    pub available: f64,
}

struct Drawer<V> {
    slots: Option<Node<V>>,
    focused: usize,
    visible: bool,
    width: f64,
}

/// The layout state: panels, drawers with their slots, editor groups, focus.
/// Generic over the drawer view id `V`, so the layout never assumes a closed
/// set of views.
pub struct PanelLayout<V> {
    left: Drawer<V>,
    right: Drawer<V>,
    editor: Node<GroupId>,
    active_group: GroupId,
    next_group: u32,
    focus: Panel,
    /// Each view's home side: the drawer it last landed in, or what
    /// [`PanelLayout::set_home_side`] recorded. Views absent here live on the
    /// left. A list, not a map, so `V` needs only `PartialEq`.
    homes: Vec<(V, Side)>,
}

/// Tolerance for comparing abstract-unit edges.
const EPS: f64 = 1e-4;

/// Length of the overlap of `[a0, a1)` and `[b0, b1)`, zero when disjoint.
fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

impl<V: Clone + PartialEq> Drawer<V> {
    fn new(slots: Option<Node<V>>, visible: bool, width: f64) -> Self {
        Self {
            slots,
            focused: 0,
            visible,
            width,
        }
    }

    fn views(&self) -> Vec<V> {
        self.slots
            .as_ref()
            .map(|n| n.leaves().into_iter().cloned().collect())
            .unwrap_or_default()
    }

    fn count(&self) -> usize {
        self.slots.as_ref().map_or(0, Node::leaf_count)
    }

    fn focused_view(&self) -> Option<V> {
        self.views().get(self.focused).cloned()
    }

    /// Point the focused slot back at `prev` after the slots changed, or
    /// clamp it when `prev` is gone.
    fn refocus(&mut self, prev: Option<V>) {
        let views = self.views();
        self.focused = prev
            .and_then(|v| views.iter().position(|x| *x == v))
            .unwrap_or_else(|| self.focused.min(views.len().saturating_sub(1)));
    }

    /// Add `view` as the bottom slot (or the only one).
    fn append(&mut self, view: V) {
        let prev = self.focused_view();
        match &mut self.slots {
            None => self.slots = Some(Node::leaf(view)),
            Some(node) => {
                let last = node.leaves().last().map(|l| (*l).clone());
                if let Some(last) = last {
                    node.split_leaf(&last, Axis::Vertical, view);
                }
            }
        }
        self.refocus(prev);
    }

    /// Swap the view in `old`'s slot for `new`, keeping slot focus.
    fn replace(&mut self, old: &V, new: V) {
        let mut prev = self.focused_view();
        if prev.as_ref() == Some(old) {
            prev = Some(new.clone());
        }
        if let Some(node) = &mut self.slots {
            node.replace_leaf(old, new);
        }
        self.refocus(prev);
    }
}

impl<V: Clone + PartialEq> PanelLayout<V> {
    /// Default layout: rail → left drawer (visible, one slot holding
    /// `initial_view`) → editor (one group, focused); the right drawer is
    /// hidden and empty. Both drawers start `drawer_width` wide.
    pub fn new(initial_view: V, drawer_width: f64) -> Self {
        let first = GroupId(0);
        Self {
            left: Drawer::new(Some(Node::leaf(initial_view)), true, drawer_width),
            right: Drawer::new(None, false, drawer_width),
            editor: Node::leaf(first),
            active_group: first,
            next_group: 1,
            focus: Panel::Editor,
            homes: Vec::new(),
        }
    }

    fn drawer(&self, side: Side) -> &Drawer<V> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn drawer_mut(&mut self, side: Side) -> &mut Drawer<V> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    // ── Panels ─────────────────────────────────────────────────────────

    /// The focused panel.
    pub fn focused(&self) -> Panel {
        self.focus
    }

    /// The focused region: the rail, the focused drawer's focused slot, or
    /// the active editor group.
    pub fn focused_region(&self) -> Region {
        match self.focus {
            Panel::Rail => Region::Rail,
            Panel::Drawer(side) => Region::Slot {
                side,
                index: self.drawer(side).focused,
            },
            Panel::Editor => Region::Group(self.active_group),
        }
    }

    /// The visible panel one step left of focus, wrapping at the left end.
    pub fn prev_panel(&self) -> Option<Panel> {
        self.step(|i, n| (i + n - 1) % n)
    }

    /// The visible panel one step right of focus, wrapping at the right end.
    pub fn next_panel(&self) -> Option<Panel> {
        self.step(|i, n| (i + 1) % n)
    }

    /// Step through the *visible* panels from the focused one.
    fn step(&self, advance: impl Fn(usize, usize) -> usize) -> Option<Panel> {
        let visible = self.visible_panels();
        let n = visible.len();
        if n < 2 {
            return None;
        }
        let i = visible.iter().position(|&p| p == self.focus)?;
        Some(visible[advance(i, n)])
    }

    /// Move focus to `panel`. No-op for a drawer with no slots.
    pub fn focus(&mut self, panel: Panel) {
        if let Panel::Drawer(side) = panel
            && self.drawer(side).slots.is_none()
        {
            return;
        }
        self.focus = panel;
    }

    /// Whether `panel` is currently visible.
    pub fn is_visible(&self, panel: Panel) -> bool {
        match panel {
            Panel::Rail | Panel::Editor => true,
            Panel::Drawer(side) => self.drawer(side).visible,
        }
    }

    /// Reveal `panel`. No-op for a drawer with no slots.
    pub fn show(&mut self, panel: Panel) {
        if let Panel::Drawer(side) = panel {
            let drawer = self.drawer_mut(side);
            if drawer.slots.is_some() {
                drawer.visible = true;
            }
        }
    }

    /// Hide `panel`. The rail and the editor are always visible. Focus on a
    /// hidden panel moves to the nearest visible one.
    pub fn hide(&mut self, panel: Panel) {
        if let Panel::Drawer(side) = panel {
            self.drawer_mut(side).visible = false;
        }
        self.fix_focus();
    }

    /// If focus sits on a hidden panel, move it to the nearest visible one,
    /// searching outward in panel order (left first at each distance).
    fn fix_focus(&mut self) {
        if self.is_visible(self.focus) {
            return;
        }
        let from = PANEL_ORDER
            .iter()
            .position(|&p| p == self.focus)
            .unwrap_or(0);
        let n = PANEL_ORDER.len();
        if let Some(i) = (1..n)
            .flat_map(|d| [from.checked_sub(d), Some(from + d).filter(|&i| i < n)])
            .flatten()
            .find(|&i| self.is_visible(PANEL_ORDER[i]))
        {
            self.focus = PANEL_ORDER[i];
        }
    }

    /// The visible panels in left → right order.
    pub fn visible_panels(&self) -> Vec<Panel> {
        PANEL_ORDER
            .into_iter()
            .filter(|&p| self.is_visible(p))
            .collect()
    }

    // ── Regions and directional focus ─────────────────────────────────

    /// Focus `region`. False (and nothing changes) when it does not exist.
    pub fn focus_region(&mut self, region: Region) -> bool {
        match region {
            Region::Rail => {}
            Region::Slot { side, index } => {
                let drawer = self.drawer_mut(side);
                if index >= drawer.count() {
                    return false;
                }
                drawer.focused = index;
            }
            Region::Group(group) => {
                if !self.editor.contains(&group) {
                    return false;
                }
                self.active_group = group;
            }
        }
        self.focus = region.panel();
        true
    }

    /// Every visible region with its rectangle in abstract units: each
    /// visible panel is one unit-wide, full-height column in panel order,
    /// subdivided by its slot or group tree.
    pub fn regions(&self) -> Vec<(Region, UnitRect)> {
        let mut out = Vec::new();
        for (i, panel) in self.visible_panels().into_iter().enumerate() {
            let column = UnitRect::new(i as f64, 0.0, 1.0, 1.0);
            match panel {
                Panel::Rail => out.push((Region::Rail, column)),
                Panel::Drawer(side) => {
                    if let Some(node) = &self.drawer(side).slots {
                        out.extend(
                            node.layout(column)
                                .into_iter()
                                .enumerate()
                                .map(|(index, (_, rect))| (Region::Slot { side, index }, rect)),
                        );
                    }
                }
                Panel::Editor => out.extend(
                    self.editor
                        .layout(column)
                        .into_iter()
                        .map(|(g, rect)| (Region::Group(g), rect)),
                ),
            }
        }
        out
    }

    /// Whether `region` is the one its panel would focus on entry: the
    /// drawer's focused slot or the active group.
    fn is_remembered(&self, region: Region) -> bool {
        match region {
            Region::Rail => true,
            Region::Slot { side, index } => self.drawer(side).focused == index,
            Region::Group(g) => g == self.active_group,
        }
    }

    /// Move focus geometrically to the nearest visible region in `dir`.
    ///
    /// Candidates lie wholly on that side of the focused region and overlap
    /// it across the axis. The nearest wins; ties go to the largest overlap,
    /// then the region its panel last focused (the drawer's focused slot or
    /// the active group), then the top-left one. No wrap:
    /// with no candidate, Left/Right report [`FocusMove::AtEdge`], and so do
    /// Up/Down when something is stacked in the same column — otherwise
    /// they are [`FocusMove::NotHandled`].
    pub fn focus_dir(&mut self, dir: Dir) -> FocusMove {
        let regions = self.regions();
        let current = self.focused_region();
        let Some(cur) = regions
            .iter()
            .find(|(r, _)| *r == current)
            .map(|(_, rect)| *rect)
        else {
            return FocusMove::NotHandled;
        };
        let across_x = |c: &UnitRect| overlap(c.x(), c.right(), cur.x(), cur.right());
        let across_y = |c: &UnitRect| overlap(c.y(), c.bottom(), cur.y(), cur.bottom());

        // (region, gap, overlap, rect) for every region in `dir`.
        let candidates: Vec<(Region, f64, f64, UnitRect)> = regions
            .iter()
            .filter(|(r, _)| *r != current)
            .filter_map(|(r, c)| {
                let (beyond, gap, ov) = match dir {
                    Dir::Left => (c.right() <= cur.x() + EPS, cur.x() - c.right(), across_y(c)),
                    Dir::Right => (c.x() >= cur.right() - EPS, c.x() - cur.right(), across_y(c)),
                    Dir::Up => (
                        c.bottom() <= cur.y() + EPS,
                        cur.y() - c.bottom(),
                        across_x(c),
                    ),
                    Dir::Down => (
                        c.y() >= cur.bottom() - EPS,
                        c.y() - cur.bottom(),
                        across_x(c),
                    ),
                };
                (beyond && ov > EPS).then_some((*r, gap, ov, *c))
            })
            .collect();

        let nearest = candidates
            .iter()
            .map(|(_, gap, _, _)| *gap)
            .fold(f64::INFINITY, f64::min);
        let best = candidates
            .iter()
            .filter(|(_, gap, _, _)| *gap <= nearest + EPS)
            .max_by(|a, b| {
                let overlap = if (a.2 - b.2).abs() <= EPS {
                    std::cmp::Ordering::Equal
                } else {
                    a.2.total_cmp(&b.2)
                };
                overlap
                    .then(self.is_remembered(a.0).cmp(&self.is_remembered(b.0)))
                    .then(b.3.y().total_cmp(&a.3.y()))
                    .then(b.3.x().total_cmp(&a.3.x()))
            })
            .map(|(r, ..)| *r);

        match best {
            Some(region) => {
                self.focus_region(region);
                FocusMove::Moved(region)
            }
            None => match dir {
                Dir::Left | Dir::Right => FocusMove::AtEdge,
                Dir::Up | Dir::Down => {
                    let stacked = regions
                        .iter()
                        .any(|(r, c)| *r != current && across_x(c) > EPS);
                    if stacked {
                        FocusMove::AtEdge
                    } else {
                        FocusMove::NotHandled
                    }
                }
            },
        }
    }

    // ── Drawers ───────────────────────────────────────────────────────

    /// The views in `side`'s slots, top → bottom.
    pub fn slot_views(&self, side: Side) -> Vec<V> {
        self.drawer(side).views()
    }

    /// The focused slot of `side`, `None` when it has no slots.
    pub fn focused_slot(&self, side: Side) -> Option<usize> {
        let drawer = self.drawer(side);
        drawer.slots.is_some().then_some(drawer.focused)
    }

    /// Where `view` is placed, if anywhere.
    pub fn view_location(&self, view: &V) -> Option<(Side, usize)> {
        [Side::Left, Side::Right].into_iter().find_map(|side| {
            self.drawer(side)
                .views()
                .iter()
                .position(|v| v == view)
                .map(|i| (side, i))
        })
    }

    /// Take `view` out of its slot, if placed. A drawer left with no slots
    /// is hidden and focus falls back.
    fn remove_view(&mut self, view: &V) {
        let Some((side, _)) = self.view_location(view) else {
            return;
        };
        let drawer = self.drawer_mut(side);
        if drawer.count() == 1 {
            drawer.slots = None;
            drawer.focused = 0;
            drawer.visible = false;
            self.fix_focus();
        } else {
            let prev = drawer.focused_view();
            if let Some(node) = &mut drawer.slots {
                node.remove_leaf(view);
            }
            drawer.refocus(prev);
        }
    }

    /// Whether the focused region is `view`'s slot.
    fn view_has_focus(&self, view: &V) -> bool {
        self.view_location(view)
            .is_some_and(|(side, index)| self.focused_region() == Region::Slot { side, index })
    }

    /// Location of a view that was just placed; records that side as the
    /// view's home.
    fn settle(&mut self, view: &V) -> (Side, usize) {
        let loc = self.view_location(view).expect("the view was just placed");
        self.set_home_side(view.clone(), loc.0);
        loc
    }

    /// The side `view` opens on when it is in no slot: the drawer it last
    /// landed in, or the one [`Self::set_home_side`] recorded; left by default.
    pub fn home_side(&self, view: &V) -> Side {
        self.homes
            .iter()
            .find(|(v, _)| v == view)
            .map_or(Side::Left, |(_, side)| *side)
    }

    /// Record `view`'s home side (restoring persisted settings). A placed
    /// view is not moved; the home applies the next time it is placed while
    /// in no slot.
    pub fn set_home_side(&mut self, view: V, side: Side) {
        match self.homes.iter_mut().find(|(v, _)| *v == view) {
            Some(entry) => entry.1 = side,
            None => self.homes.push((view, side)),
        }
    }

    /// Ensure `view` is placed somewhere without changing visibility: a
    /// placed view stays put; an unplaced one replaces the focused slot of
    /// the drawer on its [home side](Self::home_side) (or becomes its only
    /// slot).
    pub fn place_view(&mut self, view: V) -> (Side, usize) {
        if let Some(loc) = self.view_location(&view) {
            return loc;
        }
        let drawer = self.drawer_mut(self.home_side(&view));
        match drawer.focused_view() {
            Some(old) => drawer.replace(&old, view.clone()),
            None => drawer.append(view.clone()),
        }
        self.settle(&view)
    }

    /// Launch `view` from the rail: place it, make its slot its drawer's
    /// focused slot, and reveal that drawer. Panel focus is unchanged.
    pub fn open_view(&mut self, view: V) -> (Side, usize) {
        let (side, index) = self.place_view(view);
        let drawer = self.drawer_mut(side);
        drawer.focused = index;
        drawer.visible = true;
        (side, index)
    }

    /// Put `view` in slot `index` of `side`, evicting what was there. A view
    /// placed elsewhere moves (its old slot closes) and focus follows it when
    /// its slot was focused. Visibility unchanged.
    pub fn set_slot_view(&mut self, side: Side, index: usize, view: V) -> Result<(), LayoutError> {
        let Some(target) = self.drawer(side).views().get(index).cloned() else {
            return Err(LayoutError::NoSuchSlot);
        };
        if target == view {
            return Ok(());
        }
        let had_focus = self.view_has_focus(&view);
        self.remove_view(&view);
        self.drawer_mut(side).replace(&target, view.clone());
        let (side, index) = self.settle(&view);
        if had_focus {
            self.focus_region(Region::Slot { side, index });
        }
        Ok(())
    }

    /// Add a slot holding `view` at the bottom of `side` and reveal it. A view
    /// placed in the other drawer moves. Refused for a view already in `side`
    /// ([`LayoutError::AlreadyInDrawer`]) and at [`MAX_DRAWER_SLOTS`].
    pub fn split_slot(&mut self, side: Side, view: V) -> Result<usize, LayoutError> {
        if self.view_location(&view).is_some_and(|(s, _)| s == side) {
            return Err(LayoutError::AlreadyInDrawer);
        }
        if self.drawer(side).count() >= MAX_DRAWER_SLOTS {
            return Err(LayoutError::SlotCap);
        }
        let had_focus = self.view_has_focus(&view);
        self.remove_view(&view);
        let drawer = self.drawer_mut(side);
        drawer.append(view.clone());
        drawer.visible = true;
        let (side, index) = self.settle(&view);
        if had_focus {
            self.focus_region(Region::Slot { side, index });
        }
        Ok(index)
    }

    /// Close slot `index` of `side`, returning its view. A drawer left with
    /// no slots is hidden.
    pub fn close_slot(&mut self, side: Side, index: usize) -> Result<V, LayoutError> {
        let Some(view) = self.drawer(side).views().get(index).cloned() else {
            return Err(LayoutError::NoSuchSlot);
        };
        self.remove_view(&view);
        Ok(view)
    }

    /// Move `view` to the drawer on `to` and reveal it: a new slot when there
    /// is room, otherwise it replaces that drawer's focused slot. Focus
    /// follows the view when its slot was focused.
    pub fn move_view(&mut self, view: V, to: Side) -> (Side, usize) {
        if let Some(loc) = self.view_location(&view)
            && loc.0 == to
        {
            self.show(Panel::Drawer(to));
            return loc;
        }
        let had_focus = self.view_has_focus(&view);
        self.remove_view(&view);
        let drawer = self.drawer_mut(to);
        match drawer.focused_view() {
            Some(old) if drawer.count() >= MAX_DRAWER_SLOTS => drawer.replace(&old, view.clone()),
            _ => drawer.append(view.clone()),
        }
        drawer.visible = true;
        let loc = self.settle(&view);
        if had_focus {
            self.focus_region(Region::Slot {
                side: loc.0,
                index: loc.1,
            });
        }
        loc
    }

    /// The share of `side`'s height its first slot takes, `None` with fewer
    /// than two slots. Rounded to [`RATIO_DECIMALS`] places so a saved value
    /// is stable (no `0.30000000000000004` in `config.toml`).
    pub fn split_ratio(&self, side: Side) -> Option<f64> {
        let scale = 10f64.powi(RATIO_DECIMALS);
        self.drawer(side)
            .slots
            .as_ref()
            .and_then(|n| n.ratios().first().copied())
            .map(|r| (r * scale).round() / scale)
    }

    /// Set the share of `side`'s height its first slot takes (clamped).
    /// False with fewer than two slots.
    pub fn set_split_ratio(&mut self, side: Side, ratio: f64) -> bool {
        self.drawer_mut(side)
            .slots
            .as_mut()
            .is_some_and(|n| n.set_first_ratio(ratio))
    }

    /// `side`'s width in front-end units.
    pub fn drawer_width(&self, side: Side) -> f64 {
        self.drawer(side).width
    }

    /// Set `side`'s width to `width`, clamped so the drawer keeps
    /// `drawer_min` and the editor keeps `editor_min` of `available` (minus
    /// the other drawer when it is visible). Returns the width applied.
    pub fn set_drawer_width(&mut self, side: Side, width: f64, limits: WidthLimits) -> f64 {
        let other = match side {
            Side::Left => &self.right,
            Side::Right => &self.left,
        };
        let other_width = if other.visible { other.width } else { 0.0 };
        let max = limits.available - limits.editor_min - other_width;
        let applied = width.clamp(limits.drawer_min, max.max(limits.drawer_min));
        self.drawer_mut(side).width = applied;
        applied
    }

    /// Grow or shrink `side` by `delta`, clamped as [`Self::set_drawer_width`].
    pub fn resize_drawer(&mut self, side: Side, delta: f64, limits: WidthLimits) -> f64 {
        let width = self.drawer(side).width + delta;
        self.set_drawer_width(side, width, limits)
    }

    // ── Editor groups ─────────────────────────────────────────────────

    /// Every editor group in reading order.
    pub fn groups(&self) -> Vec<GroupId> {
        self.editor.leaves().into_iter().copied().collect()
    }

    /// The group that owns save, the footer and `{note}`.
    pub fn active_group(&self) -> GroupId {
        self.active_group
    }

    /// Split `group` along `axis`; the new group comes after it. The active
    /// group is unchanged. `None` when `group` does not exist.
    pub fn split_group(&mut self, group: GroupId, axis: Axis) -> Option<GroupId> {
        let new = GroupId(self.next_group);
        if !self.editor.split_leaf(&group, axis, new) {
            return None;
        }
        self.next_group += 1;
        Some(new)
    }

    /// Close `group`. The last group cannot close. When the active group
    /// closes, the group before it (or the first) becomes active.
    pub fn close_group(&mut self, group: GroupId) -> bool {
        let Some(pos) = self.groups().iter().position(|&g| g == group) else {
            return false;
        };
        if !self.editor.remove_leaf(&group) {
            return false;
        }
        if self.active_group == group {
            let groups = self.groups();
            self.active_group = groups[pos.saturating_sub(1).min(groups.len() - 1)];
        }
        true
    }
}

#[cfg(test)]
mod tests;
