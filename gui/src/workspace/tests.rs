//! UI integration tests for the shell: a real `Workspace` (rail, drawer,
//! editor) in a headless window, driven by GPUI keyboard and pointer events.

use std::path::{Path, PathBuf};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Pixels, TestAppContext, WindowBounds,
    WindowOptions, point, px, size,
};
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::AppSettings;
use kimun_notes::settings::gui::GuiSettings;
use kimun_notes::settings::themes::Theme;

use super::{LayoutStore, Workspace, rail_cell_id};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::shell_layout;

pub(super) const HEIGHT: f32 = 480.;
const DRAWER: &str = "drawer-left";
const DIVIDER: &str = "drawer-left-divider";

pub(super) struct Shell {
    pub(super) handle: AnyWindowHandle,
    pub(super) workspace: Entity<Workspace>,
}

impl Shell {
    pub(super) fn editor(&self, cx: &gpui_kit::App) -> Entity<EditorView> {
        self.workspace.read(cx).editor.clone()
    }

    pub(super) fn text(&self, cx: &gpui_kit::App) -> String {
        self.editor(cx).read(cx).editor.get_text()
    }

    pub(super) fn focused(&self, cx: &gpui_kit::App) -> Panel {
        self.workspace.read(cx).layout.focused()
    }

    pub(super) fn drawer_shown(&self, cx: &gpui_kit::App) -> bool {
        self.workspace
            .read(cx)
            .layout
            .is_visible(Panel::Drawer(Side::Left))
    }

    pub(super) fn drawer_view(&self, cx: &gpui_kit::App) -> DrawerView {
        self.workspace.read(cx).layout.slot_views(Side::Left)[0]
    }
}

pub(super) fn gui(toml: &str) -> GuiSettings {
    toml::from_str::<AppSettings>(toml).unwrap().gui
}

/// A shell `width` px wide, its layout read from `gui`, persisting to
/// `store`, the editor (plain unless `vim`) showing `text`.
pub(super) fn open_shell(
    cx: &mut TestAppContext,
    width: f32,
    gui: &GuiSettings,
    text: &str,
    vim: bool,
    store: LayoutStore,
) -> Shell {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let layout = shell_layout::restore(gui);
    let text = text.to_string();
    let (handle, workspace) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(width), px(HEIGHT)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let editor = cx.new(|cx| EditorView::new(Some(text), vim, cx));
                    let workspace = cx.new(|cx| {
                        Workspace::new(editor, Theme::default(), layout, store, window, cx)
                    });
                    workspace.update(cx, |w, cx| w.apply_focus(window, cx));
                    workspace
                },
            )
        })
        .expect("open test window");
    Shell { handle, workspace }
}

pub(super) fn open_default(cx: &mut TestAppContext, text: &str) -> Shell {
    open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        text,
        false,
        LayoutStore::Off,
    )
}

/// A config path in a fresh temp dir (kept alive by the returned guard).
pub(super) fn temp_config() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    (dir, path)
}

/// The `[gui]` table as the file now holds it.
pub(super) fn saved(path: &Path) -> GuiSettings {
    let body = std::fs::read_to_string(path).expect("the layout was saved");
    toml::from_str::<AppSettings>(&body).unwrap().gui
}

pub(super) fn close(a: Pixels, b: f32) -> bool {
    (f32::from(a) - b).abs() < 0.5
}

/// Rail, drawer and editor sit side by side, filling the window: the rail
/// at its rem width, the drawer at its preferred width clamped to the
/// window, the editor taking the rest.
#[gpui_kit::test]
fn the_shell_composes_at_several_window_widths(cx: &mut TestAppContext) {
    // 600 preferred: kept at 1200, clamped to leave the editor its minimum
    // at 800, and held at the drawer's own minimum at 420.
    for (width, drawer) in [(1200., 600.), (800., 432.), (420., 160.)] {
        let shell = open_shell(
            cx,
            width,
            &gui("[gui]\ndrawer_left_width = 600\n"),
            "",
            false,
            LayoutStore::Off,
        );
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let rem = f32::from(window.rem_size());
            assert_eq!(rem, 16.0, "the floors below assume the default rem");
            let rail = window.find("rail").bounds();
            let drawer_b = window.find(DRAWER).bounds();
            let editor = window.find("editor-panel").bounds();
            assert!(close(rail.left(), 0.0));
            assert!(close(rail.size.width, shell_layout::RAIL_REMS * rem));
            assert!(close(drawer_b.left(), f32::from(rail.right())));
            assert!(
                close(drawer_b.size.width, drawer),
                "{width}: drawer {:?}",
                drawer_b.size.width
            );
            assert!(close(editor.left(), f32::from(drawer_b.right())));
            assert!(close(editor.right(), width), "{width}: editor {editor:?}");
            assert!(close(editor.size.height, HEIGHT));
            // The preference survives a narrow window.
            assert_eq!(
                shell.workspace.read(cx).layout.drawer_width(Side::Left),
                600.0
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_shell_starts_with_the_editor_focused(cx: &mut TestAppContext) {
    let shell = open_default(cx, "");
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(window.find("editor").focused(), Some(true));
        assert_eq!(window.find("rail").focused(), Some(false));
        assert_eq!(window.find(DRAWER).focused(), Some(false));
        window.input("hi", cx);
        assert_eq!(shell.text(cx), "hi");
    })
    .unwrap();
}

/// Dragging the divider resizes the drawer, clamped to the drawer's and the
/// editor's minimum, and the width is saved when the drag ends.
#[gpui_kit::test]
fn dragging_the_divider_clamps_and_saves_on_release(cx: &mut TestAppContext) {
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
        let width = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.render_frame(cx);
            f32::from(window.find(DRAWER).bounds().size.width)
        };
        let grab = |window: &gpui_kit::Window| window.find(DIVIDER).bounds().center();
        assert_eq!(width(window, cx), 260.0);

        // 192px right: the edge follows the pointer exactly, wherever on
        // the band the press landed (no jump to the pointer).
        let from = grab(window);
        window.drag(from, point(from.x + px(192.), from.y), cx);
        assert_eq!(width(window, cx), 452.0);
        assert_eq!(saved(&path).drawer_width(Side::Left), 452.0);

        // Too narrow: held at 10rem.
        let from = grab(window);
        window.drag(from, point(px(60.), from.y), cx);
        assert_eq!(width(window, cx), 160.0);
        // Too wide: the editor keeps 20rem (1000 - 48 - 320).
        let from = grab(window);
        window.drag(from, point(px(990.), from.y), cx);
        assert_eq!(width(window, cx), 632.0);
        assert_eq!(saved(&path).drawer_width(Side::Left), 632.0);
        let editor = window.find("editor-panel").bounds();
        assert!(close(editor.size.width, 320.0), "{editor:?}");
    })
    .unwrap();
}

/// A drag released outside the window still ends, and is saved.
#[gpui_kit::test]
fn a_drag_released_outside_the_window_is_saved(cx: &mut TestAppContext) {
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
        let from = window.find(DIVIDER).bounds().center();
        window.drag(from, point(px(1050.), from.y), cx);
        window.render_frame(cx);
        // Clamped so the editor keeps 20rem; saved on the release outside.
        assert!(close(window.find(DRAWER).bounds().size.width, 632.0));
        assert_eq!(saved(&path).drawer_width(Side::Left), 632.0);
        assert!(!shell.workspace.read(cx).resizing);
    })
    .unwrap();
}

/// The drawer toggle hides the drawer, giving its width to the editor, and
/// brings it back focused (the TUI's toggle); each change is saved.
#[gpui_kit::test]
fn the_toggle_gives_the_drawer_width_to_the_editor(cx: &mut TestAppContext) {
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
        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert!(window.try_find(DRAWER).is_none());
        let rail = window.find("rail").bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(close(editor.left(), f32::from(rail.right())));
        assert!(close(editor.size.width, 1000. - f32::from(rail.size.width)));
        assert!(!saved(&path).drawer_visible(Side::Left));
        // The editor kept the keyboard and its text.
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(shell.text(cx), "abc");

        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert!(close(window.find(DRAWER).bounds().size.width, 260.0));
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        assert_eq!(window.find(DRAWER).focused(), Some(true));
        assert!(saved(&path).drawer_visible(Side::Left));
        // From the drawer the toggle works too (app chords win there), and
        // closing the focused drawer gives the keyboard to the editor, not
        // to the rail (the model's and the TUI's fallback).
        window.press("ctrl-t", cx);
        assert!(!shell.drawer_shown(cx));
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(window.find("editor").focused(), Some(true));
        window.input("d", cx);
        assert_eq!(shell.text(cx), "dabc");
    })
    .unwrap();
}

/// A rail click opens its view in the drawer and focuses it; clicking the
/// shown view closes the drawer. The view is saved.
#[gpui_kit::test]
fn a_rail_click_switches_the_drawer_view(cx: &mut TestAppContext) {
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
        assert_eq!(
            window.find(rail_cell_id(DrawerView::Files)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find(rail_cell_id(DrawerView::Tags)).selected(),
            Some(false)
        );

        window.click(rail_cell_id(DrawerView::Tags), cx);
        window.render_frame(cx);
        assert_eq!(shell.drawer_view(cx), DrawerView::Tags);
        assert_eq!(window.find(DRAWER).label(), Some("Tags"));
        assert_eq!(
            window.find(rail_cell_id(DrawerView::Tags)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find(rail_cell_id(DrawerView::Files)).selected(),
            Some(false)
        );
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        assert_eq!(
            super::drawer_tests::saved_shown(&path, Side::Left),
            [DrawerView::Tags]
        );

        // The pinned settings cell, at the bottom of the rail.
        let config = window.find(rail_cell_id(DrawerView::Config)).bounds();
        assert!(f32::from(config.bottom()) > HEIGHT - 2.0 * 16.0 - 8.0);
        window.click(rail_cell_id(DrawerView::Config), cx);
        assert_eq!(shell.drawer_view(cx), DrawerView::Config);

        // The shown view again: the drawer closes, focus stays on the rail.
        window.click(rail_cell_id(DrawerView::Config), cx);
        window.render_frame(cx);
        assert!(window.try_find(DRAWER).is_none());
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert!(!saved(&path).drawer_visible(Side::Left));
        assert_eq!(
            super::drawer_tests::saved_shown(&path, Side::Left),
            [DrawerView::Config]
        );
    })
    .unwrap();
}

/// Keys go to the focused Panel only, and a press focuses the panel it
/// lands in.
#[gpui_kit::test]
fn keys_go_to_the_focused_panel_only(cx: &mut TestAppContext) {
    let shell = open_default(cx, "");
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(DRAWER, cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        assert_eq!(window.find(DRAWER).focused(), Some(true));
        window.input("x", cx);
        window.press("enter", cx);
        assert_eq!(shell.text(cx), "", "the editor is not focused");

        window.click("editor-panel", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(window.find("editor").focused(), Some(true));
        window.input("y", cx);
        assert_eq!(shell.text(cx), "y");
    })
    .unwrap();
}

/// Alt+Arrow moves focus between the panels without wrapping; Ctrl+Tab and
/// Ctrl+Shift+Tab cycle through the visible ones, wrapping.
#[gpui_kit::test]
fn focus_moves_by_direction_and_cycles(cx: &mut TestAppContext) {
    let shell = open_default(cx, "");
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("alt-left", cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        assert_eq!(window.find(DRAWER).focused(), Some(true));
        window.press("alt-left", cx);
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert_eq!(window.find("rail").focused(), Some(true));
        // The left edge: no wrap.
        window.press("alt-left", cx);
        assert_eq!(shell.focused(cx), Panel::Rail);
        window.press("alt-right", cx);
        window.press("alt-right", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.press("alt-right", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);

        // The cycle wraps: editor -> rail, and back.
        window.press("ctrl-tab", cx);
        assert_eq!(shell.focused(cx), Panel::Rail);
        window.press("ctrl-shift-tab", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.press("ctrl-shift-tab", cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        // A hidden drawer is skipped (hiding it focused the editor).
        window.press("ctrl-t", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        window.press("ctrl-tab", cx);
        assert_eq!(shell.focused(cx), Panel::Rail);
        window.press("ctrl-tab", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(shell.text(cx), "", "no chord reached the buffer");
    })
    .unwrap();
}

/// The rail's own keys: Up/Down move between cells, Enter opens one.
#[gpui_kit::test]
fn the_rail_works_from_the_keyboard(cx: &mut TestAppContext) {
    let shell = open_default(cx, "");
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("rail", cx);
        assert_eq!(shell.focused(cx), Panel::Rail);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(shell.drawer_view(cx), DrawerView::Find);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
    })
    .unwrap();
}

/// The find bar still opens and searches inside the shell, and its chord is
/// the editor's only: with the drawer focused it does nothing.
#[gpui_kit::test]
fn the_find_bar_still_works_in_the_shell(cx: &mut TestAppContext) {
    let shell = open_default(cx, "foo bar foo");
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-f", cx);
        assert!(shell.editor(cx).read(cx).editor.find_open());
        window.input("foo", cx);
        let query = shell
            .editor(cx)
            .read(cx)
            .editor
            .find_bar()
            .map(|b| b.query().to_string());
        assert_eq!(query.as_deref(), Some("foo"));
        // An open bar still lets the shell's chords through.
        window.press("alt-left", cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        window.press("alt-right", cx);
        window.press("escape", cx);
        assert!(!shell.editor(cx).read(cx).editor.find_open());
        assert_eq!(shell.text(cx), "foo bar foo");

        window.press("alt-left", cx);
        window.press("ctrl-f", cx);
        assert!(!shell.editor(cx).read(cx).editor.find_open());
    })
    .unwrap();
}

/// Alt+Up/Down with nothing stacked is no focus move (ADR-0050 §4): the
/// workspace hands it back to the editor rather than swallowing it. Under
/// vim an unmapped key cancels a pending count, which shows it arrived.
#[gpui_kit::test]
fn alt_up_with_nothing_stacked_reaches_the_editor(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "a\nb\nc",
        true,
        LayoutStore::Off,
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let hint = |cx: &gpui_kit::App| shell.editor(cx).read(cx).editor.pending_input_hint();
        window.press("2", cx);
        assert_eq!(hint(cx).as_deref(), Some("2"));
        window.press("alt-up", cx);
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(hint(cx), None, "the editor saw Alt+Up");
    })
    .unwrap();
}

/// What was saved is what the next start shows: width, visibility, view.
#[gpui_kit::test]
fn the_saved_layout_is_restored(cx: &mut TestAppContext) {
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
        window.click(rail_cell_id(DrawerView::Outline), cx);
        let from = window.find(DIVIDER).bounds().center();
        window.drag(from, point(from.x + px(92.), from.y), cx);
        window.press("ctrl-t", cx);
    })
    .unwrap();

    let restored = shell_layout::restore(&saved(&path));
    assert_eq!(restored.slot_views(Side::Left), [DrawerView::Outline]);
    assert_eq!(restored.drawer_width(Side::Left), 352.0);
    assert!(!restored.is_visible(Panel::Drawer(Side::Left)));

    // And through a real reopen: hidden; the toggle brings Outline back at 352.
    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(DRAWER).is_none());
        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert_eq!(window.find(DRAWER).label(), Some("Outline"));
        assert!(close(window.find(DRAWER).bounds().size.width, 352.0));
    })
    .unwrap();
}
