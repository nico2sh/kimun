//! UI integration tests for the layout tree in the window (ADR-0052):
//! slots beside the editor, the rail showing views in their slots, the side
//! toggles (Ruling 13) moving views between
//! slots, splitting and closing slots (Ruling 11), directional focus, and
//! the `[gui.layout]` round trip. A real `Workspace` in a headless window,
//! driven by GPUI keyboard and pointer events.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, SharedString, TestAppContext};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::layout::{Dir, Panel, Side};
use kimun_notes::logic::layout_tree::{Focus, LayoutTree, Leaf, SlotId, TreeFocusMove, TreeShape};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;
use kimun_notes::settings::gui::GuiSettings;

use super::start_tests::chords;
use super::tests::{HEIGHT, Shell, close, gui, open_shell, saved, temp_config};
use super::{LayoutStore, rail_cell_id, slot_id};
use crate::shell_tree;

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

/// The right side's toggle with nothing placed there asks which view the
/// right slot shows (ADR-0054 §5): nothing opens, and the keyboard is the
/// chooser's, until a pick. Escape opens nothing. A pick shows the view in
/// the right slot, right of the editor, and gives it the keyboard; the
/// toggle again hides it.
#[gpui_kit::test]
fn toggle_right_side_with_no_view_asks_which_to_show(cx: &mut TestAppContext) {
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
        let offered = shell.workspace.read(cx).side_chooser_views(cx);
        let offered = offered.expect("the chooser is open");
        assert!(!offered.contains(&DrawerView::Files), "{offered:?}");
        assert!(shell.shown_on(Side::Right, cx).is_empty());

        // Cancel: still nothing, the editor has the keyboard back.
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(shell.workspace.read(cx).side_chooser_views(cx).is_none());
        assert!(shell.shown_on(Side::Right, cx).is_empty());
        assert!(close(window.find("editor-panel").bounds().right(), 1000.));
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.input("x", cx);
        assert_eq!(shell.text(cx), "xabc");
    })
    .unwrap();
    cx.update_window(shell.handle, |_, window, cx| {
        // Open again and pick the first row.
        window.press("ctrl-shift-t", cx);
        assert!(shell.workspace.read(cx).side_chooser_views(cx).is_some());
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let shown = shell.shown_on(Side::Right, cx);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Right));
        let bounds = window.find(slot_id(shown[0])).bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(close(bounds.left(), f32::from(editor.right()) + 1.));
        assert!(close(bounds.right(), 1000.));
        assert_eq!(saved_placed(&path)[1], shown);

        // Again: hidden, the editor gets the width back.
        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        assert!(shell.shown_on(Side::Right, cx).is_empty());
        assert!(close(window.find("editor-panel").bounds().right(), 1000.));
        assert_eq!(shell.focused(cx), Panel::Editor);

        // Again: the remembered view comes back, no chooser.
        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        assert!(shell.workspace.read(cx).side_chooser_views(cx).is_none());
        assert_eq!(shell.shown_on(Side::Right, cx), shown);
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
                    &ActionShortcuts::MoveViewToNeighbour(Dir::Left),
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

/// Split slot down (Alt+Shift+D) splits the focused slot: the view picked
/// in the chooser (ADR-0054 §1) shows in a new slot under it, at half the
/// height, and takes the keyboard; saved. Close slot (Alt+Shift+W) on it
/// merges its view, hidden, into the slot taking over its area. The only
/// slot stays, and says so.
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
    let left = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let left = shell.slots(cx)[0];
            // Not from the editor: the chord is the editor's there.
            window.press("alt-shift-d", cx);
            assert_eq!(shell.slots(cx).len(), 2);
            assert!(shell.workspace.read(cx).split_chooser_views(cx).is_none());
            window.press("alt-left", cx);
            left
        })
        .unwrap();
    shell.split_pick("alt-shift-d", DrawerView::Find, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        let slots = shell.slots(cx);
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0], left);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(slots[1])));
        assert_eq!(window.find(slot_id(DrawerView::Find)).focused(), Some(true));
        let top = window.find(slot_id(DrawerView::Files)).bounds();
        let bottom = window.find(slot_id(DrawerView::Find)).bounds();
        assert!(close(top.size.height, (HEIGHT - 1.) / 2.));
        assert!(close(bottom.top(), f32::from(top.bottom()) + 1.));
        assert!(close(bottom.bottom(), HEIGHT));
        assert!(close(bottom.left(), f32::from(top.left())));
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Files], vec![DrawerView::Find], vec![]]
        );

        // Closing it merges FIND, hidden, into FILES' slot, which gets its
        // height back and the keyboard.
        window.press("alt-shift-w", cx);
        window.render_frame(cx);
        assert_eq!(shell.slots(cx).len(), 2);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert!(close(
            window.find(slot_id(DrawerView::Files)).bounds().size.height,
            HEIGHT
        ));
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(left)));
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Files, DrawerView::Find], vec![]]
        );

        // Closing the right slot merges its views, hidden, into the
        // nearest slot (the left one: the editor takes the area over); the
        // keyboard goes there, since FIND, promoted when FILES left it,
        // is shown.
        window.press("alt-shift-right", cx);
        let right = shell.slots(cx)[1];
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(right)));
        window.press("alt-shift-w", cx);
        assert_eq!(shell.slots(cx), [left]);
        assert_eq!(
            shell.placed_on(Side::Left, cx),
            [DrawerView::Find, DrawerView::Files]
        );
        assert_eq!(
            shell.focused(cx),
            Panel::Drawer(Side::Left),
            "its absorber is drawn"
        );
        // The last slot stays.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.press("alt-shift-w", cx);
        assert_eq!(shell.slots(cx), [left]);
        assert_eq!(shell.flash(cx).as_deref(), Some("This is the only slot"));
    })
    .unwrap();
}

/// Move view up / down (Alt+Shift+Up / Down) trade the focused view with the one shown in the slot
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
        for action in [
            ActionShortcuts::MoveViewToNeighbour(Dir::Up),
            ActionShortcuts::MoveViewToNeighbour(Dir::Down),
        ] {
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

/// The leader gateway's label is built from the keymap once, not per
/// frame, and rebuilt when the keymap is replaced.
#[gpui_kit::test]
fn the_leader_label_follows_a_replaced_keymap(cx: &mut TestAppContext) {
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
        window.render_frame(cx);
        assert_eq!(
            shell.workspace.read(cx).leader_chord_label().as_ref(),
            chords("Ctrl+Shift+B")
        );
    })
    .unwrap();
    cx.update(|cx| {
        let overrides = [("Leader", vec![])]
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

/// Close right after Split, the keyboard on the new slot: that slot
/// closes, not FILES' (Ruling 15). FILES stays shown at full height with the
/// picked view hidden beside it, and takes the keyboard.
#[gpui_kit::test]
fn closing_right_after_a_split_closes_the_new_slot(cx: &mut TestAppContext) {
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
    })
    .unwrap();
    shell.split_pick("alt-shift-d", DrawerView::Find, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        assert_eq!(shell.slots(cx).len(), 3);
        window.press("alt-shift-w", cx);
        window.render_frame(cx);
        assert_eq!(shell.slots(cx).len(), 2);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(
            shell.placed_on(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
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
    })
    .unwrap();
    shell.split_pick("alt-shift-d", DrawerView::Find, cx);
    cx.update_window(shell.handle, |_, window, cx| {
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

/// The top-level split's weights of `tree`, in reading order.
fn top_weights(tree: &LayoutTree<DrawerView>) -> Vec<f64> {
    match tree.shape() {
        TreeShape::Split { children, .. } => children.into_iter().map(|(w, _)| w).collect(),
        TreeShape::Leaf(_) => vec![1.0],
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// Three drawn cells (FILES, editor, OUTLINE): dragging the divider after
/// FILES moves that one boundary by the pointer's travel. FILES and the
/// editor trade width; OUTLINE keeps its own, and its saved weight is the
/// same. Saved when the drag ends.
#[gpui_kit::test]
fn dragging_a_divider_changes_the_two_neighbours_only(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &three(
            "views = [\"files\"], shown = \"files\"",
            "views = [\"outline\"], shown = \"outline\"",
        ),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let width = |window: &mut gpui_kit::Window, id: SharedString| {
            f32::from(window.find(id).bounds().size.width)
        };
        let files = width(window, slot_id(DrawerView::Files));
        let editor = f32::from(window.find("editor-panel").bounds().size.width);
        let outline = width(window, slot_id(DrawerView::Outline));
        // 0.2 : 0.6 : 0.2 of the 950px left by the rail and two gaps.
        assert!((files - 190.).abs() < 0.5, "{files}");
        assert!(window.try_find("divider-slot-0").is_some());
        let from = window.find("divider-slot-0").bounds().center();
        window.drag(
            from,
            gpui_kit::point(from.x + gpui_kit::px(100.), from.y),
            cx,
        );
        window.render_frame(cx);
        let files2 = width(window, slot_id(DrawerView::Files));
        let editor2 = f32::from(window.find("editor-panel").bounds().size.width);
        let outline2 = width(window, slot_id(DrawerView::Outline));
        assert!((files2 - (files + 100.)).abs() < 1.0, "{files} -> {files2}");
        assert!(
            (editor2 - (editor - 100.)).abs() < 1.0,
            "{editor} -> {editor2}"
        );
        assert!((outline2 - outline).abs() < 0.5, "{outline} -> {outline2}");
        // Saved on release: only the dragged pair's weights changed.
        let w = top_weights(&saved_tree(&path));
        assert_eq!(w.len(), 3);
        assert!(near(w[0] + w[1], 0.8), "{w:?}");
        assert!(near(w[2], 0.2), "{w:?}");
        assert!(near(w[0], 290. / 950.), "{w:?}");
        assert!(!shell.workspace.read(cx).resizing);
    })
    .unwrap();
}

/// A dragged boundary and a placement change survive a restart: a new
/// window over the saved `[gui]` table draws the same widths and views.
#[gpui_kit::test]
fn restart_restores_dragged_weights_and_placements(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    let before = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let from = window.find("divider-slot-0").bounds().center();
            window.drag(
                from,
                gpui_kit::point(from.x + gpui_kit::px(150.), from.y),
                cx,
            );
            // A placement change: FIND takes FILES' slot.
            window.click(rail_cell_id(DrawerView::Find), cx);
            window.render_frame(cx);
            f32::from(window.find(slot_id(DrawerView::Find)).bounds().size.width)
        })
        .unwrap();
    assert!((before - (951. * 0.25 + 150.)).abs() < 1.0, "{before}");
    let again = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(again.handle, |_, window, cx| {
        window.render_frame(cx);
        let find = f32::from(window.find(slot_id(DrawerView::Find)).bounds().size.width);
        assert!((find - before).abs() < 0.5, "{before} -> {find}");
        let left = again.slots(cx)[0];
        let layout = &again.workspace.read(cx).layout;
        assert_eq!(layout.shown(left), Some(DrawerView::Find));
        assert_eq!(
            layout.slot_views(left),
            [DrawerView::Files, DrawerView::Find]
        );
    })
    .unwrap();
}

/// Leader `w l` / `w h` grow and shrink the focused leaf by 5% of the pair
/// it shares a divider with, and each step is saved. The editor, with the
/// hidden right slot after it, grows against FILES before it.
#[gpui_kit::test]
fn grow_and_shrink_resize_the_focused_leaf(cx: &mut TestAppContext) {
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
        let leader = |window: &mut gpui_kit::Window, key: &str, cx: &mut gpui_kit::App| {
            window.press("ctrl-shift-b", cx);
            window.input("w", cx);
            window.input(key, cx);
            window.render_frame(cx);
        };
        let files = |window: &mut gpui_kit::Window| {
            f32::from(window.find(slot_id(DrawerView::Files)).bounds().size.width)
        };
        window.render_frame(cx);
        // FILES: 0.25 of the pair (FILES + editor, 951px).
        window.press("alt-left", cx);
        window.render_frame(cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        leader(window, "l", cx);
        assert!(
            (files(window) - 951. * 0.30).abs() < 0.5,
            "{}",
            files(window)
        );
        assert!(near(top_weights(&saved_tree(&path))[0], 0.8 * 0.30));
        leader(window, "h", cx);
        assert!(
            (files(window) - 951. * 0.25).abs() < 0.5,
            "{}",
            files(window)
        );
        assert!(near(top_weights(&saved_tree(&path))[0], 0.8 * 0.25));
        assert!(shell.flash(cx).is_none(), "{:?}", shell.flash(cx));
        // The editor grows: FILES gives way.
        window.press("alt-right", cx);
        window.render_frame(cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        leader(window, "l", cx);
        assert!(
            (files(window) - 951. * 0.20).abs() < 0.5,
            "{}",
            files(window)
        );
        // The hidden right slot kept its weight.
        assert!(near(top_weights(&saved_tree(&path))[2], 0.2));
    })
    .unwrap();
}

/// Leader grow / shrink say why when they cannot act: the rail has no
/// divider, a zoom shows one leaf, and a divider at its floor stops. The
/// palette rows are blocked for the structural cases.
#[gpui_kit::test]
fn grow_and_shrink_say_why_they_cannot_act(cx: &mut TestAppContext) {
    use super::tree_view::{AT_MINIMUM_SIZE, FOCUS_TO_RESIZE, ZOOMED_NO_RESIZE};
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::Off,
    );
    let grow_row = |cx: &gpui_kit::App| {
        shell
            .workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .iter()
            .find(|e| e.label().to_lowercase().contains("grow drawer"))
            .map(|e| e.blocked())
            .expect("the grow row is listed")
    };
    cx.update_window(shell.handle, |_, window, cx| {
        let leader = |window: &mut gpui_kit::Window, key: &str, cx: &mut gpui_kit::App| {
            window.press("ctrl-shift-b", cx);
            window.input("w", cx);
            window.input(key, cx);
            window.render_frame(cx);
        };
        window.render_frame(cx);
        // The rail has the keyboard.
        window.press("alt-left", cx);
        window.press("alt-left", cx);
        window.render_frame(cx);
        assert_eq!(shell.focus(cx), Focus::Rail);
        leader(window, "l", cx);
        assert_eq!(shell.flash(cx).as_deref(), Some(FOCUS_TO_RESIZE));
        assert_eq!(grow_row(cx), Some(FOCUS_TO_RESIZE));
        // Zoomed.
        window.press("alt-right", cx);
        window.press("ctrl-shift-m", cx);
        window.render_frame(cx);
        leader(window, "l", cx);
        assert_eq!(shell.flash(cx).as_deref(), Some(ZOOMED_NO_RESIZE));
        assert_eq!(grow_row(cx), Some(ZOOMED_NO_RESIZE));
        window.press("ctrl-shift-m", cx);
        window.render_frame(cx);
        assert_eq!(grow_row(cx), None);
        // Shrunk to the floor: the step says so.
        for _ in 0..40 {
            leader(window, "h", cx);
        }
        assert_eq!(shell.flash(cx).as_deref(), Some(AT_MINIMUM_SIZE));
    })
    .unwrap();
}

/// Ruling 12: a slot that is not drawn has no divider. The default
/// layout's hidden right slot leaves the editor without one; once a view
/// is picked into it, it is drawn and the divider between it and the
/// editor appears.
#[gpui_kit::test]
fn a_slot_not_drawn_has_no_divider_until_a_view_shows_there(cx: &mut TestAppContext) {
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
        assert!(window.try_find("divider-slot-0").is_some());
        assert!(window.try_find("divider-group-0").is_none());
        window.press("ctrl-shift-t", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.shown_on(Side::Right, cx).len(), 1);
        assert!(window.try_find("divider-group-0").is_some());
    })
    .unwrap();
}

/// A slot narrower than its title keeps a one-line header: the title is
/// clipped, never wrapped. Slots keep their minimum width where they can,
/// so the narrow one is under the editor in a window narrower than both
/// minimums (it cannot collapse there and shows `Too small`).
#[gpui_kit::test]
fn a_narrow_slot_header_does_not_wrap(cx: &mut TestAppContext) {
    let narrow = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"tags\"], shown = \"tags\" },\n\
         { weight = 0.8, split = \"vertical\", children = [\n\
         { editor = true },\n\
         { views = [\"semantic\"], shown = \"semantic\" },\n] },\n]\n");
    // 32 px right of the rail.
    let shell = open_shell(cx, 80., &narrow, "", false, LayoutStore::Off);
    let wide = open_shell(
        cx,
        1000.,
        &three(
            "views = [\"semantic\"], shown = \"semantic\"",
            "views = [\"tags\"]",
        ),
        "",
        false,
        LayoutStore::Off,
    );
    let one_line = cx
        .update_window(wide.handle, |_, window, cx| {
            window.render_frame(cx);
            let slot = window.within(slot_id(DrawerView::Semantic));
            f32::from(slot.find("slot-header").bounds().size.height)
        })
        .unwrap();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let width = f32::from(
            window
                .find(slot_id(DrawerView::Semantic))
                .bounds()
                .size
                .width,
        );
        assert!(width < 40., "the slot is narrow: {width}");
        let slot = window.within(slot_id(DrawerView::Semantic));
        let header = slot.find("slot-header").bounds();
        assert!(
            (f32::from(header.size.height) - one_line).abs() < 0.5,
            "{header:?} vs {one_line}"
        );
    })
    .unwrap();
}

/// The three drawn cells' widths: FILES, editor, OUTLINE.
fn widths3(window: &mut gpui_kit::Window) -> (f32, f32, f32) {
    let w = |window: &mut gpui_kit::Window, id: SharedString| {
        f32::from(window.find(id).bounds().size.width)
    };
    let files = w(window, slot_id(DrawerView::Files));
    let editor = w(window, "editor-panel".into());
    let outline = w(window, slot_id(DrawerView::Outline));
    (files, editor, outline)
}

/// Ruling 16: a drag past a cell's drawn minimum stops at it, on either
/// side. The third cell keeps its width and its saved weight.
#[gpui_kit::test]
fn dragging_past_a_floor_leaves_the_third_cell_alone(cx: &mut TestAppContext) {
    for travel in [600f32, -400.] {
        let (_dir, path) = temp_config();
        let shell = open_shell(
            cx,
            1000.,
            &three(
                "views = [\"files\"], shown = \"files\"",
                "views = [\"outline\"], shown = \"outline\"",
            ),
            "",
            false,
            LayoutStore::File(path.clone()),
        );
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let (files, editor, outline) = widths3(window);
            let from = window.find("divider-slot-0").bounds().center();
            window.drag(
                from,
                gpui_kit::point(from.x + gpui_kit::px(travel), from.y),
                cx,
            );
            window.render_frame(cx);
            let (files2, editor2, outline2) = widths3(window);
            assert!(
                (outline2 - outline).abs() < 0.5,
                "{travel}: {outline} -> {outline2}"
            );
            assert!(
                (files2 + editor2 - files - editor).abs() < 1.0,
                "{travel}: the pair keeps its extent"
            );
            if travel > 0. {
                assert!(
                    (editor2 - 320.).abs() < 0.5,
                    "editor at its floor: {editor2}"
                );
            } else {
                assert!((files2 - 160.).abs() < 0.5, "FILES at its floor: {files2}");
            }
            let w = top_weights(&saved_tree(&path));
            assert!(near(w[2], 0.2), "{travel}: {w:?}");
            assert!(near(w[0] + w[1], 0.8), "{travel}: {w:?}");
        })
        .unwrap();
    }
}

/// Grow and shrink stop at a floor: a step there changes nothing, saved
/// weights included.
#[gpui_kit::test]
fn grow_and_shrink_at_a_floor_are_no_ops(cx: &mut TestAppContext) {
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
        let leader = |window: &mut gpui_kit::Window, key: &str, cx: &mut gpui_kit::App| {
            window.press("ctrl-shift-b", cx);
            window.input("w", cx);
            window.input(key, cx);
            window.render_frame(cx);
        };
        let files = |window: &mut gpui_kit::Window| {
            f32::from(window.find(slot_id(DrawerView::Files)).bounds().size.width)
        };
        window.render_frame(cx);
        window.press("alt-left", cx);
        window.render_frame(cx);
        // Shrink to FILES' 10rem floor, then once more.
        for _ in 0..4 {
            leader(window, "h", cx);
        }
        assert!((files(window) - 160.).abs() < 0.5, "{}", files(window));
        let bytes = std::fs::read(&path).unwrap();
        leader(window, "h", cx);
        assert!((files(window) - 160.).abs() < 0.5);
        assert!(
            std::fs::read(&path).unwrap() == bytes,
            "no save at the floor"
        );
        // Grow until the editor is at its 20rem floor, then once more.
        for _ in 0..12 {
            leader(window, "l", cx);
        }
        let editor = f32::from(window.find("editor-panel").bounds().size.width);
        assert!((editor - 320.).abs() < 0.5, "{editor}");
        let bytes = std::fs::read(&path).unwrap();
        let weights = top_weights(&saved_tree(&path));
        leader(window, "l", cx);
        assert!(
            std::fs::read(&path).unwrap() == bytes,
            "no save at the floor"
        );
        assert_eq!(top_weights(&saved_tree(&path)), weights);
    })
    .unwrap();
}

/// A drag move resizes without saving; only its end saves.
#[gpui_kit::test]
fn a_divider_move_does_not_save_until_the_drag_ends(cx: &mut TestAppContext) {
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
        let before = std::fs::read(&path).ok();
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let pixel = f64::from(files.right()) + 50.;
        let left = Leaf::Slot(shell.slots(cx)[0]);
        shell
            .workspace
            .update(cx, |w, cx| w.on_divider_drag(left, pixel, window, cx));
        assert!(shell.workspace.read(cx).resizing);
        assert!(std::fs::read(&path).ok() == before, "not saved per move");
        shell.workspace.update(cx, |w, cx| w.end_resize(cx));
        assert!(!shell.workspace.read(cx).resizing);
        let w = top_weights(&saved_tree(&path));
        assert!(
            near(w[0], 0.8 * (f64::from(files.size.width) + 50.) / 951.),
            "{w:?}"
        );
    })
    .unwrap();
}

/// Split slot right (Alt+Shift+R): the view picked in the chooser shows in
/// a new slot beside the focused one, the two sharing its width at full
/// height, and takes the keyboard; saved. Split slot down (Alt+Shift+D)
/// puts one under it.
#[gpui_kit::test]
fn split_slot_right_places_the_pick_beside_the_focused_one(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    let (left, width) = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let width = window.find(slot_id(DrawerView::Files)).bounds().size.width;
            // Not from the editor: the chord is the editor's there.
            window.press("alt-shift-r", cx);
            assert_eq!(shell.slots(cx).len(), 2);
            assert!(shell.workspace.read(cx).split_chooser_views(cx).is_none());
            window.press("alt-left", cx);
            (shell.slots(cx)[0], width)
        })
        .unwrap();
    shell.split_pick("alt-shift-r", DrawerView::Find, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        let slots = shell.slots(cx);
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0], left);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(slots[1])));
        assert_eq!(window.find(slot_id(DrawerView::Find)).focused(), Some(true));
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let beside = window.find(slot_id(DrawerView::Find)).bounds();
        assert!(close(beside.left(), f32::from(files.right()) + 1.));
        assert!(close(beside.top(), f32::from(files.top())));
        assert!(close(beside.size.height, f32::from(files.size.height)));
        assert!(close(beside.size.height, HEIGHT));
        // FILES gave up width to it.
        assert!(files.size.width < width, "{files:?} {width:?}");
        assert_eq!(
            saved_placed(&path),
            [vec![DrawerView::Files], vec![DrawerView::Find], vec![]]
        );
        window.click(slot_id(DrawerView::Files), cx);
    })
    .unwrap();

    // Split slot down: TAGS under FILES, same width.
    shell.split_pick("alt-shift-d", DrawerView::Tags, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        assert_eq!(shell.slots(cx).len(), 4);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(shell.slots(cx)[1])));
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let under = window.find(slot_id(DrawerView::Tags)).bounds();
        assert!(close(under.top(), f32::from(files.bottom()) + 1.));
        assert!(close(under.left(), f32::from(files.left())));
        assert!(close(under.size.width, f32::from(files.size.width)));
    })
    .unwrap();
}

/// Close slot (Alt+Shift+W) closes the focused slot: its views join the
/// slot that takes over its area (its stacked sibling), hidden there, and
/// the keyboard goes to that slot. Saved.
#[gpui_kit::test]
fn close_slot_merges_views_and_focuses_the_absorber(cx: &mut TestAppContext) {
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
        let (files_s, outline_s) = (slots[0], slots[2]);

        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-w", cx);
        window.render_frame(cx);
        assert_eq!(shell.slots(cx), [files_s, slots[2], slots[3]]);
        assert_eq!(
            shell.workspace.read(cx).layout.slot_views(files_s),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(files_s)));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        assert!(close(
            window.find(slot_id(DrawerView::Files)).bounds().size.height,
            HEIGHT
        ));

        // The upper slot of a stack closes into the lower one too.
        window.click(slot_id(DrawerView::Outline), cx);
        window.press("alt-shift-w", cx);
        window.render_frame(cx);
        let tags_s = slots[3];
        assert_eq!(shell.slots(cx), [files_s, tags_s]);
        assert_eq!(
            shell.workspace.read(cx).layout.slot_views(tags_s),
            [DrawerView::Tags, DrawerView::Outline]
        );
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(tags_s)));
        assert_eq!(window.find(slot_id(DrawerView::Tags)).focused(), Some(true));
        assert_ne!(outline_s, tags_s);
        assert_eq!(
            saved_placed(&path),
            [
                vec![DrawerView::Files, DrawerView::Find],
                vec![DrawerView::Tags, DrawerView::Outline]
            ]
        );
    })
    .unwrap();
}

/// Move view (Alt+Shift+Arrow) goes by geometry, not by side or reading
/// order: from the bottom of the left column Right lands in the right
/// column's slot that overlaps it most (TAGS', not OUTLINE's, the first
/// in reading order), and from the top of the column in OUTLINE's. The
/// keyboard follows the view; every move is saved. Each left slot keeps a
/// hidden view (LINKS, SEMANTIC), so no move empties a slot (which would
/// close it, ADR-0054).
#[gpui_kit::test]
fn move_view_to_neighbour_uses_geometry(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let stacks = gui(&[
        "[gui.layout]",
        "split = \"horizontal\"",
        "children = [",
        "{ weight = 0.2, split = \"vertical\", children = [",
        "  { weight = 0.3, views = [\"files\", \"links\"], shown = \"files\" },",
        "  { weight = 0.7, views = [\"find\", \"semantic\"], shown = \"find\" },",
        "] },",
        "{ weight = 0.6, editor = true },",
        "{ weight = 0.2, split = \"vertical\", children = [",
        "  { weight = 0.6, views = [\"outline\"], shown = \"outline\" },",
        "  { weight = 0.4, views = [\"tags\"], shown = \"tags\" },",
        "] },",
        "]",
    ]
    .join("\n"));
    let shell = open_shell(
        cx,
        1200.,
        &stacks,
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let slots = shell.slots(cx);
        let (files_s, find_s, outline_s, tags_s) = (slots[0], slots[1], slots[2], slots[3]);
        let layout = |cx: &gpui_kit::App| shell.workspace.read(cx).layout.clone();

        // FIND (0.3..1.0 of the height) overlaps TAGS (0.6..1.0) more
        // than OUTLINE (0..0.6).
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-right", cx);
        window.render_frame(cx);
        assert_eq!(layout(cx).shown(tags_s), Some(DrawerView::Find));
        assert_eq!(layout(cx).shown(outline_s), Some(DrawerView::Outline));
        assert_eq!(layout(cx).slot_views(find_s), [DrawerView::Semantic]);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(tags_s)));
        assert_eq!(window.find(slot_id(DrawerView::Find)).focused(), Some(true));
        assert_eq!(
            saved_shown(&path),
            [
                Some(DrawerView::Files),
                Some(DrawerView::Semantic),
                Some(DrawerView::Outline),
                Some(DrawerView::Find)
            ]
        );

        // FILES (0..0.3) overlaps only OUTLINE.
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-right", cx);
        assert_eq!(layout(cx).shown(outline_s), Some(DrawerView::Files));
        assert_eq!(layout(cx).slot_views(files_s), [DrawerView::Links]);
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(outline_s)));

        // Down from OUTLINE's slot: the slot under it.
        window.press("alt-shift-down", cx);
        assert_eq!(layout(cx).view_slot(&DrawerView::Files), Some(tags_s));
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(tags_s)));

        // Left from TAGS' slot (0.6..1.0): FIND's old slot (0.3..1.0)
        // overlaps it, FILES' (0..0.3) does not.
        window.press("alt-shift-left", cx);
        assert_eq!(layout(cx).view_slot(&DrawerView::Files), Some(find_s));
        assert_eq!(shell.focus(cx), Focus::Leaf(Leaf::Slot(find_s)));
        assert_eq!(
            saved_placed(&path),
            [
                vec![DrawerView::Links],
                vec![DrawerView::Semantic, DrawerView::Files],
                vec![DrawerView::Outline, DrawerView::Find],
                vec![DrawerView::Tags]
            ]
        );
    })
    .unwrap();
}

/// Alt+Arrow walks a nested arrangement by geometry: a slot split down
/// whose lower half is split again side by side. Ties go to the leaf
/// focused last, then the top-left one; Left from the leftmost slot
/// reaches the rail, and Right from the rail the top-left slot.
#[gpui_kit::test]
fn focus_region_walks_nested_slots_and_reaches_the_rail(cx: &mut TestAppContext) {
    let layout = [
        "[gui.layout]",
        "split = \"horizontal\"",
        "children = [",
        "{ weight = 0.3, split = \"vertical\", children = [",
        "  { views = [\"files\"], shown = \"files\" },",
        "  { split = \"horizontal\", children = [",
        "    { views = [\"find\"], shown = \"find\" },",
        "    { views = [\"tags\"], shown = \"tags\" },",
        "  ] },",
        "] },",
        "{ weight = 0.7, editor = true },",
        "]",
    ]
    .join("\n");
    let shell = open_shell(cx, 1200., &gui(&layout), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let slots = shell.slots(cx);
        let (files_s, find_s, tags_s) = (slots[0], slots[1], slots[2]);
        let editor = shell.focus(cx);
        assert!(matches!(editor, Focus::Leaf(Leaf::Group(_))));
        let slot = |s| Focus::Leaf(Leaf::Slot(s));
        let files = slot_id(DrawerView::Files);
        let find = slot_id(DrawerView::Find);
        let tags = slot_id(DrawerView::Tags);
        let (editor_id, rail) = (SharedString::from("editor"), SharedString::from("rail"));
        let mut step = |key: &str, expected: Focus, focused: &SharedString| {
            window.press(key, cx);
            assert_eq!(shell.focus(cx), expected, "{key}");
            assert_eq!(window.find(focused.clone()).focused(), Some(true), "{key}");
        };

        // FILES and TAGS both touch the editor, overlapping it equally:
        // the top-left one wins.
        step("alt-left", slot(files_s), &files);
        // FIND and TAGS both lie under FILES: the left one.
        step("alt-down", slot(find_s), &find);
        step("alt-right", slot(tags_s), &tags);
        step("alt-up", slot(files_s), &files);
        // Down again: the top-left one again.
        step("alt-down", slot(find_s), &find);
        step("alt-right", slot(tags_s), &tags);
        step("alt-right", editor, &editor_id);
        // Back to the slot it came from: the tie goes to the last focused.
        step("alt-left", slot(tags_s), &tags);
        step("alt-left", slot(find_s), &find);
        step("alt-left", Focus::Rail, &rail);
        step("alt-left", Focus::Rail, &rail);
        step("alt-right", slot(files_s), &files);
    })
    .unwrap();
}

// ── Minimum sizes and collapse (ADR-0052 §6) ──────────────────────────

/// FILES left, the editor, OUTLINE right: every slot shown.
fn files_editor_outline() -> GuiSettings {
    three(
        "views = [\"files\"], shown = \"files\"",
        "views = [\"outline\"], shown = \"outline\"",
    )
}

/// FILES left of a column holding the editor over TAGS.
fn files_beside_editor_over_tags() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.8, split = \"vertical\", children = [\n\
           { weight = 0.7, editor = true },\n\
           { weight = 0.3, views = [\"tags\"], shown = \"tags\" },\n] },\n]\n")
}

/// Resize the shell's window to `width` x [`HEIGHT`] px.
fn resize(cx: &mut TestAppContext, shell: &Shell, width: f32) {
    cx.simulate_window_resize(
        shell.handle,
        gpui_kit::size(gpui_kit::px(width), gpui_kit::px(HEIGHT)),
    );
}

/// The window narrows: OUTLINE (trailing) goes first, then FILES; the
/// editor is always drawn and fills what the rail leaves.
#[gpui_kit::test]
fn narrow_window_collapses_trailing_slots_not_the_editor(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_editor_outline(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_some());
        assert!(window.try_find(slot_id(DrawerView::Files)).is_some());
    })
    .unwrap();
    // 560 px right of the rail = 35 rem < 10 + 20 + 10: OUTLINE goes.
    resize(cx, &shell, 608.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(close(editor.left(), f32::from(files.right()) + 1.));
        assert!(close(editor.right(), 608.), "editor {editor:?}");
    })
    .unwrap();
    // 352 px = 22 rem < 10 + 20: FILES goes too; the editor stays.
    resize(cx, &shell, 400.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        assert!(window.try_find(slot_id(DrawerView::Files)).is_none());
        let editor = window.find("editor-panel").bounds();
        assert!(close(editor.left(), 48.), "editor {editor:?}");
        assert!(close(editor.right(), 400.), "editor {editor:?}");
    })
    .unwrap();
}

/// The trailing child holds the editor, so it is skipped and FILES before
/// it collapses instead; TAGS, inside the editor's subtree, stays. Even
/// narrower than the editor's own minimum, the editor is drawn.
#[gpui_kit::test]
fn editor_group_subtree_never_collapses(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_beside_editor_over_tags(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Files)).is_some());
        assert!(window.try_find(slot_id(DrawerView::Tags)).is_some());
    })
    .unwrap();
    for width in [400., 200.] {
        resize(cx, &shell, width);
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find(slot_id(DrawerView::Files)).is_none(),
                "{width}"
            );
            assert!(
                window.try_find(slot_id(DrawerView::Tags)).is_some(),
                "{width}"
            );
            let editor = window.find("editor-panel").bounds();
            assert!(close(editor.left(), 48.), "{width}: editor {editor:?}");
            assert!(close(editor.right(), width), "{width}: editor {editor:?}");
            let fit = shell.workspace.read(cx).fit(window);
            assert!(
                fit.rects
                    .iter()
                    .any(|(leaf, _)| matches!(leaf, Leaf::Group(_)))
            );
        })
        .unwrap();
    }
}

/// Collapse lives in the session only: the saved `config.toml` is the same
/// bytes before the shrink, while shrunk (even when saved then) and after
/// the window grows back.
#[gpui_kit::test]
fn collapse_is_session_only_saved_layout_unchanged_after_shrink_and_regrow(
    cx: &mut TestAppContext,
) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &files_editor_outline(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // Twice: the first save of a missing file stamps no config
        // version; the second, reading it back, does.
        shell.workspace.update(cx, |ws, cx| ws.save_layout(cx));
        shell.workspace.update(cx, |ws, cx| ws.save_layout(cx));
    })
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    resize(cx, &shell, 400.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before, "shrunk");
        shell.workspace.update(cx, |ws, cx| ws.save_layout(cx));
        assert_eq!(std::fs::read(&path).unwrap(), before, "saved while shrunk");
    })
    .unwrap();
    resize(cx, &shell, 1000.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_some());
        shell.workspace.update(cx, |ws, cx| ws.save_layout(cx));
    })
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before, "regrown");
}

/// Collapsed slots come back, at their old widths, when the window grows.
#[gpui_kit::test]
fn collapsed_slots_return_when_the_window_grows(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_editor_outline(),
        "",
        false,
        LayoutStore::Off,
    );
    let wide = |window: &mut gpui_kit::Window| {
        (
            window.find(slot_id(DrawerView::Files)).bounds(),
            window.find(slot_id(DrawerView::Outline)).bounds(),
        )
    };
    let before = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            wide(window)
        })
        .unwrap();
    resize(cx, &shell, 400.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Files)).is_none());
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
    })
    .unwrap();
    resize(cx, &shell, 1000.);
    let after = cx
        .update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            wide(window)
        })
        .unwrap();
    assert_eq!(before, after);
}

/// A slot's minimum is that of the view it shows: a hidden view with a
/// larger one (ASK, 18 rem) does not count, so the slot still fits at 31
/// rem beside the editor (10 + 20); showing ASK makes it 18 and collapses
/// it there.
#[gpui_kit::test]
fn slot_minimum_is_the_largest_shown_view_minimum(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::Off,
    );
    let left = cx.read(|cx| shell.slots(cx))[0];
    shell.workspace.update(cx, |ws, _| {
        assert!(ws.layout.place(DrawerView::Ask, left, false));
        assert_eq!(ws.layout.shown(left), Some(DrawerView::Files));
        assert_eq!(
            ws.slot_min(left),
            view_registry::spec(DrawerView::Files).min
        );
    });
    // 496 px right of the rail = 31 rem.
    resize(cx, &shell, 544.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Files)).is_some());
        shell.workspace.update(cx, |ws, _| {
            assert_eq!(ws.layout.show_view(&DrawerView::Ask, false), Some(left));
            assert_eq!(ws.slot_min(left), view_registry::spec(DrawerView::Ask).min);
        });
        window.render_frame(cx);
        assert!(window.try_find(slot_id(DrawerView::Ask)).is_none());
    })
    .unwrap();
}

/// A collapsed slot is no focus target: the keyboard leaves it when it
/// collapses, and neither the directional moves nor the focus cycle reach
/// it.
#[gpui_kit::test]
fn focus_skips_collapsed_slots(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_editor_outline(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Outline), cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Right));
    })
    .unwrap();
    resize(cx, &shell, 608.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let ws = shell.workspace.clone();
        let group = Focus::Leaf(Leaf::Group(ws.read(cx).layout.active_group()));
        assert_eq!(shell.focus(cx), group, "focus left the collapsed slot");
        let moved = ws.update(cx, |ws, _| ws.focus_dir(Dir::Right));
        assert_eq!(moved, TreeFocusMove::AtEdge);
        assert_eq!(shell.focus(cx), group);
        // Rail, FILES, editor: OUTLINE is not in the cycle.
        let next = ws.update(cx, |ws, _| ws.cycle_focus(true));
        assert_eq!(next, Some(Focus::Rail));
    })
    .unwrap();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("editor").focused(), Some(true));
    })
    .unwrap();
}

/// TAGS sits in the editor's subtree, so it never collapses; narrower than
/// its own minimum it keeps its header and says `Too small` instead of
/// listing tags. Wide again, the placeholder goes.
#[gpui_kit::test]
fn a_slot_below_its_minimum_shows_too_small(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_beside_editor_over_tags(),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(super::tree_view::TOO_SMALL_ID).is_none());
    })
    .unwrap();
    // 152 px right of the rail: TAGS is 152 px wide, under its 160.
    resize(cx, &shell, 200.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let tags = window.find(slot_id(DrawerView::Tags)).bounds();
        let placeholder = window.find(super::tree_view::TOO_SMALL_ID);
        assert_eq!(placeholder.label(), Some("Too small"));
        assert!(tags.contains(&placeholder.bounds().center()));
        let header = window.find("slot-header").bounds();
        assert!(tags.contains(&header.center()), "the header stays");
        // FILES, outside the editor's subtree, collapsed: no placeholder.
        assert!(window.try_find(slot_id(DrawerView::Files)).is_none());
    })
    .unwrap();
    resize(cx, &shell, 1000.);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(super::tree_view::TOO_SMALL_ID).is_none());
    })
    .unwrap();
}

/// Two drawn cells with a hidden slot between them in the tree still share
/// a divider (Ruling 23): FILES split right with TAGS picked gives
/// `[A, B, editor, right]`, TAGS (in B) hidden from the rail: FILES and the
/// editor are drawn side by side across hidden B. A drag of that divider
/// changes only those two weights (B and the right slot keep theirs), and
/// the leader's grow step resizes FILES.
#[gpui_kit::test]
fn a_hidden_slot_between_two_drawn_cells_keeps_their_divider(cx: &mut TestAppContext) {
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
        window.press("alt-left", cx);
    })
    .unwrap();
    shell.split_pick("alt-shift-r", DrawerView::Tags, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        let files = |window: &mut gpui_kit::Window| {
            f32::from(window.find(slot_id(DrawerView::Files)).bounds().size.width)
        };
        let editor = |window: &mut gpui_kit::Window| {
            f32::from(window.find("editor-panel").bounds().size.width)
        };
        window.click(rail_cell_id(DrawerView::Tags), cx);
        window.render_frame(cx);
        {
            let w = shell.workspace.read(cx);
            let slots = w.layout.slots();
            assert_eq!(slots.len(), 3, "A, B and the right slot");
            assert_eq!(w.layout.shown(slots[0]), Some(DrawerView::Files));
            assert_eq!(w.layout.slot_views(slots[1]), [DrawerView::Tags]);
            assert_eq!(w.layout.shown(slots[1]), None);
            assert_eq!(top_weights(&w.layout).len(), 4, "[A, B, editor, right]");
        }
        assert!(
            window.try_find("divider-slot-0").is_some(),
            "a divider between FILES and the editor"
        );
        let before = top_weights(&shell.workspace.read(cx).layout);
        let (t, e) = (files(window), editor(window));
        let from = window.find("divider-slot-0").bounds().center();
        window.drag(
            from,
            gpui_kit::point(from.x + gpui_kit::px(100.), from.y),
            cx,
        );
        window.render_frame(cx);
        assert!(
            (files(window) - (t + 100.)).abs() < 1.0,
            "{t} -> {}",
            files(window)
        );
        assert!(
            (editor(window) - (e - 100.)).abs() < 1.0,
            "{e} -> {}",
            editor(window)
        );
        let after = top_weights(&saved_tree(&path));
        assert!(
            near(after[1], before[1]),
            "B keeps its weight: {before:?} -> {after:?}"
        );
        assert!(
            near(after[3], before[3]),
            "right keeps its weight: {before:?} -> {after:?}"
        );
        assert!(
            near(after[0] + after[2], before[0] + before[2]),
            "{before:?} -> {after:?}"
        );
        assert!(after[0] > before[0], "{before:?} -> {after:?}");

        // The leader's grow step on FILES moves the same divider.
        window.click(slot_id(DrawerView::Files), cx);
        window.render_frame(cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        let t = files(window);
        window.press("ctrl-shift-b", cx);
        window.input("w", cx);
        window.input("l", cx);
        window.render_frame(cx);
        assert!(files(window) > t + 1.0, "{t} -> {}", files(window));
        let stepped = top_weights(&saved_tree(&path));
        assert!(
            near(stepped[1], before[1]) && near(stepped[3], before[3]),
            "{stepped:?}"
        );
    })
    .unwrap();
}

/// The rail's two side buttons are the pointer's way to the toggle chords:
/// the left one hides and shows the left side, and the right one, with
/// nothing placed there, asks which view to show (ADR-0054 §5).
#[gpui_kit::test]
fn the_rail_side_buttons_toggle_their_sides(cx: &mut TestAppContext) {
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
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);
        window.click("rail-toggle-left", cx);
        window.render_frame(cx);
        assert!(shell.shown_on(Side::Left, cx).is_empty());
        window.click("rail-toggle-left", cx);
        window.render_frame(cx);
        assert_eq!(shell.shown_on(Side::Left, cx), [DrawerView::Files]);

        window.click("rail-toggle-right", cx);
        window.render_frame(cx);
        assert!(shell.workspace.read(cx).side_chooser_views(cx).is_some());
        window.press("escape", cx);
        assert!(shell.shown_on(Side::Right, cx).is_empty());
    })
    .unwrap();
}
