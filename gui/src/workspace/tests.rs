//! UI integration tests for the shell: a real `Workspace` (rail, slot,
//! editor) in a headless window, driven by GPUI keyboard and pointer events.

use std::path::{Path, PathBuf};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Pixels, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::layout_tree::LayoutTree;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::AppSettings;
use kimun_notes::settings::gui::GuiSettings;
use kimun_notes::settings::themes::Theme;

use super::{LayoutStore, Workspace, rail_cell_id};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::{shell_layout, shell_tree};

pub(super) const HEIGHT: f32 = 480.;
/// The left slot in the default layout, showing FILES.
const FILES_SLOT: &str = "slot-files";

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
        self.workspace.read(cx).focused_panel()
    }

    /// Whether a slot left of the editor is drawn.
    pub(super) fn left_side_shown(&self, cx: &gpui_kit::App) -> bool {
        let workspace = self.workspace.read(cx);
        workspace
            .side_slots(Side::Left)
            .into_iter()
            .any(|s| workspace.slot_drawn(s))
    }

    /// The view the first slot left of the editor shows.
    pub(super) fn left_slot_view(&self, cx: &gpui_kit::App) -> DrawerView {
        let workspace = self.workspace.read(cx);
        let slot = workspace.side_slots(Side::Left)[0];
        workspace
            .layout
            .shown(slot)
            .expect("the left slot shows a view")
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
    let layout = shell_tree::restore(gui, shell_tree::RESTORE_CAPS);
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

/// The layout the saved `[gui]` table restores.
fn saved_left(path: &Path) -> LayoutTree<DrawerView> {
    shell_tree::restore(&saved(path), shell_tree::RESTORE_CAPS)
}

/// The saved layout's first slot (left of the editor in the default one).
fn left_slot(path: &Path) -> kimun_notes::logic::layout_tree::SlotId {
    saved_left(path).slots()[0]
}

/// Whether the saved layout's first slot shows a view.
fn saved_left_shows(path: &Path) -> bool {
    saved_left(path).shown(left_slot(path)).is_some()
}

pub(super) fn close(a: Pixels, b: f32) -> bool {
    (f32::from(a) - b).abs() < 0.5
}

/// Rail, left slot and editor sit side by side, filling the window: the
/// rail at its rem width, the slot and the editor sharing the rest by the
/// default layout's weights (0.2 : 0.6, the empty right slot's share given
/// back).
#[gpui_kit::test]
fn the_shell_composes_at_several_window_widths(cx: &mut TestAppContext) {
    for width in [1200., 800.] {
        let shell = open_shell(
            cx,
            width,
            &GuiSettings::default(),
            "",
            false,
            LayoutStore::Off,
        );
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            let rem = f32::from(window.rem_size());
            assert_eq!(rem, 16.0, "the widths below assume the default rem");
            let rail = window.find("rail").bounds();
            let slot_b = window.find(FILES_SLOT).bounds();
            let editor = window.find("editor-panel").bounds();
            assert!(close(rail.left(), 0.0));
            assert!(close(rail.size.width, shell_layout::RAIL_REMS * rem));
            assert!(close(slot_b.left(), f32::from(rail.right())));
            let rest = width - shell_layout::RAIL_REMS * rem;
            assert!(
                close(slot_b.size.width, (rest - 1.) * 0.25),
                "{width}: slot {:?}",
                slot_b.size.width
            );
            assert!(close(editor.left(), f32::from(slot_b.right()) + 1.));
            assert!(close(editor.right(), width), "{width}: editor {editor:?}");
            assert!(close(editor.size.height, HEIGHT));
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
        assert_eq!(window.find(FILES_SLOT).focused(), Some(false));
        window.input("hi", cx);
        assert_eq!(shell.text(cx), "hi");
    })
    .unwrap();
}

/// The left side's toggle hides its slot, giving its width to the editor,
/// and brings it back focused (the TUI's toggle); each change is saved.
#[gpui_kit::test]
fn the_toggle_gives_the_slot_width_to_the_editor(cx: &mut TestAppContext) {
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
        assert!(window.try_find(FILES_SLOT).is_none());
        let rail = window.find("rail").bounds();
        let editor = window.find("editor-panel").bounds();
        assert!(close(editor.left(), f32::from(rail.right())));
        assert!(close(editor.size.width, 1000. - f32::from(rail.size.width)));
        assert!(!saved_left_shows(&path));
        // The editor kept the keyboard and its text.
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(shell.text(cx), "abc");

        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert!(close(
            window.find(FILES_SLOT).bounds().size.width,
            951.0 * 0.25
        ));
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        assert_eq!(window.find(FILES_SLOT).focused(), Some(true));
        assert!(saved_left_shows(&path));
        // From the slot the toggle works too (app chords win there), and
        // closing the focused slot gives the keyboard to the editor, not
        // to the rail (the model's and the TUI's fallback).
        window.press("ctrl-t", cx);
        assert!(!shell.left_side_shown(cx));
        assert_eq!(shell.focused(cx), Panel::Editor);
        assert_eq!(window.find("editor").focused(), Some(true));
        window.input("d", cx);
        assert_eq!(shell.text(cx), "dabc");
    })
    .unwrap();
}

/// A rail click opens its view in the slot and focuses it; clicking the
/// shown view closes the slot. The view is saved.
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
        assert_eq!(shell.left_slot_view(cx), DrawerView::Tags);
        assert_eq!(window.find("slot-tags").label(), Some("Tags"));
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
            saved_left(&path).shown(left_slot(&path)),
            Some(DrawerView::Tags)
        );

        // The pinned settings cell, at the bottom of the rail.
        let config = window.find(rail_cell_id(DrawerView::Config)).bounds();
        assert!(f32::from(config.bottom()) > HEIGHT - 2.0 * 16.0 - 8.0);
        window.click(rail_cell_id(DrawerView::Config), cx);
        assert_eq!(shell.left_slot_view(cx), DrawerView::Config);

        // The shown view again: its slot closes, focus stays on the rail;
        // it stays placed there, hidden.
        window.click(rail_cell_id(DrawerView::Config), cx);
        window.render_frame(cx);
        assert!(window.try_find("slot-config").is_none());
        assert_eq!(shell.focused(cx), Panel::Rail);
        assert!(!saved_left_shows(&path));
        let tree = saved_left(&path);
        assert!(
            tree.slot_views(left_slot(&path))
                .contains(&DrawerView::Config)
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
        window.click(FILES_SLOT, cx);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
        assert_eq!(window.find(FILES_SLOT).focused(), Some(true));
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
        assert_eq!(window.find(FILES_SLOT).focused(), Some(true));
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
        // A hidden slot is skipped (hiding it focused the editor).
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
        assert_eq!(shell.left_slot_view(cx), DrawerView::Find);
        assert_eq!(shell.focused(cx), Panel::Drawer(Side::Left));
    })
    .unwrap();
}

/// The find bar still opens and searches inside the shell, and its chord is
/// the editor's only: with the slot focused it does nothing.
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

/// What was saved is what the next start shows: the placements, and a view
/// hidden by the toggle stays hidden. After a restart the toggle has no
/// session memory: it shows the slot's first placed view.
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
        window.press("ctrl-t", cx);
    })
    .unwrap();

    let restored = saved_left(&path);
    let left = left_slot(&path);
    assert_eq!(
        restored.slot_views(left),
        [DrawerView::Files, DrawerView::Outline]
    );
    assert_eq!(restored.shown(left), None);

    // And through a real reopen: hidden; the toggle brings the slot back
    // with its first view.
    let shell = open_shell(cx, 1000., &saved(&path), "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("slot-outline").is_none());
        assert!(window.try_find(FILES_SLOT).is_none());
        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert_eq!(window.find(FILES_SLOT).label(), Some("Files"));
    })
    .unwrap();
}
