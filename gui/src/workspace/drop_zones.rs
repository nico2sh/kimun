//! Dropping a view on a slot zone or a window-edge strip (ADR-0054 §8),
//! and an editor group on another group, a tab label or the plus button
//! (ADR-0058, [`super::group_drop`]).
//!
//! A drag of a slot header or a rail entry carries a [`ViewDrag`]; while it
//! is live each drawn slot overlays its body with a [`Workspace::zone_layer`]
//! (a centre and four edges) and the editor Panel adds a strip at its left
//! and right edge ([`Workspace::window_strips`]). A zone is drawn only where
//! [`Workspace::drop_target_allowed`] holds, with a faint outline while the
//! drag lives, and highlights while the drag is over it, saying what a
//! release there would do ([`DropOutcome`]). A release on it
//! runs [`Workspace::drop_view`], the steps of the move chooser: the slot the view leaves closes when emptied
//! ([`Workspace::close_if_vacated`]), the keyboard follows the view, the
//! layout is saved. Esc, a modal or the leader opening ends the drag with
//! nothing changed; while zoomed no zone is drawn.
//!
//! A drag of a group header carries a [`GroupDrag`] instead, through the
//! same machinery (the live drag, the outcomes, the zone look, the hover
//! label, Esc): its zones are the **inner layer**, the centre and four
//! edges of every other group of the shown tab, and the tab strip's labels
//! and plus button. The window-edge strips are the **outer layer**, and
//! they belong to a view's drag alone: a drag draws one layer or the other,
//! never both, so every pixel is one target. Within each layer the zones
//! tile their cell (a slot's or a group's body) without overlapping.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DragMoveEvent, ElementId, Entity,
    InteractiveElement as _, IntoElement, KeystrokeEvent, ParentElement as _, Pixels, Point,
    Render, Rgba, Role, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window, div, px, relative,
};
use kimun_notes::logic::editor_region::TabId;
use kimun_notes::logic::input::Key;
use kimun_notes::logic::layout::{Dir, Side};
use kimun_notes::logic::layout_tree::{GroupId, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;
use kimun_notes::settings::themes::Theme;

use super::placement::Placement;
use super::{Workspace, modal};
use crate::{input, theme};

/// How wide a window-edge strip is, in px: room to aim at without hiding
/// the editor.
const WINDOW_STRIP: f32 = 40.;

/// The least thickness of a slot's edge zone, in px: a quarter of a small
/// slot is too thin to aim at.
const EDGE_ZONE_MIN: f32 = 32.;

/// The sides the window-edge strips lie on.
const STRIP_SIDES: [Side; 2] = [Side::Left, Side::Right];

/// The opacity of a zone's highlight (the focus colour), so the view
/// under it still reads in light and dark themes.
const ZONE_FILL_ALPHA: f32 = 0.25;

/// The opacity of a zone's resting dashed outline, while the drag is not
/// over it: enough to show where the zones are, far less than the
/// highlight so the one under the pointer stands out. A resting zone has
/// no tint.
const ZONE_REST_ALPHA: f32 = 0.3;

/// The payload of a drag: the view being placed (from a slot header, or a
/// rail entry whatever slot holds it, if any).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewDrag {
    pub view: DrawerView,
    pub from: DragFrom,
}

/// The payload of a group header's drag: the editor group being moved,
/// with its note and its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GroupDrag {
    pub group: GroupId,
}

/// What a live drag carries: a view, or an editor group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dragged {
    View(ViewDrag),
    Group(GroupDrag),
}

/// Where a [`ViewDrag`] started. A slot header's drag swaps with the view
/// it lands on; a rail entry's drag replaces it, the replaced view going
/// back to the rail unplaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragFrom {
    Header,
    Rail,
}

/// The element id of the ghost a drag of `view` shows (`drag-chip-files`).
pub(crate) fn drag_chip_id(view: DrawerView) -> SharedString {
    format!("drag-chip-{}", view.id()).into()
}

/// The element id of the ghost a group header's drag shows.
pub(crate) const GROUP_DRAG_CHIP_ID: &str = "drag-chip-group";

/// What a zone's element id calls it.
fn zone_name(zone: Zone) -> &'static str {
    match zone {
        Zone::Centre => "centre",
        Zone::Edge(Dir::Up) => "up",
        Zone::Edge(Dir::Down) => "down",
        Zone::Edge(Dir::Left) => "left",
        Zone::Edge(Dir::Right) => "right",
    }
}

/// The element id of `zone` of the slot showing `view`
/// (`drop-outline-centre`, `drop-outline-down`).
pub(crate) fn zone_id(view: DrawerView, zone: Zone) -> SharedString {
    format!("drop-{}-{}", view.id(), zone_name(zone)).into()
}

/// The element id of `zone` of the group at `at` in the shown tab
/// (`drop-group-1-centre`), as [`super::group_header::group_element`]
/// counts groups.
pub(crate) fn group_zone_id(at: usize, zone: Zone) -> SharedString {
    format!("drop-group-{at}-{}", zone_name(zone)).into()
}

/// The element id of the window-edge strip of `side` (`drop-window-left`).
pub(crate) fn window_strip_id(side: Side) -> SharedString {
    match side {
        Side::Left => "drop-window-left".into(),
        Side::Right => "drop-window-right".into(),
    }
}

/// What follows the pointer during a [`ViewDrag`]: the view's title on the
/// slot-header background, in the header's text colour, so it reads as the
/// header being carried. Theme roles only, so it follows light and dark.
///
/// The drag starts when the pointer has moved more than GPUI's
/// `DRAG_THRESHOLD` (2 px, `elements/div.rs` of the pinned `gpui-pre-0.3.11`)
/// from the press; a click that wobbles less than that is still a click.
pub(crate) struct DragChip {
    id: SharedString,
    title: SharedString,
}

impl DragChip {
    /// The ghost of a view's drag: the view's title.
    pub(crate) fn new(view: DrawerView) -> Self {
        Self {
            id: drag_chip_id(view),
            title: view_registry::spec(view).title.into(),
        }
    }

    /// The ghost of a group header's drag: the group's title, as its
    /// header shows it.
    pub(crate) fn group(title: SharedString) -> Self {
        Self {
            id: GROUP_DRAG_CHIP_ID.into(),
            title,
        }
    }
}

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let title = self.title.clone();
        div()
            .id(self.id.clone())
            .test_support()
            .role(Role::Group)
            .aria_label(title.clone())
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(theme::divider(&theme))
            .bg(theme::slot_header_background(&theme))
            .text_color(theme::foreground(&theme))
            .text_sm()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(title)
    }
}

/// Where a drag can be released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DropTarget {
    /// A zone of a drawn slot.
    Slot(SlotId, Zone),
    /// The strip at a window edge: that side's first slot, else a new one.
    Window(Side),
    /// A zone of an editor group of the shown tab (the inner layer): its
    /// centre swaps the two groups, an edge puts the dragged group on that
    /// side of it.
    Group(GroupId, Zone),
    /// A tab's label: the dragged group joins that tab.
    Tab(TabId),
    /// The tab strip's plus button: the dragged group goes to a new tab.
    NewTab,
}

impl DropTarget {
    /// Where a drop of a view here places it; `None` for a target of a
    /// group's drag.
    pub(crate) fn placement(self) -> Option<Placement> {
        match self {
            DropTarget::Slot(slot, Zone::Centre) => Some(Placement::Into(slot)),
            DropTarget::Slot(slot, Zone::Edge(dir)) => Some(Placement::Beside(slot, dir)),
            DropTarget::Window(side) => Some(Placement::AtEdge(side)),
            DropTarget::Group(..) | DropTarget::Tab(_) | DropTarget::NewTab => None,
        }
    }

    /// Whether this target takes a group's drag (else a view's).
    fn takes_group(self) -> bool {
        matches!(
            self,
            DropTarget::Group(..) | DropTarget::Tab(_) | DropTarget::NewTab
        )
    }
}

/// A zone of a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Zone {
    /// Show the view in the slot.
    Centre,
    /// A new slot on that side of the slot.
    Edge(Dir),
}

impl Zone {
    /// Every zone of a slot, the centre first.
    pub(crate) const ALL: [Zone; 5] = [
        Zone::Centre,
        Zone::Edge(Dir::Left),
        Zone::Edge(Dir::Right),
        Zone::Edge(Dir::Up),
        Zone::Edge(Dir::Down),
    ];
}

/// The drag of a view or a group under way: what it carries, and the
/// ghost GPUI holds for as long as the drag lives (GPUI exposes no accessor
/// for the payload, and a divider's drag is a drag too).
pub(crate) struct LiveDrag {
    drag: Dragged,
    chip: WeakEntity<DragChip>,
}

/// The four sides a new slot can go on, in the order the move chooser
/// lists them.
pub(super) const NEW_SLOT_DIRS: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

/// What a new slot on the `dir` side is called: a move chooser row, a drop
/// zone's accessibility label. The one wording for both.
pub(super) fn new_slot_label(dir: Dir) -> &'static str {
    match dir {
        Dir::Up => "New slot above",
        Dir::Down => "New slot below",
        Dir::Left => "New slot to the left",
        Dir::Right => "New slot to the right",
    }
}

/// What a window-edge strip does, for the accessibility tree.
fn window_strip_label(side: Side) -> &'static str {
    match side {
        Side::Left => "Place at the left edge",
        Side::Right => "Place at the right edge",
    }
}

/// What a drop on a zone would do, shown on the zone while the drag is
/// over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropOutcome {
    /// The view is shown in the slot.
    Show,
    /// The view and the one the slot shows trade places.
    Swap(DrawerView),
    /// The view takes the slot, the one shown there leaving to the rail.
    Replace(DrawerView),
    /// A new slot on that side of the slot.
    NewSlot(Dir),
    /// A slot at that window edge.
    AtEdge(Side),
    /// The window has no room for the new slot: the drop changes nothing
    /// and says so.
    NoRoom(DrawerView),
    /// The dragged group and this one trade places.
    SwapGroups,
    /// The dragged group goes on that side of this one.
    SplitGroup(Dir),
    /// The dragged group joins this tab.
    IntoTab,
    /// The dragged group goes to a new tab.
    IntoNewTab,
}

impl DropOutcome {
    /// Whether the drop would change nothing for want of room: its zone and
    /// label show it in the error colour.
    pub(crate) fn is_no_room(self) -> bool {
        matches!(self, DropOutcome::NoRoom(_))
    }

    /// The colour of the zone and label this outcome belongs to.
    pub(crate) fn accent(self, theme: &Theme) -> Rgba {
        if self.is_no_room() {
            theme::error(theme)
        } else {
            theme::focus_border(theme)
        }
    }

    /// The colour of the label's text: the error colour for no room.
    pub(crate) fn label_colour(self, theme: &Theme) -> Rgba {
        if self.is_no_room() {
            theme::error(theme)
        } else {
            theme::foreground(theme)
        }
    }

    /// The words on the zone.
    pub(crate) fn label(self) -> String {
        let name = |view| view_registry::spec(view).name;
        match self {
            DropOutcome::Show => "Show here".into(),
            DropOutcome::Swap(other) => format!("Swap with {}", name(other)),
            DropOutcome::Replace(other) => format!("Replace {}", name(other)),
            DropOutcome::NewSlot(dir) => new_slot_label(dir).into(),
            DropOutcome::AtEdge(side) => window_strip_label(side).into(),
            DropOutcome::NoRoom(view) => format!("No room for {} here", name(view)),
            DropOutcome::SwapGroups => "Swap notes".into(),
            DropOutcome::SplitGroup(dir) => match dir {
                Dir::Up => "Split above",
                Dir::Down => "Split below",
                Dir::Left => "Split to the left",
                Dir::Right => "Split to the right",
            }
            .into(),
            DropOutcome::IntoTab => "Move to this tab".into(),
            DropOutcome::IntoNewTab => "Move to a new tab".into(),
        }
    }
}

impl Workspace {
    /// The `on_drag` constructor of a view's drag sources (slot header,
    /// rail cell): the ghost, remembered so the zones know a view, not a
    /// divider, is being dragged.
    pub(super) fn view_drag_ghost(
        &self,
        cx: &Context<Self>,
    ) -> impl Fn(&ViewDrag, Point<Pixels>, &mut Window, &mut App) -> Entity<DragChip> + 'static
    {
        let this = cx.entity().downgrade();
        move |drag, _, _, cx| {
            let chip = cx.new(|_| DragChip::new(drag.view));
            Self::remember_drag(&this, Dragged::View(*drag), &chip, cx);
            chip
        }
    }

    /// The `on_drag` constructor of a group header: the ghost, titled
    /// `title`, remembered so the zones know a group is being dragged.
    pub(super) fn group_drag_ghost(
        &self,
        title: SharedString,
        cx: &Context<Self>,
    ) -> impl Fn(&GroupDrag, Point<Pixels>, &mut Window, &mut App) -> Entity<DragChip> + 'static
    {
        let this = cx.entity().downgrade();
        move |drag, _, _, cx| {
            let chip = cx.new(|_| DragChip::group(title.clone()));
            Self::remember_drag(&this, Dragged::Group(*drag), &chip, cx);
            chip
        }
    }

    /// Remember the drag just started, with its ghost.
    fn remember_drag(
        this: &WeakEntity<Self>,
        drag: Dragged,
        chip: &Entity<DragChip>,
        cx: &mut App,
    ) {
        let live = LiveDrag {
            drag,
            chip: chip.downgrade(),
        };
        this.update(cx, |this, cx| {
            this.drag = Some(live);
            cx.notify();
        })
        .ok();
    }

    /// The group being dragged by its header, while its drag lives.
    pub(crate) fn dragged_group(&self, cx: &App) -> Option<GroupId> {
        match self.live_drag(cx)? {
            Dragged::Group(drag) => Some(drag.group),
            Dragged::View(_) => None,
        }
    }

    /// The drag under way, while it lives: GPUI drops the ghost when the
    /// drag ends (released, cancelled).
    fn live_drag(&self, cx: &App) -> Option<Dragged> {
        let live = self.drag.as_ref()?;
        (cx.has_active_drag() && live.chip.upgrade().is_some()).then_some(live.drag)
    }

    /// Work out [`Self::drop_outcomes`] for the drag: what a drop on each
    /// zone that is drawn would do, so the zone under the pointer can say
    /// so. Cleared when no drag is live.
    pub(super) fn refresh_drop_outcomes(&mut self, window: &Window, cx: &App) {
        let Some(drag) = self.zones_for(cx) else {
            self.drop_outcomes.clear();
            self.drop_outcomes_for = None;
            self.hovered_zone = None;
            return;
        };
        // The layout cannot change while the drag lives (a drop ends it), so
        // the outcomes are worked out once per drag and window size, not
        // every frame.
        // The window may change size mid-drag, and with it the room.
        let key = (drag, window.viewport_size());
        if self.drop_outcomes_for == Some(key) {
            return;
        }
        self.drop_outcomes.clear();
        self.drop_outcomes_for = Some(key);
        match drag {
            Dragged::View(ViewDrag { view, from }) => {
                for target in self.drop_targets() {
                    if self.drop_target_allowed(view, target) {
                        let outcome = self.drop_outcome(view, from, target, window);
                        self.drop_outcomes.insert(target, outcome);
                    }
                }
            }
            Dragged::Group(GroupDrag { group }) => {
                for target in self.group_drop_targets() {
                    if let Some(outcome) = self.group_drop_outcome(group, target) {
                        self.drop_outcomes.insert(target, outcome);
                    }
                }
            }
        }
    }

    /// Every zone drawn for a drag, in layout order: the five of each drawn
    /// slot, then the two window-edge strips. Not filtered by what a drop
    /// would change ([`Self::drop_target_allowed`]).
    pub(super) fn drop_targets(&self) -> Vec<DropTarget> {
        let slot_zones = self
            .layout
            .slots()
            .into_iter()
            .filter(|slot| self.layout.shown(*slot).is_some())
            .flat_map(|slot| Zone::ALL.map(|zone| DropTarget::Slot(slot, zone)));
        let strips = STRIP_SIDES.map(DropTarget::Window);
        slot_zones.chain(strips).collect()
    }

    /// Where a drop of `view` on `target` places it: a rail drag replaces
    /// the view a slot shows where a header drag swaps with it.
    fn resolved_placement(&self, from: DragFrom, target: DropTarget) -> Option<Placement> {
        match (from, target.placement()?) {
            (DragFrom::Rail, Placement::Into(slot)) => Some(Placement::Replace(slot)),
            (_, placement) => Some(placement),
        }
    }

    /// What dropping `view` on `target` would do, without doing it: the
    /// room for a new slot is tried and undone.
    pub(super) fn drop_outcome(
        &mut self,
        view: DrawerView,
        from: DragFrom,
        target: DropTarget,
        window: &Window,
    ) -> DropOutcome {
        let Some(placement) = self.resolved_placement(from, target) else {
            // Not a view's target ([`Self::drop_target_allowed`] says so).
            return DropOutcome::Show;
        };
        match placement {
            Placement::Into(slot) => self
                .swap_partner(view, slot)
                .map_or(DropOutcome::Show, DropOutcome::Swap),
            Placement::Replace(slot) => self
                .layout
                .shown(slot)
                .filter(|shown| *shown != view)
                .map_or(DropOutcome::Show, DropOutcome::Replace),
            Placement::Beside(slot, dir) => {
                if self.new_slot_fits(view, Leaf::Slot(slot), dir, window) {
                    DropOutcome::NewSlot(dir)
                } else {
                    DropOutcome::NoRoom(view)
                }
            }
            // A kept-empty slot is filled, which needs no new slot.
            Placement::AtEdge(side) if self.window_edge_slot(side).is_some() => {
                DropOutcome::AtEdge(side)
            }
            Placement::AtEdge(side) => {
                if self.new_slot_fits(view, Leaf::Editor, side.dir(), window) {
                    DropOutcome::AtEdge(side)
                } else {
                    DropOutcome::NoRoom(view)
                }
            }
        }
    }

    /// Forget a drag that is over. A release off every zone runs no
    /// `on_drop`, so [`Self::drag`] would keep a dead ghost; nothing
    /// reads it past [`Self::live_drag`]'s liveness check, but a stale
    /// `Some` is state that no longer describes anything. Drawing calls
    /// this first.
    pub(super) fn prune_drag(&mut self, cx: &App) {
        if self.drag.is_some() && self.live_drag(cx).is_none() {
            self.drag = None;
        }
    }

    /// The drag the zones are drawn for, not while zoomed (the other slots
    /// and groups are off screen). No modal check: a modal opening ends
    /// the drag ([`Self::end_drag`]), a dialog's overlay keeps a drag from
    /// starting under it, and a drop checks for an open dialog itself;
    /// `self.modal` can outlive a dialog the kit closed.
    pub(super) fn zones_for(&self, cx: &App) -> Option<Dragged> {
        self.live_drag(cx)
            .filter(|_| self.layout.zoomed().is_none())
    }

    /// The view whose drag the zones are drawn for (the outer layer and
    /// the slot zones), if a view is dragged.
    fn view_zones_for(&self, cx: &App) -> Option<ViewDrag> {
        match self.zones_for(cx)? {
            Dragged::View(drag) => Some(drag),
            Dragged::Group(_) => None,
        }
    }

    /// End a view's or a group's drag, if one is live, with nothing
    /// dropped; its zones go. A modal or the leader arming calls it: either
    /// takes the keys, and a modal would cover the zones. True when a drag
    /// ended.
    pub(super) fn end_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.live_drag(cx).is_none() {
            return false;
        }
        self.drag = None;
        cx.stop_active_drag(window);
        cx.notify();
        true
    }

    /// Esc during a drag ends it, and is spent on that. True when it did.
    pub(super) fn escape_ends_drag(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let esc = input::key_input(&event.keystroke).is_some_and(|key| key.key == Key::Esc);
        if !esc || !self.end_drag(window, cx) {
            return false;
        }
        cx.stop_propagation();
        true
    }

    /// The zones over a drawn slot's body while a view is dragged: a centre
    /// and four edges of `slot`, each a quarter of the body deep. A zone
    /// where the drop would change nothing is left out. `None` while no
    /// view is dragged.
    pub(super) fn zone_layer(&self, slot: SlotId, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        self.view_zones_for(cx)?;
        let shown = self.layout.shown(slot)?;
        Some(self.zone_grid(
            |zone| DropTarget::Slot(slot, zone),
            |zone| zone_id(shown, zone),
            cx,
        ))
    }

    /// The five zones over a body (a slot's, a group's): the centre and
    /// four edges, each edge a quarter of the body deep (at least
    /// [`EDGE_ZONE_MIN`]), `target` naming what each is and `id` its
    /// element id; and the label of the one under the drag, at the body's
    /// centre.
    pub(super) fn zone_grid(
        &self,
        target: impl Fn(Zone) -> DropTarget,
        id: impl Fn(Zone) -> SharedString,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let quarter = relative(0.25);
        // Each zone fills a cell of a grid that tiles the body: the left and
        // right edges run its full height, the top and bottom edges and the
        // centre share the column between them. The cells never overlap, so
        // one pointer position is one zone. A zone that is not drawn leaves
        // its cell empty, keeping the others where they are.
        let mut cell = |zone: Zone| {
            let el = self
                .drop_zone(id(zone), target(zone), cx)
                .map(|el| el.inset_0().test_support().into_any_element());
            div().relative().children(el)
        };
        let side = |cell: gpui_kit::Div| {
            cell.flex_none()
                .h_full()
                .w(quarter)
                .min_w(px(EDGE_ZONE_MIN))
        };
        let band = |cell: gpui_kit::Div| {
            cell.flex_none()
                .w_full()
                .h(quarter)
                .min_h(px(EDGE_ZONE_MIN))
        };
        let left = side(cell(Zone::Edge(Dir::Left)));
        let up = band(cell(Zone::Edge(Dir::Up)));
        let centre = cell(Zone::Centre).flex_1().min_h_0().w_full();
        let down = band(cell(Zone::Edge(Dir::Down)));
        let right = side(cell(Zone::Edge(Dir::Right)));
        let grid = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_row()
            .child(left)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(up)
                    .child(centre)
                    .child(down),
            )
            .child(right);
        // The labels lie over every zone, at the centre of the body, so no
        // zone's border crosses one and a narrow zone does not clip it.
        let tags = Zone::ALL.into_iter().filter_map(|zone| {
            let chip = self.zone_tag(target(zone), cx)?;
            Some(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(chip)
                    .into_any_element(),
            )
        });
        div().absolute().inset_0().child(grid).children(tags)
    }

    /// The window-edge strips over the editor Panel while a view is
    /// dragged: [`WINDOW_STRIP`] px at its left and right edge, filling
    /// that side's kept-empty slot, else making a column beside the editor. Empty while no view is
    /// dragged; a strip where the drop would change nothing is left out.
    pub(super) fn window_strips(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // The outer layer is a view's alone: a group's drag draws the inner
        // one, so the two never share a pixel.
        if self.view_zones_for(cx).is_none() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for side in STRIP_SIDES {
            let target = DropTarget::Window(side);
            if let Some(el) = self.drop_zone(window_strip_id(side), target, cx) {
                let el = el.top_0().bottom_0().w(px(WINDOW_STRIP));
                let el = match side {
                    Side::Left => el.left_0(),
                    Side::Right => el.right_0(),
                };
                out.push(el.test_support().into_any_element());
            }
            // The label lies over the editor, just inside the strip.
            if let Some(chip) = self.zone_tag(target, cx) {
                let row = div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .child(chip);
                let row = match side {
                    Side::Left => row.left(px(WINDOW_STRIP)),
                    Side::Right => row.right(px(WINDOW_STRIP)),
                };
                out.push(row.into_any_element());
            }
        }
        out
    }

    /// The label of the zone `target`: what a drop there would do, while
    /// the drag is over that zone ([`Self::hovered_zone`]). `None` when it
    /// is not, or the zone is not drawn. It is placed by the caller, over
    /// the zones. GPUI's group styling cannot do this: a group reaches
    /// only the elements inside it, and the label lies over the zones, not
    /// in one.
    fn zone_tag(&self, target: DropTarget, cx: &App) -> Option<gpui_kit::Div> {
        if self.hovered_zone != Some(target) {
            return None;
        }
        let outcome = self.drop_outcomes.get(&target).copied()?;
        let theme = theme::current(cx);
        let accent = outcome.accent(&theme);
        Some(
            div()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(theme::slot_header_background(&theme))
                .border_1()
                .border_color(accent)
                .text_xs()
                .text_color(outcome.label_colour(&theme))
                .whitespace_nowrap()
                .child(SharedString::from(outcome.label())),
        )
    }

    /// One zone, unplaced (the caller places it and registers it for
    /// tests, last): a dashed outline while the drag lives, filled with the
    /// focus colour while the drag is over it. `None` when dropping there
    /// would change nothing, so it neither highlights nor takes the drop.
    /// A view's zone takes a [`ViewDrag`], a group's a [`GroupDrag`].
    pub(super) fn drop_zone(
        &self,
        id: impl Into<ElementId>,
        target: DropTarget,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<gpui_kit::Div>> {
        // [`Self::refresh_drop_outcomes`] gave every allowed zone its
        // outcome, so having one is what makes a zone drawn.
        let outcome = self.drop_outcomes.get(&target).copied()?;
        let theme = theme::current(cx);
        let accent = outcome.accent(&theme);
        // What a drop here would do, for the accessibility tree too.
        let label = SharedString::from(outcome.label());
        let fill = Rgba {
            a: ZONE_FILL_ALPHA,
            ..accent
        };
        let rest_border = Rgba {
            a: ZONE_REST_ALPHA,
            ..accent
        };
        // Resting: every zone drawn for the drag shows a dashed outline, so
        // the places to drop are visible before the pointer is over one.
        let mut zone = div()
            .id(id)
            .role(Role::Group)
            .aria_label(label)
            .absolute()
            .rounded_md()
            .border_1()
            .border_color(rest_border)
            .cursor_grabbing();
        zone.style().border_style = Some(gpui_kit::BorderStyle::Dashed);
        Some(if target.takes_group() {
            Self::zone_listeners::<GroupDrag>(zone, target, fill, accent, cx)
        } else {
            Self::zone_listeners::<ViewDrag>(zone, target, fill, accent, cx)
        })
    }

    /// `zone`'s listeners for a drag carrying `P`: it notes the drag over
    /// it (its label shows), highlights under it, and takes its release.
    fn zone_listeners<P: DropPayload>(
        zone: Stateful<gpui_kit::Div>,
        target: DropTarget,
        fill: Rgba,
        accent: Rgba,
        cx: &mut Context<Self>,
    ) -> Stateful<gpui_kit::Div> {
        zone.on_drag_move::<P>(cx.listener(move |this, event: &DragMoveEvent<P>, _, cx| {
            let over = event.bounds.contains(&event.event.position);
            let hovered = if over {
                Some(target)
            } else if this.hovered_zone == Some(target) {
                None
            } else {
                return;
            };
            if this.hovered_zone != hovered {
                this.hovered_zone = hovered;
                cx.notify();
            }
        }))
        .drag_over::<P>(move |mut style, _, _, _| {
            // Under the drag: a solid, stronger border and the fill.
            style.border_style = Some(gpui_kit::BorderStyle::Solid);
            style.bg(fill).border_2().border_color(accent)
        })
        .on_drop(
            cx.listener(move |this, drag: &P, window, cx| drag.dropped(this, target, window, cx)),
        )
    }

    /// A release on a zone: drop the view there, unless a modal opened
    /// over the drag (it has the keyboard; the zones are gone with it).
    fn drop_dragged(
        &mut self,
        view: DrawerView,
        from: DragFrom,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = None;
        if modal::dialog_open(window, cx) {
            return;
        }
        self.drop_view(view, from, target, window, cx);
    }

    /// A release of a group's drag on a zone: drop the group there, unless
    /// a modal opened over the drag.
    fn drop_dragged_group(
        &mut self,
        group: GroupId,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = None;
        if modal::dialog_open(window, cx) {
            return;
        }
        self.drop_group(group, target, window, cx);
    }

    /// The slot a window-edge strip fills: the first slot outside the
    /// editor region on that side, when it is kept empty (the last slot of
    /// a side stays after its view left, drawing nothing). A side with a
    /// drawn slot has none: the strip makes a column beside the editor
    /// region there ([`Self::spawn_at`]). Not relative to the active group,
    /// so the strip means the same whatever has focus.
    pub(super) fn window_edge_slot(&self, side: Side) -> Option<SlotId> {
        self.layout
            .edge_slots(side)
            .first()
            .copied()
            .filter(|slot| self.layout.shown(*slot).is_none())
    }

    /// Whether the window-edge strip of `side` has nothing to do for
    /// `view`: the only drawn slot out there is the one showing it, so a
    /// column made there would be the column it is in.
    fn window_edge_is_view_alone(&self, side: Side, view: DrawerView) -> bool {
        let drawn: Vec<SlotId> = self
            .layout
            .edge_slots(side)
            .into_iter()
            .filter(|slot| self.layout.shown(*slot).is_some())
            .collect();
        matches!(drawn.as_slice(), [only] if self.layout.shown(*only) == Some(view))
    }

    /// Whether dropping `view` on `target` changes anything: not while
    /// zoomed (the other slots are not on screen), not on the centre of the
    /// slot already showing it (a view hidden there is shown by the drop),
    /// not on an edge of the slot already showing it (a drop would split
    /// its own stack; the keyboard and the chooser still can; a view
    /// hidden there is split out), not on a
    /// window edge whose only drawn slot already shows it, and not on a slot
    /// that is gone. A view no slot holds is placed by any zone.
    pub(crate) fn drop_target_allowed(&self, view: DrawerView, target: DropTarget) -> bool {
        if self.layout.zoomed().is_some() {
            return false;
        }
        match target {
            DropTarget::Slot(slot, zone) => {
                if !self.layout.slots().contains(&slot) {
                    return false;
                }
                match zone {
                    Zone::Centre => self.layout.shown(slot) != Some(view),
                    Zone::Edge(_) => self.layout.shown(slot) != Some(view),
                }
            }
            DropTarget::Window(side) => !self.window_edge_is_view_alone(side, view),
            // A group's targets.
            DropTarget::Group(..) | DropTarget::Tab(_) | DropTarget::NewTab => false,
        }
    }

    /// Drop `view` on `target`. A centre shows it in that slot, the view
    /// shown there staying placed, hidden ([`Self::move_into`]); an edge
    /// puts it in a new slot on that side of the slot ([`Self::spawn_beside`]);
    /// a window-edge strip does the first for that side's kept-empty slot
    /// ([`Self::window_edge_slot`]), else makes a column at that edge of the
    /// whole editor region ([`Self::spawn_at`]). A drop that makes a slot
    /// makes room for it; with no room, nothing changes and a flash says so
    /// on release (the zone said so before, in red:
    /// [`DropOutcome::NoRoom`]). A drag from the rail ([`DragFrom::Rail`])
    /// replaces the view a slot shows instead of swapping
    /// ([`Self::replace_into`]). False, nothing changed, when
    /// [`Self::drop_target_allowed`] says so.
    pub(crate) fn drop_view(
        &mut self,
        view: DrawerView,
        from: DragFrom,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.drop_target_allowed(view, target) {
            return false;
        }
        let Some(placement) = self.resolved_placement(from, target) else {
            return false;
        };
        self.place(view, placement, window, cx)
    }
}

/// What a drag carries, as a zone's release hands it on.
trait DropPayload: 'static {
    fn dropped(
        &self,
        workspace: &mut Workspace,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    );
}

impl DropPayload for ViewDrag {
    fn dropped(
        &self,
        workspace: &mut Workspace,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        workspace.drop_dragged(self.view, self.from, target, window, cx)
    }
}

impl DropPayload for GroupDrag {
    fn dropped(
        &self,
        workspace: &mut Workspace,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        workspace.drop_dragged_group(self.group, target, window, cx)
    }
}
