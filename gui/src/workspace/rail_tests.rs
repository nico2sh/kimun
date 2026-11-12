//! UI integration tests for the rail's position and visibility (ADR-0055).
//! A real `Workspace` in a headless window, the rail's place read from a
//! config written before the window opens.
#![cfg(test)]

use std::path::PathBuf;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext};
use kimun_notes::logic::layout::Side;
use kimun_notes::logic::layout_tree::{Focus, RailPosition};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::GuiSettings;

use super::palette::PaletteRun;
use super::tests::{HEIGHT, Shell, close, gui, open_shell, temp_config};
use super::{LayoutStore, rail_cell_id, slot_id};
use crate::shell_layout::{self, RAIL_REMS};

const WIDTH: f32 = 1200.;
const REM: f32 = 16.;

/// A shell whose config puts the rail at `rail`, hidden when `hidden`.
fn open_rail(
    cx: &mut TestAppContext,
    rail: RailPosition,
    hidden: bool,
) -> (tempfile::TempDir, PathBuf, Shell) {
    open_rail_with(cx, rail, hidden, GuiSettings::default())
}

/// [`open_rail`] with the layout `layout` describes.
fn open_rail_with(
    cx: &mut TestAppContext,
    rail: RailPosition,
    hidden: bool,
    layout: GuiSettings,
) -> (tempfile::TempDir, PathBuf, Shell) {
    let (dir, config) = temp_config();
    let mut gui = layout;
    gui.set_rail(rail);
    gui.set_rail_hidden(hidden);
    let mut settings = kimun_notes::settings::AppSettings::default();
    settings.gui = gui.clone();
    std::fs::write(&config, toml::to_string(&settings).unwrap()).unwrap();
    let shell = open_shell(
        cx,
        WIDTH,
        &gui,
        "",
        false,
        LayoutStore::File(config.clone()),
    );
    (dir, config, shell)
}

/// The bounds of the rail and the layout tree, after a frame.
fn rail_and_tree(shell: &Shell, cx: &mut TestAppContext) -> (Bounds<Pixels>, Bounds<Pixels>) {
    let mut out = None;
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        out = Some((
            window.find("rail").bounds(),
            window.find("layout-tree").bounds(),
        ));
    })
    .unwrap();
    out.unwrap()
}

#[gpui_kit::test]
fn a_left_rail_is_a_column_at_the_left_edge(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail(cx, RailPosition::Left, false);
    let (rail, tree) = rail_and_tree(&shell, cx);
    assert!(close(rail.left(), 0.0));
    assert!(close(rail.size.width, RAIL_REMS * REM));
    assert!(close(tree.left(), f32::from(rail.right())));
    assert!(close(rail.size.height, f32::from(tree.size.height)));
}

#[gpui_kit::test]
fn a_right_rail_is_a_column_at_the_right_edge(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail(cx, RailPosition::Right, false);
    let (rail, tree) = rail_and_tree(&shell, cx);
    assert!(close(rail.right(), WIDTH));
    assert!(close(rail.size.width, RAIL_REMS * REM));
    assert!(close(tree.right(), f32::from(rail.left())));
    assert!(close(tree.left(), 0.0));
}

#[gpui_kit::test]
fn a_top_rail_is_a_row_above_the_tree(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail(cx, RailPosition::Top, false);
    let (rail, tree) = rail_and_tree(&shell, cx);
    assert!(close(rail.size.height, RAIL_REMS * REM));
    assert!(close(rail.size.width, WIDTH));
    assert!(close(tree.top(), f32::from(rail.bottom())));
    assert!(close(tree.bottom(), HEIGHT));
}

#[gpui_kit::test]
fn a_bottom_rail_is_a_row_under_the_tree(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail(cx, RailPosition::Bottom, false);
    let (rail, tree) = rail_and_tree(&shell, cx);
    assert!(close(rail.bottom(), HEIGHT));
    assert!(close(rail.size.height, RAIL_REMS * REM));
    assert!(close(rail.size.width, WIDTH));
    assert!(close(tree.bottom(), f32::from(rail.top())));
}

#[gpui_kit::test]
fn the_tree_gets_the_rest_for_each_position(cx: &mut TestAppContext) {
    for rail in RailPosition::ALL {
        let (_dir, _config, shell) = open_rail(cx, rail, false);
        let (r, tree) = rail_and_tree(&shell, cx);
        if rail.horizontal() {
            assert!(
                close(r.size.height + tree.size.height, HEIGHT),
                "{rail:?}: {r:?} {tree:?}"
            );
            assert!(close(tree.size.width, WIDTH), "{rail:?}: {tree:?}");
        } else {
            assert!(
                close(r.size.width + tree.size.width, WIDTH),
                "{rail:?}: {r:?} {tree:?}"
            );
            assert!(close(tree.size.height, HEIGHT), "{rail:?}: {tree:?}");
        }
    }
}

#[gpui_kit::test]
fn a_hidden_rail_gives_the_tree_the_whole_window(cx: &mut TestAppContext) {
    for rail in RailPosition::ALL {
        let (_dir, _config, shell) = open_rail(cx, rail, true);
        let mut tree = None;
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            tree = Some(window.find("layout-tree").bounds());
            assert!(
                window.try_find("rail").is_none(),
                "{rail:?}: the rail is drawn"
            );
        })
        .unwrap();
        let tree = tree.unwrap();
        assert!(close(tree.size.width, WIDTH), "{rail:?}: {tree:?}");
        assert!(close(tree.size.height, HEIGHT), "{rail:?}: {tree:?}");
    }
}

#[gpui_kit::test]
fn a_horizontal_rail_keeps_every_cell_and_the_pinned_cell_last(cx: &mut TestAppContext) {
    for rail in [RailPosition::Top, RailPosition::Bottom] {
        let (_dir, _config, shell) = open_rail(cx, rail, false);
        let views = shell_layout::rail_views(Default::default());
        let mut lefts = Vec::new();
        let mut toggles = Vec::new();
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            for view in views {
                lefts.push(f32::from(window.find(rail_cell_id(*view)).bounds().left()));
            }
            for id in ["rail-toggle-left", "rail-toggle-right"] {
                toggles.push(f32::from(window.find(id).bounds().left()));
            }
        })
        .unwrap();
        assert!(lefts.windows(2).all(|w| w[0] < w[1]), "{rail:?}: {lefts:?}");
        let pinned = *lefts.last().unwrap();
        assert_eq!(views.last(), Some(&DrawerView::Config));
        let before_pinned = lefts[lefts.len() - 2];
        assert!(
            toggles.iter().all(|t| *t > before_pinned && *t < pinned),
            "{rail:?}"
        );
        assert!(toggles[0] < toggles[1]);
    }
}

#[gpui_kit::test]
fn the_side_toggles_keep_their_ids_in_every_position(cx: &mut TestAppContext) {
    for rail in RailPosition::ALL {
        let (_dir, _config, shell) = open_rail(cx, rail, false);
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            for id in ["rail-toggle-left", "rail-toggle-right"] {
                assert!(window.try_find(id).is_some(), "{rail:?}: {id}");
            }
        })
        .unwrap();
    }
}

/// One slot left of the editor holding only FILES.
fn files_alone() -> GuiSettings {
    gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.8, editor = true },\n]\n")
}

fn focus(shell: &Shell, cx: &gpui_kit::App) -> Focus {
    shell.workspace.read(cx).layout.focused()
}

/// The slots left of the editor, in reading order, with the view each shows.
fn left_shown(shell: &Shell, cx: &gpui_kit::App) -> Vec<Option<DrawerView>> {
    let w = shell.workspace.read(cx);
    w.side_slots(Side::Left)
        .into_iter()
        .map(|slot| w.layout.shown(slot))
        .collect()
}

fn cursor(shell: &Shell, cx: &gpui_kit::App) -> Option<DrawerView> {
    shell.workspace.read(cx).rail_cursor_view()
}

fn cursor_on(shell: &Shell, view: DrawerView, cx: &mut gpui_kit::App) {
    shell
        .workspace
        .update(cx, |workspace, _| workspace.place_rail_cursor(Some(view)));
}

/// The key name of `dir`.
fn key(dir: kimun_notes::logic::layout_tree::Dir) -> &'static str {
    use kimun_notes::logic::layout_tree::Dir;
    match dir {
        Dir::Left => "left",
        Dir::Right => "right",
        Dir::Up => "up",
        Dir::Down => "down",
    }
}

#[gpui_kit::test]
fn plain_arrows_move_the_cursor_along_the_rails_axis(cx: &mut TestAppContext) {
    use kimun_notes::logic::layout_tree::Dir;
    for rail in RailPosition::ALL {
        let (_dir, _config, shell) = open_rail_with(cx, rail, false, files_alone());
        let (next, prev, across) = if rail.horizontal() {
            ("right", "left", "up")
        } else {
            ("down", "up", "right")
        };
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("rail", cx);
            assert_eq!(focus(&shell, cx), Focus::Rail, "{rail:?}");
            cursor_on(&shell, DrawerView::Files, cx);
            window.press(next, cx);
            assert_eq!(cursor(&shell, cx), Some(DrawerView::Find), "{rail:?}");
            window.press(across, cx);
            assert_eq!(
                cursor(&shell, cx),
                Some(DrawerView::Find),
                "{rail:?} across"
            );
            window.press(prev, cx);
            assert_eq!(cursor(&shell, cx), Some(DrawerView::Files), "{rail:?}");
            let _ = Dir::Left;
        })
        .unwrap();
    }
}

/// ADR-0055 §6: along the rail's axis a move chord places the cursor's
/// view in a new slot, toward the layout it shows the view in the slot
/// beside the rail, and facing the rail it is not handled.
#[gpui_kit::test]
fn the_move_chords_from_the_rail_follow_the_rotation_rule(cx: &mut TestAppContext) {
    use kimun_notes::logic::layout_tree::Dir;
    for rail in RailPosition::ALL {
        for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
            let (_dir, _config, shell) = open_rail_with(cx, rail, false, files_alone());
            let chord = format!("alt-shift-{}", key(dir));
            cx.update_window(shell.handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("rail", cx);
                cursor_on(&shell, DrawerView::Find, cx);
                window.press(&chord, cx);
                window.render_frame(cx);
                let shown = left_shown(&shell, cx);
                let at = format!("{rail:?} {chord}: {shown:?}");
                if rail.place_dir(dir) {
                    // A new slot beside FILES' slot, on the `dir` side.
                    assert_eq!(shown.len(), 2, "{at}");
                    let find_first = matches!(dir, Dir::Left | Dir::Up);
                    let want = if find_first {
                        [Some(DrawerView::Find), Some(DrawerView::Files)]
                    } else {
                        [Some(DrawerView::Files), Some(DrawerView::Find)]
                    };
                    assert_eq!(shown, want, "{at}");
                } else if dir == rail.toward() {
                    // Shown in the slot beside the rail.
                    assert_eq!(shown, [Some(DrawerView::Find)], "{at}");
                } else {
                    // Facing the rail: not handled, nothing changes.
                    assert_eq!(shown, [Some(DrawerView::Files)], "{at}");
                }
                assert_eq!(focus(&shell, cx), Focus::Rail, "{at}");
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn directional_focus_reaches_and_leaves_the_rail_on_its_edge(cx: &mut TestAppContext) {
    for rail in [RailPosition::Right, RailPosition::Top, RailPosition::Bottom] {
        let (_dir, _config, shell) = open_rail_with(cx, rail, false, files_alone());
        let to_rail = format!("alt-{}", key(rail.facing()));
        let back = format!("alt-{}", key(rail.toward()));
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            // The editor group has the keyboard and nothing lies between it
            // and the rail.
            window.press(&to_rail, cx);
            assert_eq!(focus(&shell, cx), Focus::Rail, "{rail:?}");
            window.press(&back, cx);
            assert!(matches!(focus(&shell, cx), Focus::Leaf(_)), "{rail:?}");
        })
        .unwrap();
    }
}

/// The toggle chord with the default Linux/Windows keymap.
const TOGGLE_RAIL: &str = "ctrl-shift-a";
const TOGGLE_ID: &str = crate::status_row::RAIL_TOGGLE_ID;

fn hidden(shell: &Shell, cx: &gpui_kit::App) -> bool {
    shell.workspace.read(cx).rail_hidden
}

#[gpui_kit::test]
fn the_chord_hides_the_rail_and_shows_it_again(cx: &mut TestAppContext) {
    for rail in RailPosition::ALL {
        let (_dir, _config, shell) = open_rail(cx, rail, false);
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(TOGGLE_RAIL, cx);
            window.render_frame(cx);
            assert!(hidden(&shell, cx), "{rail:?}");
            assert!(window.try_find("rail").is_none(), "{rail:?}");
            let tree = window.find("layout-tree").bounds();
            assert!(close(tree.size.width, WIDTH), "{rail:?}: {tree:?}");
            assert!(close(tree.size.height, HEIGHT), "{rail:?}: {tree:?}");
            window.press(TOGGLE_RAIL, cx);
            window.render_frame(cx);
            assert!(!hidden(&shell, cx), "{rail:?}");
            assert!(window.try_find("rail").is_some(), "{rail:?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_hidden_rail_is_out_of_the_focus_cycle_and_directional_focus(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, true, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..6 {
            window.press("ctrl-tab", cx);
            assert_ne!(focus(&shell, cx), Focus::Rail);
        }
        for _ in 0..6 {
            window.press("ctrl-shift-tab", cx);
            assert_ne!(focus(&shell, cx), Focus::Rail);
        }
        // Left from the left-most leaf is the edge, not the hidden rail.
        window.click(slot_id(DrawerView::Files), cx);
        window.press("alt-left", cx);
        assert_ne!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_with_the_rail_focused_moves_focus_to_the_last_focused_slot(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, false, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(slot_id(DrawerView::Files), cx);
        let slot = match focus(&shell, cx) {
            Focus::Leaf(kimun_notes::logic::layout_tree::Leaf::Slot(slot)) => slot,
            other => panic!("the slot has the keyboard, not {other:?}"),
        };
        window.click("rail", cx);
        assert_eq!(focus(&shell, cx), Focus::Rail);
        window.press(TOGGLE_RAIL, cx);
        assert_eq!(
            focus(&shell, cx),
            Focus::Leaf(kimun_notes::logic::layout_tree::Leaf::Slot(slot))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_with_the_rail_focused_and_no_slot_before_moves_focus_to_the_editor(
    cx: &mut TestAppContext,
) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, false, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("rail", cx);
        window.press(TOGGLE_RAIL, cx);
        let group = shell.workspace.read(cx).layout.active_group();
        assert_eq!(
            focus(&shell, cx),
            Focus::Leaf(kimun_notes::logic::layout_tree::Leaf::Group(group))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn showing_the_rail_does_not_take_the_keyboard(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, true, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        let before = focus(&shell, cx);
        window.press(TOGGLE_RAIL, cx);
        assert!(!hidden(&shell, cx));
        assert_eq!(focus(&shell, cx), before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_footer_button_toggles_and_names_the_chord(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail(cx, RailPosition::Left, false);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // The chord is written the platform's way (`Ctrl+Shift+A`, `⌃⇧A`).
        let chord = crate::keymap::chord_label(
            cx,
            &kimun_notes::logic::actions::ActionShortcuts::ToggleRail,
        )
        .expect("ToggleRail is bound");
        let label = window.find(TOGGLE_ID).label().map(str::to_string);
        assert_eq!(label, Some(format!("Hide rail ({chord})")));
        window.click(TOGGLE_ID, cx);
        window.render_frame(cx);
        assert!(hidden(&shell, cx));
        let label = window.find(TOGGLE_ID).label().map(str::to_string);
        assert_eq!(label, Some(format!("Show rail ({chord})")));
        window.click(TOGGLE_ID, cx);
        window.render_frame(cx);
        assert!(!hidden(&shell, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_footer_button_is_there_in_every_position_and_while_hidden(cx: &mut TestAppContext) {
    for rail in RailPosition::ALL {
        for hide in [false, true] {
            let (_dir, _config, shell) = open_rail(cx, rail, hide);
            cx.update_window(shell.handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window.try_find(TOGGLE_ID).is_some(),
                    "{rail:?} hidden={hide}"
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn the_hidden_state_is_saved_and_restored(cx: &mut TestAppContext) {
    let (_dir, config, shell) = open_rail(cx, RailPosition::Top, false);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press(TOGGLE_RAIL, cx);
    })
    .unwrap();
    let gui = super::tests::saved(&config);
    assert!(gui.rail_hidden());
    assert_eq!(gui.rail(), RailPosition::Top);
    let again = open_shell(
        cx,
        WIDTH,
        &gui,
        "",
        false,
        LayoutStore::File(config.clone()),
    );
    cx.update(|cx| assert!(hidden(&again, cx)));
}

#[gpui_kit::test]
fn the_side_toggle_chords_still_work_with_the_rail_hidden(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, true, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(left_shown(&shell, cx), [Some(DrawerView::Files)]);
        window.press("ctrl-t", cx);
        window.render_frame(cx);
        assert_eq!(left_shown(&shell, cx), [None]);
        assert!(hidden(&shell, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn zoom_and_a_hidden_rail_do_not_interact(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, false, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-m", cx);
        assert!(shell.workspace.read(cx).layout.zoomed().is_some());
        window.press(TOGGLE_RAIL, cx);
        assert!(hidden(&shell, cx));
        assert!(shell.workspace.read(cx).layout.zoomed().is_some());
        window.press("ctrl-shift-m", cx);
        assert!(shell.workspace.read(cx).layout.zoomed().is_none());
        assert!(hidden(&shell, cx));
    })
    .unwrap();
}

/// The palette rows' labels and why each is blocked.
fn rows(shell: &Shell, cx: &mut TestAppContext) -> Vec<(String, Option<&'static str>)> {
    cx.read(|cx| {
        shell
            .workspace
            .read(cx)
            .palette_entries(kimun_notes::logic::layout::Panel::Editor, cx)
            .iter()
            .map(|e| (e.label().to_string(), e.blocked()))
            .collect()
    })
}

fn run_rail_row(shell: &Shell, cx: &mut TestAppContext, rail: RailPosition) {
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.run_palette_row(PaletteRun::RailAt(rail), window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_palette_offers_one_row_per_rail_position(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail(cx, RailPosition::Top, false);
    let listed = rows(&shell, cx);
    for (label, here) in [
        ("Rail position: Left", false),
        ("Rail position: Right", false),
        ("Rail position: Top", true),
        ("Rail position: Bottom", false),
    ] {
        let row = listed
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("{label}: {listed:?}"));
        assert_eq!(
            row.1.is_some(),
            here,
            "{label}: only the current one is blocked"
        );
    }
}

#[gpui_kit::test]
fn choosing_a_position_moves_the_rail_and_saves_it(cx: &mut TestAppContext) {
    let (_dir, config, shell) = open_rail(cx, RailPosition::Left, true);
    let layout_before = cx.read(|cx| shell.workspace.read(cx).layout.clone());
    run_rail_row(&shell, cx, RailPosition::Bottom);
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        assert_eq!(w.rail, RailPosition::Bottom);
        assert!(w.rail_hidden, "a position command leaves the hidden state");
        assert_eq!(w.layout, layout_before, "the layout tree does not change");
    });
    let gui = super::tests::saved(&config);
    assert_eq!(gui.rail(), RailPosition::Bottom);
    assert!(gui.rail_hidden());
    assert_eq!(
        crate::shell_tree::restore(&gui, crate::shell_tree::RESTORE_CAPS),
        layout_before,
        "the saved layout is the one before"
    );
}

#[gpui_kit::test]
fn a_position_command_does_not_move_focus(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, false, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("rail", cx);
        assert_eq!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
    run_rail_row(&shell, cx, RailPosition::Right);
    cx.read(|cx| assert_eq!(focus(&shell, cx), Focus::Rail));
}

#[gpui_kit::test]
fn hiding_the_rail_while_zoomed_keeps_the_zoom(cx: &mut TestAppContext) {
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, false, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // FILES had the keyboard last, then the editor group; zoom the group.
        window.click(slot_id(DrawerView::Files), cx);
        window.click("editor-panel", cx);
        assert!(matches!(
            focus(&shell, cx),
            Focus::Leaf(kimun_notes::logic::layout_tree::Leaf::Group(_))
        ));
        window.press("ctrl-shift-m", cx);
        let zoomed = shell.workspace.read(cx).layout.zoomed().expect("zoomed");
        window.press("alt-left", cx);
        assert_eq!(focus(&shell, cx), Focus::Rail);
        window.press(TOGGLE_RAIL, cx);
        window.render_frame(cx);
        assert_eq!(shell.workspace.read(cx).layout.zoomed(), Some(zoomed));
        assert_eq!(focus(&shell, cx), Focus::Leaf(zoomed));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hidden_rail_cannot_be_given_the_keyboard(cx: &mut TestAppContext) {
    use kimun_notes::logic::layout::Panel;
    let (_dir, _config, shell) = open_rail_with(cx, RailPosition::Left, true, files_alone());
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        // A modal that closes gives the keyboard back to its opener, which
        // may be the rail it was opened from.
        shell
            .workspace
            .update(cx, |w, _| w.set_focus_panel(Panel::Rail));
        assert_ne!(focus(&shell, cx), Focus::Rail);
    })
    .unwrap();
}
