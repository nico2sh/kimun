//! UI integration tests for **Zoom** (ADR-0053 decisions 2 and 3): the
//! focused slot or editor group takes over everything right of the rail
//! while the footer stays with the restore hint and button; leaving zoom
//! restores the arrangement exactly; Esc never leaves it; it is never
//! saved. A real `Workspace` in a headless window, driven by GPUI keyboard
//! and pointer events.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, Pixels, TestAppContext, Window};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::layout_tree::{Focus, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::GuiSettings;

use kimun_core::nfs::VaultPath;

use super::start_tests::{chords, drive, open_started, runtime, vault_start};
use super::tests::{HEIGHT, Shell, close, gui, open_shell, temp_config};
use super::{LayoutStore, Workspace, slot_id};
use crate::startup::FirstNote;
use crate::status_row;
use crate::test_vault::test_vault;

/// The zoom chord with the default Linux/Windows keymap.
const ZOOM: &str = "ctrl-shift-m";

/// FILES left, the editor, OUTLINE right, at 0.2 / 0.6 / 0.2.
fn files_editor_outline() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n")
}

impl Shell {
    fn zoomed(&self, cx: &gpui_kit::App) -> Option<Leaf> {
        self.workspace.read(cx).layout.zoomed()
    }

    fn tree_focus(&self, cx: &gpui_kit::App) -> Focus {
        self.workspace.read(cx).layout.focused()
    }

    fn slot_of(&self, view: DrawerView, cx: &gpui_kit::App) -> SlotId {
        self.workspace
            .read(cx)
            .layout
            .view_slot(&view)
            .expect("the view is placed")
    }

    fn group(&self, cx: &gpui_kit::App) -> Leaf {
        Leaf::Group(self.workspace.read(cx).layout.active_group())
    }
}

/// Every slot's drawn bounds (by reading order; `None` when not drawn) and
/// the editor Panel's.
fn drawn(
    workspace: &Workspace,
    window: &mut Window,
) -> (Vec<Option<Bounds<Pixels>>>, Option<Bounds<Pixels>>) {
    let slots = workspace
        .layout
        .slots()
        .into_iter()
        .map(|slot| {
            let view = workspace.layout.shown(slot)?;
            window.try_find(slot_id(view)).map(|el| el.bounds())
        })
        .collect();
    let editor = window.try_find("editor-panel").map(|el| el.bounds());
    (slots, editor)
}

/// The zoomed slot fills everything right of the rail down to the footer,
/// which stays, full width, with the restore hint; no other slot, the
/// editor or a divider is drawn. The same for the editor group.
#[gpui_kit::test]
fn toggle_zoom_covers_everything_right_of_the_rail_and_keeps_footer(cx: &mut TestAppContext) {
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
        assert!(window.try_find("divider-slot-0").is_some());
        window.press("alt-left", cx);
        window.press(ZOOM, cx);
        window.render_frame(cx);
        let files_s = shell.slot_of(DrawerView::Files, cx);
        assert_eq!(shell.zoomed(cx), Some(Leaf::Slot(files_s)));

        let rail = window.find("rail").bounds();
        let files = window.find(slot_id(DrawerView::Files)).bounds();
        let footer = window.find(status_row::ROW_ID).bounds();
        assert!(close(files.left(), f32::from(rail.right())), "{files:?}");
        assert!(close(files.right(), 1000.), "{files:?}");
        assert!(close(files.top(), 0.), "{files:?}");
        assert!(close(footer.left(), f32::from(rail.right())), "{footer:?}");
        assert!(close(footer.right(), 1000.), "{footer:?}");
        assert!(close(footer.top(), f32::from(files.bottom())), "{footer:?}");
        assert!(close(footer.bottom(), HEIGHT), "{footer:?}");
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        assert!(window.try_find("editor-panel").is_none());
        assert!(window.try_find("divider-slot-0").is_none());
        assert!(window.try_find("divider-group-0").is_none());
        assert!(window.try_find(status_row::ZOOM_ID).is_some());
        assert!(window.try_find(status_row::ZOOM_RESTORE_ID).is_some());

        // The editor group: the editor Panel, its own footer included.
        window.press(ZOOM, cx);
        window.press("alt-right", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), Some(shell.group(cx)));
        let editor = window.find("editor-panel").bounds();
        assert!(close(editor.left(), f32::from(rail.right())), "{editor:?}");
        assert!(close(editor.right(), 1000.), "{editor:?}");
        assert!(close(editor.size.height, HEIGHT), "{editor:?}");
        assert!(window.try_find(slot_id(DrawerView::Files)).is_none());
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_none());
        let footer = window.find(status_row::ROW_ID).bounds();
        // The editor's frame (margin + border) sits under its Footer.
        assert!(close(footer.bottom(), HEIGHT - 5.), "{footer:?}");
        assert!(window.try_find(status_row::ZOOM_ID).is_some());
    })
    .unwrap();
}

/// Zoom changes nothing in the layout: after leaving it every placement,
/// weight, side memory and drawn size is what it was,
/// and what collapsed in a narrow window collapses again (while zoomed
/// nothing is collapsed).
#[gpui_kit::test]
fn zoom_restores_every_placement_exactly(cx: &mut TestAppContext) {
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.23, split = \"vertical\", children = [\n\
           { weight = 0.4, views = [\"files\", \"find\"], shown = \"files\" },\n\
           { weight = 0.6, views = [\"tags\"], shown = \"tags\" },\n] },\n\
         { weight = 0.55, editor = true },\n\
         { weight = 0.22, views = [\"outline\"], shown = \"outline\" },\n]\n");
    let shell = open_shell(cx, 1200., &layout, "", false, LayoutStore::Off);
    let snapshot = |window: &mut Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        let w = shell.workspace.read(cx);
        (
            w.layout.clone(),
            w.side_memory.clone(),
            w.collapsed.clone(),
            drawn(w, window),
        )
    };
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // TAGS: it overlaps the editor most.
        window.press("alt-left", cx);
        // A side toggled off (its memory).
        window.press("ctrl-shift-t", cx);
        let before = snapshot(window, cx);
        assert_eq!(before.1[1], [DrawerView::Outline]);
        let tags = Leaf::Slot(shell.slot_of(DrawerView::Tags, cx));
        assert_eq!(shell.tree_focus(cx), Focus::Leaf(tags));

        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), Some(tags));
        assert!(window.try_find(slot_id(DrawerView::Files)).is_none());
        assert!(window.try_find(slot_id(DrawerView::Tags)).is_some());
        window.press(ZOOM, cx);
        assert_eq!(snapshot(window, cx), before);
    })
    .unwrap();

    // Narrow: the left column collapses; zoomed, nothing is collapsed.
    cx.simulate_window_resize(
        shell.handle,
        gpui_kit::size(gpui_kit::px(500.), gpui_kit::px(HEIGHT)),
    );
    cx.update_window(shell.handle, |_, window, cx| {
        let before = snapshot(window, cx);
        assert!(!before.2.is_empty(), "something collapsed");
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert!(shell.workspace.read(cx).collapsed.is_empty());
        window.press(ZOOM, cx);
        assert_eq!(snapshot(window, cx), before);
    })
    .unwrap();
}

/// Esc never leaves zoom (ADR-0053 decision 3): not from FILES' filter or
/// OUTLINE's list, whose Esc otherwise hands the keyboard to the editor;
/// the keyboard stays on the zoomed slot. A real vault, so the views have
/// content.
#[gpui_kit::test]
fn escape_does_not_exit_zoom(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let text = "# Alpha\n## Two\nbody";
    let v = rt.block_on(test_vault(
        &[("alpha.md", text), ("beta.md", "# Beta")],
        &[],
    ));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("alpha.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| {
        started.text(cx) == text && !started.indexing(cx)
    });
    let press = |key: &str, cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| {
            window.press(key, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let state = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let layout = &started.workspace.read(cx).layout;
            (layout.zoomed(), layout.focused())
        })
    };
    let slot = |view: DrawerView, cx: &mut TestAppContext| {
        cx.read(|cx| started.workspace.read(cx).layout.view_slot(&view))
            .map(Leaf::Slot)
            .expect("placed")
    };

    // Not zoomed, FILES' Esc returns to the editor (what zoom must stop).
    press("ctrl-e", cx);
    let files = slot(DrawerView::Files, cx);
    assert_eq!(state(cx), (None, Focus::Leaf(files)));
    press("escape", cx);
    assert!(matches!(state(cx).1, Focus::Leaf(Leaf::Group(_))));

    press("ctrl-e", cx);
    press(ZOOM, cx);
    assert_eq!(state(cx), (Some(files), Focus::Leaf(files)));
    press("escape", cx);
    press("escape", cx);
    assert_eq!(state(cx), (Some(files), Focus::Leaf(files)));
    press(ZOOM, cx);
    assert_eq!(state(cx).0, None);

    // OUTLINE's list (leader `o l`).
    press("ctrl-shift-b", cx);
    cx.update_window(started.handle, |_, window, cx| window.input("ol", cx))
        .unwrap();
    cx.run_until_parked();
    let outline = slot(DrawerView::Outline, cx);
    assert_eq!(state(cx), (None, Focus::Leaf(outline)));
    press(ZOOM, cx);
    press("escape", cx);
    assert_eq!(state(cx), (Some(outline), Focus::Leaf(outline)));
    // A leader sequence cancelled with Esc keeps it too.
    press("ctrl-shift-b", cx);
    press("escape", cx);
    assert_eq!(state(cx), (Some(outline), Focus::Leaf(outline)));
}

/// Esc in the vim editor (leaving insert mode, then again in Normal mode)
/// does not leave a zoomed editor group.
#[gpui_kit::test]
fn escape_in_the_vim_editor_does_not_exit_zoom(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &files_editor_outline(),
        "some text",
        true,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.press(ZOOM, cx);
        window.press("i", cx);
        window.press("escape", cx);
        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), Some(shell.group(cx)));
        assert_eq!(shell.focused(cx), Panel::Editor);
    })
    .unwrap();
}

/// The chord zooms the focused leaf, a slot or the editor group, and the
/// same chord leaves zoom; from the rail there is nothing to zoom.
#[gpui_kit::test]
fn zoom_works_on_an_editor_group_and_on_a_slot(cx: &mut TestAppContext) {
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
        assert_eq!(shell.zoomed(cx), None);
        window.press(ZOOM, cx);
        assert_eq!(shell.zoomed(cx), Some(shell.group(cx)));
        window.press(ZOOM, cx);
        assert_eq!(shell.zoomed(cx), None);

        window.press("alt-right", cx);
        let outline = Leaf::Slot(shell.slot_of(DrawerView::Outline, cx));
        assert_eq!(shell.tree_focus(cx), Focus::Leaf(outline));
        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), Some(outline));
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_some());
        assert!(window.try_find("editor-panel").is_none());
        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), None);
        assert!(window.try_find("editor-panel").is_some());

        // The rail: nothing to zoom.
        for _ in 0..3 {
            window.press("alt-left", cx);
        }
        assert_eq!(shell.tree_focus(cx), Focus::Rail);
        window.press(ZOOM, cx);
        assert_eq!(shell.zoomed(cx), None);
    })
    .unwrap();
}

/// Closing the zoomed slot leaves zoom: the remaining layout is drawn.
#[gpui_kit::test]
fn closing_zoomed_slot_clears_zoom(cx: &mut TestAppContext) {
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
        window.press("alt-right", cx);
        window.press(ZOOM, cx);
        assert!(shell.zoomed(cx).is_some());
        window.press("alt-shift-w", cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), None);
        assert_eq!(shell.workspace.read(cx).layout.slots().len(), 1);
        assert!(window.try_find("editor-panel").is_some());
        assert!(window.try_find(status_row::ZOOM_ID).is_none());
    })
    .unwrap();
}

/// A side toggle that hides the zoomed view leaves zoom: the leaf is no
/// longer drawn.
#[gpui_kit::test]
fn hiding_the_zoomed_view_clears_zoom(cx: &mut TestAppContext) {
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
        window.press("alt-left", cx);
        window.press(ZOOM, cx);
        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), None);
        assert!(window.try_find("editor-panel").is_some());
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_some());
    })
    .unwrap();
}

/// The footer names the chord the keymap binds, and follows a replaced
/// keymap; unbound, it says only that the window is zoomed.
#[gpui_kit::test]
fn zoom_hint_names_the_bound_chord(cx: &mut TestAppContext) {
    use crate::keymap::{self, Keymap, Platform};
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
        assert!(window.try_find(status_row::ZOOM_ID).is_none());
        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(status_row::ZOOM_ID).label(),
            Some(chords("Zoomed · Ctrl+Shift+M to restore").as_str())
        );
    })
    .unwrap();
    let replace = |cx: &mut TestAppContext, chords: Vec<String>| {
        cx.update(|cx| {
            let overrides = [("ToggleZoom".to_string(), chords)].into_iter().collect();
            let (map, errors) = Keymap::load(Platform::LinuxWindows, &overrides);
            assert!(errors.is_empty(), "{errors:?}");
            keymap::init(cx, map);
        });
        cx.run_until_parked();
    };
    replace(cx, vec!["alt+z".to_string()]);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(status_row::ZOOM_ID).label(),
            Some(chords("Zoomed · Alt+Z to restore").as_str())
        );
        window.press("alt-z", cx);
        assert_eq!(shell.zoomed(cx), None);
        window.press("alt-z", cx);
    })
    .unwrap();
    replace(cx, vec![]);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(status_row::ZOOM_ID).label(), Some("Zoomed"));
    })
    .unwrap();
}

/// The footer's restore button leaves zoom.
#[gpui_kit::test]
fn the_restore_button_exits_zoom(cx: &mut TestAppContext) {
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
        window.press("alt-left", cx);
        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(status_row::ZOOM_RESTORE_ID).label(),
            Some("Restore")
        );
        window.click(status_row::ZOOM_RESTORE_ID, cx);
        window.render_frame(cx);
        assert_eq!(shell.zoomed(cx), None);
        assert!(window.try_find("editor-panel").is_some());
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_some());
        assert!(window.try_find(status_row::ZOOM_RESTORE_ID).is_none());
    })
    .unwrap();
}

/// Zoom is session only: entering it saves nothing, a save while zoomed
/// writes the same bytes, and a window opened from that file is not zoomed.
#[gpui_kit::test]
fn zoom_is_never_saved(cx: &mut TestAppContext) {
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
        // Twice: the first save of a missing file stamps no config version.
        shell.workspace.update(cx, |ws, cx| ws.save_layout(cx));
        shell.workspace.update(cx, |ws, cx| ws.save_layout(cx));
    })
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    cx.update_window(shell.handle, |_, window, cx| {
        window.press("alt-left", cx);
        window.press(ZOOM, cx);
        window.render_frame(cx);
        assert!(shell.zoomed(cx).is_some());
        assert_eq!(std::fs::read(&path).unwrap(), before, "zooming saved");
        shell.workspace.update(cx, |ws, cx| ws.save_layout(cx));
        assert_eq!(std::fs::read(&path).unwrap(), before, "saved while zoomed");
    })
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("zoom"), "{text}");
    let reopened = open_shell(
        cx,
        1000.,
        &super::tests::saved(&path),
        "",
        false,
        LayoutStore::Off,
    );
    cx.update_window(reopened.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(reopened.zoomed(cx), None);
        assert!(window.try_find("editor-panel").is_some());
    })
    .unwrap();
}

/// While zoomed, directional focus sees only the zoomed leaf and the rail
/// (Ruling 10), and the panel cycle only those two.
#[gpui_kit::test]
fn directional_focus_while_zoomed_only_reaches_the_rail(cx: &mut TestAppContext) {
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
        let editor = Focus::Leaf(shell.group(cx));
        window.press(ZOOM, cx);
        window.render_frame(cx);
        for (key, expected) in [
            ("alt-left", Focus::Rail),
            ("alt-right", editor),
            ("alt-right", editor),
            ("alt-up", editor),
            ("alt-down", editor),
            ("ctrl-tab", Focus::Rail),
            ("ctrl-tab", editor),
            ("ctrl-shift-tab", Focus::Rail),
            ("ctrl-shift-tab", editor),
        ] {
            window.press(key, cx);
            window.render_frame(cx);
            assert_eq!(shell.tree_focus(cx), expected, "{key}");
            assert_eq!(shell.zoomed(cx), Some(shell.group(cx)), "{key}");
        }

        // A zoomed slot at the right edge: Left goes to the rail, not the
        // editor beside it.
        window.press(ZOOM, cx);
        window.press("alt-right", cx);
        let outline = Focus::Leaf(Leaf::Slot(shell.slot_of(DrawerView::Outline, cx)));
        assert_eq!(shell.tree_focus(cx), outline);
        window.press(ZOOM, cx);
        window.press("alt-left", cx);
        assert_eq!(shell.tree_focus(cx), Focus::Rail);
        window.press("alt-right", cx);
        assert_eq!(shell.tree_focus(cx), outline);
    })
    .unwrap();
}

/// A rail click on a view shown in a slot hidden by zoom shows it: zoom
/// ends and that slot takes the keyboard. A view the rail shows into the
/// zoomed slot itself keeps the zoom.
#[gpui_kit::test]
fn a_rail_click_on_a_view_outside_the_zoom_leaves_zoom(cx: &mut TestAppContext) {
    use super::rail_cell_id;
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
        window.press("alt-left", cx);
        window.press(ZOOM, cx);
        let files = Leaf::Slot(shell.slot_of(DrawerView::Files, cx));
        // FIND shares FILES' slot (its home): it shows there, still zoomed.
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.render_frame(cx);
        assert_eq!(
            shell.slot_of(DrawerView::Find, cx),
            shell.slot_of(DrawerView::Files, cx)
        );
        assert_eq!(shell.zoomed(cx), Some(files));
        assert!(window.try_find(slot_id(DrawerView::Find)).is_some());
        // OUTLINE is shown in the right slot, hidden by the zoom.
        window.click(rail_cell_id(DrawerView::Outline), cx);
        window.render_frame(cx);
        let outline = Leaf::Slot(shell.slot_of(DrawerView::Outline, cx));
        assert_eq!(shell.zoomed(cx), None);
        assert_eq!(shell.tree_focus(cx), Focus::Leaf(outline));
        assert!(window.try_find(slot_id(DrawerView::Outline)).is_some());
        assert!(window.try_find("editor-panel").is_some());
    })
    .unwrap();
}

/// A rail click on a view placed but hidden (it shares a slot with the
/// shown one) while another leaf, or the editor group, is zoomed: zoom
/// ends and the view's slot takes the keyboard.
#[gpui_kit::test]
fn a_rail_click_on_a_hidden_placed_view_ends_zoom(cx: &mut TestAppContext) {
    use super::rail_cell_id;
    for zoom_editor in [false, true] {
        let shell = open_shell(
            cx,
            1000.,
            &gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
                 { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
                 { weight = 0.6, editor = true },\n\
                 { weight = 0.2, views = [\"outline\", \"tags\"], shown = \"outline\" },\n]\n"),
            "",
            false,
            LayoutStore::Off,
        );
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            if !zoom_editor {
                window.press("alt-left", cx);
            }
            window.press(ZOOM, cx);
            assert!(shell.zoomed(cx).is_some());
            window.click(rail_cell_id(DrawerView::Tags), cx);
            window.render_frame(cx);
            let tags = Leaf::Slot(shell.slot_of(DrawerView::Tags, cx));
            assert_eq!(shell.zoomed(cx), None, "zoom_editor={zoom_editor}");
            assert_eq!(shell.tree_focus(cx), Focus::Leaf(tags));
        })
        .unwrap();
    }
}
