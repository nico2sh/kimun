//! Slice three of GUI motion (ADR-0060): a cell that changes size moves
//! over time, and a view that moves is drawn once. Real windows with
//! motion on, time stepped by the executor clock.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use kimun_notes::logic::layout_tree::{Dir, Leaf};
use kimun_notes::logic::nav::DrawerView;

use super::motion_tests::{frame, open_files_and_outline, resize, wait};
use super::palette::PaletteRun;
use super::presets::PresetCommand;
use super::tests::Shell;
use super::tree_view::DividerAt;
use super::{slot_content_id, slot_gone_id, slot_id};

/// The width of the slot showing `view`, `None` while it is not drawn.
fn width(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(slot_id(view))
            .map(|slot| f32::from(slot.bounds().size.width))
    })
    .unwrap()
}

fn files_slot(cx: &TestAppContext, shell: &Shell) -> kimun_notes::logic::layout_tree::SlotId {
    cx.read(|cx| shell.workspace.read(cx).layout.slots()[0])
}

#[gpui_kit::test]
fn a_keyboard_resize_moves_the_divider_over_time(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let before = width(cx, &shell, DrawerView::Files).expect("FILES is drawn");
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.focus_slot(slot, window, cx);
            w.step_focused_leaf(true, window, cx);
        })
    })
    .unwrap();
    frame(cx, &shell);
    let start = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (start - before).abs() < 0.5,
        "at 0 ms: {start} against {before}"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let middle = width(cx, &shell, DrawerView::Files).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let end = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (end - before).abs() > 1.0,
        "the step moved the divider: {before} to {end}"
    );
    let (low, high) = (before.min(end), before.max(end));
    assert!(
        middle > low + 0.5 && middle < high - 0.5,
        "at 30 ms: {middle} between {before} and {end}"
    );
}

#[gpui_kit::test]
fn a_divider_drag_follows_the_pointer_on_every_frame(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let before = width(cx, &shell, DrawerView::Files).unwrap();
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            // The divider after FILES, 40 px to the right of where it is.
            let pixel = 48.0 + f64::from(before) + 40.0;
            w.on_divider_drag(DividerAt::Leaf(Leaf::Slot(slot)), pixel, window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    let after = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        after > before + 20.0,
        "the drag was followed at once: {before} to {after}"
    );
}

#[gpui_kit::test]
fn a_window_resize_moves_no_weight(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    // A weight is moving when the window is resized: the resize ends it.
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.focus_slot(slot, window, cx);
            w.step_focused_leaf(true, window, cx);
        })
    })
    .unwrap();
    frame(cx, &shell);
    wait(cx, 50);
    frame(cx, &shell);
    resize(cx, &shell, 900.);
    frame(cx, &shell);
    let at_once = width(cx, &shell, DrawerView::Files).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let rest = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (at_once - rest).abs() < 0.5,
        "{at_once} at once, {rest} at rest"
    );
}

#[gpui_kit::test]
fn a_new_column_does_not_make_its_neighbor_jump(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let before = width(cx, &shell, DrawerView::Files).unwrap();
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.place_new_slot(DrawerView::Find, Leaf::Slot(slot), Dir::Right, window, cx)
                .expect("room for a new column");
        })
    })
    .unwrap();
    frame(cx, &shell);
    let first = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (first - before).abs() < 1.0,
        "FILES jumped: {before} to {first}"
    );
    wait(cx, 400);
    frame(cx, &shell);
    let files = width(cx, &shell, DrawerView::Files).unwrap();
    let find = width(cx, &shell, DrawerView::Find).expect("the new column is drawn");
    assert!(find > 1.0 && files > 1.0);
    // The sidebar grew to give FIND its minimum: the two columns are equal and
    // together 320 px, for the 230 px of FILES before.
    assert!(
        (files - find).abs() < 1.0,
        "{files} and {find} share equally"
    );
    assert!(
        (files + find - 320.0).abs() < 1.5,
        "{files} and {find} fill the room: 320 px, from {before}"
    );
}

/// The width of the editor panel.
fn editor_width(cx: &mut TestAppContext, shell: &Shell) -> f32 {
    cx.update_window(shell.handle, |_, window, _| {
        f32::from(window.find("editor-panel").bounds().size.width)
    })
    .unwrap()
}

/// A second column beside FILES, at rest: FILES, then FIND, the editor and
/// OUTLINE. Returns the slot of FIND.
fn two_columns(cx: &mut TestAppContext, shell: &Shell) -> kimun_notes::logic::layout_tree::SlotId {
    let slot = files_slot(cx, shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.place_new_slot(DrawerView::Find, Leaf::Slot(slot), Dir::Right, window, cx)
                .expect("room for a new column");
        })
    })
    .unwrap();
    frame(cx, shell);
    wait(cx, 400);
    frame(cx, shell);
    cx.read(|cx| shell.workspace.read(cx).layout.slots()[1])
}

#[gpui_kit::test]
fn closing_a_slot_gives_its_room_back_over_time(cx: &mut TestAppContext) {
    // FILES, FIND, the editor, OUTLINE. FILES is held at its minimum, so the
    // room of FIND goes to the editor and OUTLINE: the editor grows into it.
    let shell = open_files_and_outline(cx);
    let find = two_columns(cx, &shell);
    let files = width(cx, &shell, DrawerView::Files).unwrap();
    let before = editor_width(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_slot(find, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    assert_eq!(
        width(cx, &shell, DrawerView::Find),
        None,
        "the closed slot is not drawn"
    );
    let first = editor_width(cx, &shell);
    assert!(
        (first - before).abs() < 1.0,
        "the editor starts where it was: {before} to {first}"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let middle = editor_width(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    let after = editor_width(cx, &shell);
    assert!(
        after > before + 1.0,
        "the editor takes the room: {before} to {after}"
    );
    assert!(
        middle > before + 0.5 && middle < after - 0.5,
        "at 30 ms: {middle} between {before} and {after}"
    );
    assert_eq!(width(cx, &shell, DrawerView::Files), Some(files));
}

#[gpui_kit::test]
fn applying_a_preset_is_drawn_at_once(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    // A changed layout, at rest: FILES was made wider with the keyboard.
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.focus_slot(slot, window, cx);
            w.step_focused_leaf(true, window, cx);
        })
    })
    .unwrap();
    frame(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.run_palette_row(PaletteRun::Preset(PresetCommand::ResetDefault), window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    let at_once = width(cx, &shell, DrawerView::Files).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let rest = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (at_once - rest).abs() < 0.5,
        "{at_once} at once, {rest} at rest"
    );
}

// ---- A moved view.

/// FILES shown left, OUTLINE placed right but hidden, with motion on.
fn open_files_left_outline_hidden(cx: &mut TestAppContext) -> Shell {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"] },\n]\n");
    let shell = open_shell_motion(cx, 1200., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    shell
}

fn move_files_right(cx: &mut TestAppContext, shell: &Shell) {
    let right = cx.read(|cx| shell.workspace.read(cx).layout.slots()[1]);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.move_into(DrawerView::Files, right, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_moved_views_old_frame_closes_over_time(cx: &mut TestAppContext) {
    let shell = open_files_left_outline_hidden(cx);
    move_files_right(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    let gone = cx
        .update_window(shell.handle, |_, window, _| {
            window
                .try_find(slot_gone_id(DrawerView::Files))
                .map(|frame| f32::from(frame.bounds().size.width))
        })
        .unwrap();
    assert!(
        gone.is_some_and(|width| width > 0.0),
        "the old frame is drawn while it closes: {gone:?}"
    );
    wait(cx, 400);
    frame(cx, &shell);
    assert!(
        cx.update_window(shell.handle, |_, window, _| {
            window.try_find(slot_gone_id(DrawerView::Files)).is_none()
        })
        .unwrap(),
        "the old frame is gone once it closed"
    );
}

#[gpui_kit::test]
fn a_moved_view_is_drawn_once(cx: &mut TestAppContext) {
    let shell = open_files_left_outline_hidden(cx);
    move_files_right(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    // One slot and one content hold the view, and they are on the right.
    let (slots, contents) = cx
        .update_window(shell.handle, |_, window, _| {
            (
                window.find_all(slot_id(DrawerView::Files)),
                window.find_all(slot_content_id(DrawerView::Files)),
            )
        })
        .unwrap();
    assert_eq!(slots.len(), 1, "one slot element for FILES");
    assert_eq!(contents.len(), 1, "one content element for FILES");
    assert!(f32::from(slots[0].bounds().left()) > 600.0, "on the right");
}

#[gpui_kit::test]
fn a_drag_release_does_not_move_the_divider(cx: &mut TestAppContext) {
    // The last pointer move and the release arrive between two frames.
    let shell = open_files_and_outline(cx);
    let before = width(cx, &shell, DrawerView::Files).unwrap();
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            let pixel = 48.0 + f64::from(before) + 40.0;
            w.on_divider_drag(DividerAt::Leaf(Leaf::Slot(slot)), pixel, window, cx);
            w.end_resize(cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    let after = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        after > before + 20.0,
        "the drag was kept: {before} to {after}"
    );
}

// ---- A moved view is drawn once, always.

fn count(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> (usize, usize) {
    cx.update_window(shell.handle, |_, window, _| {
        (
            window.find_all(slot_id(view)).len(),
            window.find_all(slot_content_id(view)).len(),
        )
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_moved_view_hidden_while_the_old_frame_closes_is_drawn_once(cx: &mut TestAppContext) {
    let shell = open_files_left_outline_hidden(cx);
    move_files_right(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, _, cx| {
        shell
            .workspace
            .update(cx, |w, _| w.layout.hide_view(&DrawerView::Files))
    })
    .unwrap();
    frame(cx, &shell);
    let (slots, contents) = count(cx, &shell, DrawerView::Files);
    assert_eq!(slots, 1, "one slot element for FILES");
    assert_eq!(contents, 1, "one content element for FILES");
    let gone = cx
        .update_window(shell.handle, |_, window, _| {
            window.try_find(slot_gone_id(DrawerView::Files)).is_some()
        })
        .unwrap();
    assert!(gone, "and the old frame is blank, closing");
}

#[gpui_kit::test]
fn the_old_frame_stays_blank_when_the_new_slot_is_closed(cx: &mut TestAppContext) {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    // Two slots on the right, so the one FILES moves into can be closed.
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"outline\"], shown = \"outline\" },\n\
           { weight = 0.5, views = [\"links\"] },\n] },\n]\n");
    let shell = open_shell_motion(cx, 1200., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    let target = cx.read(|cx| shell.workspace.read(cx).layout.slots()[2]);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.move_into(DrawerView::Files, target, window, cx);
            w.close_slot(target, window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    wait(cx, 40);
    frame(cx, &shell);
    let gone = cx
        .update_window(shell.handle, |_, window, _| {
            window.try_find(slot_gone_id(DrawerView::Files)).is_some()
        })
        .unwrap();
    assert!(gone, "the old frame is blank and still closing");
    assert_eq!(
        count(cx, &shell, DrawerView::Files).0,
        0,
        "FILES is not drawn"
    );
}

#[gpui_kit::test]
fn a_gone_frame_has_no_role_and_no_label(cx: &mut TestAppContext) {
    let shell = open_files_left_outline_hidden(cx);
    move_files_right(cx, &shell);
    wait(cx, 40);
    frame(cx, &shell);
    let label = cx
        .update_window(shell.handle, |_, window, _| {
            window
                .try_find(slot_gone_id(DrawerView::Files))
                .map(|frame| frame.label().map(str::to_string))
        })
        .unwrap();
    assert_eq!(
        label,
        Some(None),
        "the blank frame is drawn and says nothing"
    );
}

// ---- A column that wraps a cell keeps the cell's place (seeded weights).

/// FILES over FIND on the left, the editor, OUTLINE on the right, in a window
/// 1000 px wide: a second column beside the stack needs room the window must
/// make. Motion on.
fn open_stacked_left(cx: &mut TestAppContext) -> Shell {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n");
    let shell = open_shell_motion(cx, 1000., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    shell
}

#[gpui_kit::test]
fn a_column_beside_a_nested_last_leaf_does_not_make_its_cell_jump(cx: &mut TestAppContext) {
    let shell = open_stacked_left(cx);
    let before = width(cx, &shell, DrawerView::Files).unwrap();
    let find = cx.read(|cx| shell.workspace.read(cx).layout.slots()[1]);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.place_new_slot(DrawerView::Links, Leaf::Slot(find), Dir::Right, window, cx)
                .expect("room for a new column");
        })
    })
    .unwrap();
    frame(cx, &shell);
    let first = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (first - before).abs() < 1.0,
        "the stack jumped: {before} to {first}"
    );
    wait(cx, 400);
    frame(cx, &shell);
    let after = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(after > 1.0, "the stack is drawn at rest: {after}");
}

/// A column placed on `dir` of FILES, in the flat layout: FILES, or the cell
/// that now holds it, keeps its width on the first frame.
fn assert_no_jump_for(cx: &mut TestAppContext, dir: Dir) {
    let shell = open_files_and_outline(cx);
    let before = width(cx, &shell, DrawerView::Files).unwrap();
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.place_new_slot(DrawerView::Find, Leaf::Slot(slot), dir, window, cx)
                .expect("room for a new slot");
        })
    })
    .unwrap();
    frame(cx, &shell);
    let first = width(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (first - before).abs() < 1.0,
        "{dir:?}: FILES jumped from {before} to {first}"
    );
}

#[gpui_kit::test]
fn a_column_placed_to_the_left_does_not_make_its_neighbor_jump(cx: &mut TestAppContext) {
    assert_no_jump_for(cx, Dir::Left);
}

#[gpui_kit::test]
fn a_slot_placed_above_does_not_make_its_neighbor_jump(cx: &mut TestAppContext) {
    assert_no_jump_for(cx, Dir::Up);
}

#[gpui_kit::test]
fn a_slot_placed_below_does_not_make_its_neighbor_jump(cx: &mut TestAppContext) {
    assert_no_jump_for(cx, Dir::Down);
}

// ---- A rollback does not cut a motion.

#[gpui_kit::test]
fn a_failed_placement_does_not_cut_a_motion(cx: &mut TestAppContext) {
    use kimun_notes::logic::layout::Side;
    let shell = open_files_and_outline(cx);
    // OUTLINE is closing: past its content fade, well into its frame.
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.toggle_side(Side::Right, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    wait(cx, 100);
    frame(cx, &shell);
    let during = width(cx, &shell, DrawerView::Outline).expect("OUTLINE is still closing");
    // A layout change that fails and is rolled back: the one rollback every
    // refused placement goes through.
    let refused = cx
        .update_window(shell.handle, |_, _, cx| {
            shell
                .workspace
                .update(cx, |w, _| w.try_layout(|_| Err::<(), &str>("refused")))
        })
        .unwrap();
    assert!(refused.is_err());
    frame(cx, &shell);
    let after = width(cx, &shell, DrawerView::Outline)
        .expect("OUTLINE is still closing after the refused placement");
    assert!(
        (after - during).abs() < 1.0,
        "the motion went on: {during} to {after}"
    );
}

#[gpui_kit::test]
fn a_view_moved_down_past_a_hidden_slot_animates_and_cuts_nothing(cx: &mut TestAppContext) {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    use kimun_notes::logic::layout::Side;
    // The left slot holds FILES and FIND, and a hidden slot lies below it:
    // moving FILES down tries that slot first, is rolled back, and then goes
    // to a new slot beside it. OUTLINE is closing when that happens.
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\", \"find\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"links\"] },\n] },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n");
    let shell = open_shell_motion(cx, 1200., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.toggle_side(Side::Right, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    wait(cx, 100);
    frame(cx, &shell);
    let during = width(cx, &shell, DrawerView::Outline).expect("OUTLINE is closing");
    let slot = files_slot(cx, &shell);
    let moved = cx
        .update_window(shell.handle, |_, window, cx| {
            shell.workspace.update(cx, |w, cx| {
                w.focus_slot(slot, window, cx);
                w.move_view_vertically(false, window, cx)
            })
        })
        .unwrap();
    assert!(moved, "the view went to a new slot");
    frame(cx, &shell);
    let after =
        width(cx, &shell, DrawerView::Outline).expect("OUTLINE is still closing after the move");
    assert!(
        (after - during).abs() < 1.0,
        "the rollback cut the motion: {during} to {after}"
    );
    let height = |cx: &mut TestAppContext| {
        cx.update_window(shell.handle, |_, window, _| {
            window
                .try_find(slot_id(DrawerView::Files))
                .map(|slot| f32::from(slot.bounds().size.height))
        })
        .unwrap()
    };
    let start = height(cx).expect("the new slot is drawn");
    wait(cx, 400);
    frame(cx, &shell);
    let rest = height(cx).expect("and stays");
    assert!(start < rest - 5.0, "it opens over time: {start} to {rest}");
}

#[gpui_kit::test]
fn a_drop_zone_probe_does_not_cut_a_motion(cx: &mut TestAppContext) {
    use kimun_notes::logic::layout::Side;
    let shell = open_files_and_outline(cx);
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.toggle_side(Side::Right, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    wait(cx, 100);
    frame(cx, &shell);
    let during = width(cx, &shell, DrawerView::Outline).expect("OUTLINE is closing");
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, _| {
            // The question a view drag asks of each zone: would it fit?
            w.new_slot_fits(DrawerView::Find, Leaf::Slot(slot), Dir::Right, window);
        })
    })
    .unwrap();
    frame(cx, &shell);
    let after = width(cx, &shell, DrawerView::Outline).expect("OUTLINE is still closing");
    assert!(
        (after - during).abs() < 1.0,
        "the probe cut the motion: {during} to {after}"
    );
}

// ---- One clock for a new column and its neighbor.

/// The editor's width on frames from 0 to 200 ms after a column is placed
/// beside FILES, and its width before and at rest: (before, frames, after).
fn editor_widths_while_a_column_opens(
    cx: &mut TestAppContext,
    window_width: f32,
) -> (f32, Vec<f32>, f32) {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n");
    let shell = open_shell_motion(
        cx,
        window_width,
        &layout,
        "",
        false,
        LayoutStore::Off,
        false,
    );
    frame(cx, &shell);
    let start = editor_width(cx, &shell);
    let slot = files_slot(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.place_new_slot(DrawerView::Find, Leaf::Slot(slot), Dir::Right, window, cx)
                .expect("room for a new column");
        })
    })
    .unwrap();
    frame(cx, &shell);
    let mut widths = vec![editor_width(cx, &shell)];
    let mut at = 0;
    for millis in [20, 40, 60, 80, 100, 140, 200] {
        wait(cx, millis - at);
        at = millis;
        frame(cx, &shell);
        widths.push(editor_width(cx, &shell));
    }
    wait(cx, 400);
    frame(cx, &shell);
    (start, widths, editor_width(cx, &shell))
}

fn assert_no_dip(start: f32, widths: &[f32], end: f32) {
    let (low, high) = (start.min(end) - 0.5, start.max(end) + 0.5);
    for (i, width) in widths.iter().enumerate() {
        assert!(
            *width >= low && *width <= high,
            "frame {i}: the editor is {width}, outside {start} to {end}: {widths:?}"
        );
    }
}

#[gpui_kit::test]
fn the_editor_edge_does_not_dip_while_a_column_opens(cx: &mut TestAppContext) {
    // A window where the sidebar's minimum binds: the cells are held.
    let (start, widths, end) = editor_widths_while_a_column_opens(cx, 1200.);
    assert_no_dip(start, &widths, end);
}

#[gpui_kit::test]
fn the_editor_edge_does_not_dip_in_a_wide_window(cx: &mut TestAppContext) {
    // A window wide enough that no minimum binds: only the clocks count.
    let (start, widths, end) = editor_widths_while_a_column_opens(cx, 1800.);
    assert_no_dip(start, &widths, end);
}

// ---- Ghosts: the room of a removed slot is given back over time.

fn height_of(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(slot_id(view))
            .map(|slot| f32::from(slot.bounds().size.height))
    })
    .unwrap()
}

#[gpui_kit::test]
fn closing_a_stacked_slot_gives_its_height_back_over_time(cx: &mut TestAppContext) {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    // FILES, FIND and LINKS stacked on the left: closing FIND leaves a split
    // of two, which survives, so FIND's room can be given back over time.
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.34, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.33, views = [\"find\"], shown = \"find\" },\n\
           { weight = 0.33, views = [\"links\"], shown = \"links\" },\n] },\n\
         { weight = 0.8, editor = true },\n]\n");
    let shell = open_shell_motion(cx, 1200., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    let before = height_of(cx, &shell, DrawerView::Files).unwrap();
    let find = cx.read(|cx| shell.workspace.read(cx).layout.slots()[1]);
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_slot(find, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    assert_eq!(
        height_of(cx, &shell, DrawerView::Find),
        None,
        "FIND is not drawn"
    );
    let first = height_of(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        (first - before).abs() < 1.5,
        "FILES starts where it was: {before} to {first}"
    );
    wait(cx, 30);
    frame(cx, &shell);
    let middle = height_of(cx, &shell, DrawerView::Files).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let after = height_of(cx, &shell, DrawerView::Files).unwrap();
    assert!(
        after > before + 1.0,
        "FILES takes the room: {before} to {after}"
    );
    assert!(
        middle > before + 0.5 && middle < after - 0.5,
        "at 30 ms: {middle} between {before} and {after}"
    );
}

#[gpui_kit::test]
fn closing_a_slot_under_a_row_grows_the_row_down_without_moving_its_width(cx: &mut TestAppContext) {
    // FILES and FIND side by side over LINKS: closing LINKS leaves the row
    // alone in the column. The row grows down, and its two cells keep their
    // width on every frame (no stale ghost of FILES or FIND beside them).
    let [before, first, middle, rest] = close_and_watch(
        cx,
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.5, split = \"vertical\", children = [\n\
           { weight = 0.7, split = \"horizontal\", children = [\n\
             { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
             { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
           { weight = 0.3, views = [\"links\"], shown = \"links\" },\n] },\n\
         { weight = 0.5, editor = true },\n]\n",
        DrawerView::Links,
        DrawerView::Files,
    );
    assert!(
        (first.1 - before.1).abs() < 1.0,
        "starts where it was: {before:?} {first:?}"
    );
    assert!(
        rest.1 > before.1 + 1.0,
        "grows down: {before:?} to {rest:?}"
    );
    assert!(
        middle.1 > before.1 + 0.5 && middle.1 < rest.1 - 0.5,
        "at 30 ms: {middle:?} between {before:?} and {rest:?}"
    );
    for now in [first, middle, rest] {
        assert!(
            (now.0 - before.0).abs() < 1.5,
            "the width moved: {before:?} {now:?}"
        );
    }
}

// ---- Ghosts: only where they belong (review of the ghost cell).

#[gpui_kit::test]
fn a_ghost_is_not_drawn_in_a_split_that_took_the_place_of_its_own(cx: &mut TestAppContext) {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    // FILES and FIND side by side over LINKS, in a column: closing LINKS
    // leaves the row alone in the column. LINKS stood in a vertical split;
    // its ghost has no place in the row, which is a horizontal one.
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.5, split = \"vertical\", children = [\n\
           { weight = 0.7, split = \"horizontal\", children = [\n\
             { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
             { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
           { weight = 0.3, views = [\"links\"], shown = \"links\" },\n] },\n\
         { weight = 0.5, editor = true },\n]\n");
    let shell = open_shell_motion(cx, 1600., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    let files = width(cx, &shell, DrawerView::Files).unwrap();
    let find = width(cx, &shell, DrawerView::Find).unwrap();
    let links = cx.read(|cx| {
        let w = shell.workspace.read(cx);
        w.layout.view_slot(&DrawerView::Links).unwrap()
    });
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_slot(links, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    for millis in [0, 30, 40, 60, 200] {
        wait(cx, millis);
        frame(cx, &shell);
        let now = (
            width(cx, &shell, DrawerView::Files).unwrap(),
            width(cx, &shell, DrawerView::Find).unwrap(),
        );
        assert!(
            (now.0 - files).abs() < 1.5 && (now.1 - find).abs() < 1.5,
            "after {millis} ms the row is {now:?}, it was {files} and {find}"
        );
    }
}

#[gpui_kit::test]
fn a_slot_that_was_not_drawn_leaves_no_ghost(cx: &mut TestAppContext) {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    // LINKS collapses in a narrow window: it takes no room. Closing it must
    // not make the others give room back that it never held.
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.1, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.1, views = [\"find\"], shown = \"find\" },\n\
         { weight = 0.1, views = [\"links\"], shown = \"links\" },\n\
         { weight = 0.7, editor = true },\n]\n");
    let shell = open_shell_motion(cx, 1000., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    super::motion_tests::resize(cx, &shell, 800.);
    frame(cx, &shell);
    assert_eq!(
        width(cx, &shell, DrawerView::Links),
        None,
        "LINKS collapsed"
    );
    let files = width(cx, &shell, DrawerView::Files).unwrap();
    let editor = editor_width(cx, &shell);
    let links = cx.read(|cx| {
        let w = shell.workspace.read(cx);
        w.layout.view_slot(&DrawerView::Links).unwrap()
    });
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_slot(links, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    let now = (
        width(cx, &shell, DrawerView::Files).unwrap(),
        editor_width(cx, &shell),
    );
    assert!(
        (now.0 - files).abs() < 1.5 && (now.1 - editor).abs() < 1.5,
        "closing a cell that held no room moved the rest: {now:?}, was {files} and {editor}"
    );
}

#[gpui_kit::test]
fn a_snapping_frame_drops_a_ghost_that_already_exists(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    let find = two_columns(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_slot(find, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    // The window is resized while the ghost is still there: it draws in place.
    resize(cx, &shell, 1100.);
    frame(cx, &shell);
    let at_once = editor_width(cx, &shell);
    wait(cx, 400);
    frame(cx, &shell);
    let rest = editor_width(cx, &shell);
    assert!(
        (at_once - rest).abs() < 1.0,
        "{at_once} on the resize frame, {rest} at rest: the ghost was kept"
    );
}

/// The size of the slot showing `view`, as (width, height).
fn size_of(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> Option<(f32, f32)> {
    cx.update_window(shell.handle, |_, window, _| {
        window.try_find(slot_id(view)).map(|slot| {
            let b = slot.bounds();
            (f32::from(b.size.width), f32::from(b.size.height))
        })
    })
    .unwrap()
}

/// Close the slot that shows `closing` in a window with `layout`, and
/// return the size of the slot `staying` before, on the first frame after,
/// at 30 ms, and at rest.
fn close_and_watch(
    cx: &mut TestAppContext,
    layout: &str,
    closing: DrawerView,
    staying: DrawerView,
) -> [(f32, f32); 4] {
    use super::LayoutStore;
    use super::tests::{gui, open_shell_motion};
    let shell = open_shell_motion(cx, 1600., &gui(layout), "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    let before = size_of(cx, &shell, staying.clone()).unwrap();
    let slot = cx.read(|cx| shell.workspace.read(cx).layout.view_slot(&closing).unwrap());
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.close_slot(slot, window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    assert_eq!(
        size_of(cx, &shell, closing),
        None,
        "the closed slot is not drawn"
    );
    let first = size_of(cx, &shell, staying.clone()).unwrap();
    wait(cx, 30);
    frame(cx, &shell);
    let middle = size_of(cx, &shell, staying.clone()).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let rest = size_of(cx, &shell, staying).unwrap();
    [before, first, middle, rest]
}

#[gpui_kit::test]
fn closing_one_of_two_columns_in_a_cell_grows_the_survivor_over_time(cx: &mut TestAppContext) {
    // FILES and FIND side by side in one cell: closing FIND leaves FILES
    // alone in the cell, and FILES grows into the width it gave up.
    let [before, first, middle, rest] = close_and_watch(
        cx,
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.4, split = \"horizontal\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.6, editor = true },\n]\n",
        DrawerView::Find,
        DrawerView::Files,
    );
    assert!(
        (first.0 - before.0).abs() < 1.0,
        "starts where it was: {before:?} {first:?}"
    );
    assert!(
        rest.0 > before.0 + 1.0,
        "takes the room: {before:?} to {rest:?}"
    );
    assert!(
        middle.0 > before.0 + 0.5 && middle.0 < rest.0 - 0.5,
        "at 30 ms: {middle:?} between {before:?} and {rest:?}"
    );
}

#[gpui_kit::test]
fn closing_one_of_two_stacked_slots_grows_the_survivor_over_time(cx: &mut TestAppContext) {
    // FILES over FIND: closing FIND leaves FILES alone in the column, and
    // FILES grows down into the height it gave up.
    let [before, first, middle, rest] = close_and_watch(
        cx,
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.3, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.7, editor = true },\n]\n",
        DrawerView::Find,
        DrawerView::Files,
    );
    assert!(
        (first.1 - before.1).abs() < 1.0,
        "starts where it was: {before:?} {first:?}"
    );
    assert!(
        rest.1 > before.1 + 1.0,
        "takes the room: {before:?} to {rest:?}"
    );
    assert!(
        middle.1 > before.1 + 0.5 && middle.1 < rest.1 - 0.5,
        "at 30 ms: {middle:?} between {before:?} and {rest:?}"
    );
    assert_eq!(first.0, rest.0, "the width does not move");
}
