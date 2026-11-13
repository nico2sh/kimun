//! Dropping a view on a slot zone or a window-edge strip (ADR-0054 §8).
//! A drag of a slot header or a rail entry carries a [`ViewDrag`]; while it
//! is live each drawn slot overlays its body with a [`Workspace::zone_layer`]
//! (a centre and four edges) and the editor Panel adds a strip at its left
//! and right edge ([`Workspace::window_strips`]). A zone is drawn only where
//! [`Workspace::drop_target_allowed`] holds, and highlights while the drag
//! is over it. A release on it runs [`Workspace::drop_view`], the steps of
//! the move chooser: the slot the view leaves closes when emptied
//! ([`Workspace::close_if_vacated`]), the keyboard follows the view, the
//! layout is saved. Esc, a modal or the leader opening ends the drag with
//! nothing changed; while zoomed no zone is drawn.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    KeystrokeEvent, ParentElement as _, Pixels, Point, Render, Rgba, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px, relative,
};
use kimun_notes::logic::input::Key;
use kimun_notes::logic::layout::{Dir, Side};
use kimun_notes::logic::layout_tree::SlotId;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::placement::Placement;
use super::{Workspace, modal};
use crate::{input, theme};

/// How wide a window-edge strip is, in px: room to aim at without hiding
/// the editor.
const WINDOW_STRIP: f32 = 24.;

/// The opacity of a zone's highlight (the focus colour), so the view
/// under it still reads in light and dark themes.
const ZONE_FILL_ALPHA: f32 = 0.25;

/// The payload of a drag: the view being placed (from a slot header, or a
/// rail entry whatever slot holds it, if any).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewDrag(pub DrawerView);

/// The element id of the ghost a drag of `view` shows (`drag-chip-files`).
pub(crate) fn drag_chip_id(view: DrawerView) -> SharedString {
    format!("drag-chip-{}", view.id()).into()
}

/// The element id of `zone` of the slot showing `view`
/// (`drop-outline-centre`, `drop-outline-down`).
pub(crate) fn zone_id(view: DrawerView, zone: Zone) -> SharedString {
    let zone = match zone {
        Zone::Centre => "centre",
        Zone::Edge(Dir::Up) => "up",
        Zone::Edge(Dir::Down) => "down",
        Zone::Edge(Dir::Left) => "left",
        Zone::Edge(Dir::Right) => "right",
    };
    format!("drop-{}-{zone}", view.id()).into()
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
    view: DrawerView,
}

impl DragChip {
    pub(crate) fn new(view: DrawerView) -> Self {
        Self { view }
    }
}

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let title: SharedString = view_registry::spec(self.view).title.into();
        div()
            .id(drag_chip_id(self.view))
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropTarget {
    /// A zone of a drawn slot.
    Slot(SlotId, Zone),
    /// The strip at a window edge: that side's first slot, else a new one.
    Window(Side),
}

impl DropTarget {
    /// Where a drop here places the view.
    pub(crate) fn placement(self) -> Placement {
        match self {
            DropTarget::Slot(slot, Zone::Centre) => Placement::Into(slot),
            DropTarget::Slot(slot, Zone::Edge(dir)) => Placement::Beside(slot, dir),
            DropTarget::Window(side) => Placement::AtEdge(side),
        }
    }
}

/// A zone of a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Zone {
    /// Show the view in the slot.
    Centre,
    /// A new slot on that side of the slot.
    Edge(Dir),
}

/// The drag of a view under way: the view, and the ghost GPUI holds for as
/// long as the drag lives (GPUI exposes no accessor for the payload, and a
/// divider's drag is a drag too).
pub(crate) struct LiveViewDrag {
    view: DrawerView,
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

/// What the zone does, for the accessibility tree.
fn zone_label(zone: Zone) -> &'static str {
    match zone {
        Zone::Centre => "Show here",
        Zone::Edge(dir) => new_slot_label(dir),
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
            let chip = cx.new(|_| DragChip::new(drag.0));
            let live = LiveViewDrag {
                view: drag.0,
                chip: chip.downgrade(),
            };
            this.update(cx, |this, cx| {
                this.view_drag = Some(live);
                cx.notify();
            })
            .ok();
            chip
        }
    }

    /// The view being dragged, while its drag lives: GPUI drops the ghost
    /// when the drag ends (released, cancelled).
    pub(crate) fn dragged_view(&self, cx: &App) -> Option<DrawerView> {
        let live = self.view_drag.as_ref()?;
        (cx.has_active_drag() && live.chip.upgrade().is_some()).then_some(live.view)
    }

    /// Forget a view's drag that is over. A release off every zone runs no
    /// `on_drop`, so [`Self::view_drag`] would keep a dead ghost; nothing
    /// reads it past [`Self::dragged_view`]'s liveness check, but a stale
    /// `Some` is state that no longer describes anything. Drawing calls
    /// this first.
    pub(super) fn prune_view_drag(&mut self, cx: &App) {
        if self.view_drag.is_some() && self.dragged_view(cx).is_none() {
            self.view_drag = None;
        }
    }

    /// The view the zones are drawn for: a view's drag, not while zoomed
    /// (the other slots are off screen). No modal check: a modal opening
    /// ends the drag ([`Self::end_view_drag`]), a dialog's overlay keeps a
    /// drag from starting under it, and a drop checks for an open dialog
    /// itself; `self.modal` can outlive a dialog the kit closed.
    fn zones_for(&self, cx: &App) -> Option<DrawerView> {
        self.dragged_view(cx)
            .filter(|_| self.layout.zoomed().is_none())
    }

    /// End a view's drag, if one is live, with nothing dropped; its zones
    /// go. A modal or the leader arming calls it: either takes the keys,
    /// and a modal would cover the zones. True when a drag ended.
    pub(super) fn end_view_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.dragged_view(cx).is_none() {
            return false;
        }
        self.view_drag = None;
        cx.stop_active_drag(window);
        cx.notify();
        true
    }

    /// Esc during a view's drag ends it, and is spent on that. True when
    /// it did.
    pub(super) fn escape_ends_view_drag(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let esc = input::key_input(&event.keystroke).is_some_and(|key| key.key == Key::Esc);
        if !esc || !self.end_view_drag(window, cx) {
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
        let dragged = self.zones_for(cx)?;
        let shown = self.layout.shown(slot)?;
        let quarter = relative(0.25);
        let zones = [
            Zone::Centre,
            Zone::Edge(Dir::Left),
            Zone::Edge(Dir::Right),
            Zone::Edge(Dir::Up),
            Zone::Edge(Dir::Down),
        ]
        .into_iter()
        .filter_map(|zone| {
            let target = DropTarget::Slot(slot, zone);
            let el = self.drop_zone(zone_id(shown, zone), zone_label(zone), dragged, target, cx)?;
            let el = match zone {
                Zone::Centre => el.top(quarter).bottom(quarter).left(quarter).right(quarter),
                Zone::Edge(Dir::Left) => el.top_0().bottom_0().left_0().w(quarter),
                Zone::Edge(Dir::Right) => el.top_0().bottom_0().right_0().w(quarter),
                Zone::Edge(Dir::Up) => el.top_0().left(quarter).right(quarter).h(quarter),
                Zone::Edge(Dir::Down) => el.bottom_0().left(quarter).right(quarter).h(quarter),
            };
            Some(el.test_support().into_any_element())
        });
        Some(div().absolute().inset_0().children(zones))
    }

    /// The window-edge strips over the editor Panel while a view is
    /// dragged: [`WINDOW_STRIP`] px at its left and right edge, filling
    /// that side's first slot, else making one. Empty while no view is
    /// dragged; a strip where the drop would change nothing is left out.
    pub(super) fn window_strips(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(dragged) = self.zones_for(cx) else {
            return Vec::new();
        };
        [Side::Left, Side::Right]
            .into_iter()
            .filter_map(|side| {
                let el = self.drop_zone(
                    window_strip_id(side),
                    window_strip_label(side),
                    dragged,
                    DropTarget::Window(side),
                    cx,
                )?;
                let el = el.top_0().bottom_0().w(px(WINDOW_STRIP));
                let el = match side {
                    Side::Left => el.left_0(),
                    Side::Right => el.right_0(),
                };
                Some(el.test_support().into_any_element())
            })
            .collect()
    }

    /// One zone, unplaced (the caller places it and registers it for
    /// tests, last): invisible until the drag of `dragged` is over
    /// it, then filled with the focus colour. `None` when dropping there
    /// would change nothing, so it neither highlights nor takes the drop.
    fn drop_zone(
        &self,
        id: SharedString,
        label: &'static str,
        dragged: DrawerView,
        target: DropTarget,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<gpui_kit::Div>> {
        if !self.drop_target_allowed(dragged, target) {
            return None;
        }
        let theme = theme::current(cx);
        let accent = theme::focus_border(&theme);
        let fill = Rgba {
            a: ZONE_FILL_ALPHA,
            ..accent
        };
        Some(
            div()
                .id(id)
                .role(Role::Group)
                .aria_label(label)
                .absolute()
                .rounded_md()
                .drag_over::<ViewDrag>(move |style, _, _, _| {
                    style.bg(fill).border_2().border_color(accent)
                })
                .on_drop(cx.listener(move |this, drag: &ViewDrag, window, cx| {
                    this.drop_dragged(drag.0, target, window, cx)
                })),
        )
    }

    /// A release on a zone: drop the view there, unless a modal opened
    /// over the drag (it has the keyboard; the zones are gone with it).
    fn drop_dragged(
        &mut self,
        view: DrawerView,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.view_drag = None;
        if modal::dialog_open(window, cx) {
            return;
        }
        self.drop_view(view, target, window, cx);
    }

    /// The slot a window-edge strip fills: the first slot outside the
    /// editor region on that side, drawn or kept empty. Not relative to
    /// the active group, so the strip means the same whatever has focus.
    pub(super) fn window_edge_slot(&self, side: Side) -> Option<SlotId> {
        self.layout.edge_slots(side).first().copied()
    }

    /// Whether dropping `view` on `target` changes anything: not while
    /// zoomed (the other slots are not on screen), not on the centre of the
    /// slot already showing it (a view hidden there is shown by the drop),
    /// not on an edge of the slot holding only that view (nothing to leave
    /// behind there), not on a window edge whose first slot already shows
    /// it, and not on a slot that is gone. A view
    /// no slot holds is placed by any zone.
    pub(crate) fn drop_target_allowed(&self, view: DrawerView, target: DropTarget) -> bool {
        if self.layout.zoomed().is_some() {
            return false;
        }
        let from = self.layout.view_slot(&view);
        match target {
            DropTarget::Slot(slot, zone) => {
                if !self.layout.slots().contains(&slot) {
                    return false;
                }
                match zone {
                    Zone::Centre => self.layout.shown(slot) != Some(view),
                    Zone::Edge(_) => {
                        !(from == Some(slot) && self.layout.slot_views(slot).len() < 2)
                    }
                }
            }
            DropTarget::Window(side) => self
                .window_edge_slot(side)
                .is_none_or(|slot| self.layout.shown(slot) != Some(view)),
        }
    }

    /// Drop `view` on `target`. A centre shows it in that slot, the view
    /// shown there staying placed, hidden ([`Self::move_into`]); an edge
    /// puts it in a new slot on that side of the slot ([`Self::spawn_beside`]);
    /// a window-edge strip does the first for the side's first slot, else
    /// makes a slot at that edge of the whole editor region
    /// ([`Self::spawn_at`]). A drop that makes a slot makes room for it;
    /// with no room, nothing changes and a flash says so on release (the
    /// zone is drawn all the same: whether it fits is worked out only
    /// then). False, nothing changed, when [`Self::drop_target_allowed`]
    /// says so.
    pub(crate) fn drop_view(
        &mut self,
        view: DrawerView,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.drop_target_allowed(view, target) {
            return false;
        }
        self.place(view, target.placement(), window, cx)
    }
}
