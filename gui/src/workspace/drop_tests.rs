//! Tests for dropping a view on a slot zone or a window-edge strip
//! (ADR-0054 §8): the resolution of a drop to the same steps as the move
//! chooser, with no pointer involved. A real `Workspace` in a headless
//! window.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, point, px,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout::{Dir, Side};
use kimun_notes::logic::layout_tree::{Focus, LayoutTree, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::GuiSettings;

use super::drop_zones::{DropTarget, Zone, window_strip_id, zone_id};
use super::modal::ModalKind;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use super::tests::{Shell, gui, open_shell, saved, temp_config};
use super::tree_view::move_button_id;
use super::{LayoutStore, rail_cell_id, slot_id};
use crate::shell_tree;
use crate::startup::FirstNote;
use crate::test_vault::test_vault;

/// FILES over FIND on the left (one view each), OUTLINE right of the editor.
fn stacked_left_and_outline_right() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

/// One slot left of the editor holding FILES (shown) and FIND (hidden).
fn files_over_find_hidden() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\", \"find\"], shown = \"files\" },\n\
         { weight = 0.8, editor = true },\n]\n")
}

/// One slot left of the editor holding only FILES.
fn files_alone() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.8, editor = true },\n]\n")
}

fn layout(shell: &Shell, cx: &gpui_kit::App) -> LayoutTree<DrawerView> {
    shell.workspace.read(cx).layout.clone()
}

fn slots_of(shell: &Shell, side: Side, cx: &gpui_kit::App) -> Vec<SlotId> {
    shell.workspace.read(cx).side_slots(side)
}

fn slot_of(shell: &Shell, view: DrawerView, cx: &gpui_kit::App) -> SlotId {
    layout(shell, cx).view_slot(&view).expect("view is placed")
}

/// Drop `view` on `target` as the pointer would: ask, then run.
fn drop_view(
    shell: &Shell,
    view: DrawerView,
    target: DropTarget,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) -> bool {
    let allowed = shell.workspace.read(cx).drop_target_allowed(view, target);
    let ran = shell
        .workspace
        .update(cx, |w, cx| w.drop_view(view, target, window, cx));
    assert_eq!(allowed, ran, "highlighting and the drop agree");
    ran
}

/// The views placed in the saved layout's slots, in reading order.
fn saved_placed(path: &std::path::Path) -> Vec<Vec<DrawerView>> {
    let tree = shell_tree::restore(&saved(path), shell_tree::RESTORE_CAPS);
    tree.slots()
        .into_iter()
        .map(|s| tree.slot_views(s))
        .collect()
}

/// The centre of another slot shows the view there; the slot it left,
/// holding nothing now, closes (another slot is on its side).
#[gpui_kit::test]
fn centre_drop_shows_the_view_in_that_slot_and_closes_the_vacated_one(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let files = slot_of(&shell, DrawerView::Files, cx);
        let find = slot_of(&shell, DrawerView::Find, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Find,
            DropTarget::Slot(files, Zone::Centre),
            window,
            cx
        ));
        let tree = layout(&shell, cx);
        assert_eq!(tree.shown(files), Some(DrawerView::Find));
        assert_eq!(
            tree.slot_views(files),
            [DrawerView::Files, DrawerView::Find]
        );
        assert!(!tree.slots().contains(&find), "the vacated slot closed");
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(files)));
    })
    .unwrap();
}

/// An edge makes a slot on that side of the target; the view's slot holds
/// other views, so it stays. The keyboard follows the view.
#[gpui_kit::test]
fn edge_drop_makes_a_slot_on_that_side(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let old = slot_of(&shell, DrawerView::Files, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Slot(old, Zone::Edge(Dir::Down)),
            window,
            cx
        ));
        let left = slots_of(&shell, Side::Left, cx);
        assert_eq!(left.len(), 2);
        assert_eq!(left[0], old);
        let tree = layout(&shell, cx);
        assert_eq!(tree.shown(old), Some(DrawerView::Find));
        assert_eq!(tree.shown(left[1]), Some(DrawerView::Files));
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(left[1])));
    })
    .unwrap();
}

/// A view alone in its slot has nothing to leave behind on its own edge.
#[gpui_kit::test]
fn edge_drop_of_the_only_view_of_a_slot_does_nothing(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &files_alone(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let own = slot_of(&shell, DrawerView::Files, cx);
        let before = layout(&shell, cx);
        for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
            assert!(!drop_view(
                &shell,
                DrawerView::Files,
                DropTarget::Slot(own, Zone::Edge(dir)),
                window,
                cx
            ));
        }
        assert_eq!(layout(&shell, cx), before);
    })
    .unwrap();
}

/// The centre of the view's own slot changes nothing.
#[gpui_kit::test]
fn centre_drop_on_own_slot_does_nothing(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let own = slot_of(&shell, DrawerView::Files, cx);
        let before = layout(&shell, cx);
        assert!(!drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Slot(own, Zone::Centre),
            window,
            cx
        ));
        assert_eq!(layout(&shell, cx), before);
    })
    .unwrap();
}

/// A view hidden in a slot is not "already there": the centre drop shows
/// it, so the zone is allowed and the drop changes the layout.
#[gpui_kit::test]
fn centre_drop_of_a_hidden_view_on_its_own_slot_shows_it(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let own = slot_of(&shell, DrawerView::Find, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Find,
            DropTarget::Slot(own, Zone::Centre),
            window,
            cx
        ));
        assert_eq!(layout(&shell, cx).shown(own), Some(DrawerView::Find));
    })
    .unwrap();
}

/// While zoomed the other slots are not on screen: no drop lands.
#[gpui_kit::test]
fn drop_while_zoomed_does_nothing(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let files = slot_of(&shell, DrawerView::Files, cx);
        shell
            .workspace
            .update(cx, |w, _| w.layout.zoom(Some(Leaf::Slot(files))));
        let before = layout(&shell, cx);
        for target in [
            DropTarget::Slot(files, Zone::Edge(Dir::Down)),
            DropTarget::Slot(files, Zone::Centre),
            DropTarget::Window(Side::Right),
        ] {
            assert!(!drop_view(&shell, DrawerView::Find, target, window, cx));
        }
        assert_eq!(layout(&shell, cx), before);
    })
    .unwrap();
}

/// The window edge fills the first slot of that side, here the one kept
/// (undrawn) after its view left; the left slot FILES left closes.
#[gpui_kit::test]
fn window_edge_fills_the_kept_slot_of_an_emptied_side(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let files = slot_of(&shell, DrawerView::Files, cx);
        let right = slot_of(&shell, DrawerView::Outline, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Outline,
            DropTarget::Slot(files, Zone::Centre),
            window,
            cx
        ));
        assert_eq!(layout(&shell, cx).shown(right), None, "kept, undrawn");
        assert_eq!(slots_of(&shell, Side::Right, cx), [right]);

        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Window(Side::Right),
            window,
            cx
        ));
        let tree = layout(&shell, cx);
        assert_eq!(tree.shown(right), Some(DrawerView::Files));
        assert_eq!(slots_of(&shell, Side::Right, cx), [right], "no new slot");
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(right)));
    })
    .unwrap();
}

/// A side with no slot at all gets one at the editor's edge.
#[gpui_kit::test]
fn window_edge_creates_a_slot_when_the_side_has_none(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(slots_of(&shell, Side::Right, cx).is_empty());
        let old = slot_of(&shell, DrawerView::Files, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Window(Side::Right),
            window,
            cx
        ));
        let right = slots_of(&shell, Side::Right, cx);
        assert_eq!(right.len(), 1);
        let tree = layout(&shell, cx);
        assert_eq!(tree.shown(right[0]), Some(DrawerView::Files));
        assert_eq!(tree.shown(old), Some(DrawerView::Find));
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(right[0])));

        // And the left edge of a window whose left side is that slot.
        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Window(Side::Left),
            window,
            cx
        ));
        assert_eq!(
            slots_of(&shell, Side::Left, cx).len(),
            1,
            "joined the first"
        );
    })
    .unwrap();
}

/// A view no slot holds (a rail entry) is placed by the drop, on a zone
/// of a slot or a window edge.
#[gpui_kit::test]
fn drop_of_a_view_no_slot_holds_places_it(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &files_alone(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let files = slot_of(&shell, DrawerView::Files, cx);
        assert_eq!(layout(&shell, cx).view_slot(&DrawerView::Find), None);
        assert!(drop_view(
            &shell,
            DrawerView::Find,
            DropTarget::Slot(files, Zone::Edge(Dir::Down)),
            window,
            cx
        ));
        let tree = layout(&shell, cx);
        let find = tree.view_slot(&DrawerView::Find).expect("placed");
        assert_ne!(find, files);
        assert_eq!(tree.shown(find), Some(DrawerView::Find));
        assert_eq!(tree.shown(files), Some(DrawerView::Files), "FILES stays");
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(find)));

        // Another unplaced view, on a centre and on a window edge.
        assert_eq!(tree.view_slot(&DrawerView::Outline), None);
        assert!(drop_view(
            &shell,
            DrawerView::Outline,
            DropTarget::Window(Side::Right),
            window,
            cx
        ));
        let right = slots_of(&shell, Side::Right, cx);
        assert_eq!(
            layout(&shell, cx).shown(right[0]),
            Some(DrawerView::Outline)
        );
        assert!(drop_view(
            &shell,
            DrawerView::Tags,
            DropTarget::Slot(files, Zone::Centre),
            window,
            cx
        ));
        assert_eq!(
            layout(&shell, cx).shown(files),
            Some(DrawerView::Tags),
            "shown in the slot, FILES hidden there"
        );
    })
    .unwrap();
}

/// A drop is persisted: the saved layout restores the placement.
#[gpui_kit::test]
fn drop_saves_the_layout(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let old = slot_of(&shell, DrawerView::Files, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Slot(old, Zone::Edge(Dir::Down)),
            window,
            cx
        ));
        window.render_frame(cx);
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Find], vec![DrawerView::Files]]
        );
    })
    .unwrap();
}

// --- Drag sources (ADR-0054 §8) ---

/// A point inside the element `id`, `dx`/`dy` from its top-left corner
/// (the centre when `None`), in a fresh frame.
fn point_in(
    shell: &Shell,
    id: impl Into<gpui_kit::ElementId>,
    offset: Option<Point<gpui_kit::Pixels>>,
    cx: &mut TestAppContext,
) -> Point<gpui_kit::Pixels> {
    let id = id.into();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find(id).bounds();
        match offset {
            Some(o) => bounds.origin + o,
            None => bounds.center(),
        }
    })
    .unwrap()
}

/// Press at `from`, then move `by` px right in small steps, the button
/// held: a drag when past the pointer threshold, else a wobble. Returns
/// the pointer's position; the button is still down.
fn press_and_move(
    shell: &Shell,
    from: Point<gpui_kit::Pixels>,
    by: f32,
    cx: &mut TestAppContext,
) -> Point<gpui_kit::Pixels> {
    let mut visual = VisualTestContext::from_window(shell.handle, cx);
    visual.simulate_mouse_move(from, None, Modifiers::default());
    visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    let mut at = from;
    for _ in 0..4 {
        at = point(at.x + px(by / 4.), at.y);
        visual.simulate_mouse_move(at, MouseButton::Left, Modifiers::default());
    }
    at
}

fn release(shell: &Shell, at: Point<gpui_kit::Pixels>, cx: &mut TestAppContext) {
    let mut visual = VisualTestContext::from_window(shell.handle, cx);
    visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
}

/// Whether the drag chip of `view` is drawn in a fresh frame (it names the
/// view the live drag carries).
fn chip_drawn(shell: &Shell, view: DrawerView, cx: &mut TestAppContext) -> bool {
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(super::drop_zones::drag_chip_id(view))
            .is_some_and(|e| e.visible())
    })
    .unwrap()
}

/// Pressing a slot header and moving past the threshold starts a drag of
/// the view that slot shows.
#[gpui_kit::test]
fn dragging_a_header_starts_a_view_drag(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let from = files_header(&shell, cx);
    let at = press_and_move(&shell, from, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "a drag is live");
    assert!(
        chip_drawn(&shell, DrawerView::Files, cx),
        "it carries FILES"
    );
    release(&shell, at, cx);
    assert!(!cx.update(|cx| cx.has_active_drag()));
}

/// Pressing a rail cell and moving past the threshold starts a drag of
/// that view, placed or not.
#[gpui_kit::test]
fn dragging_a_rail_cell_starts_a_view_drag(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let from = point_in(&shell, rail_cell_id(DrawerView::Tags), None, cx);
    let at = press_and_move(&shell, from, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "a drag is live");
    assert!(chip_drawn(&shell, DrawerView::Tags, cx), "it carries TAGS");
    release(&shell, at, cx);
}

/// A press and release on a rail cell, the pointer moving less than the
/// threshold, is still the cell's click: it toggles the view.
#[gpui_kit::test]
fn a_click_without_movement_still_activates_the_rail_cell(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let shown = |cx: &mut TestAppContext| {
        cx.read(|cx| shell.workspace.read(cx).is_shown(DrawerView::Files))
    };
    assert!(shown(cx));
    let from = point_in(&shell, rail_cell_id(DrawerView::Files), None, cx);
    let at = press_and_move(&shell, from, 1., cx);
    assert!(!cx.update(|cx| cx.has_active_drag()), "a wobble is no drag");
    release(&shell, at, cx);
    assert!(!shown(cx), "the click hid FILES");
}

/// The `⋯` button is not part of the drag source: a click opens the move
/// chooser.
#[gpui_kit::test]
fn the_move_button_still_clicks(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let button = move_button_id(DrawerView::Files);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(button, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(
            shell.workspace.read(cx).move_chooser_rows(cx).is_some(),
            "the move chooser is open"
        );
    });
}

// --- Carried over from the resolution tasks (ADR-0054 §1, §2) ---

/// FILES left of the editor, OUTLINE right of it, one view each.
fn files_left_outline_right() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

/// The editor with OUTLINE right of it: no slot on the left at all.
fn outline_right_only() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.8, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

/// A view alone in its slot, dropped on an edge of ANOTHER slot, moves
/// there; the slot it left, holding nothing and not the last on its side,
/// closes (§1, §2: only its own edges are refused).
#[gpui_kit::test]
fn a_lone_view_dropped_on_another_slots_edge_moves_and_its_slot_closes(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let files = slot_of(&shell, DrawerView::Files, cx);
        let find = slot_of(&shell, DrawerView::Find, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Slot(find, Zone::Edge(Dir::Down)),
            window,
            cx
        ));
        let tree = layout(&shell, cx);
        assert!(!tree.slots().contains(&files), "the vacated slot closed");
        let left = slots_of(&shell, Side::Left, cx);
        assert_eq!(left.len(), 2);
        assert_eq!(left[0], find);
        assert_eq!(tree.shown(left[1]), Some(DrawerView::Files));
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(left[1])));
    })
    .unwrap();
}

/// The only view of the last slot on the left, dropped on an edge of the
/// right side's slot: the left slot is kept, undrawn (§1), and the right
/// side shows the view.
#[gpui_kit::test]
fn the_last_view_of_a_side_dropped_on_the_other_side_keeps_its_slot_undrawn(
    cx: &mut TestAppContext,
) {
    let shell = open_shell(
        cx,
        1000.,
        &files_left_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let files = slot_of(&shell, DrawerView::Files, cx);
        let outline = slot_of(&shell, DrawerView::Outline, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Slot(outline, Zone::Edge(Dir::Down)),
            window,
            cx
        ));
        let tree = layout(&shell, cx);
        assert_eq!(slots_of(&shell, Side::Left, cx), [files], "kept");
        assert_eq!(tree.shown(files), None, "undrawn");
        assert!(tree.slot_views(files).is_empty());
        let right = slots_of(&shell, Side::Right, cx);
        assert_eq!(right.len(), 2);
        assert_eq!(tree.shown(right[1]), Some(DrawerView::Files));
        window.render_frame(cx);
        assert!(
            window.try_find("slot-files").is_some(),
            "drawn on the right"
        );
    })
    .unwrap();
}

/// A window-edge drop on the left, with no slot on the left at all, makes
/// one there holding the view.
#[gpui_kit::test]
fn window_edge_creates_a_left_slot_when_the_side_has_none(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &outline_right_only(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(slots_of(&shell, Side::Left, cx).is_empty());
        assert!(drop_view(
            &shell,
            DrawerView::Files,
            DropTarget::Window(Side::Left),
            window,
            cx
        ));
        let left = slots_of(&shell, Side::Left, cx);
        assert_eq!(left.len(), 1);
        let tree = layout(&shell, cx);
        assert_eq!(tree.shown(left[0]), Some(DrawerView::Files));
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(left[0])));
        assert_eq!(
            slots_of(&shell, Side::Right, cx).len(),
            1,
            "right untouched"
        );
    })
    .unwrap();
}

// --- The zone layer (ADR-0054 §8) ---

const EDGES: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

/// Whether the element `id` is drawn in a fresh frame.
fn drawn(shell: &Shell, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

/// The point on FILES's header a press grabs (clear of the `⋯` button).
fn files_header(shell: &Shell, cx: &mut TestAppContext) -> Point<gpui_kit::Pixels> {
    point_in(shell, "slot-files", Some(point(px(20.), px(18.))), cx)
}

/// Start a drag at `from` and hold the pointer over the element `to`.
/// Returns where the pointer is; the button is still down.
fn drag_onto(
    shell: &Shell,
    from: Point<gpui_kit::Pixels>,
    to: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) -> Point<gpui_kit::Pixels> {
    press_and_move(shell, from, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "a drag is live");
    let target = point_in(shell, to, None, cx);
    move_held(shell, target, cx);
    target
}

/// Move the pointer to `to`, the button held.
fn move_held(shell: &Shell, to: Point<gpui_kit::Pixels>, cx: &mut TestAppContext) {
    let mut visual = VisualTestContext::from_window(shell.handle, cx);
    visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
    cx.update_window(shell.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn press_key(shell: &Shell, key: &str, cx: &mut TestAppContext) {
    cx.update_window(shell.handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Every slot with its views and the one it shows, and the zoom: the
/// arrangement a drop would change. Not the focus: the press that starts
/// a drag gives its slot the keyboard, as any press does.
type Arrangement = (
    Vec<(SlotId, Vec<DrawerView>, Option<DrawerView>)>,
    Option<Leaf>,
);

fn layout_now(shell: &Shell, cx: &mut TestAppContext) -> Arrangement {
    cx.read(|cx| {
        let tree = layout(shell, cx);
        let slots = tree
            .slots()
            .into_iter()
            .map(|s| (s, tree.slot_views(s), tree.shown(s)))
            .collect();
        (slots, tree.zoomed())
    })
}

/// The zones are drawn while a view is dragged and gone once it is
/// released: every other drawn slot's centre and edges, and the
/// window-edge strip of a side that can take the view.
#[gpui_kit::test]
fn zones_show_only_while_dragging(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let all = |cx: &mut TestAppContext| {
        let mut ids = vec![
            zone_id(DrawerView::Find, Zone::Centre),
            zone_id(DrawerView::Outline, Zone::Centre),
            window_strip_id(Side::Right),
        ];
        for dir in EDGES {
            ids.push(zone_id(DrawerView::Find, Zone::Edge(dir)));
            ids.push(zone_id(DrawerView::Outline, Zone::Edge(dir)));
        }
        ids.into_iter()
            .map(|id| (id.clone(), drawn(&shell, id, cx)))
            .collect::<Vec<_>>()
    };
    assert!(all(cx).iter().all(|(_, d)| !d), "no zone before a drag");
    let before = layout_now(&shell, cx);

    let from = files_header(&shell, cx);
    let at = press_and_move(&shell, from, 40., cx);
    for (id, d) in all(cx) {
        assert!(d, "{id} is drawn during the drag");
    }
    release(&shell, at, cx);
    assert!(all(cx).iter().all(|(_, d)| !d), "no zone after the release");
    assert_eq!(layout_now(&shell, cx), before, "released off a zone");
}

/// Releasing the pointer on an edge zone of another slot puts the view in
/// a new slot on that side of it; the slot it left closes, the keyboard
/// follows it.
#[gpui_kit::test]
fn dropping_on_a_slot_edge_places_the_view_there(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let (files, outline) = cx.read(|cx| {
        (
            slot_of(&shell, DrawerView::Files, cx),
            slot_of(&shell, DrawerView::Outline, cx),
        )
    });
    let from = files_header(&shell, cx);
    let at = drag_onto(
        &shell,
        from,
        zone_id(DrawerView::Outline, Zone::Edge(Dir::Down)),
        cx,
    );
    release(&shell, at, cx);
    assert!(!cx.update(|cx| cx.has_active_drag()));
    cx.read(|cx| {
        let tree = layout(&shell, cx);
        assert!(!tree.slots().contains(&files), "the vacated slot closed");
        let right = slots_of(&shell, Side::Right, cx);
        assert_eq!(right.len(), 2);
        assert_eq!(right[0], outline);
        assert_eq!(tree.shown(right[1]), Some(DrawerView::Files));
        assert_eq!(tree.focused(), Focus::Leaf(Leaf::Slot(right[1])));
    });
}

/// A zone where a drop would change nothing is not drawn, so it never
/// highlights and a release there does nothing: the centre of the view's
/// own slot, and the window edge whose first slot holds it. The edges of
/// its slot, which holds another view, are drawn.
#[gpui_kit::test]
fn a_disallowed_zone_is_not_drawn(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    let before = layout_now(&shell, cx);
    let from = files_header(&shell, cx);
    press_and_move(&shell, from, 40., cx);
    assert!(!drawn(&shell, zone_id(DrawerView::Files, Zone::Centre), cx));
    assert!(!drawn(&shell, window_strip_id(Side::Left), cx));
    for dir in EDGES {
        assert!(drawn(
            &shell,
            zone_id(DrawerView::Files, Zone::Edge(dir)),
            cx
        ));
    }
    assert!(drawn(&shell, window_strip_id(Side::Right), cx));
    let centre = point_in(&shell, "slot-files", None, cx);
    move_held(&shell, centre, cx);
    release(&shell, centre, cx);
    assert_eq!(layout_now(&shell, cx), before);
}

/// Esc while dragging ends the drag: the zones go, and the release that
/// follows, over what was a zone, changes nothing.
#[gpui_kit::test]
fn escape_cancels_the_drag_and_changes_nothing(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let before = layout_now(&shell, cx);
    let zone = zone_id(DrawerView::Outline, Zone::Edge(Dir::Down));
    let from = files_header(&shell, cx);
    let at = drag_onto(&shell, from, zone.clone(), cx);
    press_key(&shell, "escape", cx);
    assert!(!cx.update(|cx| cx.has_active_drag()), "Esc ended the drag");
    assert!(!drawn(&shell, zone, cx), "the zones are gone");
    release(&shell, at, cx);
    assert_eq!(layout_now(&shell, cx), before);
}

/// The palette opening mid-drag (a modal) ends the drag: the zones go and
/// the release changes nothing.
#[gpui_kit::test]
fn opening_the_palette_mid_drag_cancels(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let before = layout_now(&shell, cx);
    let zone = zone_id(DrawerView::Outline, Zone::Edge(Dir::Down));
    let from = files_header(&shell, cx);
    let at = drag_onto(&shell, from, zone.clone(), cx);
    press_key(&shell, "ctrl-shift-p", cx);
    let workspace = shell.workspace.clone();
    let shown = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |w, cx| w.open_modal(window, cx))
        })
        .unwrap();
    assert_eq!(shown, Some(ModalKind::Palette), "the palette opened");
    assert!(!cx.update(|cx| cx.has_active_drag()), "the drag ended");
    assert!(!drawn(&shell, zone, cx), "the zones are gone");
    release(&shell, at, cx);
    assert_eq!(layout_now(&shell, cx), before);
}

/// The leader armed mid-drag (it takes the keys that follow) ends the
/// drag: the zones go and the release changes nothing.
#[gpui_kit::test]
fn arming_the_leader_mid_drag_cancels(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let before = layout_now(&shell, cx);
    let zone = zone_id(DrawerView::Outline, Zone::Edge(Dir::Down));
    let from = files_header(&shell, cx);
    let at = drag_onto(&shell, from, zone.clone(), cx);
    press_key(&shell, "ctrl-shift-b", cx);
    assert!(
        cx.read(|cx| shell.workspace.read(cx).leader.is_pending()),
        "the leader armed"
    );
    assert!(!cx.update(|cx| cx.has_active_drag()), "the drag ended");
    assert!(!drawn(&shell, zone, cx), "the zones are gone");
    release(&shell, at, cx);
    assert_eq!(layout_now(&shell, cx), before);
}

/// While zoomed only the zoomed slot is on screen: a drag shows no zone.
#[gpui_kit::test]
fn zones_do_not_show_while_zoomed(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        let files = slot_of(&shell, DrawerView::Files, cx);
        shell.workspace.update(cx, |w, cx| {
            w.focus_slot(files, window, cx);
            w.toggle_zoom(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    assert!(layout_now(&shell, cx).1.is_some(), "zoomed");
    let before = layout_now(&shell, cx);
    let from = files_header(&shell, cx);
    let at = press_and_move(&shell, from, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "a drag is live");
    for dir in EDGES {
        assert!(!drawn(
            &shell,
            zone_id(DrawerView::Files, Zone::Edge(dir)),
            cx
        ));
    }
    assert!(!drawn(&shell, window_strip_id(Side::Right), cx));
    release(&shell, at, cx);
    assert_eq!(layout_now(&shell, cx), before);
}

/// A divider's drag is not a view's: no zone, even right after a view's
/// drag ended.
#[gpui_kit::test]
fn a_divider_drag_shows_no_zones(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let from = files_header(&shell, cx);
    let at = press_and_move(&shell, from, 40., cx);
    release(&shell, at, cx);
    let divider = point_in(&shell, "divider-slot-0", None, cx);
    let at = press_and_move(&shell, divider, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "the divider drags");
    assert!(!drawn(
        &shell,
        zone_id(DrawerView::Outline, Zone::Centre),
        cx
    ));
    assert!(!drawn(&shell, window_strip_id(Side::Right), cx));
    release(&shell, at, cx);
}

/// A rail cell's drag released back on the cell is no click: the view's
/// visibility is unchanged.
#[gpui_kit::test]
fn releasing_a_rail_drag_does_not_click_the_rail_cell(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let shown = |cx: &mut TestAppContext| {
        cx.read(|cx| shell.workspace.read(cx).is_shown(DrawerView::Files))
    };
    assert!(shown(cx));
    let before = layout_now(&shell, cx);
    let from = point_in(&shell, rail_cell_id(DrawerView::Files), None, cx);
    press_and_move(&shell, from, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "a drag is live");
    move_held(&shell, from, cx);
    release(&shell, from, cx);
    assert!(shown(cx), "no click: FILES still shown");
    assert_eq!(layout_now(&shell, cx), before);
}

/// A header's drag released on that same header changes nothing: the
/// header is not a zone, even of a slot whose edges take the view.
#[gpui_kit::test]
fn a_header_drag_released_on_its_own_header_changes_nothing(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    let before = layout_now(&shell, cx);
    let from = files_header(&shell, cx);
    press_and_move(&shell, from, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "a drag is live");
    move_held(&shell, from, cx);
    release(&shell, from, cx);
    assert_eq!(layout_now(&shell, cx), before);
}

fn shell_of(started: &Started) -> Shell {
    Shell {
        handle: started.handle,
        workspace: started.workspace.clone(),
    }
}

/// A dialog cancelled through the kit (a click on the quick note's
/// overlay, which no workspace code sees) leaves no
/// trace on a later drag: the zones are drawn again. The workspace's
/// remembered modal kind can outlive its dialog; only an open dialog
/// counts.
#[gpui_kit::test]
fn zones_show_after_a_cancelled_dialog(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha")], &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| {
        started.text(cx) == "alpha" && !started.indexing(cx)
    });
    press_key(&shell_of(&started), "ctrl-shift-w", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::QuickNote));
    // A click on the overlay: the kit's own cancel, which no workspace
    // code sees.
    {
        let mut visual = VisualTestContext::from_window(started.handle, cx);
        visual.simulate_click(point(px(990.), px(400.)), Modifiers::default());
    }
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None, "the kit closed it");

    let shell = shell_of(&started);
    let header = files_header(&shell, cx);
    let at = press_and_move(&shell, header, 40., cx);
    assert!(cx.update(|cx| cx.has_active_drag()), "a drag is live");
    assert!(
        drawn(&shell, window_strip_id(Side::Right), cx),
        "the zones are drawn"
    );
    release(&shell, at, cx);
}

// ── Making room (a placement that makes a slot keeps it visible) ──────

/// The user's layout: FILES left, the editor, and one slot right holding
/// ASK, FIND and OUTLINE (OUTLINE shown), weighted `left : editor : right`.
fn files_editor_three_right(left: f64, editor: f64, right: f64) -> GuiSettings {
    gui(&format!(
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         {{ weight = {left}, views = [\"files\"], shown = \"files\" }},\n\
         {{ weight = {editor}, editor = true }},\n\
         {{ weight = {right}, views = [\"ask\", \"find\", \"outline\"], shown = \"outline\" }},\n]\n"
    ))
}

/// Drop FIND on FILES' Down edge, then ASK on FILES' Right edge (the
/// user's steps). Both drops are handled.
fn find_below_then_ask_right_of_files(
    shell: &Shell,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    window.render_frame(cx);
    let files = slot_of(shell, DrawerView::Files, cx);
    assert!(drop_view(
        shell,
        DrawerView::Find,
        DropTarget::Slot(files, Zone::Edge(Dir::Down)),
        window,
        cx
    ));
    window.render_frame(cx);
    assert!(drop_view(
        shell,
        DrawerView::Ask,
        DropTarget::Slot(files, Zone::Edge(Dir::Right)),
        window,
        cx
    ));
    window.render_frame(cx);
}

/// Every leaf of the layout is on screen: nothing collapsed, each view's
/// slot and the editor visible.
fn assert_all_visible(shell: &Shell, views: &[DrawerView], cx: &gpui_kit::App) {
    let w = shell.workspace.read(cx);
    assert!(w.collapsed.is_empty(), "collapsed: {:?}", w.collapsed);
    for view in views {
        let slot = w.layout.view_slot(view).expect("placed");
        assert!(w.leaf_visible(Leaf::Slot(slot)), "{view:?} is visible");
    }
    assert!(w.leaf_visible(Leaf::Group(w.layout.active_group())));
}

/// The user's report: FIND below FILES, then ASK right of FILES. The left
/// column's 0.2 cannot hold FILES and ASK side by side, so the placement
/// makes room: at 1000 px and at 1400 px both new slots are visible, and
/// OUTLINE and the editor still are. The keyboard is on ASK; saved.
#[gpui_kit::test]
fn ask_dropped_right_of_files_is_visible(cx: &mut TestAppContext) {
    for width in [1000., 1400.] {
        let (_dir, path) = temp_config();
        let shell = open_shell(
            cx,
            width,
            &files_editor_three_right(0.2, 0.6, 0.2),
            "",
            false,
            LayoutStore::File(path.clone()),
        );
        cx.update_window(shell.handle, |_, window, cx| {
            find_below_then_ask_right_of_files(&shell, window, cx);
            assert_all_visible(
                &shell,
                &[
                    DrawerView::Files,
                    DrawerView::Find,
                    DrawerView::Ask,
                    DrawerView::Outline,
                ],
                cx,
            );
            assert!(
                window.try_find(slot_id(DrawerView::Ask)).is_some(),
                "{width}"
            );
            let ask = slot_of(&shell, DrawerView::Ask, cx);
            assert_eq!(layout(&shell, cx).focused(), Focus::Leaf(Leaf::Slot(ask)));
            assert_eq!(
                saved_placed(&path),
                [
                    vec![DrawerView::Files],
                    vec![DrawerView::Ask],
                    vec![DrawerView::Find],
                    vec![DrawerView::Outline],
                ],
                "{width}"
            );
        })
        .unwrap();
    }
}

/// At 700 px the left column cannot sit beside the editor with FILES and
/// ASK side by side at all: the drop changes nothing (the layout, the
/// keyboard, the saved layout) and says so.
#[gpui_kit::test]
fn a_placement_that_cannot_fit_changes_nothing_and_says_so(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        700.,
        &files_editor_three_right(0.2, 0.6, 0.2),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let files = slot_of(&shell, DrawerView::Files, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Find,
            DropTarget::Slot(files, Zone::Edge(Dir::Down)),
            window,
            cx
        ));
        window.render_frame(cx);
        let before = layout(&shell, cx);
        let saved_before = saved_placed(&path);
        assert!(drop_view(
            &shell,
            DrawerView::Ask,
            DropTarget::Slot(files, Zone::Edge(Dir::Right)),
            window,
            cx
        ));
        window.render_frame(cx);
        assert_eq!(layout(&shell, cx), before, "nothing changed");
        assert_eq!(saved_placed(&path), saved_before, "nothing saved");
        assert_eq!(
            shell.workspace.read(cx).last_flash(),
            Some(("No room for Ask here", false))
        );
        assert_all_visible(
            &shell,
            &[DrawerView::Files, DrawerView::Find, DrawerView::Outline],
            cx,
        );
    })
    .unwrap();
}

/// The editor sits just above its minimum and the right slot has plenty:
/// the room comes from what each cell has above its minimum, so the
/// editor never goes under its minimum.
#[gpui_kit::test]
fn making_room_never_shrinks_the_editor_below_its_minimum(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_editor_three_right(0.2, 0.35, 0.45),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        find_below_then_ask_right_of_files(&shell, window, cx);
        assert_all_visible(
            &shell,
            &[
                DrawerView::Files,
                DrawerView::Find,
                DrawerView::Ask,
                DrawerView::Outline,
            ],
            cx,
        );
        let w = shell.workspace.read(cx);
        let group = Leaf::Group(w.layout.active_group());
        let fit = w.fit(window);
        let width = fit
            .rects
            .iter()
            .find(|(leaf, _)| *leaf == group)
            .map(|(_, rect)| rect.width())
            .expect("the editor has a rectangle");
        assert!(
            width >= f64::from(crate::shell_layout::EDITOR_MIN_REMS),
            "the editor is {width} rems wide"
        );
    })
    .unwrap();
}

/// FILES and TAGS side by side in the left cell (its own horizontal
/// split), weighted `left`, and the editor. ASK is placed nowhere.
fn files_beside_tags(left: f64) -> GuiSettings {
    gui(&format!(
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         {{ weight = {left}, split = \"horizontal\", children = [\n\
           {{ weight = 0.5, views = [\"files\"], shown = \"files\" }},\n\
           {{ weight = 0.5, views = [\"tags\"], shown = \"tags\" }},\n] }},\n\
         {{ weight = {editor}, editor = true }},\n]\n",
        editor = 1.0 - left
    ))
}

/// ASK dropped on FILES' Right edge joins the cell's split between FILES
/// and TAGS. The split then holds ASK but not TAGS, so TAGS (visible
/// before) collapses, not the new slot: room is made for TAGS too, and
/// FILES, ASK, TAGS and the editor are all visible.
#[gpui_kit::test]
fn placing_between_two_slots_of_a_column_makes_room_for_both(cx: &mut TestAppContext) {
    for (width, left) in [(1000., 0.5), (1400., 0.35)] {
        let shell = open_shell(
            cx,
            width,
            &files_beside_tags(left),
            "",
            false,
            LayoutStore::Off,
        );
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let files = slot_of(&shell, DrawerView::Files, cx);
            assert!(drop_view(
                &shell,
                DrawerView::Ask,
                DropTarget::Slot(files, Zone::Edge(Dir::Right)),
                window,
                cx
            ));
            window.render_frame(cx);
            assert_eq!(shell.workspace.read(cx).last_flash(), None, "{width}");
            assert_all_visible(
                &shell,
                &[DrawerView::Files, DrawerView::Ask, DrawerView::Tags],
                cx,
            );
        })
        .unwrap();
    }
}

/// A window-edge drop on a side with no slot makes a slot through the
/// same helper: at 600 px the new slot beside the editor would collapse
/// OUTLINE, so nothing changes and a flash says so.
#[gpui_kit::test]
fn a_window_edge_drop_that_cannot_fit_changes_nothing_and_says_so(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        600.,
        &gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
              { weight = 0.8, editor = true },\n\
              { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n"),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(slots_of(&shell, Side::Left, cx).is_empty());
        let before = layout(&shell, cx);
        assert!(drop_view(
            &shell,
            DrawerView::Find,
            DropTarget::Window(Side::Left),
            window,
            cx
        ));
        window.render_frame(cx);
        assert_eq!(layout(&shell, cx), before, "nothing changed");
        assert_eq!(
            shell.workspace.read(cx).last_flash(),
            Some(("No room for Find here", false))
        );
        assert_all_visible(&shell, &[DrawerView::Outline], cx);
    })
    .unwrap();
}

/// A view's drag released where no zone is drawn runs no drop; the next
/// frame still forgets it, so no stale drag outlives it.
#[gpui_kit::test]
fn a_drag_released_off_every_zone_is_forgotten_on_the_next_frame(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    let before = layout_now(&shell, cx);
    let from = files_header(&shell, cx);
    let at = press_and_move(&shell, from, 40., cx);
    assert!(
        shell.workspace.read_with(cx, |w, _| w.view_drag.is_some()),
        "the drag is recorded"
    );
    // Over the editor text: no zone there.
    let off = point(px(900.), px(500.));
    move_held(&shell, off, cx);
    release(&shell, off, cx);
    let _ = at;
    cx.update_window(shell.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(
        shell.workspace.read_with(cx, |w, _| w.view_drag.is_none()),
        "the dead drag is dropped"
    );
    assert_eq!(layout_now(&shell, cx), before);
}
