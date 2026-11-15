//! The focus indicator fades (ADR-0060): the border of a surface, the
//! title of a slot and the focus bar of the rail change color over time
//! when focus moves. Real windows with motion on, time stepped by the
//! executor clock.
#![cfg(test)]

use gpui_kit::{AppContext as _, Rgba, TestAppContext};
use kimun_notes::logic::layout_tree::{Focus, Leaf, SlotId};

use super::motion::FocusTarget;
use super::motion_tests::{frame, open_files_and_outline, wait};
use super::tests::Shell;
use crate::theme;

/// The focus amount of `focus` as the last frame drew it.
fn amount(cx: &TestAppContext, shell: &Shell, focus: Focus) -> f32 {
    cx.read(|cx| {
        shell
            .workspace
            .read(cx)
            .focus_amount(FocusTarget::Surface(focus))
    })
}

/// Move the model's focus to `focus`. The keyboard follows at once.
fn focus(cx: &mut TestAppContext, shell: &Shell, to: Focus) {
    cx.update_window(shell.handle, |_, _, cx| {
        shell.workspace.update(cx, |w, _| w.layout.focus(to))
    })
    .unwrap();
}

fn files_focus(cx: &TestAppContext, shell: &Shell) -> Focus {
    Focus::Leaf(Leaf::Slot(
        cx.read(|cx| shell.workspace.read(cx).layout.slots()[0]),
    ))
}

const EDITOR: Focus = Focus::Leaf(Leaf::Editor);

/// FILES focused and at rest.
fn open_focused_on_files(cx: &mut TestAppContext) -> (Shell, Focus) {
    let shell = open_files_and_outline(cx);
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    (shell, files)
}

#[gpui_kit::test]
fn focus_moving_between_slots_moves_two_amounts(cx: &mut TestAppContext) {
    let (shell, files) = open_focused_on_files(cx);
    assert_eq!(amount(cx, &shell, files), 1.0);
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    assert!(
        (amount(cx, &shell, files) - 1.0).abs() < 0.05,
        "starts as it was"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let (old, new) = (amount(cx, &shell, files), amount(cx, &shell, EDITOR));
    assert!(old > 0.0 && old < 1.0, "the old focus fades out: {old}");
    assert!(new > 0.0 && new < 1.0, "the new focus fades in: {new}");
    assert!(
        (old + new - 1.0).abs() < 0.05,
        "together about one: {old} {new}"
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(amount(cx, &shell, files), 0.0);
    assert_eq!(amount(cx, &shell, EDITOR), 1.0);
}

#[gpui_kit::test]
fn a_focus_that_goes_back_does_not_jump(cx: &mut TestAppContext) {
    let (shell, files) = open_focused_on_files(cx);
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    wait(cx, 60);
    frame(cx, &shell);
    let before = amount(cx, &shell, files);
    focus(cx, &shell, files);
    frame(cx, &shell);
    let after = amount(cx, &shell, files);
    assert!((after - before).abs() < 0.05, "{before} then {after}");
}

#[gpui_kit::test]
fn the_first_frame_draws_the_final_amounts(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let focused = cx.read(|cx| shell.workspace.read(cx).layout.focused());
    assert_eq!(amount(cx, &shell, focused), 1.0);
    let other = if focused == EDITOR {
        Focus::Rail
    } else {
        EDITOR
    };
    assert_eq!(amount(cx, &shell, other), 0.0);
}

// ---- The three signals.

/// The resting and the focus color of the border and the title.
struct Colors {
    divider: Rgba,
    foreground: Rgba,
    accent: Rgba,
}

fn colors(cx: &TestAppContext) -> Colors {
    cx.read(|cx| {
        let t = theme::current(cx);
        Colors {
            divider: theme::divider(&t),
            foreground: theme::foreground(&t),
            accent: theme::focus_border(&t),
        }
    })
}

fn same(a: Rgba, b: Rgba) -> bool {
    [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a]
        .iter()
        .all(|d| d.abs() < 1e-4)
}

/// How far `c` is from `from` toward `to`, from 0 to 1, over all channels.
fn progress(c: Rgba, from: Rgba, to: Rgba) -> f32 {
    let dist = |a: Rgba, b: Rgba| {
        ((a.r - b.r).powi(2) + (a.g - b.g).powi(2) + (a.b - b.b).powi(2) + (a.a - b.a).powi(2))
            .sqrt()
    };
    dist(c, from) / dist(to, from)
}

fn border(cx: &TestAppContext, shell: &Shell, focus: Focus) -> Rgba {
    cx.read(|cx| shell.workspace.read(cx).leaf_border(focus, cx))
}

fn title(cx: &TestAppContext, shell: &Shell, focus: Focus) -> Rgba {
    let Focus::Leaf(Leaf::Slot(slot)) = focus else {
        panic!("a slot has a title");
    };
    cx.read(|cx| shell.workspace.read(cx).slot_title_color(slot, cx))
}

fn bar(cx: &TestAppContext, shell: &Shell) -> Option<Rgba> {
    cx.read(|cx| shell.workspace.read(cx).focus_bar_color(Focus::Rail, cx))
}

#[gpui_kit::test]
fn the_border_fades_between_the_two_colors(cx: &mut TestAppContext) {
    let (shell, files) = open_focused_on_files(cx);
    let c = colors(cx);
    assert!(same(border(cx, &shell, files), c.accent), "at rest");
    assert!(same(border(cx, &shell, EDITOR), c.divider), "at rest");
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    assert!(
        same(border(cx, &shell, files), c.accent),
        "starts as it was"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let old = progress(border(cx, &shell, files), c.accent, c.divider);
    let new = progress(border(cx, &shell, EDITOR), c.divider, c.accent);
    assert!(old > 0.05 && old < 0.95, "the old border fades out: {old}");
    assert!(new > 0.05 && new < 0.95, "the new border fades in: {new}");
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(border(cx, &shell, files), c.divider));
    assert!(same(border(cx, &shell, EDITOR), c.accent));
}

#[gpui_kit::test]
fn the_slot_title_fades_with_its_border(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let files = files_focus(cx, &shell);
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    let c = colors(cx);
    assert!(same(title(cx, &shell, files), c.foreground), "at rest");
    focus(cx, &shell, files);
    frame(cx, &shell);
    assert!(
        same(title(cx, &shell, files), c.foreground),
        "starts as it was"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let now = progress(title(cx, &shell, files), c.foreground, c.accent);
    assert!(now > 0.05 && now < 0.95, "the title fades in: {now}");
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(title(cx, &shell, files), c.accent));
}

#[gpui_kit::test]
fn the_rail_bar_fades_in_and_out(cx: &mut TestAppContext) {
    let (shell, _) = open_focused_on_files(cx);
    let c = colors(cx);
    assert_eq!(bar(cx, &shell), None, "no bar while the rail has no focus");
    focus(cx, &shell, Focus::Rail);
    frame(cx, &shell);
    assert_eq!(bar(cx, &shell), None, "nothing yet on the first frame");
    wait(cx, 30);
    frame(cx, &shell);
    let middle = bar(cx, &shell).expect("the bar is fading in");
    assert!(
        middle.a > 0.0 && middle.a < c.accent.a,
        "alpha {}",
        middle.a
    );
    assert_eq!(
        (middle.r, middle.g, middle.b),
        (c.accent.r, c.accent.g, c.accent.b)
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(bar(cx, &shell).expect("the full bar"), c.accent));
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    assert!(bar(cx, &shell).is_some(), "the bar is fading out");
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(bar(cx, &shell), None, "and gone");
}

#[gpui_kit::test]
fn at_rest_only_the_focused_target_shows_a_focus_color(cx: &mut TestAppContext) {
    let (shell, files) = open_focused_on_files(cx);
    let c = colors(cx);
    assert!(same(border(cx, &shell, files), c.accent));
    assert!(same(border(cx, &shell, EDITOR), c.divider));
    let outline = Focus::Leaf(Leaf::Slot(
        cx.read(|cx| shell.workspace.read(cx).layout.slots()[1]),
    ));
    assert!(same(border(cx, &shell, outline), c.divider));
    assert_eq!(bar(cx, &shell), None);
}

/// FILES, FIND, the editor and OUTLINE, with FIND focused and at rest.
fn open_focused_on_find(cx: &mut TestAppContext) -> (Shell, SlotId) {
    use kimun_notes::logic::layout_tree::Dir;
    use kimun_notes::logic::nav::DrawerView;
    let shell = open_files_and_outline(cx);
    let files = cx.read(|cx| shell.workspace.read(cx).layout.slots()[0]);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.place_new_slot(DrawerView::Find, Leaf::Slot(files), Dir::Right, window, cx)
                .expect("room for a new column");
        })
    })
    .unwrap();
    let find = cx.read(|cx| shell.workspace.read(cx).layout.slots()[1]);
    focus(cx, &shell, Focus::Leaf(Leaf::Slot(find)));
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    (shell, find)
}

#[gpui_kit::test]
fn closing_a_focused_slot_hands_the_focus_to_a_neighbor_that_fades_in(cx: &mut TestAppContext) {
    // The model moves the focus to a surviving slot and removes the closed
    // one, which is drawn as a spacer with no border. The survivor's border
    // fades in over the quick speed.
    let (shell, find) = open_focused_on_find(cx);
    let c = colors(cx);
    let closed = Focus::Leaf(Leaf::Slot(find));
    assert!(same(border(cx, &shell, closed), c.accent), "at rest");
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_slot(find, window, cx))
    })
    .unwrap();
    let (gone, survivor) = cx.read(|cx| {
        let w = shell.workspace.read(cx);
        (!w.layout.slots().contains(&find), w.layout.focused())
    });
    assert!(gone, "the slot left the tree");
    assert_ne!(survivor, closed, "the focus moved off the closed slot");
    frame(cx, &shell);
    assert!(
        same(border(cx, &shell, closed), c.divider),
        "no border on a removed slot"
    );
    assert!(
        amount(cx, &shell, survivor) < 0.2,
        "the survivor starts unlit"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let now = amount(cx, &shell, survivor);
    assert!(now > 0.05 && now < 0.95, "the survivor fades in: {now}");
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(amount(cx, &shell, survivor), 1.0);
}

#[gpui_kit::test]
fn hiding_a_focused_slot_fades_its_border_out_while_it_closes(cx: &mut TestAppContext) {
    use super::motion_tests::toggle_left;
    use crate::workspace::tests::open_animated;
    let shell = open_animated(cx);
    frame(cx, &shell);
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    let c = colors(cx);
    assert!(same(border(cx, &shell, files), c.accent), "at rest");
    toggle_left(cx, &shell);
    let still_there = cx.read(|cx| {
        let w = shell.workspace.read(cx);
        w.layout.slots().len() == 2 && w.layout.focused() != files
    });
    assert!(
        still_there,
        "the slot stays in the tree, hidden, and the focus left it"
    );
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    let now = progress(border(cx, &shell, files), c.accent, c.divider);
    assert!(
        now > 0.05 && now < 0.95,
        "the closing slot's border fades out: {now}"
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(border(cx, &shell, files), c.divider));
}

// ---- One rule for every focus signal (the target list, the one table).

#[gpui_kit::test]
fn a_new_focused_slot_fades_its_border_in(cx: &mut TestAppContext) {
    use kimun_notes::logic::layout_tree::Dir;
    use kimun_notes::logic::nav::DrawerView;
    let shell = open_files_and_outline(cx);
    let c = colors(cx);
    let files = cx.read(|cx| shell.workspace.read(cx).layout.slots()[0]);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.place_new_slot(DrawerView::Links, Leaf::Slot(files), Dir::Right, window, cx)
                .expect("room for a new column");
        })
    })
    .unwrap();
    let new = cx.read(|cx| shell.workspace.read(cx).layout.slots()[1]);
    assert_ne!(new, files, "a second slot stands beside FILES");
    let new = Focus::Leaf(Leaf::Slot(new));
    focus(cx, &shell, new);
    frame(cx, &shell);
    assert!(same(border(cx, &shell, new), c.divider), "it starts unlit");
    wait(cx, 30);
    frame(cx, &shell);
    let now = progress(border(cx, &shell, new), c.divider, c.accent);
    assert!(now > 0.05 && now < 0.95, "its border fades in: {now}");
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(border(cx, &shell, new), c.accent));
}

#[gpui_kit::test]
fn a_hidden_slot_keeps_its_focus_key_so_it_is_never_new(cx: &mut TestAppContext) {
    use super::motion::MotionKey;
    use super::motion_tests::toggle_left;
    use crate::workspace::tests::open_animated;
    let shell = open_animated(cx);
    frame(cx, &shell);
    let files = files_focus(cx, &shell);
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    toggle_left(cx, &shell);
    for millis in [0, 30, 400] {
        wait(cx, millis);
        frame(cx, &shell);
        assert_eq!(amount(cx, &shell, files), 0.0, "hidden, after {millis} ms");
        let held = cx.read(|cx| {
            shell
                .workspace
                .read(cx)
                .motion
                .borrow()
                .try_presence(MotionKey::Focus(FocusTarget::Surface(files)))
                .is_some()
        });
        assert!(held, "the store still holds its key after {millis} ms");
    }
}

#[gpui_kit::test]
fn the_target_list_names_every_slot_group_and_rail_view(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        let targets = w.focus_targets();
        let count =
            |keep: &dyn Fn(&FocusTarget) -> bool| targets.iter().filter(|t| keep(t)).count();
        assert_eq!(count(&|t| *t == FocusTarget::Surface(Focus::Rail)), 1);
        assert_eq!(count(&|t| *t == FocusTarget::Surface(EDITOR)), 1);
        for slot in w.layout.slots() {
            let target = FocusTarget::Surface(Focus::Leaf(Leaf::Slot(slot)));
            assert_eq!(count(&|t| *t == target), 1, "{slot:?}");
        }
        let groups = w.layout.region().all_groups();
        assert_eq!(count(&|t| matches!(t, FocusTarget::Title(_))), groups.len());
        assert_eq!(
            count(&|t| matches!(t, FocusTarget::Marker(_))),
            groups.len()
        );
        assert_eq!(
            count(&|t| matches!(t, FocusTarget::RailCursor(_))),
            w.rail_views().len()
        );
    });
}

#[gpui_kit::test]
fn is_focused_follows_the_model_for_a_surface(cx: &mut TestAppContext) {
    let (shell, files) = open_focused_on_files(cx);
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        assert!(w.is_focused(FocusTarget::Surface(files)));
        assert!(!w.is_focused(FocusTarget::Surface(EDITOR)));
        assert!(!w.is_focused(FocusTarget::Surface(Focus::Rail)));
    });
}

// ---- The editor group's title and target marker.

use kimun_notes::logic::layout_tree::{Axis, GroupId};

use super::motion_chrome_tests::{new_tab, open_vault_shell};
use super::start_tests::{drive, runtime};

fn group_title(cx: &TestAppContext, shell: &Shell, group: GroupId) -> Rgba {
    cx.read(|cx| shell.workspace.read(cx).group_title_color(group, cx))
}

fn marker(cx: &TestAppContext, shell: &Shell, group: GroupId) -> Option<Rgba> {
    cx.read(|cx| shell.workspace.read(cx).target_marker_color(group, cx))
}

/// A window on a vault with two editor groups in the one tab: the first is
/// active. The editor has the keyboard and everything is at rest.
fn open_two_groups(
    cx: &mut TestAppContext,
    rt: &tokio::runtime::Runtime,
) -> (crate::test_vault::TestVault, Shell, GroupId, GroupId) {
    let (v, shell) = open_vault_shell(cx, rt);
    let (first, second) = cx.update(|cx| {
        shell.workspace.update(cx, |w, cx| {
            let first = w.layout.active_group();
            let second = w
                .layout
                .region_mut()
                .split_group(first, Axis::Horizontal)
                .expect("the first group splits");
            w.add_group_for_test(second, Some(v.settings.clone()), cx);
            (first, second)
        })
    });
    assert_ne!(first, second, "the tab holds two groups");
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    (v, shell, first, second)
}

#[gpui_kit::test]
fn the_group_title_fades_with_the_editor_focus(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell, first, _second) = open_two_groups(cx, &rt);
    let c = colors(cx);
    assert!(same(group_title(cx, &shell, first), c.accent), "at rest");
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    let now = progress(group_title(cx, &shell, first), c.accent, c.foreground);
    assert!(now > 0.05 && now < 0.95, "the title fades out: {now}");
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(group_title(cx, &shell, first), c.foreground));
}

#[gpui_kit::test]
fn the_group_marker_fades_in_when_the_keyboard_leaves_the_editor(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell, first, _second) = open_two_groups(cx, &rt);
    let c = colors(cx);
    assert_eq!(
        marker(cx, &shell, first),
        None,
        "no marker while the editor has the keyboard"
    );
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    assert_eq!(
        marker(cx, &shell, first),
        None,
        "nothing yet on the first frame"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let middle = marker(cx, &shell, first).expect("the marker is fading in");
    assert!(
        middle.a > 0.0 && middle.a < c.accent.a,
        "alpha {}",
        middle.a
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(
        marker(cx, &shell, first).expect("the full marker"),
        c.accent
    ));
}

#[gpui_kit::test]
fn the_group_marker_fades_out_when_the_keyboard_returns(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell, first, _second) = open_two_groups(cx, &rt);
    let c = colors(cx);
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    assert!(
        same(marker(cx, &shell, first).expect("lit"), c.accent),
        "at rest"
    );
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    let middle = marker(cx, &shell, first).expect("the marker is fading out");
    assert!(
        middle.a > 0.0 && middle.a < c.accent.a,
        "alpha {}",
        middle.a
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(marker(cx, &shell, first), None);
}

#[gpui_kit::test]
fn switching_the_shown_tab_fades_no_group_signal(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell) = open_vault_shell(cx, &rt);
    new_tab(cx, &shell);
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    let c = colors(cx);
    let (groups, first_tab) = cx.read(|cx| {
        let region = shell.workspace.read(cx).layout.region();
        (region.all_groups(), region.tabs()[0])
    });
    assert_eq!(groups.len(), 2, "one group in each of two tabs");
    cx.update_window(shell.handle, |_, _, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.show_tab(first_tab, cx))
    })
    .unwrap();
    for millis in [0, 30, 400] {
        wait(cx, millis);
        frame(cx, &shell);
        for group in &groups {
            assert!(
                same(group_title(cx, &shell, *group), c.accent),
                "after {millis} ms the title of {group:?} still shows the focus"
            );
        }
    }
}

#[gpui_kit::test]
fn a_closed_group_leaves_no_marker(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell, first, _second) = open_two_groups(cx, &rt);
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    assert!(marker(cx, &shell, first).is_some(), "lit before the close");
    cx.update_window(shell.handle, |_, _, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_editor_group(first, cx))
    })
    .unwrap();
    drive(&rt, cx, |cx| {
        cx.read(|cx| shell.workspace.read(cx).layout.region().all_groups().len() == 1)
    });
    frame(cx, &shell);
    assert_eq!(
        marker(cx, &shell, first),
        None,
        "the closed group has no marker"
    );
}

#[gpui_kit::test]
fn a_zoom_changes_the_marker_at_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, shell, first, _second) = open_two_groups(cx, &rt);
    let files = files_focus(cx, &shell);
    focus(cx, &shell, files);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    assert!(marker(cx, &shell, first).is_some(), "lit before the zoom");
    // The model zooms the leaf that has the keyboard.
    cx.update_window(shell.handle, |_, _, cx| {
        shell.workspace.update(cx, |w, cx| {
            let slot = w.layout.slots()[0];
            w.layout.zoom(Some(Leaf::Slot(slot)));
            cx.notify();
        })
    })
    .unwrap();
    assert!(
        cx.read(|cx| shell.workspace.read(cx).layout.zoomed().is_some()),
        "the slot is zoomed"
    );
    frame(cx, &shell);
    assert_eq!(
        marker(cx, &shell, first),
        None,
        "a zoom takes the marker off at once"
    );
}

// ---- The rail cell cursor.

use kimun_notes::logic::nav::DrawerView;

fn ring(cx: &TestAppContext, shell: &Shell, view: DrawerView) -> Option<Rgba> {
    cx.read(|cx| shell.workspace.read(cx).rail_cursor_ring(view, cx))
}

fn glyph(cx: &TestAppContext, shell: &Shell, view: DrawerView, active: bool) -> Rgba {
    cx.read(|cx| shell.workspace.read(cx).rail_glyph_color(view, active, cx))
}

/// The rail has the keyboard, and the cursor is on `index`. Draws a frame.
fn put_cursor(cx: &mut TestAppContext, shell: &Shell, index: usize) {
    cx.update_window(shell.handle, |_, _, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.layout.focus(Focus::Rail);
            w.rail_cursor = index;
            cx.notify();
        })
    })
    .unwrap();
    frame(cx, shell);
}

fn rail_views(cx: &TestAppContext, shell: &Shell) -> Vec<DrawerView> {
    cx.read(|cx| shell.workspace.read(cx).rail_views().to_vec())
}

#[gpui_kit::test]
fn the_rail_cursor_ring_follows_the_arrow_keys(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let views = rail_views(cx, &shell);
    assert!(views.len() >= 2, "the rail has two views to walk between");
    let c = colors(cx);
    put_cursor(cx, &shell, 0);
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(
        ring(cx, &shell, views[0]).expect("the cursor ring"),
        c.accent
    ));
    put_cursor(cx, &shell, 1);
    wait(cx, 30);
    frame(cx, &shell);
    let old = ring(cx, &shell, views[0]).expect("the old ring fades out");
    let new = ring(cx, &shell, views[1]).expect("the new ring fades in");
    assert!(old.a > 0.0 && old.a < c.accent.a, "old alpha {}", old.a);
    assert!(new.a > 0.0 && new.a < c.accent.a, "new alpha {}", new.a);
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(ring(cx, &shell, views[0]), None);
    assert!(same(
        ring(cx, &shell, views[1]).expect("the full ring"),
        c.accent
    ));
}

#[gpui_kit::test]
fn the_rail_glyph_fades_between_muted_and_bright(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let views = rail_views(cx, &shell);
    let hidden = cx.read(|cx| {
        let w = shell.workspace.read(cx);
        views.iter().position(|v| !w.is_shown(*v))
    });
    let at = hidden.expect("a view that is not shown");
    let shown = cx.read(|cx| {
        let w = shell.workspace.read(cx);
        views.iter().copied().find(|v| w.is_shown(*v))
    });
    let (muted, bright, accent) = cx.read(|cx| {
        let t = theme::current(cx);
        (theme::muted(&t), theme::bright(&t), theme::focus_border(&t))
    });
    // The cursor starts on a view other than `at`.
    put_cursor(cx, &shell, if at == 0 { 1 } else { 0 });
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(glyph(cx, &shell, views[at], false), muted), "at rest");
    put_cursor(cx, &shell, at);
    assert!(
        same(glyph(cx, &shell, views[at], false), muted),
        "starts unlit"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let now = progress(glyph(cx, &shell, views[at], false), muted, bright);
    assert!(
        now > 0.05 && now < 0.95,
        "the glyph fades toward bright: {now}"
    );
    if let Some(shown) = shown {
        assert!(
            same(glyph(cx, &shell, shown, true), accent),
            "a shown view keeps the accent"
        );
    }
    wait(cx, 400);
    frame(cx, &shell);
    assert!(same(glyph(cx, &shell, views[at], false), bright));
}

#[gpui_kit::test]
fn a_rail_with_no_focus_draws_no_cursor(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    focus(cx, &shell, EDITOR);
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    for view in rail_views(cx, &shell) {
        assert_eq!(ring(cx, &shell, view), None, "{view:?}");
    }
}
