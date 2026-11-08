//! UI integration tests for the layout tree in the window (ADR-0052):
//! slots beside the editor, the rail showing views in their slots, the side
//! toggles (Ruling 13), empty slots (Ruling 12), moving views between
//! slots, splitting and closing slots (Ruling 11), directional focus, and
//! the `[gui.layout]` round trip. A real `Workspace` in a headless window,
//! driven by GPUI keyboard and pointer events.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, SharedString, TestAppContext};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::layout_tree::{Focus, LayoutTree, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;
use kimun_notes::settings::gui::GuiSettings;

use super::tests::{HEIGHT, Shell, close, gui, open_shell, saved, temp_config};
use super::{LayoutStore, empty_slot_id, rail_cell_id, slot_id};
use crate::shell_tree;

/// The hint an empty slot shows with the default keymap.
const HINT: &str =
    "No view here. Move one with Alt+Shift+Right / Alt+Shift+Left, or pick one from the rail.";

/// The default layout's slots by reading order: 0 left, 1 right.
const RIGHT_EMPTY: usize = 1;

impl Shell {
    /// The views the slots of `side` show, in reading order.
    fn shown_on(&self, side: Side, cx: &gpui_kit::App) -> Vec<DrawerView> {
        let workspace = self.workspace.read(cx);
        workspace
            .side_slots(side)
            .into_iter()
            .filter_map(|s| workspace.layout.shown(s))
            .collect()
    }

    /// The views placed in the slots of `side`, shown or hidden.
    fn placed_on(&self, side: Side, cx: &gpui_kit::App) -> Vec<DrawerView> {
        let workspace = self.workspace.read(cx);
        workspace
            .side_slots(side)
            .into_iter()
            .flat_map(|s| workspace.layout.slot_views(s))
            .collect()
    }

    fn slots(&self, cx: &gpui_kit::App) -> Vec<SlotId> {
        self.workspace.read(cx).layout.slots()
    }

    fn focus(&self, cx: &gpui_kit::App) -> Focus {
        self.workspace.read(cx).layout.focused()
    }

    fn flash(&self, cx: &gpui_kit::App) -> Option<String> {
        self.workspace
            .read(cx)
            .last_flash()
            .map(|(text, _)| text.to_string())
    }
}

/// The layout the saved `[gui]` table restores.
fn saved_tree(path: &std::path::Path) -> LayoutTree<DrawerView> {
    shell_tree::restore(&saved(path), shell_tree::RESTORE_CAPS)
}

/// The views placed in the saved layout's slots, shown or hidden, in
/// reading order of the slots.
fn saved_placed(path: &std::path::Path) -> Vec<Vec<DrawerView>> {
    let tree = saved_tree(path);
    tree.slots()
        .into_iter()
        .map(|s| tree.slot_views(s))
        .collect()
}

/// The views the saved layout's slots show, in reading order.
fn saved_shown(path: &std::path::Path) -> Vec<Option<DrawerView>> {
    let tree = saved_tree(path);
    tree.slots().into_iter().map(|s| tree.shown(s)).collect()
}

/// A `[gui.layout]` of the default shape (left slot, editor, right slot)
/// with these slots, written as spec nodes.
fn three(left: &str, right: &str) -> GuiSettings {
    gui(&format!(
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         {{ weight = 0.2, {left} }},\n{{ weight = 0.6, editor = true }},\n\
         {{ weight = 0.2, {right} }},\n]\n"
    ))
}

/// Same visible structure as the drawer era: FILES in a slot right of the
/// rail, the editor filling the rest, and no right area drawn — the empty
/// right slot takes no space (Ruling 12). Weights give the widths.
#[gpui_kit::test]
fn default_window_has_left_files_slot_editor_and_empty_right_slot(cx: &mut TestAppContext) {
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
        let slots = shell.slots(cx);
        assert_eq!(slots.len(), 2, "a left and a right slot");
        let layout = &shell.workspace.read(cx).layout;
        assert_eq!(layout.shown(slots[0]), Some(DrawerView::Files));
        assert!(layout.slot_views(slots[1]).is_empty());
        assert_eq!(shell.focused(cx), Panel::Editor);

        let rail = window.find("rail").bounds();
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(close(files.left(), f32::from(rail.right())));
        assert!(close(editor.left(), f32::from(files.right()) + 1.));
        assert!(close(editor.right(), 1000.));
        assert!(close(files.size.height, HEIGHT));
        // 0.2 : 0.6 of the 952px right of the rail, the empty right
        // slot's share given back.
        assert!(close(files.size.width, 951. * 0.25), "{files:?}");
        assert!(window.try_find(empty_slot_id(RIGHT_EMPTY)).is_none());
        assert_eq!(window.find("slot-files").label(), Some("Files"));
    })
    .unwrap();
}

/// FIND shown from the rail takes FILES' slot: FILES stays placed there,
/// hidden, and comes back in the same slot when shown again.
#[gpui_kit::test]
fn showing_find_replaces_files_in_a_shared_slot(cx: &mut TestAppContext) {
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
        let left = shell.slots(cx)[0];
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Files)).is_none());
        let find = window.find(slot_id(DrawerView::Find)).bounds();
        assert!(close(
            find.left(),
            f32::from(window.find("rail").bounds().right())
        ));
        let layout = &shell.workspace.read(cx).layout;
        assert_eq!(layout.shown(left), Some(DrawerView::Find));
        assert_eq!(
            layout.slot_views(left),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(left)));
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Files, DrawerView::Find], vec![]]
        );

        window.click(rail_cell_id(DrawerView::Files), cx);
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Find)).is_none());
        assert!(window.try_find(slot_id(DrawerView::Files)).is_some());
        let layout = &shell.workspace.read(cx).layout;
        assert_eq!(layout.shown(left), Some(DrawerView::Files));
        assert_eq!(layout.view_slot(&DrawerView::Find), Some(left));
        assert_eq!(saved_shown(&path), [Some(DrawerView::Files), None]);
    })
    .unwrap();
}

/// A rail click shows its view in the slot it is placed in: OUTLINE, placed
/// hidden in the right slot, opens right of the editor and takes the
/// keyboard, FILES staying on the left; its cell reads as selected. The
/// cell again hides it, and the right area goes with it.
#[gpui_kit::test]
fn rail_click_shows_view_in_its_slot(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &three(
            "views = [\"files\"], shown = \"files\"",
            "views = [\"outline\"]",
        ),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let selected = |window: &gpui_kit::Window, view| window.find(rail_cell_id(view)).selected();
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        assert_eq!(selected(window, DrawerView::Outline), Some(false));

        window.click(rail_cell_id(DrawerView::Outline), cx);
        window.render_frame(cx);
        let editor = window.find("editor-panel").bounds();
        let outline = window.find(slot_id(DrawerView::Outline)).bounds();
        assert!(close(outline.left(), f32::from(editor.right()) + 1.));
        assert!(close(outline.right(), 1200.));
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Right));
        assert_eq!(
            window.find(slot_id(DrawerView::Outline)).focused(),
            Some(true)
        );
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(selected(window, DrawerView::Outline), Some(true));
        assert_eq!(selected(window, DrawerView::Files), Some(true));
        assert_eq!(
            saved_shown(&path),
            [Some(DrawerView::Files), Some(DrawerView::Outline)]
        );

        // Tags is placed nowhere: it opens in the first slot, over FILES.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Tags]);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));

        // The shown view's cell hides it; its slot is not drawn any more.
        window.click(rail_cell_id(DrawerView::Outline), cx);
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        assert!(close(window.find("editor-panel").bounds().right(), 1200.));
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert_eq!(saved_placed(&path)[1], [DrawerView::Outline]);
        assert_eq!(saved_shown(&path)[1], None);
    })
    .unwrap();
}

/// The right side's toggle with nothing placed there opens the right slot
/// empty, with the hint, right of the editor; the keyboard stays in the
/// editor. Again closes it. A view moved there fills it and takes the
/// keyboard.
#[gpui_kit::test]
fn toggle_right_side_with_no_view_shows_hint(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "abc",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        let empty = window.find(empty_slot_id(RIGHT_EMPTY));
        assert_eq!(empty.label(), Some(HINT));
        let empty = empty.bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(close(empty.left(), f32::from(editor.right()) + 1.));
        assert!(close(empty.right(), 1000.));
        assert!(close(empty.size.height, HEIGHT));
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.input("x", cx);
        assert_eq!(shell.text(cx), "xabc");
        // Nothing placed: nothing to save there.
        assert_eq!(saved_placed(&path)[1], Vec::<DrawerView>::new());

        // Again: gone, the editor gets the width back.
        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        assert!(window.try_find(empty_slot_id(RIGHT_EMPTY)).is_none());
        assert!(close(window.find("editor-panel").bounds().right(), 1000.));
        assert_eq!(shell.focused(cx), Panel::Editor);

        // Open, then FILES moved over fills it and has the keyboard.
        window.press("ctrl-shift-t", cx);
        window.press("alt-left", cx);
        window.press("alt-shift-right", cx);
        window.render_frame(cx);
        assert_eq!(shell.shown_on(Side::Right, cx), [DrawerView::Files]);
        assert!(shell.shown_on(Side::Left, cx).is_empty());
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Right));
        assert!(window.try_find(empty_slot_id(RIGHT_EMPTY)).is_none());
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        assert!(close(files.right(), 1000.));
        assert!(close(
            window.find("editor-panel").bounds().left(),
            f32::from(window.find("rail").bounds().right())
        ));
    })
    .unwrap();
}

/// The left side's toggle hides the view shown there and gives the editor
/// its width; the keyboard goes back to the document. Toggled on again it
/// brings back the view it hid (TAGS, not FILES), focused. Nothing but
/// placements is saved: the hidden view stays placed.
#[gpui_kit::test]
fn toggling_left_side_off_and_on_restores_the_shown_view(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "abc",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Tags]);

        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Tags)).is_none());
        let rail = window.find("rail").bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(close(editor.left(), f32::from(rail.right())));
        assert!(close(editor.size.width, 1000. - f32::from(rail.size.width)));
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(shell.text(cx), "abc");
        assert_eq!(
            saved_placed(&path)[0],
            [DrawerView::Files, DrawerView::Tags]
        );
        assert_eq!(saved_shown(&path)[0], None);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("visible"), "{written}");

        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Tags)).is_some());
        assert!(window.try_find(slot_id(DrawerView::Files)).is_none());
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        assert_eq!(window.find(slot_id(DrawerView::Tags)).focused(), Some(true));
        assert_eq!(saved_shown(&path)[0], Some(DrawerView::Tags));
    })
    .unwrap();
}

/// The move chords carry the focused view to the nearest slot that way —
/// moved, never copied — and the keyboard goes with it; a slot left with no
/// shown view is not drawn. From the rail the move is spatial: Right moves
/// the cursor's view into the slot beside the rail; the rail keeps the
/// keyboard. Every move is saved and restored.
#[gpui_kit::test]
fn views_move_between_slots_and_persist(cx: &mut TestAppContext) {
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
        let (left, right) = (shell.slots(cx)[0], shell.slots(cx)[1]);
        window.press("alt-left", cx);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(left)));

        window.press("alt-shift-right", cx);
        window.render_frame(cx);
        assert!(shell.placed_on(Side::Left, cx).is_empty());
        assert_eq!(shell.shown_on(Side::Right, cx), [DrawerView::Files]);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(right)));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        assert_eq!(
            window.find(rail_cell_id(DrawerView::Files)).selected(),
            Some(true)
        );
        let editor = window.find("editor-panel").bounds();
        let rail = window.find("rail").bounds();
        assert!(close(editor.left(), f32::from(rail.right())));
        assert_eq!(saved_placed(&path), [vec![], vec![DrawerView::Files]]);

        // Nothing further right: handled, nothing moves.
        window.press("alt-shift-right", cx);
        assert_eq!(shell.shown_on(Side::Right, cx), [DrawerView::Files]);

        // And back.
        window.press("alt-shift-left", cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert!(shell.placed_on(Side::Right, cx).is_empty());
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(left)));
        assert_eq!(saved_placed(&path), [vec![DrawerView::Files], vec![]]);

        // Files to the right again, for the rail moves below.
        window.press("alt-shift-right", cx);
        // From the rail: Right moves the cursor's view (placed nowhere)
        // into the slot beside the rail; the rail keeps the keyboard.
        window.click("rail", cx);
        for _ in 0..4 {
            window.press("down", cx);
        }
        window.press("alt-shift-right", cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Outline]);
        assert_eq!(shell.shown_on(Side::Right, cx), [DrawerView::Files]);
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert_eq!(window.find("rail").focused(), Some(true));
        // A view shown on the right comes over to the left too.
        for _ in 0..4 {
            window.press("up", cx);
        }
        window.press("alt-shift-right", cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert!(shell.shown_on(Side::Right, cx).is_empty());
        assert_eq!(shell.focused(cx), Panel::Rail);
        // The editor keeps Alt+Shift+Arrow: no move from there.
        window.click("editor-panel", cx);
        window.press("alt-shift-right", cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
    })
    .unwrap();

    // A reopen restores the slots.
    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).label(),
            Some("Files")
        );
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        assert_eq!(shell.focused(cx), Panel::Editor);
    })
    .unwrap();
}

/// From the rail, Alt+Shift+Left points at the window's edge: nothing is
/// there, so the key is not handled and nothing moves.
#[gpui_kit::test]
fn the_rail_move_toward_the_edge_passes_on(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &three(
            "views = [\"files\"], shown = \"files\"",
            "views = [\"outline\"], shown = \"outline\"",
        ),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let ws = shell.workspace.clone();
        window.click("rail", cx);
        let key =
            crate::keymap::parse_chord("alt+shift+left", crate::keymap::Platform::LinuxWindows)
                .unwrap();
        for cursor in [0, 4] {
            ws.update(cx, |workspace, _| workspace.rail_cursor = cursor);
            let handled = ws.update(cx, |workspace, cx| {
                workspace.workspace_command(
                    &ActionShortcuts::MoveViewToLeftDrawer,
                    Some(key),
                    window,
                    cx,
                )
            });
            assert!(!handled, "cursor {cursor}");
            assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
            assert_eq!(shell.shown_on(Side::Right, cx), [DrawerView::Outline]);
            assert_eq!(shell.focused(cx), Panel::Rail);
        }
    })
    .unwrap();
}

/// The split chord splits the focused slot: an empty slot opens under it,
/// at half the height, with the hint; the keyboard stays on the view and
/// nothing is saved. The close chord on the empty slot closes it; on a slot
/// with views it merges them, hidden, into the slot taking over its area.
/// The only slot stays, and says so.
#[gpui_kit::test]
fn a_slot_splits_and_closes(cx: &mut TestAppContext) {
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
        // Something saved first, to compare against.
        window.press("ctrl-shift-t", cx);
        window.press("ctrl-shift-t", cx);
        let before = std::fs::read_to_string(&path).unwrap();
        let left = shell.slots(cx)[0];
        // Not from the editor: the chord is the editor's there.
        window.press("alt-shift-s", cx);
        assert_eq!(shell.slots(cx).len(), 2);

        window.press("alt-left", cx);
        window.press("alt-shift-s", cx);
        window.render_frame(cx);
        let slots = shell.slots(cx);
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0], left);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(left)));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        let top = window.find(slot_id(DrawerView::Files)).bounds();
        let empty = window.find(empty_slot_id(1));
        assert_eq!(empty.label(), Some(HINT));
        let bottom = empty.bounds();
        assert!(close(top.size.height, (HEIGHT - 1.) / 2.));
        assert!(close(bottom.top(), f32::from(top.bottom()) + 1.));
        assert!(close(bottom.bottom(), HEIGHT));
        assert!(close(bottom.left(), f32::from(top.left())));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "session only"
        );

        // Closing the empty slot gives FILES its height back.
        window.click(empty_slot_id(1), cx);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(slots[1])));
        window.press("alt-shift-u", cx);
        window.render_frame(cx);
        assert_eq!(shell.slots(cx).len(), 2);
        assert!(close(
            window.find(slot_id(DrawerView::Files)).bounds().size.height,
            HEIGHT
        ));
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(left)));

        // Two views stacked: closing the bottom one merges it into FILES'.
        window.press("alt-shift-s", cx);
        window.click("rail", cx);
        window.press("down", cx);
        window.press("alt-shift-down", cx);
        assert_eq!(
            shell.shown_on(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-u", cx);
        window.render_frame(cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(left)));
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Files, DrawerView::Find], vec![]]
        );

        // Closing the right slot merges its views, hidden, into the
        // nearest slot (the left one: the editor takes the area over); the
        // keyboard goes to the editor, since that slot shows nothing.
        window.press("alt-shift-right", cx);
        let right = shell.slots(cx)[1];
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(right)));
        window.press("alt-shift-u", cx);
        assert_eq!(shell.slots(cx), [left]);
        assert_eq!(
            shell.placed_on(Side::Left, cx),
            [DrawerView::Find, DrawerView::Files]
        );
        assert_eq!(
            shell.focused(cx),
            Panel::Editor,
            "its absorber is not drawn"
        );
        // The last slot stays.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.press("alt-shift-u", cx);
        assert_eq!(shell.slots(cx), [left]);
        assert_eq!(shell.flash(cx).as_deref(), Some("This is the only slot"));
    })
    .unwrap();
}

/// The swap chords trade the focused view with the one shown in the slot
/// above or below, the keyboard following the view; at the end of the
/// column nothing moves. In a slot with nothing stacked they pass on.
#[gpui_kit::test]
fn the_swap_chords_trade_stacked_views(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let stacked = "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
        { weight = 0.2, split = \"vertical\", children = [\n\
          { weight = 0.3, views = [\"files\"], shown = \"files\" },\n\
          { weight = 0.7, views = [\"find\"], shown = \"find\" },\n] },\n\
        { weight = 0.8, editor = true },\n]\n";
    let shell = open_shell(
        cx,
        1000.,
        &gui(stacked),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let (top, bottom) = (shell.slots(cx)[0], shell.slots(cx)[1]);
        window.click(slot_id(DrawerView::Files), cx);
        // The top of the column: nothing moves, but the key is handled.
        window.press("alt-shift-up", cx);
        assert_eq!(
            shell.shown_on(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );

        window.press("alt-shift-down", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.shown_on(Side::Left, cx),
            [DrawerView::Find, DrawerView::Files]
        );
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(bottom)));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        // Each slot keeps its size: Find is now the 30% top slot.
        let find = window.find(slot_id(DrawerView::Find)).bounds();
        assert!(close(find.size.height, (HEIGHT - 1.) * 0.3));
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Find], vec![DrawerView::Files]]
        );
        window.press("alt-shift-up", cx);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(top)));
        assert_eq!(
            shell.shown_on(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );

        // Nothing stacked (FIND hidden): not handled.
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.click(slot_id(DrawerView::Files), cx);
        let key = crate::keymap::parse_chord("alt+shift+up", crate::keymap::Platform::LinuxWindows)
            .unwrap();
        let ws = shell.workspace.clone();
        for action in [ActionShortcuts::MoveViewUp, ActionShortcuts::MoveViewDown] {
            let handled = ws.update(cx, |workspace, cx| {
                workspace.workspace_command(&action, Some(key), window, cx)
            });
            assert!(!handled, "{action:?} passes on");
        }
    })
    .unwrap();
}

/// Alt+Arrow moves focus geometrically over the drawn slots and the
/// editor: no wrap, undrawn slots skipped, Up/Down only between stacked
/// slots. GPUI focus follows the model's.
#[gpui_kit::test]
fn directional_focus_crosses_slots(cx: &mut TestAppContext) {
    let layout = "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
        { weight = 0.2, split = \"vertical\", children = [\n\
          { views = [\"files\"], shown = \"files\" },\n\
          { views = [\"find\"], shown = \"find\" },\n] },\n\
        { weight = 0.6, editor = true },\n\
        { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n";
    let shell = open_shell(cx, 1200., &gui(layout), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let slots = shell.slots(cx);
        let (files_s, find_s, outline_s) = (slots[0], slots[1], slots[2]);
        let editor = shell.focus(cx);
        assert!(matches!(editor, Focus::Leaf(Leaf::Group(_))));
        let slot = |s| Focus::Leaf(Leaf::Slot(s));
        let files = slot_id(DrawerView::Files);
        let find = slot_id(DrawerView::Find);
        let outline = slot_id(DrawerView::Outline);
        let (editor_id, rail) = (SharedString::from("editor"), SharedString::from("rail"));
        let mut step = |key: &str, expected: Focus, focused: &SharedString| {
            window.press(key, cx);
            assert_eq!(shell.focus(cx), expected, "{key}");
            assert_eq!(window.find(focused.clone()).focused(), Some(true), "{key}");
        };

        step("alt-right", slot(outline_s), &outline);
        // The right edge, and nothing stacked: no move.
        step("alt-right", slot(outline_s), &outline);
        step("alt-up", slot(outline_s), &outline);
        step("alt-left", editor, &editor_id);
        step("alt-left", slot(files_s), &files);
        step("alt-down", slot(find_s), &find);
        // The bottom of the stack: handled, no wrap.
        step("alt-down", slot(find_s), &find);
        step("alt-right", editor, &editor_id);
        // Back to the slot it last had.
        step("alt-left", slot(find_s), &find);
        step("alt-up", slot(files_s), &files);
        step("alt-left", Focus::Rail, &rail);
        step("alt-left", Focus::Rail, &rail);
        step("alt-right", slot(files_s), &files);
        step("alt-right", editor, &editor_id);
        // A side toggled off is skipped.
        step("ctrl-shift-t", editor, &editor_id);
        step("alt-right", editor, &editor_id);
        step("ctrl-t", editor, &editor_id);
        step("alt-left", Focus::Rail, &rail);
    })
    .unwrap();
}

/// The hint is built from the keymap once, not per frame, and rebuilt when
/// the keymap is replaced: with the move chords unbound it stops naming them,
/// and the leader gateway's label follows too.
#[gpui_kit::test]
fn the_empty_slot_hint_follows_a_replaced_keymap(cx: &mut TestAppContext) {
    use crate::keymap::{self, Keymap, Platform};
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        assert_eq!(window.find(empty_slot_id(RIGHT_EMPTY)).label(), Some(HINT));
        assert_eq!(
            shell.workspace.read(cx).leader_chord_label().as_ref(),
            "Ctrl+Shift+B"
        );
    })
    .unwrap();
    cx.update(|cx| {
        let overrides = [
            ("MoveViewToRightDrawer", vec![]),
            ("MoveViewToLeftDrawer", vec![]),
            ("Leader", vec![]),
        ]
        .into_iter()
        .map(|(name, chords)| (name.to_string(), chords))
        .collect();
        let (unbound, errors) = Keymap::load(Platform::LinuxWindows, &overrides);
        assert!(errors.is_empty(), "{errors:?}");
        keymap::init(cx, unbound);
    });
    cx.run_until_parked();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(empty_slot_id(RIGHT_EMPTY)).label(),
            Some("No view here. Pick one from the rail.")
        );
        assert_eq!(
            shell.workspace.read(cx).leader_chord_label().as_ref(),
            "Leader"
        );
    })
    .unwrap();
}

/// A saved `[gui.layout]` opens as written, weights included; one that
/// does not build opens the default layout.
#[gpui_kit::test]
fn the_saved_layout_is_restored_and_a_bad_one_falls_back(cx: &mut TestAppContext) {
    let layout = "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
        { weight = 0.25, split = \"vertical\", children = [\n\
          { weight = 0.3, views = [\"files\"], shown = \"files\" },\n\
          { weight = 0.7, views = [\"tags\"], shown = \"tags\" },\n] },\n\
        { weight = 0.5, editor = true },\n\
        { weight = 0.25, views = [\"outline\"], shown = \"outline\" },\n]\n";
    let shell = open_shell(cx, 1248., &gui(layout), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // 1200px right of the rail less two 1px gaps: 299.5 / 599 / 299.5.
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let tags = window.find(slot_id(DrawerView::Tags)).bounds();
        let outline = window.find(slot_id(DrawerView::Outline)).bounds();
        assert!(close(files.size.width, 299.5), "{files:?}");
        assert!(close(files.size.height, (HEIGHT - 1.) * 0.3));
        assert!(close(tags.top(), (HEIGHT - 1.) * 0.3 + 1.));
        assert!(close(outline.size.width, 299.5));
        assert!(close(outline.right(), 1248.));
        assert_eq!(shell.focused(cx), Panel::Editor);
    })
    .unwrap();

    let bad = "[gui.layout]\nsplit = \"diagonal\"\nchildren = [\n\
        { views = [\"nope\", \"files\"] },\n{ editor = true },\n]\n";
    let shell = open_shell(cx, 1000., &gui(bad), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            shell.workspace.read(cx).layout,
            LayoutTree::default_layout(DrawerView::Files)
        );
        assert!(window.try_find(slot_id(DrawerView::Files)).is_some());
    })
    .unwrap();
}

/// With a slot opened empty, the rail cursor's Move Down puts the cursor's
/// view (shown nowhere) there; the rail keeps the keyboard.
#[gpui_kit::test]
fn the_rail_cursor_moves_a_view_into_an_empty_slot(cx: &mut TestAppContext) {
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
        window.press("ctrl-shift-t", cx);
        window.click("rail", cx);
        window.press("down", cx);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);
        assert_eq!(shell.shown_on(Side::Right, cx), [DrawerView::Find]);
        assert!(window.try_find(empty_slot_id(RIGHT_EMPTY)).is_none());
        assert_eq!(shell.focused(cx), Panel::Rail);
    })
    .unwrap();
}

/// Both sides stacked: FILES over FIND (0.3 : 0.7) on the left, OUTLINE
/// over TAGS (0.6 : 0.4) on the right.
fn two_stacks() -> GuiSettings {
    gui(&[
        "[gui.layout]",
        "split = \"horizontal\"",
        "children = [",
        "{ weight = 0.2, split = \"vertical\", children = [",
        "  { weight = 0.3, views = [\"files\"], shown = \"files\" },",
        "  { weight = 0.7, views = [\"find\"], shown = \"find\" },",
        "] },",
        "{ weight = 0.6, editor = true },",
        "{ weight = 0.2, split = \"vertical\", children = [",
        "  { weight = 0.6, views = [\"outline\"], shown = \"outline\" },",
        "  { weight = 0.4, views = [\"tags\"], shown = \"tags\" },",
        "] },",
        "]",
    ]
    .join("\n"))
}

/// Clicking the rail cell of one view of a stacked pair hides only that
/// view: its slot takes no space, the other fills the column and takes the
/// keyboard. The last view of a side hides that side. Shown again, a view
/// comes back to its own slot, at its weight, with the keyboard; across a
/// reopen too.
#[gpui_kit::test]
fn the_rail_hides_and_shows_one_view_of_a_stacked_pair(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &two_stacks(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let slots = shell.slots(cx);
        let (files_s, find_s, outline_s, tags_s) = (slots[0], slots[1], slots[2], slots[3]);
        let height = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, view| {
            window.render_frame(cx);
            f32::from(window.find(slot_id(view)).bounds().size.height)
        };

        // FILES, the top of the left stack: FIND fills the column.
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Find]);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(find_s)));
        assert!(close(
            gpui_kit::px(height(window, cx, DrawerView::Find)),
            HEIGHT
        ));
        assert_eq!(window.find(slot_id(DrawerView::Find)).focused(), Some(true));
        assert_eq!(
            window.find(rail_cell_id(DrawerView::Files)).selected(),
            Some(false)
        );
        assert_eq!(saved_shown(&path)[..2], [None, Some(DrawerView::Find)]);

        // Back on top at its 30%, with the keyboard.
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert!(close(
            gpui_kit::px(height(window, cx, DrawerView::Files)),
            (HEIGHT - 1.) * 0.3
        ));
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(files_s)));

        // TAGS, the bottom of the right stack: OUTLINE stays, focused.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(shell.shown_on(Side::Right, cx), [DrawerView::Outline]);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(outline_s)));
        // And back below it at 40%.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert!(close(
            gpui_kit::px(height(window, cx, DrawerView::Outline)),
            (HEIGHT - 1.) * 0.6
        ));
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(tags_s)));

        // The last view of a side: the side hides, focus stays on the rail.
        window.click(rail_cell_id(DrawerView::Outline), cx);
        window.click(rail_cell_id(DrawerView::Tags), cx);
        window.render_frame(cx);
        assert!(shell.shown_on(Side::Right, cx).is_empty());
        assert!(close(window.find("editor-panel").bounds().right(), 1200.));
        assert_eq!(shell.focused(cx), Panel::Rail);
        // Hidden on the left too, for the reopen below.
        window.click(rail_cell_id(DrawerView::Find), cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
    })
    .unwrap();

    // Placements are saved: after a reopen, showing the others restores
    // both stacks at their weights.
    let shell = open_shell(cx, 1200., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.click(rail_cell_id(DrawerView::Outline), cx);
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(
            shell.shown_on(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(
            shell.shown_on(Side::Right, cx),
            [DrawerView::Outline, DrawerView::Tags]
        );
        window.render_frame(cx);
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        assert!(close(files.size.height, (HEIGHT - 1.) * 0.3));
        let outline = window.find(slot_id(DrawerView::Outline)).bounds();
        assert!(close(outline.size.height, (HEIGHT - 1.) * 0.6));
    })
    .unwrap();
}

/// Close right after Split, the keyboard still on FILES: the new empty slot
/// closes, not FILES' (Ruling 15). FILES stays shown, no hint is left.
#[gpui_kit::test]
fn closing_right_after_a_split_closes_the_empty_slot(cx: &mut TestAppContext) {
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
        window.press("alt-left", cx);
        window.press("alt-shift-s", cx);
        assert_eq!(shell.slots(cx).len(), 3);
        window.press("alt-shift-u", cx);
        window.render_frame(cx);
        assert_eq!(shell.slots(cx).len(), 2);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert!(window.try_find(empty_slot_id(1)).is_none());
        assert!(close(
            window.find(slot_id(DrawerView::Files)).bounds().size.height,
            HEIGHT
        ));
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
    })
    .unwrap();
}

/// A side with no slot at all: its toggle says so.
#[gpui_kit::test]
fn toggling_a_side_without_a_slot_says_so(cx: &mut TestAppContext) {
    let layout = [
        "[gui.layout]",
        "split = \"horizontal\"",
        "children = [",
        "{ weight = 0.2, views = [\"files\"], shown = \"files\" },",
        "{ weight = 0.8, editor = true },",
        "]",
    ]
    .join("\n");
    let shell = open_shell(cx, 1000., &gui(&layout), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-t", cx);
        assert_eq!(shell.flash(cx).as_deref(), Some("No slot on that side"));
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
    })
    .unwrap();
}

/// Every drawn slot carries a header with the title of the view it shows.
#[gpui_kit::test]
fn every_slot_has_a_header_with_its_view_title(cx: &mut TestAppContext) {
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
        window.press("alt-left", cx);
        window.press("alt-shift-s", cx);
        window.click("rail", cx);
        window.press("down", cx);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.shown_on(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        let mut headers = window.find_all("slot-header");
        headers.extend(window.find_all("slot-header-focused"));
        assert_eq!(headers.len(), 2, "one header per slot");
        headers.sort_by(|a, b| f32::from(a.bounds().top()).total_cmp(&f32::from(b.bounds().top())));
        let titles: Vec<_> = headers
            .iter()
            .map(|h| h.label().map(str::to_owned))
            .collect();
        let expect = |v| Some(view_registry::spec(v).title.to_string());
        assert_eq!(
            titles,
            [expect(DrawerView::Files), expect(DrawerView::Find)]
        );
    })
    .unwrap();
}

/// A slot drawn through `opened_empty` shows `Empty` in its header.
#[gpui_kit::test]
fn empty_slot_header_says_empty(cx: &mut TestAppContext) {
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
        let before =
            window.find_all("slot-header").len() + window.find_all("slot-header-focused").len();
        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        assert!(window.try_find(empty_slot_id(RIGHT_EMPTY)).is_some());
        let all: Vec<_> = window
            .find_all("slot-header")
            .into_iter()
            .chain(window.find_all("slot-header-focused"))
            .collect();
        assert_eq!(all.len(), before + 1);
        assert!(all.iter().any(|h| h.label() == Some("Empty")));
    })
    .unwrap();
}

/// Exactly the focused slot's header is marked focused.
#[gpui_kit::test]
fn focused_slot_header_is_marked_focused(cx: &mut TestAppContext) {
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
        assert!(window.try_find("slot-header-focused").is_none());
        assert_eq!(window.find_all("slot-header").len(), 1);
        window.press("alt-left", cx);
        window.render_frame(cx);
        assert_eq!(window.find_all("slot-header-focused").len(), 1);
        assert_eq!(window.find_all("slot-header").len(), 0);
    })
    .unwrap();
}

/// The editor group is not a slot: no header, only its focus bar.
#[gpui_kit::test]
fn editor_group_has_no_slot_header(cx: &mut TestAppContext) {
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
        let editor = window.find("editor-panel").bounds();
        let headers: Vec<_> = window
            .find_all("slot-header")
            .into_iter()
            .chain(window.find_all("slot-header-focused"))
            .collect();
        // Only FILES' header: none sits within the editor's bounds.
        assert_eq!(headers.len(), 1);
        assert!(headers.iter().all(|h| !editor.contains(&h.bounds().origin)));
    })
    .unwrap();
}
