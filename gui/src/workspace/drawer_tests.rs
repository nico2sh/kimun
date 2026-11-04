//! UI integration tests for the M5b-3 shell: the right drawer, moving views
//! between drawers, stacked drawer slots and their divider, directional
//! focus across all of them, and the `[gui]` round trip. A real `Workspace`
//! in a headless window, driven by GPUI keyboard and pointer events.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, SharedString, TestAppContext, point, px};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::layout::{Panel, Region, Side};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::{DEFAULT_DRAWER_WIDTH, GuiSettings};

use super::tests::{HEIGHT, Shell, close, gui, open_shell, saved, temp_config};
use super::{LayoutStore, drawer_id, empty_slot_id, rail_cell_id, slot_id, split_divider_id};

const LEFT: &str = "drawer-left";
const RIGHT: &str = "drawer-right";
const RIGHT_DIVIDER: &str = "drawer-right-divider";

impl Shell {
    fn region(&self, cx: &gpui_kit::App) -> Region {
        self.workspace.read(cx).layout.focused_region()
    }

    fn views(&self, side: Side, cx: &gpui_kit::App) -> Vec<DrawerView> {
        self.workspace.read(cx).layout.slot_views(side)
    }

    fn shown(&self, side: Side, cx: &gpui_kit::App) -> bool {
        self.workspace
            .read(cx)
            .layout
            .is_visible(Panel::Drawer(side))
    }

    fn flash(&self, cx: &gpui_kit::App) -> Option<String> {
        self.workspace
            .read(cx)
            .status
            .flash
            .as_ref()
            .map(|f| f.to_string())
    }
}

fn slot(side: Side, index: usize) -> Region {
    Region::Slot { side, index }
}

/// The ids of the views the saved `[gui]` table places in `side`'s drawer,
/// shown or hidden, top slot first.
pub(super) fn saved_placed(path: &std::path::Path, side: Side) -> Vec<&'static str> {
    crate::shell_layout::restore(&saved(path))
        .placed_views(side)
        .into_iter()
        .map(|v| v.id())
        .collect()
}

/// The views the saved `[gui]` table shows in `side`'s drawer.
pub(super) fn saved_shown(path: &std::path::Path, side: Side) -> Vec<DrawerView> {
    crate::shell_layout::restore(&saved(path)).slot_views(side)
}

/// The hint an empty drawer or slot shows with the default keymap.
const HINT: &str =
    "No view here. Move one with Alt+Shift+Right / Alt+Shift+Left, or pick one from the rail.";

/// The right drawer's chord shows it right of the editor, empty (no view is
/// placed there) with a hint, and the keyboard stays in the editor; again
/// hides it. Its divider drags from its leading edge. Each change is saved.
#[gpui_kit::test]
fn the_right_drawer_toggles_and_resizes_beside_the_editor(cx: &mut TestAppContext) {
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
        assert!(
            window.try_find(RIGHT).is_none(),
            "hidden and empty by default"
        );

        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        assert!(shell.views(Side::Right, cx).is_empty());
        assert!(shell.shown(Side::Right, cx));
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(window.find(empty_slot_id(Side::Right)).label(), Some(HINT));
        assert!(close(
            window.find(empty_slot_id(Side::Right)).bounds().size.height,
            HEIGHT
        ));
        let editor = window.find("editor-panel").bounds();
        let right = window.find(RIGHT).bounds();
        assert!(close(right.left(), f32::from(editor.right())));
        assert!(close(right.right(), 1000.));
        assert!(close(right.size.width, DEFAULT_DRAWER_WIDTH as f32));
        // The left drawer is untouched.
        assert!(close(
            window.find(LEFT).bounds().size.width,
            DEFAULT_DRAWER_WIDTH as f32
        ));
        assert!(saved_placed(&path, Side::Right).is_empty());
        assert!(saved(&path).drawer_visible(Side::Right));
        // Typing still goes to the note.
        window.input("x", cx);
        assert_eq!(shell.text(cx), "xabc");

        // Its divider is on its leading edge: dragging left widens it.
        let from = window.find(RIGHT_DIVIDER).bounds().center();
        window.drag(from, point(from.x - px(100.), from.y), cx);
        window.render_frame(cx);
        assert!(close(window.find(RIGHT).bounds().size.width, 360.));
        assert_eq!(saved(&path).drawer_width(Side::Right), 360.0);
        // Clamped so the editor keeps 20rem beside both drawers:
        // 1000 - 48 rail - 260 left - 320 editor = 372.
        let from = window.find(RIGHT_DIVIDER).bounds().center();
        window.drag(from, point(px(10.), from.y), cx);
        window.render_frame(cx);
        assert!(close(window.find(RIGHT).bounds().size.width, 372.));
        assert!(close(window.find("editor-panel").bounds().size.width, 320.));

        // Hidden again: the editor gets its width back.
        window.press("ctrl-shift-t", cx);
        window.render_frame(cx);
        assert!(window.try_find(RIGHT).is_none());
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert!(close(window.find("editor-panel").bounds().right(), 1000.));
        assert!(!saved(&path).drawer_visible(Side::Right));

        // Back, still empty; a view moved in fills it and has the keyboard.
        window.press("ctrl-shift-t", cx);
        assert!(shell.views(Side::Right, cx).is_empty());
        window.press("alt-left", cx);
        window.press("alt-shift-right", cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Files]);
        assert_eq!(shell.region(cx), slot(Side::Right, 0));
        assert!(window.try_find(empty_slot_id(Side::Right)).is_none());
    })
    .unwrap();

    // Saved visible and empty, it reopens so.
    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Files]);
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
        assert_eq!(window.find(empty_slot_id(Side::Right)).label(), Some(HINT));
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
            window.find(empty_slot_id(Side::Right)).label(),
            Some("No view here. Pick one from the rail.")
        );
        assert_eq!(
            shell.workspace.read(cx).leader_chord_label().as_ref(),
            "Leader"
        );
    })
    .unwrap();
}

/// A drawer toggled open empty shows the hint in its whole height, survives
/// a reopen, and the move chords from a drawer's view fill it.
#[gpui_kit::test]
fn an_empty_drawer_reopens_empty(cx: &mut TestAppContext) {
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
        window.press("ctrl-shift-t", cx);
        assert!(saved(&path).drawer_visible(Side::Right));
    })
    .unwrap();
    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(shell.shown(Side::Right, cx));
        assert_eq!(window.find(empty_slot_id(Side::Right)).label(), Some(HINT));
        assert_eq!(shell.focused(cx), Panel::Editor);
        // Focus cycling skips it: nothing to focus there.
        window.press("alt-right", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
    })
    .unwrap();
}

/// The move chords carry the focused view to the other drawer — moved, never
/// copied — and the keyboard goes with it; a drawer left empty hides, and
/// the chord brings the view back. From the rail the move is spatial: the
/// rail is at the left edge, so Right moves the cursor's view into the left
/// drawer and Left is not handled; the rail keeps the keyboard. Every move
/// is saved and restored.
#[gpui_kit::test]
fn views_move_between_drawers_and_persist(cx: &mut TestAppContext) {
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
        assert_eq!(shell.region(cx), slot(Side::Left, 0));

        window.press("alt-shift-right", cx);
        window.render_frame(cx);
        assert!(shell.views(Side::Left, cx).is_empty());
        assert!(!shell.shown(Side::Left, cx));
        assert!(window.try_find(LEFT).is_none());
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Files]);
        assert_eq!(shell.region(cx), slot(Side::Right, 0));
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
        let s = saved(&path);
        assert!(saved_placed(&path, Side::Left).is_empty());
        assert_eq!(saved_placed(&path, Side::Right), ["files"]);
        assert!(!s.drawer_visible(Side::Left));
        assert!(s.drawer_visible(Side::Right));

        // Moving to where it already is changes nothing.
        window.press("alt-shift-right", cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Files]);

        // And back.
        window.press("alt-shift-left", cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert!(shell.views(Side::Right, cx).is_empty());
        assert!(window.try_find(RIGHT).is_none());
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(saved_placed(&path, Side::Left), ["files"]);
        assert!(saved_placed(&path, Side::Right).is_empty());

        // Files to the right again, for the rail moves below.
        window.press("alt-shift-right", cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Files]);

        // From the rail: Right moves the cursor's view (placed nowhere)
        // into the drawer beside the rail, the left one; the rail keeps the
        // keyboard.
        window.click("rail", cx);
        for _ in 0..4 {
            window.press("down", cx);
        }
        window.press("alt-shift-right", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Outline]);
        assert!(shell.shown(Side::Left, cx));
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Files]);
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert_eq!(window.find("rail").focused(), Some(true));
        // A view shown on the right comes over to the left too.
        for _ in 0..4 {
            window.press("up", cx);
        }
        window.press("alt-shift-right", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert!(!shell.shown(Side::Right, cx));
        assert_eq!(shell.focused(cx), Panel::Rail);
        // The editor keeps Alt+Shift+Arrow: no move from there.
        window.click("editor-panel", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.press("alt-shift-right", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
    })
    .unwrap();

    // A reopen restores both drawers.
    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(LEFT).label(), Some("Files"));
        assert!(window.try_find(RIGHT).is_none());
        assert_eq!(shell.focused(cx), Panel::Editor);
    })
    .unwrap();
}

/// From the rail, Alt+Shift+Left points at the window's edge: nothing is
/// there, so the key is not handled and nothing moves or is saved.
#[gpui_kit::test]
fn the_rail_move_toward_the_edge_passes_on(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &gui("[gui]
drawer_right = [\"outline\"]\ndrawer_right_visible = true\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
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
            assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
            assert_eq!(shell.views(Side::Right, cx), [DrawerView::Outline]);
            assert_eq!(shell.focused(cx), Panel::Rail);
        }
        assert!(!path.exists() || saved_placed(&path, Side::Right) == ["outline"]);
    })
    .unwrap();
}

/// The split chord opens an empty slot under the focused drawer's view, at
/// half the height, with the hint; the keyboard stays on the view and
/// nothing is saved. A second split is refused, and says so. Unsplit closes
/// the empty slot; on two views it merges the focused slot into the other
/// (its views stay placed, sharing that slot); a drawer's only slot stays.
#[gpui_kit::test]
fn a_drawer_splits_into_two_slots_and_back(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // Merge Find away first: one slot, Find hidden in it.
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-u", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        let before = std::fs::read_to_string(&path).unwrap();
        // Not from the editor: the chord is the editor's there.
        window.click("editor-panel", cx);
        window.press("alt-shift-s", cx);
        assert_eq!(shell.workspace.read(cx).layout.empty_slot(Side::Left), None);

        window.press("alt-left", cx);
        window.press("alt-shift-s", cx);
        window.render_frame(cx);
        // An empty slot below, not the hidden Find.
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        let top = window.find(slot_id(DrawerView::Files)).bounds();
        let bottom = window.find(empty_slot_id(Side::Left)).bounds();
        assert_eq!(window.find(empty_slot_id(Side::Left)).label(), Some(HINT));
        assert!(close(top.top(), 0.));
        assert!(close(top.size.height, HEIGHT / 2.));
        assert!(close(bottom.top(), f32::from(top.bottom())));
        assert!(close(bottom.bottom(), HEIGHT));
        assert!(window.try_find(split_divider_id(Side::Left)).is_some());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "session only"
        );

        // Full.
        window.press("alt-shift-s", cx);
        assert_eq!(
            shell.flash(cx).as_deref(),
            Some("This drawer already holds two views")
        );

        // Unsplit closes the empty slot.
        window.press("alt-shift-u", cx);
        window.render_frame(cx);
        assert!(window.try_find(empty_slot_id(Side::Left)).is_none());
        assert!(window.try_find(split_divider_id(Side::Left)).is_none());
        assert!(close(
            window.find(slot_id(DrawerView::Files)).bounds().size.height,
            HEIGHT
        ));
        assert_eq!(shell.region(cx), slot(Side::Left, 0));

        // Two views: unsplit merges.
        window.press("alt-shift-s", cx);
        window.click("rail", cx);
        window.press("down", cx);
        window.press("alt-shift-down", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-u", cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(saved_shown(&path, Side::Left), [DrawerView::Files]);
        let back = crate::shell_layout::restore(&saved(&path));
        for view in [DrawerView::Files, DrawerView::Find] {
            assert_eq!(back.placement(&view), Some((Side::Left, 0)), "{view:?}");
        }

        window.press("alt-shift-u", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert!(shell.shown(Side::Left, cx));
        assert_eq!(
            shell.flash(cx).as_deref(),
            Some("This drawer holds one view")
        );
    })
    .unwrap();
}

/// The swap chords on a drawer's view move it into the drawer's empty
/// slot, which takes the slot the view left; the keyboard stays on the
/// view. A view moved from the other drawer takes the empty slot first.
#[gpui_kit::test]
fn views_move_into_an_empty_slot(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &gui("[gui]\ndrawer_left = [\"files\"]\n\
             drawer_right = [\"outline\"]\ndrawer_right_visible = true\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-s", cx);
        // Up: already on top, nothing moves, but the key is handled.
        window.press("alt-shift-up", cx);
        assert_eq!(
            shell.workspace.read(cx).layout.slots(Side::Left),
            [Some(DrawerView::Files), None]
        );
        // Down: Files goes below, the empty slot moves up.
        window.press("alt-shift-down", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.workspace.read(cx).layout.slots(Side::Left),
            [None, Some(DrawerView::Files)]
        );
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        let empty = window.find(empty_slot_id(Side::Left)).bounds();
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        assert!(close(empty.top(), 0.));
        assert!(close(files.top(), f32::from(empty.bottom())));
        assert_eq!(saved_placed(&path, Side::Left), ["files"]);
        assert_eq!(
            crate::shell_layout::restore(&saved(&path)).placement(&DrawerView::Files),
            Some((Side::Left, 1))
        );

        // Outline, from the right, lands in the empty top slot.
        window.click(slot_id(DrawerView::Outline), cx);
        window.press("alt-shift-left", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Outline, DrawerView::Files]
        );
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert!(window.try_find(empty_slot_id(Side::Left)).is_none());
        assert!(!shell.shown(Side::Right, cx));
    })
    .unwrap();
}

/// With the rail cursor on a view no drawer shows, the up/down chords put
/// it into the empty slot (the left drawer's first), the rail keeping the
/// keyboard. A rail click never fills it: the clicked view takes the shown
/// slot. Without an empty slot the chords pass on.
#[gpui_kit::test]
fn the_rail_cursor_moves_a_view_into_the_empty_slot(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("alt-left", cx);
        window.press("alt-shift-s", cx);
        // A rail click on Tags: it replaces Files, the empty slot stays.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(
            shell.workspace.read(cx).layout.slots(Side::Left),
            [Some(DrawerView::Tags), None]
        );
        // The cursor on Links: Alt+Shift+Down moves it into the empty slot.
        window.click("rail", cx);
        ws_cursor(&shell, cx, DrawerView::Links);
        window.press("alt-shift-down", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Tags, DrawerView::Links]
        );
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert_eq!(window.find("rail").focused(), Some(true));
        assert_eq!(
            saved_shown(&path, Side::Left),
            [DrawerView::Tags, DrawerView::Links]
        );
        // No empty slot left: a hidden view's chord passes on.
        ws_cursor(&shell, cx, DrawerView::Outline);
        let key = crate::keymap::parse_chord("alt+shift+up", crate::keymap::Platform::LinuxWindows)
            .unwrap();
        let ws = shell.workspace.clone();
        let handled = ws.update(cx, |workspace, cx| {
            workspace.workspace_command(&ActionShortcuts::MoveViewUp, Some(key), window, cx)
        });
        assert!(!handled);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Tags, DrawerView::Links]
        );
    })
    .unwrap();
}

/// Point the rail's cursor at `view`.
fn ws_cursor(shell: &Shell, cx: &mut gpui_kit::App, view: DrawerView) {
    let i = crate::shell_layout::RAIL_VIEWS
        .iter()
        .position(|v| *v == view)
        .unwrap();
    shell
        .workspace
        .update(cx, |workspace, _| workspace.rail_cursor = i);
}

/// The divider between two slots drags the split, clamped so each slot
/// keeps a tenth of the drawer, and the ratio is saved when the drag ends.
#[gpui_kit::test]
fn the_split_divider_drags_clamps_and_saves(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    let divider = split_divider_id(Side::Left);
    cx.update_window(shell.handle, |_, window, cx| {
        let top = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.render_frame(cx);
            f32::from(window.find(slot_id(DrawerView::Files)).bounds().size.height)
        };
        assert!(close(px(top(window, cx)), HEIGHT / 2.));

        // 96 px down: the boundary follows the pointer's travel.
        let from = window.find(divider).bounds().center();
        window.drag(from, point(from.x, from.y + px(96.)), cx);
        assert!(close(px(top(window, cx)), HEIGHT / 2. + 96.));
        assert_eq!(saved(&path).split_ratio(Side::Left), 0.7);
        assert!(!shell.workspace.read(cx).resizing);

        // Past either end: clamped at a tenth.
        let from = window.find(divider).bounds().center();
        window.drag(from, point(from.x, px(HEIGHT + 40.)), cx);
        assert!(close(px(top(window, cx)), HEIGHT * 0.9));
        let from = window.find(divider).bounds().center();
        window.drag(from, point(from.x, px(0.)), cx);
        assert!(close(px(top(window, cx)), HEIGHT * 0.1));
        assert_eq!(saved(&path).split_ratio(Side::Left), 0.1);
        // The other drawer's split is not this one's.
        assert_eq!(
            shell.workspace.read(cx).layout.split_ratio(Side::Right),
            None
        );
    })
    .unwrap();
}

/// Alt+Arrow moves focus geometrically over both drawers, their stacked
/// slots and the editor: no wrap, hidden drawers skipped, Up/Down only
/// between stacked slots. GPUI focus follows the model's.
#[gpui_kit::test]
fn directional_focus_crosses_drawers_and_slots(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1200.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n\
             drawer_right = [\"outline\"]\ndrawer_right_visible = true\n"),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let editor = shell.region(cx);
        assert!(matches!(editor, Region::Group(_)));
        let files = slot_id(DrawerView::Files);
        let find = slot_id(DrawerView::Find);
        let outline = slot_id(DrawerView::Outline);
        let (editor_id, rail) = (SharedString::from("editor"), SharedString::from("rail"));
        let mut step = |key: &str, expected: Region, focused: &SharedString| {
            window.press(key, cx);
            assert_eq!(shell.region(cx), expected, "{key}");
            assert_eq!(window.find(focused.clone()).focused(), Some(true), "{key}");
        };

        step("alt-right", slot(Side::Right, 0), &outline);
        // The right edge, and nothing stacked: no move.
        step("alt-right", slot(Side::Right, 0), &outline);
        step("alt-up", slot(Side::Right, 0), &outline);
        step("alt-down", slot(Side::Right, 0), &outline);
        step("alt-left", editor, &editor_id);
        step("alt-left", slot(Side::Left, 0), &files);
        step("alt-down", slot(Side::Left, 1), &find);
        // The bottom of the stack: handled, no wrap.
        step("alt-down", slot(Side::Left, 1), &find);
        step("alt-right", editor, &editor_id);
        // Back into the drawer: the slot it last had.
        step("alt-left", slot(Side::Left, 1), &find);
        step("alt-up", slot(Side::Left, 0), &files);
        step("alt-up", slot(Side::Left, 0), &files);
        step("alt-left", Region::Rail, &rail);
        step("alt-left", Region::Rail, &rail);
        step("alt-right", slot(Side::Left, 0), &files);
        step("alt-right", editor, &editor_id);
        // A hidden drawer is skipped.
        step("ctrl-shift-t", editor, &editor_id);
        step("alt-right", editor, &editor_id);
        step("ctrl-t", editor, &editor_id);
        step("alt-left", Region::Rail, &rail);
    })
    .unwrap();
}

/// The rail opens each view in the drawer where it is placed: a view placed
/// on the right opens there; one placed nowhere opens on the left. Its cell is selected while its drawer is shown, and clicking the
/// cell of a drawer's only shown view hides that drawer.
#[gpui_kit::test]
fn the_rail_opens_a_view_where_it_lives(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &gui("[gui]\ndrawer_right = [\"outline\"]\ndrawer_right_visible = false\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let selected = |window: &gpui_kit::Window, view: DrawerView| {
            window.find(rail_cell_id(view)).selected()
        };
        assert_eq!(selected(window, DrawerView::Outline), Some(false));

        window.click(rail_cell_id(DrawerView::Outline), cx);
        window.render_frame(cx);
        assert!(shell.shown(Side::Right, cx));
        assert_eq!(shell.region(cx), slot(Side::Right, 0));
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(selected(window, DrawerView::Outline), Some(true));
        assert_eq!(selected(window, DrawerView::Files), Some(true));
        assert!(saved(&path).drawer_visible(Side::Right));

        // Tags is placed nowhere: it opens on the left, in the left
        // drawer's focused slot.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Tags]);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Outline]);
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(selected(window, DrawerView::Files), Some(false));

        // Moved right, Tags now lives there, at the top position it had:
        // it takes Outline's turn, and after Outline's it comes back there.
        window.press("alt-shift-right", cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Tags]);
        assert!(!shell.shown(Side::Left, cx));
        window.click(rail_cell_id(DrawerView::Outline), cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Outline]);
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Tags]);
        // Find is placed nowhere: the left drawer, showing nothing.
        window.click(rail_cell_id(DrawerView::Find), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        assert!(shell.shown(Side::Left, cx));

        // A shown view's cell hides its drawer when it is the drawer's
        // only view; the other drawer stays.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        window.render_frame(cx);
        assert!(window.try_find(RIGHT).is_none());
        assert!(window.try_find(LEFT).is_some());
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert_eq!(selected(window, DrawerView::Tags), Some(false));
        assert!(!saved(&path).drawer_visible(Side::Right));
    })
    .unwrap();
}

/// A `[gui]` table with both drawers, a split and widths opens as written;
/// one full of stale or wrong values opens on safe defaults.
#[gpui_kit::test]
fn the_gui_table_is_restored_and_bad_values_fall_back(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1200.,
        &gui(
            "[gui]\ndrawer_left = [\"files\", \"tags\"]\ndrawer_left_split = 0.3\n\
              drawer_left_width = 280\ndrawer_right = [\"outline\"]\n\
              drawer_right_width = 300\ndrawer_right_visible = true\n",
        ),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(close(window.find(LEFT).bounds().size.width, 280.));
        assert!(close(window.find(RIGHT).bounds().size.width, 300.));
        assert!(close(window.find(RIGHT).bounds().right(), 1200.));
        let top = window.find(slot_id(DrawerView::Files)).bounds();
        assert!(close(top.size.height, HEIGHT * 0.3));
        let tags = window.find(slot_id(DrawerView::Tags)).bounds();
        assert!(close(tags.top(), HEIGHT * 0.3));
        assert_eq!(shell.focused(cx), Panel::Editor);
    })
    .unwrap();

    let shell = open_shell(
        cx,
        1000.,
        &gui(
            "[gui]\ndrawer_left = [\"nope\", \"files\", \"files\", \"ask\", \"links\", \"tags\"]\n\
              drawer_left_split = 7\ndrawer_left_width = -3\n\
              drawer_right = [\"links\", 5]\ndrawer_right_width = \"wide\"\n\
              drawer_right_visible = \"yes\"\n",
        ),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // Unknown and off-rail ids skipped, duplicates and the third view
        // dropped; the right drawer's only view is already on the left.
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Links]
        );
        assert!(shell.views(Side::Right, cx).is_empty());
        assert!(window.try_find(RIGHT).is_none());
        assert!(close(
            window.find(LEFT).bounds().size.width,
            DEFAULT_DRAWER_WIDTH as f32
        ));
        assert!(close(
            window.find(slot_id(DrawerView::Files)).bounds().size.height,
            HEIGHT / 2.
        ));
        assert!(window.try_find(drawer_id(Side::Left)).is_some());
    })
    .unwrap();
}

/// The swap chords exchange the focused view with the slot above or below
/// it in a split drawer, the keyboard following the view; at the end of the
/// stack nothing moves. From the rail they swap the cursor's view. The new order is saved and restored.
#[gpui_kit::test]
fn the_swap_chords_reorder_a_split_drawer(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\ndrawer_left_split = 0.3\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // Not from the editor: the chord is the editor's there.
        window.press("alt-shift-down", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );

        window.click(slot_id(DrawerView::Files), cx);
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        // The top of the stack: nothing moves.
        window.press("alt-shift-up", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );

        window.press("alt-shift-down", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Find, DrawerView::Files]
        );
        assert_eq!(shell.region(cx), slot(Side::Left, 1), "focus follows");
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        // Each slot keeps its size: Find is now the 30% top slot.
        let top = window.find(slot_id(DrawerView::Find)).bounds();
        assert!(close(top.top(), 0.));
        assert!(close(top.size.height, HEIGHT * 0.3));
        let s = saved(&path);
        assert_eq!(saved_placed(&path, Side::Left), ["find", "files"]);
        assert_eq!(s.split_ratio(Side::Left), 0.3);

        // The bottom: nothing moves.
        window.press("alt-shift-down", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Find, DrawerView::Files]
        );
        window.press("alt-shift-up", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(saved_placed(&path, Side::Left), ["files", "find"]);
        window.press("alt-shift-down", cx);
        assert_eq!(saved_placed(&path, Side::Left), ["find", "files"]);

        // From the rail it swaps the cursor's view (Files), and the rail
        // keeps the keyboard.
        window.click("rail", cx);
        window.press("alt-shift-up", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(shell.focused(cx), Panel::Rail);
        // A cursor on a view in no slot swaps nothing.
        window.press("down", cx);
        window.press("down", cx);
        window.press("alt-shift-down", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        // Back to Find's order for the reopen below.
        window.press("up", cx);
        window.press("alt-shift-up", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Find, DrawerView::Files]
        );
    })
    .unwrap();

    // A reopen restores the new order.
    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(LEFT).label(), Some("Find, Files"));
    })
    .unwrap();
}

/// In an unsplit drawer the swap chords do nothing and are not handled: the
/// key goes on past the workspace, and nothing changes on screen, in the
/// model or in the saved file. Also after a plain hide left one view of a
/// split shown, and from the rail.
#[gpui_kit::test]
fn the_swap_chords_pass_on_in_an_unsplit_drawer(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let ws = shell.workspace.clone();
        let key = crate::keymap::parse_chord("alt+shift+up", crate::keymap::Platform::LinuxWindows)
            .unwrap();
        // Hide Files: Find, at the bottom position, fills the drawer.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.click(slot_id(DrawerView::Find), cx);
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        let before = std::fs::read_to_string(&path).unwrap();
        let placements = |cx: &mut gpui_kit::App| {
            let layout = &ws.read(cx).layout;
            (
                layout.placement(&DrawerView::Files),
                layout.placement(&DrawerView::Find),
            )
        };
        let at_start = placements(cx);
        for from_rail in [false, true] {
            if from_rail {
                window.click("rail", cx);
            }
            for action in [ActionShortcuts::MoveViewUp, ActionShortcuts::MoveViewDown] {
                let handled = ws.update(cx, |workspace, cx| {
                    workspace.workspace_command(&action, Some(key), window, cx)
                });
                assert!(!handled, "{action:?} passes on (rail: {from_rail})");
                assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
                assert_eq!(placements(cx), at_start);
            }
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(at_start, (Some((Side::Left, 0)), Some((Side::Left, 1))));
    })
    .unwrap();
}

/// Clicking the rail cell of a shown view in a split drawer hides only that
/// view: its slot closes, the other view fills the drawer and takes the
/// keyboard. Clicking a drawer's last view hides the drawer. A view hidden
/// from the right drawer comes back there, at its slot position.
#[gpui_kit::test]
fn the_rail_hides_one_view_of_a_split_drawer(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n\
             drawer_right = [\"outline\", \"tags\"]\ndrawer_right_visible = true\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let selected = |window: &gpui_kit::Window, view: DrawerView| {
            window.find(rail_cell_id(view)).selected()
        };

        // Files is the top of the left stack: Find fills the drawer.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        assert!(shell.shown(Side::Left, cx));
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(window.find(slot_id(DrawerView::Find)).focused(), Some(true));
        assert!(close(
            window.find(slot_id(DrawerView::Find)).bounds().size.height,
            HEIGHT
        ));
        assert!(window.try_find(split_divider_id(Side::Left)).is_none());
        assert_eq!(selected(window, DrawerView::Files), Some(false));
        assert_eq!(selected(window, DrawerView::Find), Some(true));
        assert_eq!(saved_shown(&path, Side::Left), [DrawerView::Find]);

        // Tags, the bottom of the right stack: Outline stays, focused.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Outline]);
        assert!(shell.shown(Side::Right, cx));
        assert_eq!(shell.region(cx), slot(Side::Right, 0));
        assert_eq!(
            window.find(slot_id(DrawerView::Outline)).focused(),
            Some(true)
        );
        assert_eq!(saved_shown(&path, Side::Right), [DrawerView::Outline]);

        // Find is now the left drawer's last view: the drawer hides.
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.render_frame(cx);
        assert!(window.try_find(LEFT).is_none());
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        assert!(!saved(&path).drawer_visible(Side::Left));

        // Tags comes back where it is placed: the right, below Outline.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        window.render_frame(cx);
        assert_eq!(
            shell.views(Side::Right, cx),
            [DrawerView::Outline, DrawerView::Tags]
        );
        assert_eq!(shell.region(cx), slot(Side::Right, 1));
        assert!(!shell.shown(Side::Left, cx));
    })
    .unwrap();
}

/// Hiding one view of a split drawer keeps its placement: showing it again
/// from the rail puts it back at its slot position (above or below) at the
/// drawer's ratio, the other view untouched, and the keyboard on it. In
/// both drawers, and across a reopen.
#[gpui_kit::test]
fn showing_a_hidden_view_restores_the_split(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &gui(
            "[gui]\ndrawer_left = [\"files\", \"find\"]\ndrawer_left_split = 0.3\n\
             drawer_right = [\"outline\", \"tags\"]\ndrawer_right_split = 0.6\n\
             drawer_right_visible = true\n",
        ),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let top_height = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, view| {
            window.render_frame(cx);
            f32::from(window.find(slot_id(view)).bounds().size.height)
        };

        // Left, the top view: hidden, then back on top at 30%.
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        assert_eq!(saved_shown(&path, Side::Left), [DrawerView::Find]);
        assert_eq!(saved_placed(&path, Side::Left), ["files", "find"]);
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert!(close(
            px(top_height(window, cx, DrawerView::Files)),
            HEIGHT * 0.3
        ));
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );
        assert_eq!(
            saved_shown(&path, Side::Left),
            [DrawerView::Files, DrawerView::Find]
        );

        // Right, the bottom view: back below Outline at 60%.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Outline]);
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(
            shell.views(Side::Right, cx),
            [DrawerView::Outline, DrawerView::Tags]
        );
        assert!(close(
            px(top_height(window, cx, DrawerView::Outline)),
            HEIGHT * 0.6
        ));
        assert_eq!(shell.region(cx), slot(Side::Right, 1));
        assert_eq!(window.find(slot_id(DrawerView::Tags)).focused(), Some(true));
        // The left drawer was not touched.
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );

        // Hidden again, in both drawers, for the reopen below.
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.click(rail_cell_id(DrawerView::Outline), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Tags]);
    })
    .unwrap();

    // Placements are saved: after a reopen, the lone bottom view (Tags)
    // still sits below, and showing the others restores both splits.
    let shell = open_shell(cx, 1200., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        window.click(rail_cell_id(DrawerView::Find), cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(shell.region(cx), slot(Side::Left, 1));
        window.click(rail_cell_id(DrawerView::Outline), cx);
        assert_eq!(
            shell.views(Side::Right, cx),
            [DrawerView::Outline, DrawerView::Tags]
        );
        assert_eq!(shell.region(cx), slot(Side::Right, 0));
        window.render_frame(cx);
        let outline = window.find(slot_id(DrawerView::Outline)).bounds();
        assert!(close(outline.size.height, HEIGHT * 0.6));
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        assert!(close(files.size.height, HEIGHT * 0.3));
    })
    .unwrap();
}

/// Views placed at the same slot position take turns: showing one hides
/// the other, which stays placed and comes back when shown. Placements
/// outlive a view shown in place of another and a split; an unsplit merges
/// the closed slot's views into the other one.
#[gpui_kit::test]
fn views_sharing_a_slot_position_take_turns(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n\
             drawer_right = [\"outline\", \"tags\"]\ndrawer_right_visible = true\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);

        // Left: hide Files; Links takes Find's (bottom) slot; Files then
        // comes back on top, and Find takes its turn below.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.click(rail_cell_id(DrawerView::Links), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Links]);
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Links]
        );
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        window.click(rail_cell_id(DrawerView::Find), cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(shell.region(cx), slot(Side::Left, 1));
        assert_eq!(saved_placed(&path, Side::Left), ["files", "find", "links"]);
        assert_eq!(
            saved_shown(&path, Side::Left),
            [DrawerView::Files, DrawerView::Find]
        );

        // Right: hide Tags (Outline fills the drawer, no empty slot); the
        // split opens the free bottom position empty, not with Tags; Tags'
        // cell then never fills it: Tags takes Outline's shown slot.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Outline]);
        assert_eq!(
            shell.workspace.read(cx).layout.empty_slot(Side::Right),
            None
        );
        assert_eq!(shell.region(cx), slot(Side::Right, 0));
        window.press("alt-shift-s", cx);
        assert_eq!(
            shell.workspace.read(cx).layout.slots(Side::Right),
            [Some(DrawerView::Outline), None]
        );
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(
            shell.workspace.read(cx).layout.slots(Side::Right),
            [Some(DrawerView::Tags), None]
        );
        assert_eq!(shell.region(cx), slot(Side::Right, 0));
    })
    .unwrap();

    // A single-view drawer: two views at its one position toggle.
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
        for view in [DrawerView::Tags, DrawerView::Files, DrawerView::Tags] {
            window.click(rail_cell_id(view), cx);
            assert_eq!(shell.views(Side::Left, cx), [view]);
            assert!(shell.shown(Side::Left, cx));
        }
    })
    .unwrap();
}

/// The hidden view outlives hiding the whole drawer: toggled off and on, or
/// closed by hiding its other view too, the drawer still restores the split
/// when the hidden view is shown again, also after a reopen.
#[gpui_kit::test]
fn a_hidden_view_survives_hiding_the_drawer(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\ndrawer_left_split = 0.3\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    let files_height = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        f32::from(window.find(slot_id(DrawerView::Files)).bounds().size.height)
    };
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);

        // Hide Files, toggle the drawer off and on: Find alone, then Files
        // comes back on top at 30%, focused.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.press("ctrl-t", cx);
        assert!(!shell.shown(Side::Left, cx));
        assert_eq!(saved_placed(&path, Side::Left), ["files", "find"]);
        window.press("ctrl-t", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert!(close(px(files_height(window, cx)), HEIGHT * 0.3));
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        assert_eq!(
            window.find(slot_id(DrawerView::Files)).focused(),
            Some(true)
        );

        // Hide Files, then Find (the drawer closes). Find's cell brings
        // Find back alone; Files' cell then restores the split.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.click(rail_cell_id(DrawerView::Find), cx);
        assert!(!shell.shown(Side::Left, cx));
        window.click(rail_cell_id(DrawerView::Find), cx);
        assert!(shell.shown(Side::Left, cx));
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert!(close(px(files_height(window, cx)), HEIGHT * 0.3));

        // Hidden again, drawer and all, for the reopen.
        window.click(rail_cell_id(DrawerView::Files), cx);
        window.click(rail_cell_id(DrawerView::Find), cx);
        assert!(!saved(&path).drawer_visible(Side::Left));
        assert_eq!(saved_shown(&path, Side::Left), [DrawerView::Find]);
    })
    .unwrap();

    // After a reopen the drawer is hidden; Files' cell shows it split,
    // Files above Find.
    let shell = open_shell(cx, 1200., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!shell.shown(Side::Left, cx));
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert!(shell.shown(Side::Left, cx));
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        assert!(close(px(files_height(window, cx)), HEIGHT * 0.3));
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
    })
    .unwrap();
}

/// Moving a view to the other drawer fits the target: into a split drawer
/// it keeps its slot position, replacing the view shown there (which stays
/// placed); into an unsplit or empty drawer it joins the single slot and
/// replaces the shown view — never a split. Saved.
#[gpui_kit::test]
fn moving_fits_the_target_drawer(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1200.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n\
             drawer_right = [\"outline\", \"tags\"]\ndrawer_right_visible = true\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // Split target: Find (bottom) replaces Tags at the bottom.
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-right", cx);
        assert_eq!(
            shell.views(Side::Right, cx),
            [DrawerView::Outline, DrawerView::Find]
        );
        assert_eq!(shell.region(cx), slot(Side::Right, 1));
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        // Tags stays placed at the bottom of the right drawer.
        window.click(rail_cell_id(DrawerView::Tags), cx);
        assert_eq!(
            shell.views(Side::Right, cx),
            [DrawerView::Outline, DrawerView::Tags]
        );

        // Unsplit target: Outline (top) moved left replaces Files there,
        // in one slot; Files' cell swaps back, still one slot.
        window.click(slot_id(DrawerView::Outline), cx);
        window.press("alt-shift-left", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Outline]);
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        assert_eq!(saved_shown(&path, Side::Left), [DrawerView::Files]);

        // Tags, now the right drawer's only shown view, moved left joins
        // Files' single slot too; the right drawer, showing nothing (Find
        // hidden there), hides.
        window.click(slot_id(DrawerView::Tags), cx);
        window.press("alt-shift-left", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Tags]);
        assert!(shell.views(Side::Right, cx).is_empty());
        assert!(!shell.shown(Side::Right, cx));

        // Empty target: Files moved right lands alone in its top slot.
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        window.press("alt-shift-right", cx);
        assert_eq!(shell.views(Side::Right, cx), [DrawerView::Files]);
        assert!(shell.shown(Side::Right, cx));
        assert_eq!(
            shell
                .workspace
                .read(cx)
                .layout
                .placement(&DrawerView::Files),
            Some((Side::Right, 0))
        );
        assert_eq!(saved_shown(&path, Side::Right), [DrawerView::Files]);
    })
    .unwrap();
}

/// Unsplit, then the rail cell of the merged view: it replaces the shown
/// view, never re-splits. Unsplit then Split opens an empty slot at the
/// drawer's ratio, not the merged view.
#[gpui_kit::test]
fn unsplit_merges_and_split_brings_the_pair_back(cx: &mut TestAppContext) {
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
        window.press("alt-shift-s", cx);
        window.click("rail", cx);
        ws_cursor(&shell, cx, DrawerView::Find);
        window.press("alt-shift-down", cx);
        assert_eq!(
            shell.views(Side::Left, cx),
            [DrawerView::Files, DrawerView::Find]
        );
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-u", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        assert!(window.try_find(split_divider_id(Side::Left)).is_none());
        assert_eq!(saved_shown(&path, Side::Left), [DrawerView::Find]);
        window.click(rail_cell_id(DrawerView::Files), cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
    })
    .unwrap();

    // Unsplit, then Split: an empty slot at the same ratio.
    let shell = open_shell(
        cx,
        1000.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\ndrawer_left_split = 0.3\n"),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // Close the top slot: Files merges below. The split opens the
        // free top position empty.
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-shift-u", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        window.press("alt-shift-s", cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        assert_eq!(shell.region(cx), slot(Side::Left, 0));
        let empty = window.find(empty_slot_id(Side::Left)).bounds();
        assert!(close(empty.size.height, HEIGHT * 0.3));
    })
    .unwrap();
}

/// After an unsplit and a restart the drawer is still unsplit: its views
/// share one slot, and a rail click on the merged view replaces the shown
/// one.
#[gpui_kit::test]
fn an_unsplit_drawer_stays_merged_after_a_reopen(cx: &mut TestAppContext) {
    let (_dir, path) = temp_config();
    let shell = open_shell(
        cx,
        1000.,
        &gui("[gui]\ndrawer_left = [\"files\", \"find\"]\ndrawer_left_split = 0.3\n"),
        "",
        false,
        LayoutStore::File(path.clone()),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Find), cx);
        window.press("alt-shift-u", cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
    })
    .unwrap();

    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Files]);
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.render_frame(cx);
        assert_eq!(shell.views(Side::Left, cx), [DrawerView::Find]);
        assert!(window.try_find(split_divider_id(Side::Left)).is_none());
        assert!(close(
            window.find(slot_id(DrawerView::Find)).bounds().size.height,
            HEIGHT
        ));
    })
    .unwrap();
}
