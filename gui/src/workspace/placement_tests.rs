//! UI integration tests for placing a view in one step (ADR-0054 §1-§3):
//! a move toward nothing makes a slot on that side when the view's slot
//! holds other views, a move that takes the last view out of a slot
//! closes that slot, a split asks which view the new slot holds, and the
//! rail places its cursor's view above or below the last focused slot. A
//! real `Workspace` in a headless window, driven by GPUI keyboard and
//! pointer events.

use gpui_kit::component::IndexPath;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::KeyInput;
use kimun_notes::logic::layout::{Dir, Side};
use kimun_notes::logic::layout_tree::{Focus, LayoutTree, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;
use kimun_notes::settings::gui::GuiSettings;

use super::modal::ModalKind;
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

// --- Split asks which view the new slot holds (ADR-0054 §1) ---

/// FILES left (shown, FIND hidden with it), the editor, OUTLINE right.
fn files_find_hidden_and_outline_right() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\", \"find\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

/// Every view the rail offers (no server) shown in a slot of its own.
fn every_view_shown() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.3, split = \"vertical\", children = [\n\
           { weight = 0.34, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.33, views = [\"find\"], shown = \"find\" },\n\
           { weight = 0.33, views = [\"tags\"], shown = \"tags\" },\n] },\n\
         { weight = 0.4, editor = true },\n\
         { weight = 0.3, split = \"vertical\", children = [\n\
           { weight = 0.34, views = [\"links\"], shown = \"links\" },\n\
           { weight = 0.33, views = [\"outline\"], shown = \"outline\" },\n\
           { weight = 0.33, views = [\"config\"], shown = \"config\" },\n] },\n]\n")
}

fn chooser_open(shell: &Shell, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> bool {
    window.render_frame(cx);
    shell.workspace.update(cx, |w, cx| w.open_modal(window, cx)) == Some(ModalKind::ViewChooser)
}

/// The views the split chooser offers, in its order.
fn chooser_views(shell: &Shell, cx: &gpui_kit::App) -> Vec<DrawerView> {
    shell
        .workspace
        .read(cx)
        .split_chooser_views(cx)
        .unwrap_or_default()
}

/// Let a pick land: the kit runs its confirm callback deferred, after the
/// key's own update.
fn settle(shell: &Shell, cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.update_window(shell.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Split Down from FILES opens the chooser over the views not shown now,
/// in registry order, each row named after its view.
#[gpui_kit::test]
fn split_opens_a_chooser_of_views_not_shown(cx: &mut TestAppContext) {
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
        window.click(slot_id(DrawerView::Files), cx);
        let before = layout(&shell, cx);
        window.press("alt-shift-d", cx);
        window.render_frame(cx);
        assert!(chooser_open(&shell, window, cx), "the chooser is open");
        let want = [
            DrawerView::Find,
            DrawerView::Tags,
            DrawerView::Links,
            DrawerView::Outline,
            DrawerView::Config,
        ];
        assert_eq!(chooser_views(&shell, cx), want);
        for (i, view) in want.iter().enumerate() {
            assert_eq!(
                window.find(IndexPath::new(i)).label(),
                Some(view_registry::spec(*view).name),
                "row {i} is drawn"
            );
        }
        assert_eq!(layout(&shell, cx), before, "nothing splits yet");
    })
    .unwrap();
}

/// Picking OUTLINE fills a new slot under FILES with it; the keyboard goes
/// there, and the layout is saved with it.
#[gpui_kit::test]
fn picking_a_view_fills_the_new_slot(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    let old = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(slot_id(DrawerView::Files), cx);
            window.press("alt-shift-d", cx);
            window.render_frame(cx);
            // FIND, TAGS, LINKS, OUTLINE: the fourth row.
            for _ in 0..3 {
                window.press("down", cx);
            }
            window.press("enter", cx);
            left_slots(&shell, cx)[0]
        })
        .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        assert!(!chooser_open(&shell, window, cx), "the chooser closed");
        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2, "two stacked slots on the left");
        assert_eq!(left[0], old);
        assert_eq!(shown_in(&shell, left[0], cx), Some(DrawerView::Files));
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Outline));
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(left[1])));
        assert_eq!(
            window.find(slot_id(DrawerView::Outline)).focused(),
            Some(true)
        );
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let outline = window.find(slot_id(DrawerView::Outline)).bounds();
        assert!(outline.top() > files.top(), "OUTLINE drawn under FILES");
        assert!(
            saved_placed(&path).contains(&vec![DrawerView::Outline]),
            "{:?}",
            saved_placed(&path)
        );
    })
    .unwrap();
}

/// Split Right puts the picked view in a new slot right of the focused one.
#[gpui_kit::test]
fn split_right_places_the_pick_to_the_right(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &files_alone(), "", false, LayoutStore::Off);
    let old = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(slot_id(DrawerView::Files), cx);
            window.press("alt-shift-r", cx);
            window.render_frame(cx);
            window.press("enter", cx);
            left_slots(&shell, cx)[0]
        })
        .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        let slots = layout(&shell, cx).slots();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0], old);
        assert_eq!(views_of(&shell, slots[1], cx), [DrawerView::Find]);
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(slots[1])));
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let find = window.find(slot_id(DrawerView::Find)).bounds();
        assert!(find.left() > files.left(), "FIND drawn right of FILES");
    })
    .unwrap();
}

/// Escape in the chooser splits nothing; the keyboard goes back to FILES.
#[gpui_kit::test]
fn escape_in_the_chooser_splits_nothing(cx: &mut TestAppContext) {
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
        window.click(slot_id(DrawerView::Files), cx);
        let before = layout(&shell, cx);
        window.press("alt-shift-d", cx);
        window.render_frame(cx);
        assert!(chooser_open(&shell, window, cx));
        window.press("escape", cx);
        assert!(!chooser_open(&shell, window, cx), "Escape closes it");
        assert_eq!(layout(&shell, cx), before);
        assert_eq!(flash(&shell, cx), None);
    })
    .unwrap();
}

/// With every view shown there is nothing to pick: the split says so and
/// does nothing.
#[gpui_kit::test]
fn split_with_every_view_shown_flashes(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1400., &every_view_shown(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        let before = layout(&shell, cx);
        window.press("alt-shift-d", cx);
        window.render_frame(cx);
        assert!(!chooser_open(&shell, window, cx), "no chooser");
        assert_eq!(
            flash(&shell, cx).as_deref(),
            Some("Every view is already shown")
        );
        assert_eq!(layout(&shell, cx), before);
    })
    .unwrap();
}

/// FIND is hidden in the left slot; split OUTLINE's slot and pick FIND: it
/// leaves the left slot (which keeps FILES) and shows in the new slot.
#[gpui_kit::test]
fn a_picked_view_that_was_hidden_elsewhere_moves_there(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_find_hidden_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    let (left, right) = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let slots = layout(&shell, cx).slots();
            window.click(slot_id(DrawerView::Outline), cx);
            window.press("alt-shift-d", cx);
            window.render_frame(cx);
            assert_eq!(
                chooser_views(&shell, cx),
                [
                    DrawerView::Find,
                    DrawerView::Tags,
                    DrawerView::Links,
                    DrawerView::Config
                ]
            );
            window.press("enter", cx);
            (slots[0], slots[1])
        })
        .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, _, cx| {
        let slots = layout(&shell, cx).slots();
        assert_eq!(slots.len(), 3);
        assert_eq!(views_of(&shell, left, cx), [DrawerView::Files]);
        assert_eq!(views_of(&shell, right, cx), [DrawerView::Outline]);
        let new = slots[2];
        assert_eq!(views_of(&shell, new, cx), [DrawerView::Find]);
        assert_eq!(shown_in(&shell, new, cx), Some(DrawerView::Find));
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(new)));
    })
    .unwrap();
}

/// What is typed in the chooser stays there: the query takes the text,
/// and the window's chords (a move, another split) do not run under it.
#[gpui_kit::test]
fn keys_in_the_chooser_do_not_reach_the_view_below(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "hello",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        let before = layout(&shell, cx);
        window.press("alt-shift-d", cx);
        window.render_frame(cx);
        assert!(chooser_open(&shell, window, cx));
        window.input("zz", cx);
        for key in [
            "alt-shift-down",
            "alt-shift-r",
            "alt-shift-d",
            "ctrl-shift-t",
        ] {
            window.press(key, cx);
            assert!(chooser_open(&shell, window, cx), "{key}: still open");
            assert_eq!(layout(&shell, cx), before, "{key}");
        }
        assert_eq!(
            shell.workspace.read(cx).split_chooser_query(cx).as_deref(),
            Some("zz")
        );
        assert_eq!(shell.text(cx), "hello", "the editor got nothing");
    })
    .unwrap();
}

// --- The rail places (ADR-0054 §3) ---

/// Put the rail's cursor on `view` (the rail has the keyboard).
fn rail_cursor_on(shell: &Shell, view: DrawerView, cx: &mut gpui_kit::App) {
    shell
        .workspace
        .update(cx, |workspace, _| workspace.place_rail_cursor(Some(view)));
}

/// FILES focused in the left slot, then the rail (`alt-left`) with its
/// cursor on FIND (placed nowhere): Move Down puts FIND in a new slot
/// under FILES' slot. The rail keeps the keyboard.
#[gpui_kit::test]
fn rail_down_places_the_cursor_view_below_the_last_focused_slot(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &files_alone(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let old = left_slots(&shell, cx)[0];
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-left", cx);
        assert_eq!(focus(&shell, cx), Focus::Rail);
        rail_cursor_on(&shell, DrawerView::Find, cx);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);

        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2, "{left:?}");
        assert_eq!(left[0], old);
        assert_eq!(shown_in(&shell, left[0], cx), Some(DrawerView::Files));
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Find));
        assert_eq!(focus(&shell, cx), Focus::Rail);
        assert_eq!(window.find("rail").focused(), Some(true));
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Files], vec![DrawerView::Find]]
        );
    })
    .unwrap();
}

/// Move Up from the rail puts the cursor's view above the last focused
/// slot.
#[gpui_kit::test]
fn rail_up_places_above(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &files_alone(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let old = left_slots(&shell, cx)[0];
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-left", cx);
        rail_cursor_on(&shell, DrawerView::Find, cx);
        window.press("alt-shift-up", cx);
        window.render_frame(cx);

        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2, "{left:?}");
        assert_eq!(left[1], old);
        assert_eq!(shown_in(&shell, left[0], cx), Some(DrawerView::Find));
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Files));
        assert_eq!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}

/// No slot has had the keyboard yet: the rail places beside the slot
/// beside the rail. Here the last focused slot would be OUTLINE's, on the
/// right, had any slot been focused.
#[gpui_kit::test]
fn rail_down_with_no_focused_slot_yet_uses_the_slot_beside_the_rail(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_find_hidden_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let old = left_slots(&shell, cx)[0];
        window.click("rail", cx);
        assert_eq!(focus(&shell, cx), Focus::Rail);
        rail_cursor_on(&shell, DrawerView::Tags, cx);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);

        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2, "{left:?}");
        assert_eq!(left[0], old);
        assert_eq!(shown_in(&shell, left[0], cx), Some(DrawerView::Files));
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Tags));
        assert_eq!(shell.workspace.read(cx).side_slots(Side::Right).len(), 1);
        assert_eq!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}

/// The last focused slot (FIND's, lower left) closes: the rail moves FIND
/// right into the slot beside the rail, vacating it. The next placement
/// from the rail goes beside the slot beside the rail.
#[gpui_kit::test]
fn last_slot_follows_a_closed_slot(cx: &mut TestAppContext) {
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
        let top = left_slots(&shell, cx)[0];
        window.click(slot_id(DrawerView::Find), cx);
        window.click("rail", cx);
        rail_cursor_on(&shell, DrawerView::Find, cx);
        window.press("alt-shift-right", cx);
        window.render_frame(cx);
        assert_eq!(left_slots(&shell, cx), [top], "FIND's slot closed");

        rail_cursor_on(&shell, DrawerView::Tags, cx);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);

        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2, "{left:?}");
        assert_eq!(left[0], top);
        assert_eq!(shown_in(&shell, left[0], cx), Some(DrawerView::Find));
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Tags));
        assert_eq!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}

/// With a slot on the right focused last, Right from the rail still shows
/// the cursor's view in the slot beside the rail (ADR-0052), not a new
/// slot.
#[gpui_kit::test]
fn rail_right_still_shows_in_the_slot_beside_the_rail(cx: &mut TestAppContext) {
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
        let slots = layout(&shell, cx).slots();
        let top = left_slots(&shell, cx)[0];
        window.click(slot_id(DrawerView::Outline), cx);
        window.click("rail", cx);
        rail_cursor_on(&shell, DrawerView::Tags, cx);
        window.press("alt-shift-right", cx);
        window.render_frame(cx);

        assert_eq!(layout(&shell, cx).slots(), slots, "no new slot");
        assert_eq!(shown_in(&shell, top, cx), Some(DrawerView::Tags));
        assert_eq!(
            views_of(&shell, top, cx),
            [DrawerView::Files, DrawerView::Tags]
        );
        assert_eq!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}

/// The cursor's view is the one the last focused slot shows, with FIND
/// hidden beside it: Move Down puts it in a new slot below, and FIND shows
/// in the slot it left, as a move toward nothing does.
#[gpui_kit::test]
fn rail_down_on_the_last_slots_view_shows_the_next_one_there(cx: &mut TestAppContext) {
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
        window.press("alt-left", cx);
        rail_cursor_on(&shell, DrawerView::Files, cx);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);

        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2, "{left:?}");
        assert_eq!(left[0], old);
        assert_eq!(shown_in(&shell, left[0], cx), Some(DrawerView::Find));
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Files));
        assert_eq!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}

// --- Move view to… (ADR-0054 §4) ---

/// The `⋯` button in the header of the slot showing `view`.
fn move_button(view: DrawerView) -> gpui_kit::SharedString {
    super::tree_view::move_button_id(view)
}

/// The labels the open move chooser offers, in its order.
fn move_rows(shell: &Shell, cx: &gpui_kit::App) -> Vec<String> {
    shell
        .workspace
        .read(cx)
        .move_chooser_rows(cx)
        .unwrap_or_default()
}

fn into(view: DrawerView) -> String {
    format!("Into {} slot", view_registry::spec(view).title)
}

const NEW_SIDES: [&str; 4] = [
    "New slot above",
    "New slot below",
    "New slot to the left",
    "New slot to the right",
];

/// The move chord toward `dir` in the active keymap.
fn move_chord(dir: Dir, cx: &gpui_kit::App) -> Option<KeyInput> {
    crate::keymap::active(cx).chord_for(&ActionShortcuts::MoveViewToNeighbour(dir))
}

/// FILES (FIND hidden with it) left, OUTLINE right: FILES' chooser offers
/// OUTLINE's slot and a new slot on each side, every row drawn, and each
/// row names the move chord that does the same: Right reaches OUTLINE's
/// slot, the other three sides have nothing there, so their chords make
/// the new slot.
#[gpui_kit::test]
fn the_move_chooser_lists_other_slots_and_four_new_sides(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_find_hidden_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        let before = layout(&shell, cx);
        window.click(move_button(DrawerView::Files), cx);
        window.render_frame(cx);
        assert!(chooser_open(&shell, window, cx), "the chooser is open");
        let mut want = vec![into(DrawerView::Outline)];
        want.extend(NEW_SIDES.map(String::from));
        assert_eq!(move_rows(&shell, cx), want);
        for (i, label) in want.iter().enumerate() {
            assert_eq!(
                window.find(IndexPath::new(i)).label(),
                Some(label.as_str()),
                "row {i} is drawn"
            );
        }
        assert_eq!(
            shell.workspace.read(cx).move_chooser_hints(cx),
            Some(vec![
                move_chord(Dir::Right, cx),
                move_chord(Dir::Up, cx),
                move_chord(Dir::Down, cx),
                move_chord(Dir::Left, cx),
                None,
            ])
        );
        assert!(move_chord(Dir::Up, cx).is_some(), "the chords are bound");
        assert_eq!(layout(&shell, cx), before, "nothing moves yet");
    })
    .unwrap();
}

/// FILES over FIND, one view each: picking FIND's slot moves FILES there
/// (shown) and FIND takes the slot FILES left, so no slot closes; the
/// keyboard follows FILES. Saved.
#[gpui_kit::test]
fn picking_a_slot_moves_the_view_there(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &stacked_left(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    let (top, bottom) = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let left = left_slots(&shell, cx);
            window.click(move_button(DrawerView::Files), cx);
            window.render_frame(cx);
            assert_eq!(move_rows(&shell, cx), [into(DrawerView::Find)]);
            window.press("enter", cx);
            (left[0], left[1])
        })
        .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        assert!(!chooser_open(&shell, window, cx), "the chooser closed");
        let slots = layout(&shell, cx).slots();
        assert!(slots.contains(&top), "a swap closes no slot");
        assert_eq!(views_of(&shell, top, cx), [DrawerView::Find]);
        assert_eq!(shown_in(&shell, top, cx), Some(DrawerView::Find));
        assert_eq!(views_of(&shell, bottom, cx), [DrawerView::Files]);
        assert_eq!(shown_in(&shell, bottom, cx), Some(DrawerView::Files));
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(bottom)));
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Find], vec![DrawerView::Files]]
        );
    })
    .unwrap();
}

/// FILES shares the only slot with FIND (hidden): picking "New slot to the
/// right" puts FILES in a new slot right of it, FIND showing in its
/// place, and the keyboard follows FILES.
#[gpui_kit::test]
fn picking_a_new_side_places_the_view_in_a_new_slot(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_over_find_hidden(),
        "",
        false,
        LayoutStore::Off,
    );
    let old = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(move_button(DrawerView::Files), cx);
            window.render_frame(cx);
            assert_eq!(move_rows(&shell, cx), NEW_SIDES);
            for _ in 0..3 {
                window.press("down", cx);
            }
            window.press("enter", cx);
            layout(&shell, cx).slots()[0]
        })
        .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        let slots = layout(&shell, cx).slots();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0], old);
        assert_eq!(views_of(&shell, old, cx), [DrawerView::Find]);
        assert_eq!(shown_in(&shell, old, cx), Some(DrawerView::Find));
        assert_eq!(views_of(&shell, slots[1], cx), [DrawerView::Files]);
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(slots[1])));
        let find = window.find(slot_id(DrawerView::Find)).bounds();
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        assert!(files.left() > find.left(), "FILES drawn right of FIND");
    })
    .unwrap();
}

/// FILES has the keyboard; a click on OUTLINE's `⋯` focuses OUTLINE's slot
/// and opens the chooser for OUTLINE: the two slots on the left.
#[gpui_kit::test]
fn the_header_button_opens_the_chooser_for_its_slot(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1200.,
        &stacked_left_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let outline = layout(&shell, cx).slots()[2];
        window.click(slot_id(DrawerView::Files), cx);
        assert_ne!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(outline)));
        assert!(window.try_find(move_button(DrawerView::Outline)).is_some());
        window.click(move_button(DrawerView::Outline), cx);
        window.render_frame(cx);
        assert_eq!(focus(&shell, cx), Focus::Leaf(Leaf::Slot(outline)));
        assert!(chooser_open(&shell, window, cx), "the chooser is open");
        assert_eq!(
            move_rows(&shell, cx),
            [into(DrawerView::Files), into(DrawerView::Find)]
        );
    })
    .unwrap();
}

/// A view alone in its slot leaves nothing behind: only the other slots
/// are offered. Alone in the only slot, there is nowhere to go: the
/// button says so and opens nothing.
#[gpui_kit::test]
fn a_slot_with_a_lone_view_offers_no_new_slot_rows(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &stacked_left(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(move_button(DrawerView::Find), cx);
        window.render_frame(cx);
        assert!(chooser_open(&shell, window, cx));
        assert_eq!(move_rows(&shell, cx), [into(DrawerView::Files)]);
    })
    .unwrap();

    let shell = open_shell(cx, 1000., &files_alone(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        let before = layout(&shell, cx);
        window.click(move_button(DrawerView::Files), cx);
        window.render_frame(cx);
        assert!(!chooser_open(&shell, window, cx), "no chooser");
        assert_eq!(
            flash(&shell, cx).as_deref(),
            Some("Nowhere else to move this view")
        );
        assert_eq!(layout(&shell, cx), before);
    })
    .unwrap();
}

/// While zoomed the other slots are not on screen: the button does
/// nothing.
#[gpui_kit::test]
fn the_move_chooser_does_not_open_while_zoomed(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &stacked_left(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        window.press(ZOOM, cx);
        window.render_frame(cx);
        window.click(move_button(DrawerView::Files), cx);
        window.render_frame(cx);
        assert!(!chooser_open(&shell, window, cx));
    })
    .unwrap();
}

/// A query narrows the chooser; Enter picks the row the query kept, not
/// the row at that place in the full list: "outl" keeps OUTLINE alone,
/// and the new slot shows OUTLINE.
#[gpui_kit::test]
fn a_filtered_pick_runs_the_row_the_query_kept(cx: &mut TestAppContext) {
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
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-d", cx);
        window.render_frame(cx);
        assert_eq!(chooser_views(&shell, cx)[0], DrawerView::Find);
        window.input("outl", cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        assert!(!chooser_open(&shell, window, cx), "the chooser closed");
        let left = left_slots(&shell, cx);
        assert_eq!(left.len(), 2);
        assert_eq!(shown_in(&shell, left[1], cx), Some(DrawerView::Outline));
    })
    .unwrap();
}

/// FILES over FIND, one view each: the Up chord reaches FILES' slot from
/// FIND, but it trades places there, which picking "Into Files slot" does
/// not: that row names no chord.
#[gpui_kit::test]
fn an_into_row_above_names_no_chord(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &stacked_left(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(move_button(DrawerView::Find), cx);
        window.render_frame(cx);
        assert_eq!(move_rows(&shell, cx), [into(DrawerView::Files)]);
        assert_eq!(
            shell.workspace.read(cx).move_chooser_hints(cx),
            Some(vec![None])
        );
    })
    .unwrap();
}

/// The rows' chords are the keymap's: with the move chords unbound, no
/// row names one.
#[gpui_kit::test]
fn the_move_chooser_hints_follow_a_replaced_keymap(cx: &mut TestAppContext) {
    use crate::keymap::{self, Keymap, Platform};
    let shell = open_shell(
        cx,
        1000.,
        &files_find_hidden_and_outline_right(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update(|cx| {
        let overrides = [
            "MoveViewToNeighbourRight",
            "MoveViewToNeighbourLeft",
            "MoveViewToNeighbourUp",
            "MoveViewToNeighbourDown",
        ]
        .into_iter()
        .map(|name| (name.to_string(), vec![]))
        .collect();
        let (unbound, errors) = Keymap::load(Platform::LinuxWindows, &overrides);
        assert!(errors.is_empty(), "{errors:?}");
        keymap::init(cx, unbound);
    });
    cx.run_until_parked();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(move_button(DrawerView::Files), cx);
        window.render_frame(cx);
        assert_eq!(move_rows(&shell, cx).len(), 5);
        assert_eq!(
            shell.workspace.read(cx).move_chooser_hints(cx),
            Some(vec![None; 5])
        );
    })
    .unwrap();
}

/// The last slot on a side is not closed when a move empties it: FILES
/// moved out of the default left slot leaves that slot in the tree, empty
/// and undrawn. A left-home view (LINKS) shown from the rail opens there,
/// left of the editor, and the left side toggle works again. The empty
/// slot is saved and restored with the layout.
#[gpui_kit::test]
fn the_last_slot_on_a_side_stays_when_vacated(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let slots = layout(&shell, cx).slots();
        let (left, right) = (slots[0], slots[1]);
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-right", cx);
        window.render_frame(cx);
        assert_eq!(layout(&shell, cx).slots(), [left, right]);
        assert!(views_of(&shell, left, cx).is_empty());
        assert!(!shell.workspace.read(cx).slot_drawn(left), "undrawn");
        assert_eq!(saved_placed(&path), [vec![], vec![DrawerView::Files]]);
        let restored = shell_tree::restore(&saved(&path), shell_tree::RESTORE_CAPS);
        assert_eq!(restored.slots().len(), 2, "the empty left slot restores");

        window.click(super::rail_cell_id(DrawerView::Links), cx);
        window.render_frame(cx);
        assert_eq!(shown_in(&shell, left, cx), Some(DrawerView::Links));
        let links = window.find(slot_id(DrawerView::Links)).bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(links.right() <= editor.left(), "left of the editor");

        // The left side toggle hides LINKS and shows it again.
        window.press("ctrl-t", cx);
        assert_eq!(shown_in(&shell, left, cx), None);
        window.press("ctrl-t", cx);
        assert_eq!(shown_in(&shell, left, cx), Some(DrawerView::Links));
        assert_ne!(flash(&shell, cx).as_deref(), Some("No slot on that side"));
    })
    .unwrap();
}

/// FILES (FIND hidden with it) over TAGS in a 0.2 column, the editor,
/// OUTLINE right.
fn files_find_over_tags() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\", \"find\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"tags\"], shown = \"tags\" },\n] },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

/// The move chooser's "New slot to the right" makes room as a drop does:
/// the column cannot hold FIND and FILES side by side at its 0.2, so it
/// grows, and the new slot is visible with every other leaf. The keyboard
/// follows FILES.
#[gpui_kit::test]
fn the_chooser_new_slot_rows_make_room_too(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_find_over_tags(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(move_button(DrawerView::Files), cx);
        window.render_frame(cx);
        let mut want = vec![into(DrawerView::Tags), into(DrawerView::Outline)];
        want.extend(NEW_SIDES.map(String::from));
        assert_eq!(move_rows(&shell, cx), want);
        for _ in 0..5 {
            window.press("down", cx);
        }
        window.press("enter", cx);
    })
    .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let w = shell.workspace.read(cx);
        assert!(w.collapsed.is_empty(), "collapsed: {:?}", w.collapsed);
        let files = w.layout.view_slot(&DrawerView::Files).expect("placed");
        let find = w.layout.view_slot(&DrawerView::Find).expect("placed");
        assert_ne!(files, find, "FILES is in a new slot");
        for view in [
            DrawerView::Files,
            DrawerView::Find,
            DrawerView::Tags,
            DrawerView::Outline,
        ] {
            let slot = w.layout.view_slot(&view).expect("placed");
            assert!(w.leaf_visible(Leaf::Slot(slot)), "{view:?} is visible");
        }
        assert_eq!(w.layout.focused(), Focus::Leaf(Leaf::Slot(files)));
        assert!(window.try_find(slot_id(DrawerView::Files)).is_some());
    })
    .unwrap();
}

/// FILES, FIND, TAGS and LINKS stacked left of the editor, a slot each.
fn four_stacked_left() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.25, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.25, views = [\"find\"], shown = \"find\" },\n\
           { weight = 0.25, views = [\"tags\"], shown = \"tags\" },\n\
           { weight = 0.25, views = [\"links\"], shown = \"links\" },\n] },\n\
         { weight = 0.8, editor = true },\n]\n")
}

/// The rail's Move Down makes a slot too: a fifth slot in the column
/// would collapse LINKS (the window is not tall enough, and nothing above
/// the column can grow it), so nothing changes and a flash says so.
#[gpui_kit::test]
fn rail_placement_with_no_room_changes_nothing_and_says_so(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &four_stacked_left(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-left", cx);
        rail_cursor_on(&shell, DrawerView::Outline, cx);
        window.render_frame(cx);
        let before = layout(&shell, cx);
        assert!(shell.workspace.read(cx).collapsed.is_empty());
        window.press("alt-shift-down", cx);
        window.render_frame(cx);
        assert_eq!(layout(&shell, cx), before, "nothing changed");
        assert_eq!(
            flash(&shell, cx).as_deref(),
            Some("No room for Outline here")
        );
        assert!(shell.workspace.read(cx).collapsed.is_empty());
    })
    .unwrap();
}

/// FILES over TAGS in a 0.2 column, the editor, OUTLINE right.
fn files_over_tags() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"tags\"], shown = \"tags\" },\n] },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

/// Split Right from FILES and pick FIND: the column cannot hold FILES and
/// FIND side by side at its 0.2, so the split chooser makes room through
/// the same helper, and every leaf is visible with FIND focused.
#[gpui_kit::test]
fn the_split_chooser_makes_room_too(cx: &mut TestAppContext) {
    let shell = open_shell(cx, 1000., &files_over_tags(), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-r", cx);
        window.render_frame(cx);
        assert_eq!(chooser_views(&shell, cx).first(), Some(&DrawerView::Find));
        window.press("enter", cx);
    })
    .unwrap();
    settle(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let w = shell.workspace.read(cx);
        assert!(w.collapsed.is_empty(), "collapsed: {:?}", w.collapsed);
        for view in [
            DrawerView::Files,
            DrawerView::Find,
            DrawerView::Tags,
            DrawerView::Outline,
        ] {
            let slot = w.layout.view_slot(&view).expect("placed");
            assert!(w.leaf_visible(Leaf::Slot(slot)), "{view:?} is visible");
        }
        let find = w.layout.view_slot(&DrawerView::Find).expect("placed");
        assert_eq!(w.layout.focused(), Focus::Leaf(Leaf::Slot(find)));
    })
    .unwrap();
}
