//! The editor screen's layout model (ADR-0050): which **Panels** exist, which
//! are visible, how each **drawer** is split into **drawer slots**, how the
//! editor Panel is split into **editor groups**, and where focus is.
//! Toolkit-free — widths are abstract units the front end maps to columns or
//! pixels, and geometry is in [`UnitRect`]s, never terminal cells.
//!
//! Panel order is rail → left drawer → editor → right drawer. The right
//! drawer starts hidden and empty, so the default layout is the TUI's
//! original fixed line.

pub(crate) mod node;

pub use node::{Axis, UnitRect};
use node::{MIN_RATIO, Node};

/// The most drawer slots one drawer holds. The node type allows more; the
/// model enforces this cap.
pub const MAX_DRAWER_SLOTS: usize = 2;

/// Decimal places [`PanelLayout::split_ratio`] reports. Ratios and widths are
/// `f64` throughout; the rounding keeps persisted ratios short and exact.
pub const RATIO_DECIMALS: i32 = 4;

/// A drawer's first-slot share until one is set.
const DEFAULT_RATIO: f64 = 0.5;

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

impl Side {
    /// The other side of the editor.
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
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

impl GroupId {
    /// The group with id `n` (for layouts built outside this module).
    pub(crate) fn new(n: u32) -> GroupId {
        GroupId(n)
    }
}

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

/// One view placed in a drawer: its slot position and whether it is shown.
#[derive(Debug, Clone, PartialEq)]
struct Placed<V> {
    view: V,
    /// The drawer slot it sits in: 0 (top) or 1 (bottom).
    slot: usize,
    shown: bool,
}

/// A drawer: the views placed in it (each at a slot position, top or
/// bottom; several may share one), which of them are shown (at most one per
/// position), and its own geometry. The shown views are its slots, top to
/// bottom.
struct Drawer<V> {
    /// Kept in slot-position order (top first), stable within a position.
    placed: Vec<Placed<V>>,
    /// Index of the focused slot among the shown views.
    focused: usize,
    visible: bool,
    width: f64,
    /// The first slot's share when both positions show a view.
    ratio: f64,
    /// The **empty slot** a split opened: a slot position that shows no
    /// view, only a hint, beside the drawer's one shown view. Session-only,
    /// never persisted; cleared when a view lands there, on unsplit, when
    /// the drawer hides or when its other slot empties.
    empty: Option<usize>,
}

/// The layout state: panels, drawers with their placed and shown views,
/// editor groups, focus. Generic over the drawer view id `V`, so the layout
/// never assumes a closed set of views.
///
/// Every view placed in a drawer keeps its **placement** — a side and a
/// drawer slot position, top (0) or bottom (1) — while it is hidden;
/// whether it is **shown** is a separate flag. Several views may share a
/// position, like tabs; at most one per position is shown, so showing one
/// hides the other. A view is placed at most once. A view placed nowhere
/// opens on the left.
pub struct PanelLayout<V> {
    left: Drawer<V>,
    right: Drawer<V>,
    editor: Node<GroupId>,
    active_group: GroupId,
    next_group: u32,
    focus: Panel,
}

/// Tolerance for comparing abstract-unit edges.
const EPS: f64 = 1e-4;

/// Length of the overlap of `[a0, a1)` and `[b0, b1)`, zero when disjoint.
fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

impl<V: Clone + PartialEq> Drawer<V> {
    fn new(visible: bool, width: f64) -> Self {
        Self {
            placed: Vec::new(),
            focused: 0,
            visible,
            width,
            ratio: DEFAULT_RATIO,
            empty: None,
        }
    }

    /// The shown views, top → bottom.
    fn views(&self) -> Vec<V> {
        self.placed
            .iter()
            .filter(|p| p.shown)
            .map(|p| p.view.clone())
            .collect()
    }

    /// Every placed view, shown or not, top position first.
    fn placed_views(&self) -> Vec<V> {
        self.placed.iter().map(|p| p.view.clone()).collect()
    }

    /// Number of shown views (slots).
    fn count(&self) -> usize {
        self.placed.iter().filter(|p| p.shown).count()
    }

    fn get(&self, view: &V) -> Option<&Placed<V>> {
        self.placed.iter().find(|p| p.view == *view)
    }

    fn get_mut(&mut self, view: &V) -> Option<&mut Placed<V>> {
        self.placed.iter_mut().find(|p| p.view == *view)
    }

    fn focused_view(&self) -> Option<V> {
        self.views().get(self.focused).cloned()
    }

    /// The slot position of the focused slot's view; top with none shown.
    fn focused_pos(&self) -> usize {
        self.focused_view()
            .and_then(|v| self.get(&v).map(|p| p.slot))
            .unwrap_or(0)
    }

    /// The first slot position no shown view sits in.
    fn free_slot(&self) -> Option<usize> {
        (0..MAX_DRAWER_SLOTS).find(|pos| !self.placed.iter().any(|p| p.shown && p.slot == *pos))
    }

    /// Point the focused slot back at `prev` after the slots changed, or
    /// clamp it when `prev` is gone.
    fn refocus(&mut self, prev: Option<V>) {
        let views = self.views();
        self.focused = prev
            .and_then(|v| views.iter().position(|x| *x == v))
            .unwrap_or_else(|| self.focused.min(views.len().saturating_sub(1)));
    }

    /// Re-establish slot-position order after a position changed.
    fn normalize(&mut self) {
        self.placed.sort_by_key(|p| p.slot);
    }

    /// Place `view` at slot position `slot` (moving it there when it is
    /// placed here already), hidden unless it was shown. Slot focus stays.
    fn put(&mut self, view: V, slot: usize) {
        let prev = self.focused_view();
        match self.get_mut(&view) {
            Some(p) => p.slot = slot,
            None => self.placed.push(Placed {
                view,
                slot,
                shown: false,
            }),
        }
        self.normalize();
        self.refocus(prev);
    }

    /// Show `view`, placed here: the view shown at its slot position, if
    /// any, is hidden (and stays placed). Slot focus on that view moves to
    /// `view`. A view shown at the empty slot's position fills it.
    fn show(&mut self, view: &V) {
        let Some(slot) = self.get(view).map(|p| p.slot) else {
            return;
        };
        if self.empty == Some(slot) {
            self.empty = None;
        }
        let mut prev = self.focused_view();
        for p in &mut self.placed {
            if p.view == *view {
                p.shown = true;
            } else if p.slot == slot && p.shown {
                p.shown = false;
                if prev.as_ref() == Some(&p.view) {
                    prev = Some(view.clone());
                }
            }
        }
        self.refocus(prev);
    }

    /// Hide `view` (its flag only). Slot focus stays on the view it was on,
    /// or clamps.
    fn conceal(&mut self, view: &V) {
        let prev = self.focused_view();
        if let Some(p) = self.get_mut(view) {
            p.shown = false;
        }
        self.refocus(prev);
    }

    /// Move every view placed at slot position `from` to `to`, after the
    /// views already there, keeping their shown flags. Slot focus stays on
    /// the view it was on.
    fn merge(&mut self, from: usize, to: usize) {
        let prev = self.focused_view();
        let (mut moved, kept): (Vec<_>, Vec<_>) =
            self.placed.drain(..).partition(|p| p.slot == from);
        for p in &mut moved {
            p.slot = to;
        }
        self.placed = kept;
        self.placed.extend(moved);
        self.normalize();
        self.refocus(prev);
    }

    /// The slot position of the single slot an unsplit drawer shows: the
    /// shown view's, top with none shown. `None` while both positions show
    /// a view.
    fn single_pos(&self) -> Option<usize> {
        match self.views().as_slice() {
            [] => Some(0),
            [view] => self.get(view).map(|p| p.slot),
            _ => None,
        }
    }

    /// The displayed slots, top → bottom: each shown view, and `None` at
    /// the empty slot's position.
    fn slots(&self) -> Vec<Option<V>> {
        let mut slots: Vec<(usize, Option<V>)> = self
            .placed
            .iter()
            .filter(|p| p.shown)
            .map(|p| (p.slot, Some(p.view.clone())))
            .collect();
        if let Some(pos) = self.empty {
            slots.push((pos, None));
            slots.sort_by_key(|(pos, _)| *pos);
        }
        slots.into_iter().map(|(_, view)| view).collect()
    }

    /// Hide the drawer; its empty slot goes with it.
    fn conceal_drawer(&mut self) {
        self.visible = false;
        self.empty = None;
    }

    /// The shown views as a slot tree, the first at the drawer's ratio;
    /// `None` with nothing shown.
    fn slot_node(&self) -> Option<Node<V>> {
        let views = self.views();
        let mut node = Node::leaf(views.first()?.clone());
        for pair in views.windows(2) {
            node.split_leaf(&pair[0], Axis::Vertical, pair[1].clone());
        }
        node.set_first_ratio(self.ratio);
        Some(node)
    }
}

impl<V: Clone + PartialEq> PanelLayout<V> {
    /// Default layout: rail → left drawer (visible, one slot holding
    /// `initial_view`) → editor (one group, focused); the right drawer is
    /// hidden and empty. Both drawers start `drawer_width` wide.
    pub fn new(initial_view: V, drawer_width: f64) -> Self {
        let mut layout = Self::empty(drawer_width);
        layout.left.put(initial_view.clone(), 0);
        layout.left.show(&initial_view);
        layout.left.visible = true;
        layout
    }

    /// No view placed anywhere, both drawers hidden, the editor focused: the
    /// starting point for restoring persisted placements with
    /// [`Self::place`].
    pub fn empty(drawer_width: f64) -> Self {
        let first = GroupId(0);
        Self {
            left: Drawer::new(false, drawer_width),
            right: Drawer::new(false, drawer_width),
            editor: Node::leaf(first),
            active_group: first,
            next_group: 1,
            focus: Panel::Editor,
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

    /// Step through the *focusable* panels from the focused one.
    fn step(&self, advance: impl Fn(usize, usize) -> usize) -> Option<Panel> {
        let visible: Vec<Panel> = PANEL_ORDER
            .into_iter()
            .filter(|&p| self.is_focusable(p))
            .collect();
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
            && self.drawer(side).count() == 0
        {
            return;
        }
        self.focus = panel;
    }

    /// Whether `panel` can take focus: visible, and for a drawer showing a
    /// view (an empty drawer is visible but has nothing to focus).
    fn is_focusable(&self, panel: Panel) -> bool {
        match panel {
            Panel::Drawer(side) => self.is_visible(panel) && self.drawer(side).count() > 0,
            _ => self.is_visible(panel),
        }
    }

    /// Whether `panel` is currently visible.
    pub fn is_visible(&self, panel: Panel) -> bool {
        match panel {
            Panel::Rail | Panel::Editor => true,
            Panel::Drawer(side) => self.drawer(side).visible,
        }
    }

    /// Reveal `panel`. No-op for a drawer with no slots (no shown view).
    pub fn show(&mut self, panel: Panel) {
        if let Panel::Drawer(side) = panel {
            let drawer = self.drawer_mut(side);
            if drawer.count() > 0 {
                drawer.visible = true;
            }
        }
    }

    /// Toggle `side`'s drawer open (the GUI's drawer toggle): it becomes
    /// visible even when it shows nothing. A drawer showing nothing but
    /// holding hidden placed views shows the first of them (top position
    /// first); one with no placed view opens **empty**, a hint in place of
    /// a view, and focus stays where it is. Returns whether it shows a view.
    pub fn open_drawer(&mut self, side: Side) -> bool {
        let drawer = self.drawer_mut(side);
        if drawer.count() == 0
            && let Some(first) = drawer.placed.first().map(|p| p.view.clone())
        {
            drawer.show(&first);
        }
        drawer.visible = true;
        drawer.count() > 0
    }

    /// Hide `panel`. The rail and the editor are always visible. Focus on a
    /// hidden panel moves to the nearest visible one.
    pub fn hide(&mut self, panel: Panel) {
        if let Panel::Drawer(side) = panel {
            self.drawer_mut(side).conceal_drawer();
        }
        self.fix_focus();
    }

    /// If focus sits on a panel it cannot stay on (hidden, or a drawer
    /// showing nothing), move it to the nearest focusable one, searching
    /// outward in panel order (left first at each distance).
    fn fix_focus(&mut self) {
        if self.is_focusable(self.focus) {
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
            .find(|&i| self.is_focusable(PANEL_ORDER[i]))
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
                    if let Some(node) = self.drawer(side).slot_node() {
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

    /// The views shown in `side`'s slots, top → bottom (whether or not the
    /// drawer itself is visible).
    pub fn slot_views(&self, side: Side) -> Vec<V> {
        self.drawer(side).views()
    }

    /// Every view placed in `side`, shown or hidden, top position first
    /// (placement order within a position).
    pub fn placed_views(&self, side: Side) -> Vec<V> {
        self.drawer(side).placed_views()
    }

    /// `side`'s displayed slots, top → bottom: the shown views, and `None`
    /// at its empty slot ([`Self::split_empty`]). Empty for a drawer that
    /// shows nothing.
    pub fn slots(&self, side: Side) -> Vec<Option<V>> {
        self.drawer(side).slots()
    }

    /// The slot position of `side`'s empty slot (0 top, 1 bottom; also its
    /// index in [`Self::slots`]), `None` without one.
    pub fn empty_slot(&self, side: Side) -> Option<usize> {
        self.drawer(side).empty
    }

    /// The focused slot of `side`, `None` when it shows no view.
    pub fn focused_slot(&self, side: Side) -> Option<usize> {
        let drawer = self.drawer(side);
        (drawer.count() > 0).then_some(drawer.focused)
    }

    /// The slot `view` is shown in (its index among the shown slots, top
    /// first), if it is shown.
    pub fn view_location(&self, view: &V) -> Option<(Side, usize)> {
        [Side::Left, Side::Right].into_iter().find_map(|side| {
            self.drawer(side)
                .views()
                .iter()
                .position(|v| v == view)
                .map(|i| (side, i))
        })
    }

    /// Where `view` is placed, shown or hidden: its side and slot position
    /// (0 top, 1 bottom). `None` when it is placed nowhere.
    pub fn placement(&self, view: &V) -> Option<(Side, usize)> {
        [Side::Left, Side::Right]
            .into_iter()
            .find_map(|side| self.drawer(side).get(view).map(|p| (side, p.slot)))
    }

    /// Place `view` at slot position `slot` of `side`, shown or hidden
    /// (restoring persisted settings). Visibility is unchanged. Refused
    /// (false) when `view` is already placed, `slot` is not a position
    /// (0 or 1), or it should be shown and another view is shown there.
    pub fn place(&mut self, side: Side, view: V, slot: usize, shown: bool) -> bool {
        let drawer = self.drawer(side);
        let taken = drawer.placed.iter().any(|p| p.shown && p.slot == slot);
        if self.placement(&view).is_some() || slot >= MAX_DRAWER_SLOTS || (shown && taken) {
            return false;
        }
        let drawer = self.drawer_mut(side);
        drawer.put(view.clone(), slot);
        if shown {
            drawer.show(&view);
        }
        true
    }

    /// Take `view` out of its placement, if placed. A drawer left showing
    /// nothing is hidden and focus falls back.
    fn remove_view(&mut self, view: &V) {
        let Some((side, _)) = self.placement(view) else {
            return;
        };
        let drawer = self.drawer_mut(side);
        let prev = drawer.focused_view();
        drawer.placed.retain(|p| p.view != *view);
        drawer.refocus(prev);
        self.settle_empty(side);
    }

    /// A drawer showing nothing is hidden, and focus falls back.
    fn settle_empty(&mut self, side: Side) {
        let drawer = self.drawer_mut(side);
        if drawer.count() == 0 {
            drawer.focused = 0;
            drawer.conceal_drawer();
            self.fix_focus();
        }
    }

    /// Whether the focused region is `view`'s slot.
    fn view_has_focus(&self, view: &V) -> bool {
        self.view_location(view)
            .is_some_and(|(side, index)| self.focused_region() == Region::Slot { side, index })
    }

    /// Location of a view that was just shown.
    fn shown_at(&self, view: &V) -> (Side, usize) {
        self.view_location(view).expect("the view was just shown")
    }

    /// Focus `view`'s slot when `had_focus`, and return where it is shown.
    fn follow(&mut self, view: &V, had_focus: bool) -> (Side, usize) {
        let (side, index) = self.shown_at(view);
        if had_focus {
            self.focus_region(Region::Slot { side, index });
        }
        (side, index)
    }

    /// Ensure `view` is shown without changing visibility. A shown view
    /// stays put. A hidden one is shown where it is placed, hiding the view
    /// shown at its slot position (which stays placed there). A view placed
    /// nowhere is placed on the left at the focused slot's position (top in
    /// an empty drawer), so it takes that slot. Never fills an empty slot:
    /// a view placed at an empty slot's position is shown at the drawer's
    /// shown slot instead (only a move reaches an empty slot).
    pub fn place_view(&mut self, view: V) -> (Side, usize) {
        if let Some(loc) = self.view_location(&view) {
            return loc;
        }
        match self.placement(&view) {
            Some((side, slot)) => {
                let drawer = self.drawer_mut(side);
                if drawer.empty == Some(slot) {
                    let to = drawer.focused_pos();
                    drawer.put(view.clone(), to);
                }
                drawer.show(&view);
            }
            None => {
                let slot = self.left.focused_pos();
                self.left.put(view.clone(), slot);
                self.left.show(&view);
            }
        }
        self.shown_at(&view)
    }

    /// Launch `view` from the rail: show it ([`Self::place_view`]), make
    /// its slot its drawer's focused slot, and reveal that drawer. Panel
    /// focus is unchanged.
    pub fn open_view(&mut self, view: V) -> (Side, usize) {
        let (side, index) = self.place_view(view);
        let drawer = self.drawer_mut(side);
        drawer.focused = index;
        drawer.visible = true;
        (side, index)
    }

    /// Show `view` in slot `index` of `side`: it is placed at that slot's
    /// position and the view there is hidden (it stays placed). A view
    /// placed in the other drawer moves (a drawer left showing nothing
    /// hides) and focus follows it when its slot was focused. Visibility
    /// unchanged.
    pub fn set_slot_view(&mut self, side: Side, index: usize, view: V) -> Result<(), LayoutError> {
        let Some(target) = self.drawer(side).views().get(index).cloned() else {
            return Err(LayoutError::NoSuchSlot);
        };
        if target == view {
            return Ok(());
        }
        let had_focus = self.view_has_focus(&view);
        if self.placement(&view).map(|(s, _)| s) != Some(side) {
            self.remove_view(&view);
        }
        let drawer = self.drawer_mut(side);
        let slot = drawer.get(&target).map_or(0, |p| p.slot);
        drawer.put(view.clone(), slot);
        drawer.show(&view);
        self.follow(&view, had_focus);
        Ok(())
    }

    /// Show `view` at `side`'s free slot position (placing it there, moving
    /// it from the other drawer if need be) and reveal the drawer. Refused
    /// for a view already shown in `side` ([`LayoutError::AlreadyInDrawer`])
    /// and when both positions show a view ([`LayoutError::SlotCap`]).
    pub fn split_slot(&mut self, side: Side, view: V) -> Result<usize, LayoutError> {
        if self.view_location(&view).is_some_and(|(s, _)| s == side) {
            return Err(LayoutError::AlreadyInDrawer);
        }
        let Some(free) = self.drawer(side).free_slot() else {
            return Err(LayoutError::SlotCap);
        };
        let had_focus = self.view_has_focus(&view);
        if self.placement(&view).map(|(s, _)| s) != Some(side) {
            self.remove_view(&view);
        }
        let drawer = self.drawer_mut(side);
        drawer.put(view.clone(), free);
        drawer.show(&view);
        drawer.visible = true;
        Ok(self.follow(&view, had_focus).1)
    }

    /// Open an **empty slot** in `side` (Split): the free slot position
    /// beside the drawer's one shown view shows a hint instead of a view
    /// until a view is moved there ([`Self::fill_empty_slot`]). Session
    /// only, never persisted. Slot focus stays on the shown view. Returns
    /// the empty slot's position. Refused with nothing shown
    /// ([`LayoutError::NoSuchSlot`]) and in a full drawer, both positions
    /// taken by views or the empty slot ([`LayoutError::SlotCap`]).
    pub fn split_empty(&mut self, side: Side) -> Result<usize, LayoutError> {
        let drawer = self.drawer_mut(side);
        if drawer.count() == 0 {
            return Err(LayoutError::NoSuchSlot);
        }
        if drawer.empty.is_some() {
            return Err(LayoutError::SlotCap);
        }
        let free = drawer.free_slot().ok_or(LayoutError::SlotCap)?;
        drawer.empty = Some(free);
        drawer.visible = true;
        Ok(free)
    }

    /// Close `side`'s empty slot (Unsplit on a drawer with one): the shown
    /// view fills the drawer again. False when there is none.
    pub fn close_empty_slot(&mut self, side: Side) -> bool {
        self.drawer_mut(side).empty.take().is_some()
    }

    /// Move `view` into `side`'s empty slot and show it there; the drawer is
    /// revealed. A view shown in `side` leaves its own slot, which becomes
    /// the empty one (the marker follows it). Any other view — hidden here,
    /// in the other drawer, or placed nowhere — is placed at the empty slot
    /// (a drawer it leaves showing nothing hides). Focus follows the view
    /// when its slot was focused. `None` (nothing changes) without an empty
    /// slot in `side`.
    pub fn fill_empty_slot(&mut self, side: Side, view: V) -> Option<(Side, usize)> {
        let pos = self.drawer(side).empty?;
        let had_focus = self.view_has_focus(&view);
        let shown_here = self.view_location(&view).is_some_and(|(s, _)| s == side);
        if self.placement(&view).map(|(s, _)| s) != Some(side) {
            self.remove_view(&view);
        }
        let drawer = self.drawer_mut(side);
        let from = drawer.get(&view).map(|p| p.slot);
        drawer.put(view.clone(), pos);
        drawer.show(&view);
        if shown_here {
            drawer.empty = from;
        }
        drawer.visible = true;
        Some(self.follow(&view, had_focus))
    }

    /// Close slot `index` of `side` (an explicit unsplit) and return the
    /// view it showed, now hidden. In a split drawer the slot **merges**
    /// into the other one: every view placed at the closed slot's position
    /// (shown or hidden) moves to the other position, so all the drawer's
    /// views share one slot and showing any of them replaces the shown one
    /// rather than splitting the drawer again. A drawer's only slot just
    /// hides its view, and the drawer, left showing nothing, is hidden.
    /// An empty slot is closed by [`Self::close_empty_slot`].
    pub fn close_slot(&mut self, side: Side, index: usize) -> Result<V, LayoutError> {
        let Some(view) = self.drawer(side).views().get(index).cloned() else {
            return Err(LayoutError::NoSuchSlot);
        };
        let drawer = self.drawer_mut(side);
        let from = drawer.get(&view).map_or(0, |p| p.slot);
        drawer.conceal(&view);
        match drawer.single_pos() {
            Some(to) if drawer.count() == 1 => drawer.merge(from, to),
            _ => self.settle_empty(side),
        }
        Ok(view)
    }

    /// Hide `view` (the rail's toggle). In a split drawer only its flag
    /// clears: it stays placed, so showing it again puts it back at its slot
    /// position, at the drawer's ratio. As a drawer's only shown view, the
    /// drawer hides and the view stays shown in it, so showing the drawer
    /// brings it back. Returns the view's side, `None` when it is not shown.
    pub fn hide_view(&mut self, view: &V) -> Option<Side> {
        let (side, _) = self.view_location(view)?;
        if self.drawer(side).count() < 2 {
            self.hide(Panel::Drawer(side));
        } else {
            self.drawer_mut(side).conceal(view);
        }
        Some(side)
    }

    /// Move `view` to the drawer on `to`, shown, and reveal it, fitting the
    /// target: in a split drawer it keeps its slot position (top when it
    /// was placed nowhere) and replaces the view shown there; in an unsplit
    /// or empty drawer it joins the single slot (the top one when nothing
    /// is shown) and replaces the shown view — it never splits the target.
    /// The replaced view stays placed, hidden. A view placed in `to`
    /// already is just shown. Focus follows the view when its slot was
    /// focused. A target with an empty slot takes any view not shown there
    /// into that slot first ([`Self::fill_empty_slot`]).
    pub fn move_view(&mut self, view: V, to: Side) -> (Side, usize) {
        if self.view_location(&view).map(|(s, _)| s) != Some(to)
            && let Some(loc) = self.fill_empty_slot(to, view.clone())
        {
            return loc;
        }
        let from = self.placement(&view);
        if from.is_some_and(|(side, _)| side != to) || from.is_none() {
            let had_focus = self.view_has_focus(&view);
            self.remove_view(&view);
            let slot = self
                .drawer(to)
                .single_pos()
                .unwrap_or_else(|| from.map_or(0, |(_, slot)| slot));
            self.drawer_mut(to).put(view.clone(), slot);
            self.drawer_mut(to).show(&view);
            self.drawer_mut(to).visible = true;
            return self.follow(&view, had_focus);
        }
        self.drawer_mut(to).show(&view);
        self.drawer_mut(to).visible = true;
        self.shown_at(&view)
    }

    /// Put `view` at slot position `slot` of the split drawer it is placed
    /// in. A shown view trades positions with the view shown at `slot`; a
    /// hidden one just changes position. Slot focus follows the view.
    /// False (nothing changes) when `view` is placed nowhere, its drawer is
    /// unsplit (fewer than two shown views), `slot` is not a position, or
    /// it is there already. A shown view moved to its drawer's empty slot
    /// goes there and leaves the empty slot behind it.
    pub fn set_view_slot(&mut self, view: &V, slot: usize) -> bool {
        let Some((side, from)) = self.placement(view) else {
            return false;
        };
        if self.drawer(side).empty == Some(slot)
            && self.view_location(view).is_some_and(|(s, _)| s == side)
        {
            return self.fill_empty_slot(side, view.clone()).is_some();
        }
        if slot >= MAX_DRAWER_SLOTS || slot == from || self.drawer(side).count() < 2 {
            return false;
        }
        let drawer = self.drawer_mut(side);
        let prev = drawer.focused_view();
        let shown = drawer.get(view).is_some_and(|p| p.shown);
        for p in &mut drawer.placed {
            if p.view == *view {
                p.slot = slot;
            } else if shown && p.shown && p.slot == slot {
                p.slot = from;
            }
        }
        drawer.normalize();
        drawer.refocus(prev);
        true
    }

    /// Swap the views in slots `a` and `b` of `side`: they trade slot
    /// positions. The drawer's slot focus follows the view it was on, and
    /// each slot keeps its size.
    pub fn swap_slots(&mut self, side: Side, a: usize, b: usize) -> Result<(), LayoutError> {
        let views = self.drawer(side).views();
        let (Some(va), Some(_)) = (views.get(a), views.get(b)) else {
            return Err(LayoutError::NoSuchSlot);
        };
        if a != b {
            let slot = self.drawer(side).get(&views[b]).map_or(0, |p| p.slot);
            self.set_view_slot(va, slot);
        }
        Ok(())
    }

    /// The share of `side`'s height its first slot takes when both slot
    /// positions are displayed (two views, or a view and the empty slot);
    /// `None` while fewer than two views are placed there and it has no
    /// empty slot. Rounded to [`RATIO_DECIMALS`] places so a saved value is
    /// stable (no `0.30000000000000004` in `config.toml`).
    pub fn split_ratio(&self, side: Side) -> Option<f64> {
        let scale = 10f64.powi(RATIO_DECIMALS);
        let drawer = self.drawer(side);
        (drawer.placed.len() >= 2 || drawer.empty.is_some())
            .then(|| (drawer.ratio * scale).round() / scale)
    }

    /// Set the share of `side`'s height its first slot takes (clamped to
    /// leave each slot a usable share). One ratio per drawer: it applies to
    /// whichever pair is shown. False (and nothing changes) while fewer than
    /// two views are placed there and it has no empty slot.
    pub fn set_split_ratio(&mut self, side: Side, ratio: f64) -> bool {
        let drawer = self.drawer_mut(side);
        if (drawer.placed.len() < 2 && drawer.empty.is_none()) || !ratio.is_finite() {
            return false;
        }
        drawer.ratio = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        true
    }

    /// `side`'s width in front-end units.
    pub fn drawer_width(&self, side: Side) -> f64 {
        self.drawer(side).width
    }

    /// Set `side`'s width to `width`, clamped so the drawer keeps
    /// `drawer_min` and the editor keeps `editor_min` of `available` (minus
    /// the other drawer when it is visible). Returns the width applied.
    pub fn set_drawer_width(&mut self, side: Side, width: f64, limits: WidthLimits) -> f64 {
        let applied = self.clamp_drawer_width(side, width, limits);
        self.drawer_mut(side).width = applied;
        applied
    }

    /// `side`'s stored width clamped to `limits` as [`Self::set_drawer_width`]
    /// clamps, without storing it: what a front end draws when the window
    /// is narrower than the width was set for, keeping the preference.
    pub fn clamped_drawer_width(&self, side: Side, limits: WidthLimits) -> f64 {
        self.clamp_drawer_width(side, self.drawer(side).width, limits)
    }

    /// The one clamping rule: at least `drawer_min`, and at most what leaves
    /// the editor `editor_min` beside the other drawer when it is visible.
    fn clamp_drawer_width(&self, side: Side, width: f64, limits: WidthLimits) -> f64 {
        let other = match side {
            Side::Left => &self.right,
            Side::Right => &self.left,
        };
        let other_width = if other.visible { other.width } else { 0.0 };
        let max = limits.available - limits.editor_min - other_width;
        width.clamp(limits.drawer_min, max.max(limits.drawer_min))
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
