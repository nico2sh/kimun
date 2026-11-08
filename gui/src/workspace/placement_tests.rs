//! UI integration tests for placing a view by a move (ADR-0054 §1, §2):
//! a move toward nothing makes a slot on that side when the view's slot
//! holds other views, and a move that takes the last view out of a slot
//! closes that slot. A real `Workspace` in a headless window, driven by
//! GPUI keyboard and pointer events.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use kimun_notes::logic::layout::Side;
use kimun_notes::logic::layout_tree::{Focus, LayoutTree, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::GuiSettings;

use super::tests::{Shell, gui, open_shell, saved, temp_config};
use super::{LayoutStore, slot_id};
use crate::shell_tree;

/// The zoom chord with the default Linux/Windows keymap.
const ZOOM: &str = "ctrl-shift-m";

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

/// FILES over FIND on the left (one view each), the editor, OUTLINE right.
fn stacked_left_and_outline_right() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

/// FILES over FIND on the left (one view each), and the editor.
fn stacked_left() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.8, editor = true },\n]\n")
}

fn layout(shell: &Shell, cx: &gpui_kit::App) -> LayoutTree<DrawerView> {
    shell.workspace.read(cx).layout.clone()
}

fn left_slots(shell: &Shell, cx: &gpui_kit::App) -> Vec<SlotId> {
    shell.workspace.read(cx).side_slots(Side::Left)
}

fn views_of(shell: &Shell, slot: SlotId, cx: &gpui_kit::App) -> Vec<DrawerView> {
    shell.workspace.read(cx).layout.slot_views(slot)
}

fn shown_in(shell: &Shell, slot: SlotId, cx: &gpui_kit::App) -> Option<DrawerView> {
    shell.workspace.read(cx).layout.shown(slot)
}

fn focus(shell: &Shell, cx: &gpui_kit::App) -> Focus {
    shell.workspace.read(cx).layout.focused()
}

fn flash(shell: &Shell, cx: &gpui_kit::App) -> Option<String> {
    shell
        .workspace
        .read(cx)
        .last_flash()
        .map(|(text, _)| text.to_string())
}

/// The views placed in the saved layout's slots, in reading order.
fn saved_placed(path: &std::path::Path) -> Vec<Vec<DrawerView>> {
    let tree = shell_tree::restore(&saved(path), shell_tree::RESTORE_CAPS);
    tree.slots()
        .into_iter()
        .map(|s| tree.slot_views(s))
        .collect()
}

/// FILES shares its slot with FIND (hidden) and nothing is below: Move
/// Down puts FILES in a new slot under it, FIND showing in the slot above,
/// and the keyboard follows FILES. Saved.
#[gpui_kit::test]
fn moving_down_with_no_slot_below_spawns_one(cx: &mut TestAppContext) {
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
        let old = left_slots(&shell, cx)[0];
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);

        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2, "two stacked slots on the left");
        assert_eq!(left[0], old, "the slot above is the one FILES left");
        assert_eq!(shown_in(&shell, left[0], cx), Some(DrawerView::Find));
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Files));
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(left[1])));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        let find = window.find(slot_id(DrawerView::Find)).bounds();
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        assert!(files.top() > find.top(), "FILES drawn under FIND");
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Find], vec![DrawerView::Files]]
        );
        assert_eq!(flash(&shell, cx), None);
    })
    .unwrap();
}

/// Left and Right toward nothing make a slot on that side as well.
#[gpui_kit::test]
fn moving_sideways_toward_nothing_spawns_a_slot_on_that_side(cx: &mut TestAppContext) {
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
        let old = left_slots(&shell, cx)[0];
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-left", cx);
        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2);
        assert_eq!(left[1], old, "the new slot comes before the old one");
        assert_eq!(views_of(&shell, left[0], cx), [DrawerView::Files]);
        assert_eq!(views_of(&shell, left[1], cx), [DrawerView::Find]);
        assert_eq!(shown_in(&shell, old, cx), Some(DrawerView::Find));
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(left[0])));
    })
    .unwrap();

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
        let old = left_slots(&shell, cx)[0];
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-right", cx);
        let slots = layout(&shell, cx).slots();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0], old);
        assert_eq!(views_of(&shell, slots[1], cx), [DrawerView::Files]);
        assert_eq!(shown_in(&shell, old, cx), Some(DrawerView::Find));
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(slots[1])));
    })
    .unwrap();
}

/// A view alone in its slot has nothing to leave behind: a move toward
/// nothing changes nothing and says nothing.
#[gpui_kit::test]
fn moving_the_only_view_toward_nothing_changes_nothing(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &files_alone(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        let before = layout(&shell, cx);
        for key in ["alt-shift-down", "alt-shift-up", "alt-shift-left"] {
            window.press(key, cx);
            assert_eq!(layout(&shell, cx), before, "{key}");
            assert_eq!(flash(&shell, cx), None, "{key}");
        }
    })
    .unwrap();
}

/// FIND alone in the lower left slot moves right (to OUTLINE's slot): the
/// lower slot holds nothing any more and closes; FILES' slot fills the
/// column. The keyboard follows FIND. Saved.
#[gpui_kit::test]
fn a_move_that_vacates_a_slot_closes_it(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let slots = layout(&shell, cx).slots();
        assert_eq!(slots.len(), 3);
        let (top, right) = (slots[0], slots[2]);
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-right", cx);
        window.render_frame(cx);

        assert_eq!(layout(&shell, cx).slots(), [top, right]);
        assert_eq!(left_slots(&shell, cx), [top]);
        assert_eq!(shown_in(&shell, right, cx), Some(DrawerView::Find));
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(right)));
        assert_eq!(
            saved_placed(&path),
            [
                vec![DrawerView::Files],
                vec![DrawerView::Outline, DrawerView::Find]
            ]
        );
    })
    .unwrap();
}

/// The default layout's empty right slot is not vacated by a move that
/// does not take a view out of it: it stays in the tree.
#[gpui_kit::test]
fn the_default_empty_right_slot_is_not_closed(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let slots = layout(&shell, cx).slots();
        assert_eq!(slots.len(), 2);
        let right = slots[1];
        assert!(views_of(&shell, right, cx).is_empty());
        // FIND shows in FILES' slot, then moves down into a new slot.
        window.click(super::rail_cell_id(DrawerView::Find), cx);
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-down", cx);
        let slots = layout(&shell, cx).slots();
        assert_eq!(slots.len(), 3, "{slots:?}");
        assert!(slots.contains(&right));
    })
    .unwrap();
}

/// Zoomed on the lower left slot, the rail moves its only view (FIND) into
/// the slot beside the rail: the lower slot closes, and the zoom moves to
/// FIND's new slot rather than ending.
#[gpui_kit::test]
fn a_vacating_move_while_zoomed_keeps_zoom_on_the_new_slot(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &stacked_left(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let (top, bottom) = (left_slots(&shell, cx)[0], left_slots(&shell, cx)[1]);
        window.click(slot_id(DrawerView::Find), cx);
        window.press(ZOOM, cx);
        assert_eq!(layout(&shell, cx).zoomed(), Some(Leaf::Slot(bottom)));
        window.press("alt-left", cx);
        assert_eq!(focus(&shell, cx), Focus::Rail);
        let ws = shell.workspace.clone();
        ws.update(cx, |workspace, _| {
            workspace.place_rail_cursor(Some(DrawerView::Find))
        });
        window.press("alt-shift-right", cx);
        window.render_frame(cx);

        assert_eq!(layout(&shell, cx).slots(), [top]);
        assert_eq!(shown_in(&shell, top, cx), Some(DrawerView::Find));
        assert_eq!(layout(&shell, cx).zoomed(), Some(Leaf::Slot(top)));
        assert_eq!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}

/// The flow ADR-0054 starts from, in two steps: FILES and FIND share a
/// slot; Move Down stacks them (FIND above, FILES below), and Move Up then
/// trades their places as before.
#[gpui_kit::test]
fn stacking_two_views_from_one_slot_takes_one_move(cx: &mut TestAppContext) {
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
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-down", cx);
        let left = left_slots(&shell, cx);
        assert_eq!(views_of(&shell, left[0], cx), [DrawerView::Find]);
        assert_eq!(views_of(&shell, left[1], cx), [DrawerView::Files]);

        window.press("alt-shift-up", cx);
        assert_eq!(left_slots(&shell, cx), left, "a swap keeps both slots");
        assert_eq!(views_of(&shell, left[0], cx), [DrawerView::Files]);
        assert_eq!(views_of(&shell, left[1], cx), [DrawerView::Find]);
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(left[0])));
    })
    .unwrap();
}
