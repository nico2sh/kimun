//! Slice two of GUI motion (ADR-0060): the rail, the tab strip and the tab
//! labels open and close over time. Real windows with motion on, time
//! stepped by the executor clock. The slot tests are in `motion_tests`.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use kimun_notes::logic::layout_tree::{Focus, RailPosition};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::AppSettings;
use kimun_notes::settings::gui::GuiSettings;

use super::LayoutStore;
use super::motion_tests::{frame, resize, wait};
use super::rail_cell_id;
use super::tests::{Shell, open_animated, open_shell_motion, temp_config};

const REM: f32 = 16.;
const RAIL: f32 = 3. * REM;

/// A shell whose configuration hides the rail, with motion on. The guard
/// keeps the configuration's folder alive.
fn open_hidden_rail(cx: &mut TestAppContext) -> (tempfile::TempDir, Shell) {
    let (dir, config) = temp_config();
    let mut gui = GuiSettings::default();
    gui.set_rail_hidden(true);
    let mut settings = AppSettings::default();
    settings.gui = gui.clone();
    std::fs::write(&config, toml::to_string(&settings).unwrap()).unwrap();
    let shell = open_shell_motion(cx, 1000., &gui, "", false, LayoutStore::File(config), false);
    frame(cx, &shell);
    (dir, shell)
}

fn toggle_rail(cx: &mut TestAppContext, shell: &Shell) {
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.toggle_rail(window, cx))
    })
    .unwrap();
}

/// The width of the rail, `None` while it is not drawn.
fn rail_width(cx: &mut TestAppContext, shell: &Shell) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find("rail")
            .map(|rail| f32::from(rail.bounds().size.width))
    })
    .unwrap()
}

fn tree_width(cx: &mut TestAppContext, shell: &Shell) -> f32 {
    cx.update_window(shell.handle, |_, window, _| {
        f32::from(window.find("layout-tree").bounds().size.width)
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_rail_closes_over_time_and_the_tree_follows(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    assert_eq!(rail_width(cx, &shell), Some(RAIL));
    let before = tree_width(cx, &shell);
    toggle_rail(cx, &shell);
    frame(cx, &shell);
    assert_eq!(
        rail_width(cx, &shell),
        Some(RAIL),
        "the content fades first"
    );
    wait(cx, 100);
    frame(cx, &shell);
    let middle = rail_width(cx, &shell).expect("the leaving rail is still drawn");
    assert!(middle > 0.0 && middle < RAIL, "at 100 ms: {middle}");
    let tree = tree_width(cx, &shell);
    assert!(tree > before && tree < 1000., "the tree follows: {tree}");
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(rail_width(cx, &shell), None);
    assert_eq!(tree_width(cx, &shell), 1000.);
}

#[gpui_kit::test]
fn the_rail_opens_over_time(cx: &mut TestAppContext) {
    let (_dir, shell) = open_hidden_rail(cx);
    assert_eq!(
        rail_width(cx, &shell),
        None,
        "drawn hidden on the first frame"
    );
    toggle_rail(cx, &shell);
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    let middle = rail_width(cx, &shell).expect("the opening rail is drawn");
    assert!(middle > 1.0 && middle < RAIL - 1.0, "at 30 ms: {middle}");
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(rail_width(cx, &shell), Some(RAIL));
    assert_eq!(tree_width(cx, &shell), 1000. - RAIL);
}

#[gpui_kit::test]
fn the_model_changes_on_the_toggle_frame(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, _, cx| {
        shell
            .workspace
            .update(cx, |w, _| w.layout.focus(Focus::Rail))
    })
    .unwrap();
    toggle_rail(cx, &shell);
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        assert!(w.rail_hidden, "hidden in the model");
        assert!(
            matches!(w.layout.focused(), Focus::Leaf(_)),
            "the keyboard left the rail"
        );
    });
    frame(cx, &shell);
    assert!(rail_width(cx, &shell).is_some(), "still drawn");
}

/// Click the rail's FIND cell: whether FIND is then shown.
fn find_shown_after_click(cx: &mut TestAppContext, shell: &Shell) -> bool {
    cx.update_window(shell.handle, |_, window, cx| {
        window.click(rail_cell_id(DrawerView::Find), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.read(|cx| shell.workspace.read(cx).is_shown(DrawerView::Find))
}

#[gpui_kit::test]
fn a_resting_rail_cell_shows_its_view(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    assert!(find_shown_after_click(cx, &shell));
}

#[gpui_kit::test]
fn a_leaving_rail_takes_no_click(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    toggle_rail(cx, &shell);
    frame(cx, &shell);
    // The content fades and the frame is still whole: the cell is in view.
    wait(cx, 40);
    frame(cx, &shell);
    assert!(rail_width(cx, &shell).is_some(), "still leaving");
    assert!(
        !find_shown_after_click(cx, &shell),
        "the leaving rail took a click"
    );
}

#[gpui_kit::test]
fn a_reversed_rail_toggle_does_not_jump(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    toggle_rail(cx, &shell);
    frame(cx, &shell);
    wait(cx, 150);
    frame(cx, &shell);
    let closing = tree_width(cx, &shell);
    assert!(
        closing > 1000. - RAIL + 1. && closing < 1000. - 1.,
        "{closing}"
    );
    toggle_rail(cx, &shell);
    frame(cx, &shell);
    let reversed = tree_width(cx, &shell);
    assert!(
        (reversed - closing).abs() < 1.0,
        "{reversed} after {closing}"
    );
}

#[gpui_kit::test]
fn a_resting_rail_fills_its_frame_at_any_scale(cx: &mut TestAppContext) {
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let shell = open_animated(cx);
        cx.simulate_window_scale_factor_change(shell.handle, scale);
        resize(cx, &shell, 1001.);
        frame(cx, &shell);
        wait(cx, 400);
        frame(cx, &shell);
        cx.update_window(shell.handle, |_, window, _| {
            let frame = window.find("rail").bounds();
            let content = window.find("rail-content").bounds();
            let gap = f32::from(frame.right()) - f32::from(content.right());
            assert!(gap.abs() < 0.01, "scale {scale}: {gap} px apart");
            let tall = f32::from(frame.size.height) - f32::from(content.size.height);
            assert!(tall.abs() < 0.01, "scale {scale}: {tall} px apart");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_rail_position_change_is_instant(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, _, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.set_rail(RailPosition::Right, cx))
    })
    .unwrap();
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, window, _| {
        let rail = window.find("rail").bounds();
        assert_eq!(f32::from(rail.right()), 1000.);
        assert_eq!(f32::from(rail.size.width), RAIL);
    })
    .unwrap();
}

// ---- The tab strip.

use kimun_core::nfs::VaultPath;
use tokio::runtime::Runtime;

use super::start_tests::{drive, open_started_motion, runtime, vault_start};
use super::tab_strip::{TAB_NEW_ID, TAB_STRIP_ID};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

/// A window on a vault with two notes, motion on, showing `a.md` in one
/// tab. A tab command needs a vault.
fn open_vault_shell(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Shell) {
    let v = rt.block_on(test_vault(&[("a.md", "alpha"), ("b.md", "beta")], &[]));
    let started = open_started_motion(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
        false,
        false,
    );
    drive(rt, cx, |cx| started.text(cx) == "alpha");
    let shell = Shell {
        handle: started.handle,
        workspace: started.workspace.clone(),
    };
    frame(cx, &shell);
    (v, shell)
}

fn new_tab(cx: &mut TestAppContext, shell: &Shell) {
    cx.update_window(shell.handle, |_, _, cx| {
        shell.workspace.update(cx, |w, cx| w.new_tab(cx))
    })
    .unwrap();
    cx.run_until_parked();
}

/// Close the last tab of the region.
fn close_last_tab(cx: &mut TestAppContext, shell: &Shell) {
    cx.update_window(shell.handle, |_, _, cx| {
        shell.workspace.update(cx, |w, cx| {
            let tab = *w.layout.region().tabs().last().unwrap();
            w.close_tab(tab, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

fn tab_count(cx: &mut TestAppContext, shell: &Shell) -> usize {
    cx.read(|cx| shell.workspace.read(cx).layout.region().tabs().len())
}

/// The height of the tab strip, `None` while it is not drawn.
pub(super) fn strip_height_of(cx: &mut TestAppContext, shell: &Shell) -> Option<f32> {
    strip_height(cx, shell)
}

fn strip_height(cx: &mut TestAppContext, shell: &Shell) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(TAB_STRIP_ID)
            .map(|strip| f32::from(strip.bounds().size.height))
    })
    .unwrap()
}

/// Open a second tab and let the strip come to rest.
fn open_second_tab(cx: &mut TestAppContext, shell: &Shell) {
    new_tab(cx, shell);
    frame(cx, shell);
    wait(cx, 400);
    frame(cx, shell);
    assert!(strip_height(cx, shell).is_some(), "the strip is up");
}

#[gpui_kit::test]
fn a_window_with_one_tab_draws_no_strip(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    assert_eq!(strip_height(cx, &shell), None);
}

#[gpui_kit::test]
fn the_second_tab_opens_the_strip_over_time(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    new_tab(cx, &shell);
    frame(cx, &shell);
    let start = strip_height(cx, &shell).expect("the opening strip is drawn");
    assert!(start < 1.0, "at 0 ms: {start}");
    wait(cx, 30);
    frame(cx, &shell);
    let middle = strip_height(cx, &shell).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let rest = strip_height(cx, &shell).unwrap();
    assert!(rest > 20.0, "at rest: {rest}");
    assert!(
        middle > 1.0 && middle < rest - 1.0,
        "at 30 ms: {middle} of {rest}"
    );
}

#[gpui_kit::test]
fn closing_to_one_tab_closes_the_strip_over_time(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    open_second_tab(cx, &shell);
    let rest = strip_height(cx, &shell).unwrap();
    close_last_tab(cx, &shell);
    assert_eq!(tab_count(cx, &shell), 1);
    frame(cx, &shell);
    wait(cx, 100);
    frame(cx, &shell);
    let middle = strip_height(cx, &shell).expect("the leaving strip is still drawn");
    assert!(
        middle > 0.0 && middle < rest - 1.0,
        "at 100 ms: {middle} of {rest}"
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(strip_height(cx, &shell), None);
}

/// Click the strip's plus button: how many tabs there are then.
fn tabs_after_plus_click(cx: &mut TestAppContext, shell: &Shell) -> usize {
    cx.update_window(shell.handle, |_, window, cx| {
        window.click(TAB_NEW_ID, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    tab_count(cx, shell)
}

#[gpui_kit::test]
fn a_resting_strip_takes_a_click(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    open_second_tab(cx, &shell);
    assert_eq!(tabs_after_plus_click(cx, &shell), 3);
}

#[gpui_kit::test]
fn a_leaving_strip_takes_no_click(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    open_second_tab(cx, &shell);
    close_last_tab(cx, &shell);
    frame(cx, &shell);
    // The content fades and the frame is whole: the button is in view.
    wait(cx, 40);
    frame(cx, &shell);
    assert!(strip_height(cx, &shell).is_some(), "still leaving");
    assert_eq!(
        tabs_after_plus_click(cx, &shell),
        1,
        "the leaving strip took a click"
    );
}

#[gpui_kit::test]
fn a_resting_strip_fills_its_frame_at_any_scale(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let (_v, shell) = open_vault_shell(cx, &rt);
        cx.simulate_window_scale_factor_change(shell.handle, scale);
        resize(cx, &shell, 1001.);
        frame(cx, &shell);
        open_second_tab(cx, &shell);
        cx.update_window(shell.handle, |_, window, _| {
            let outer = window.find(TAB_STRIP_ID).bounds();
            let inner = window.find("tab-strip-content").bounds();
            let tall = f32::from(outer.size.height) - f32::from(inner.size.height);
            assert!(tall.abs() < 0.01, "scale {scale}: {tall} px apart");
        })
        .unwrap();
    }
}

// ---- The tab labels.

use super::tab_strip::{tab_box_id, tab_label_id};

/// Close the tab at `index` of the region.
fn close_tab_at(cx: &mut TestAppContext, shell: &Shell, index: usize) {
    cx.update_window(shell.handle, |_, _, cx| {
        shell.workspace.update(cx, |w, cx| {
            let tab = w.layout.region().tabs()[index];
            w.close_tab(tab, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

/// The width of the clip box of the label at `at`, `None` while it is not
/// drawn.
fn box_width(cx: &mut TestAppContext, shell: &Shell, at: usize) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(tab_box_id(at))
            .map(|clip| f32::from(clip.bounds().size.width))
    })
    .unwrap()
}

/// A window with three tabs and everything at rest.
fn open_three_tabs(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Shell) {
    let (v, shell) = open_vault_shell(cx, rt);
    open_second_tab(cx, &shell);
    new_tab(cx, &shell);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(tab_count(cx, &shell), 3);
    (v, shell)
}

#[gpui_kit::test]
fn a_new_tab_label_grows_in(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    open_second_tab(cx, &shell);
    new_tab(cx, &shell);
    frame(cx, &shell);
    let start = box_width(cx, &shell, 2).expect("the entering label is drawn");
    assert!(start < 1.0, "at 0 ms: {start}");
    wait(cx, 5);
    frame(cx, &shell);
    let middle = box_width(cx, &shell, 2).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let rest = box_width(cx, &shell, 2).unwrap();
    assert!(rest > 20.0, "at rest: {rest}");
    assert!(
        middle > 1.0 && middle < rest - 1.0,
        "at 5 ms: {middle} of {rest}"
    );
}

#[gpui_kit::test]
fn the_first_labels_do_not_animate_when_the_strip_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    new_tab(cx, &shell);
    frame(cx, &shell);
    wait(cx, 5);
    frame(cx, &shell);
    let early = [box_width(cx, &shell, 0), box_width(cx, &shell, 1)];
    wait(cx, 400);
    frame(cx, &shell);
    let rest = [box_width(cx, &shell, 0), box_width(cx, &shell, 1)];
    for (early, rest) in early.into_iter().zip(rest) {
        let (early, rest) = (early.expect("drawn"), rest.expect("drawn"));
        assert!(
            (early - rest).abs() < 0.5,
            "{early} while the strip opens, {rest} at rest"
        );
    }
}

#[gpui_kit::test]
fn a_closed_tab_label_leaves_over_time(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_three_tabs(cx, &rt);
    close_tab_at(cx, &shell, 1);
    assert_eq!(tab_count(cx, &shell), 2);
    frame(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    assert!(
        box_width(cx, &shell, 2).is_some(),
        "three labels are still drawn"
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert!(box_width(cx, &shell, 1).is_some());
    assert_eq!(box_width(cx, &shell, 2), None, "two labels at rest");
}

#[gpui_kit::test]
fn a_leaving_label_takes_no_click(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_three_tabs(cx, &rt);
    close_tab_at(cx, &shell, 1);
    frame(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    let shown = cx.read(|cx| shell.workspace.read(cx).layout.region().shown());
    cx.update_window(shell.handle, |_, window, cx| {
        window.click(tab_label_id(1), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let after = cx.read(|cx| shell.workspace.read(cx).layout.region().shown());
    assert_eq!(after, shown, "the leaving label took a click");
}

#[gpui_kit::test]
fn resting_labels_fill_their_boxes_at_any_scale(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let (_v, shell) = open_vault_shell(cx, &rt);
        cx.simulate_window_scale_factor_change(shell.handle, scale);
        resize(cx, &shell, 1001.);
        frame(cx, &shell);
        open_second_tab(cx, &shell);
        cx.update_window(shell.handle, |_, window, _| {
            for at in 0..2 {
                let outer = window.find(tab_box_id(at)).bounds();
                let inner = window.find(tab_label_id(at)).bounds();
                let wide = f32::from(outer.size.width) - f32::from(inner.size.width);
                assert!(
                    wide.abs() < 0.01,
                    "scale {scale}, label {at}: {wide} px apart"
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_new_label_is_scrolled_into_view_in_a_crowded_strip(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    resize(cx, &shell, 700.);
    frame(cx, &shell);
    for _ in 0..9 {
        new_tab(cx, &shell);
        frame(cx, &shell);
        wait(cx, 400);
        frame(cx, &shell);
    }
    let count = tab_count(cx, &shell);
    cx.update_window(shell.handle, |_, window, _| {
        let strip = window.find("tab-strip-labels").bounds();
        let last = window.find(tab_box_id(count - 2)).bounds();
        assert!(
            f32::from(last.right()) <= f32::from(strip.right()) + 0.5,
            "the strip must overflow for this test: {last:?} in {strip:?}"
        );
    })
    .unwrap();
    // One more tab, with motion on: its label enters from nothing.
    new_tab(cx, &shell);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, window, _| {
        let strip = window.find("tab-strip-labels").bounds();
        let last = window.find(tab_box_id(count)).bounds();
        assert!(
            f32::from(last.right()) <= f32::from(strip.right()) + 0.5
                && f32::from(last.left()) >= f32::from(strip.left()) - 0.5,
            "the shown tab's label is out of view: {last:?} in {strip:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn replacing_the_whole_region_is_drawn_in_place(cx: &mut TestAppContext) {
    // A workspace switch replaces the region: the strip of the old one does
    // not close over time.
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    open_second_tab(cx, &shell);
    cx.update_window(shell.handle, |_, _, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.replace_region(kimun_notes::logic::editor_region::EditorRegion::new(), cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_count(cx, &shell), 1);
    frame(cx, &shell);
    assert_eq!(strip_height(cx, &shell), None, "the strip closed at once");
}

#[gpui_kit::test]
fn a_rail_position_change_during_a_toggle_ends_the_motion(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    toggle_rail(cx, &shell);
    frame(cx, &shell);
    wait(cx, 100);
    frame(cx, &shell);
    assert!(
        rail_width(cx, &shell).is_some(),
        "the rail is still leaving"
    );
    cx.update_window(shell.handle, |_, _, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.set_rail(RailPosition::Right, cx))
    })
    .unwrap();
    frame(cx, &shell);
    assert_eq!(rail_width(cx, &shell), None, "the position change snapped");
}

// ---- The strip supplies its own labels (consolidation).

#[gpui_kit::test]
fn a_tab_closed_before_the_strip_was_drawn_leaves_no_label(cx: &mut TestAppContext) {
    // A tab opened and closed between two frames was never drawn, so it
    // has no snapshot: nothing leaves, and nothing panics.
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    new_tab(cx, &shell);
    close_last_tab(cx, &shell);
    assert_eq!(tab_count(cx, &shell), 1);
    frame(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    assert_eq!(box_width(cx, &shell, 0), None);
    assert_eq!(box_width(cx, &shell, 1), None);
    assert_eq!(strip_height(cx, &shell), None);
}

/// The spoken label of the pill at `at`, `None` while it is not drawn.
fn pill_label(cx: &mut TestAppContext, shell: &Shell, at: usize) -> Option<String> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(tab_label_id(at))
            .and_then(|pill| pill.label().map(str::to_string))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_closed_label_still_leaves_with_its_own_text(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_three_tabs(cx, &rt);
    let before = pill_label(cx, &shell, 1).expect("the middle label is drawn");
    close_tab_at(cx, &shell, 1);
    frame(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    assert_eq!(
        pill_label(cx, &shell, 1).as_deref(),
        Some(before.as_str()),
        "the leaving label keeps the text of the tab it was"
    );
}

#[gpui_kit::test]
fn two_closed_labels_leave_in_order(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_three_tabs(cx, &rt);
    new_tab(cx, &shell);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(tab_count(cx, &shell), 4);
    // Close the second tab, then, 60 ms later, the one that took its place.
    close_tab_at(cx, &shell, 1);
    frame(cx, &shell);
    wait(cx, 60);
    frame(cx, &shell);
    close_tab_at(cx, &shell, 1);
    frame(cx, &shell);
    // At 100 ms both labels are leaving, side by side at the same place.
    // The first to go is further along, so it is the narrower one, and it
    // comes first. The last tab is whole.
    wait(cx, 40);
    frame(cx, &shell);
    let widths: Vec<f32> = (0..4)
        .map(|at| box_width(cx, &shell, at).expect("four labels are drawn"))
        .collect();
    assert!(
        widths[1] < widths[2] - 1.0,
        "the first label to leave comes first: {widths:?}"
    );
}

// ---- Labels that leave keep their place among the rest.

/// A window on a vault of four notes, each open in a tab of its own, with
/// motion on and everything at rest.
fn open_four_note_tabs(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Shell) {
    let notes = [("a.md", "a"), ("b.md", "b"), ("c.md", "c"), ("d.md", "d")];
    let v = rt.block_on(test_vault(&notes, &[]));
    let started = open_started_motion(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
        false,
        false,
    );
    drive(rt, cx, |cx| started.text(cx) == "a");
    for name in ["b.md", "c.md", "d.md"] {
        cx.update(|cx| {
            started
                .workspace
                .update(cx, |w, cx| w.open_note_in_new_tab(VaultPath::new(name), cx))
        });
    }
    drive(rt, cx, |cx| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.layout.region().tabs().len() == 4
                && w.layout.region().all_groups().into_iter().all(|group| {
                    w.state(group)
                        .and_then(|s| s.notes.as_ref())
                        .is_some_and(|h| {
                            !h.is_switching() && !h.is_open_wanted() && !h.session.is_open_pending()
                        })
                })
        })
    });
    let shell = Shell {
        handle: started.handle,
        workspace: started.workspace.clone(),
    };
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    (v, shell)
}

#[gpui_kit::test]
fn labels_closed_apart_keep_the_order_of_the_rest(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_four_note_tabs(cx, &rt);
    let before: Vec<String> = (0..4)
        .map(|at| pill_label(cx, &shell, at).expect("four labels are drawn"))
        .collect();
    let mut sorted = before.clone();
    sorted.sort();
    assert_eq!(before, sorted, "the labels read in order: {before:?}");
    // Close the first tab, then, 60 ms later, the one now in second place
    // (the third of the four): both are leaving at 100 ms. A tab that holds
    // a note flushes it first, so the runtime is driven until the close.
    let close = |cx: &mut TestAppContext, at: usize, left: usize| {
        cx.update_window(shell.handle, |_, _, cx| {
            shell.workspace.update(cx, |w, cx| {
                let tab = w.layout.region().tabs()[at];
                w.close_tab(tab, cx)
            })
        })
        .unwrap();
        drive(&rt, cx, |cx| tab_count(cx, &shell) == left);
    };
    close(cx, 0, 3);
    frame(cx, &shell);
    wait(cx, 60);
    frame(cx, &shell);
    close(cx, 1, 2);
    frame(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    let during: Vec<String> = (0..4)
        .map(|at| pill_label(cx, &shell, at).expect("four labels are still drawn"))
        .collect();
    assert_eq!(during, before, "each label keeps its place while two leave");
}
