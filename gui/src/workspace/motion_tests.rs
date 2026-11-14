//! Slot motion (ADR-0060, slice one): the per-slot store, then the layout
//! pass that draws it. A real `Workspace` in a headless window with motion
//! on (`open_animated`), time stepped by the executor clock.
#![cfg(test)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, IntoElement, Render, TestAppContext, Window,
    WindowOptions, div,
};
use kimun_notes::logic::layout::Side;
use kimun_notes::logic::layout_tree::{RailPosition, SlotId};

use super::LayoutStore;
use super::motion::{Enter, Motion, MotionKey, Spec};
use super::tests::{Shell, gui, open_animated, open_shell_motion};
use crate::motion::{SnapFacts, Speed};

fn left_slot(shell: &Shell, cx: &TestAppContext) -> SlotId {
    cx.read(|cx| shell.workspace.read(cx).side_slots(Side::Left)[0])
}

pub(super) fn wait(cx: &mut TestAppContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
}

/// A view that samples one slot's presence in its render, as the layout
/// pass does: the kit asks for the next frame only during a draw.
struct Probe {
    motion: Rc<RefCell<Motion>>,
    key: MotionKey,
    enter: Enter,
    speed: Speed,
    present: Rc<Cell<bool>>,
    out: Rc<Cell<f32>>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.motion.borrow_mut().sample(
            self.key,
            self.present.get(),
            self.speed,
            self.enter,
            window,
            cx,
        );
        self.out.set(value.frame);
        div()
    }
}

struct Rig {
    handle: AnyWindowHandle,
    motion: Rc<RefCell<Motion>>,
    present: Rc<Cell<bool>>,
    out: Rc<Cell<f32>>,
}

impl Rig {
    fn new(cx: &mut TestAppContext, present: bool) -> Rig {
        let shell = open_animated(cx);
        let slot = left_slot(&shell, cx);
        Rig::keyed(
            cx,
            MotionKey::Slot(slot),
            present,
            Enter::AtRest,
            Speed::Standard,
        )
    }

    /// A rig sampling `key`, in a window of its own.
    fn keyed(
        cx: &mut TestAppContext,
        key: MotionKey,
        present: bool,
        enter: Enter,
        speed: Speed,
    ) -> Rig {
        let motion = Rc::new(RefCell::new(Motion::default()));
        let present = Rc::new(Cell::new(present));
        let out = Rc::new(Cell::new(f32::NAN));
        let probe = Probe {
            motion: motion.clone(),
            key,
            enter,
            speed,
            present: present.clone(),
            out: out.clone(),
        };
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| probe))
            })
            .expect("open the probe window");
        Rig {
            handle,
            motion,
            present,
            out,
        }
    }

    /// Draw one frame toward `present`: the presence it sampled.
    fn frame(&self, cx: &mut TestAppContext, present: bool) -> f32 {
        self.present.set(present);
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        self.out.get()
    }
}

#[gpui_kit::test]
fn a_new_slot_adopts_its_target_on_the_first_sample(cx: &mut TestAppContext) {
    assert_eq!(Rig::new(cx, true).frame(cx, true), 1.0);
    assert_eq!(Rig::new(cx, false).frame(cx, false), 0.0);
}

#[gpui_kit::test]
fn presence_moves_over_the_standard_duration(cx: &mut TestAppContext) {
    let rig = Rig::new(cx, false);
    assert_eq!(rig.frame(cx, false), 0.0);
    rig.frame(cx, true);
    wait(cx, 100);
    let middle = rig.frame(cx, true);
    assert!(middle > 0.0 && middle < 1.0, "after 100 ms: {middle}");
    wait(cx, 300);
    assert_eq!(rig.frame(cx, true), 1.0);
}

#[gpui_kit::test]
fn reduced_motion_adopts_the_end_state_at_once(cx: &mut TestAppContext) {
    let rig = Rig::new(cx, false);
    assert_eq!(rig.frame(cx, false), 0.0);
    cx.update(|cx| cx.set_reduce_motion(true));
    assert_eq!(rig.frame(cx, true), 1.0);
}

#[gpui_kit::test]
fn a_snapping_frame_adopts_the_target_at_once(cx: &mut TestAppContext) {
    let rig = Rig::new(cx, false);
    assert_eq!(rig.frame(cx, false), 0.0);
    rig.motion.borrow_mut().begin_frame(SnapFacts {
        viewport_changed: true,
        ..SnapFacts::default()
    });
    assert_eq!(rig.frame(cx, true), 1.0);
}

// ---- The layout pass: a slot opens and closes over time.

use kimun_notes::logic::layout_tree::{Focus, Leaf};
use kimun_notes::logic::nav::DrawerView;

use super::tree_view::move_button_id;
use super::{slot_content_id, slot_id};

fn toggle_left(cx: &mut TestAppContext, shell: &Shell) {
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.toggle_side(Side::Left, window, cx))
    })
    .unwrap();
}

pub(super) fn frame(cx: &mut TestAppContext, shell: &Shell) {
    cx.update_window(shell.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// The width of the FILES slot's cell, `None` while it is not drawn.
fn slot_width(cx: &mut TestAppContext, shell: &Shell) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(slot_id(DrawerView::Files))
            .map(|slot| f32::from(slot.bounds().size.width))
    })
    .unwrap()
}

fn content_width(cx: &mut TestAppContext, shell: &Shell) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(slot_content_id(DrawerView::Files))
            .map(|content| f32::from(content.bounds().size.width))
    })
    .unwrap()
}

/// A shell whose left slot shows FILES at rest, and that width.
fn open_with_files(cx: &mut TestAppContext) -> (Shell, f32) {
    let shell = open_animated(cx);
    frame(cx, &shell);
    let full = slot_width(cx, &shell).expect("the left slot shows FILES");
    (shell, full)
}

/// The left slot closed and finished closing.
fn close_left_and_rest(cx: &mut TestAppContext, shell: &Shell) {
    toggle_left(cx, shell);
    frame(cx, shell);
    wait(cx, 400);
    frame(cx, shell);
    assert_eq!(slot_width(cx, shell), None, "the slot rests closed");
}

#[gpui_kit::test]
fn a_slot_opens_over_time_and_the_editor_follows(cx: &mut TestAppContext) {
    let (shell, full) = open_with_files(cx);
    close_left_and_rest(cx, &shell);
    toggle_left(cx, &shell);
    frame(cx, &shell);
    let start = slot_width(cx, &shell).expect("the opening slot is drawn");
    assert!(start < 1.0, "at 0 ms: {start}");
    // The frame moves in the first part of the motion and eases out: it is
    // well on its way, and not yet there, a little into it.
    wait(cx, 30);
    frame(cx, &shell);
    let middle = slot_width(cx, &shell).unwrap();
    assert!(middle > 1.0 && middle < full - 1.0, "at 30 ms: {middle}");
    wait(cx, 400);
    frame(cx, &shell);
    let end = slot_width(cx, &shell).unwrap();
    assert!((end - full).abs() < 0.5, "at rest: {end} against {full}");
}

#[gpui_kit::test]
fn a_slot_closing_is_drawn_until_its_motion_ends(cx: &mut TestAppContext) {
    let (shell, full) = open_with_files(cx);
    toggle_left(cx, &shell);
    frame(cx, &shell);
    wait(cx, 100);
    frame(cx, &shell);
    let middle = slot_width(cx, &shell).expect("the leaving slot is still drawn");
    assert!(middle > 0.0 && middle < full, "at 100 ms: {middle}");
    wait(cx, 400);
    frame(cx, &shell);
    assert_eq!(slot_width(cx, &shell), None);
}

#[gpui_kit::test]
fn the_model_changes_on_the_toggle_frame(cx: &mut TestAppContext) {
    let (shell, _) = open_with_files(cx);
    let slot = left_slot(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.focus_slot(slot, window, cx))
    })
    .unwrap();
    assert!(cx.read(|cx| matches!(
        shell.workspace.read(cx).layout.focused(),
        Focus::Leaf(Leaf::Slot(_))
    )));
    toggle_left(cx, &shell);
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        assert_eq!(w.layout.shown(slot), None, "no view shown");
        assert!(matches!(w.layout.focused(), Focus::Leaf(Leaf::Editor)));
    });
    // Still drawn: the screen has not caught up, the model has.
    frame(cx, &shell);
    assert!(slot_width(cx, &shell).is_some());
}

#[gpui_kit::test]
fn a_pinned_slot_keeps_its_content_width_while_it_opens(cx: &mut TestAppContext) {
    let (shell, full) = open_with_files(cx);
    close_left_and_rest(cx, &shell);
    toggle_left(cx, &shell);
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    let moving = content_width(cx, &shell).expect("the content is drawn");
    assert!(
        slot_width(cx, &shell).unwrap() < full - 1.0,
        "still opening"
    );
    wait(cx, 400);
    frame(cx, &shell);
    let rest = content_width(cx, &shell).unwrap();
    assert!(moving > 1.0);
    assert!(
        (moving - rest).abs() < 0.5,
        "{moving} while opening, {rest} at rest"
    );
}

/// Click the `⋯` of the FILES slot and ask for its move chooser: whether
/// the chooser opened.
fn move_chooser_opens(cx: &mut TestAppContext, shell: &Shell) -> bool {
    cx.update_window(shell.handle, |_, window, cx| {
        window.click(move_button_id(DrawerView::Files), cx);
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.read(|cx| shell.workspace.read(cx).move_chooser_rows(cx).is_some())
}

/// FILES over FIND on the left, then the editor, with motion on: the move
/// chooser has a slot to move into.
fn open_stacked(cx: &mut TestAppContext) -> Shell {
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.8, editor = true },\n]\n");
    let shell = open_shell_motion(cx, 1000., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    shell
}

#[gpui_kit::test]
fn a_resting_slot_opens_its_move_chooser(cx: &mut TestAppContext) {
    let shell = open_stacked(cx);
    assert!(move_chooser_opens(cx, &shell));
}

#[gpui_kit::test]
fn a_leaving_slot_takes_no_click(cx: &mut TestAppContext) {
    let shell = open_stacked(cx);
    toggle_left(cx, &shell);
    frame(cx, &shell);
    // The content is fading and the frame is still whole: the button is in
    // view, and the slot has already left the model.
    wait(cx, 40);
    frame(cx, &shell);
    assert!(slot_width(cx, &shell).is_some(), "still leaving");
    assert!(
        !move_chooser_opens(cx, &shell),
        "the leaving slot took a click"
    );
    cx.read(|cx| {
        let focus = shell.workspace.read(cx).layout.focused();
        assert!(matches!(focus, Focus::Leaf(Leaf::Editor)), "{focus:?}");
    });
}

// ---- The snap list.

use gpui_kit::{px, size};

use super::tests::HEIGHT;

pub(super) fn resize(cx: &mut TestAppContext, shell: &Shell, width: f32) {
    cx.simulate_window_resize(shell.handle, size(px(width), px(HEIGHT)));
}

fn files_over_find_height(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> Option<f32> {
    cx.update_window(shell.handle, |_, window, _| {
        window
            .try_find(slot_id(view))
            .map(|slot| f32::from(slot.bounds().size.height))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_reversed_toggle_does_not_jump(cx: &mut TestAppContext) {
    let (shell, full) = open_with_files(cx);
    toggle_left(cx, &shell);
    frame(cx, &shell);
    // The content fades first, then the frame closes: well into the close.
    wait(cx, 150);
    frame(cx, &shell);
    let closing = slot_width(cx, &shell).expect("still closing");
    assert!(closing < full - 1.0 && closing > 1.0, "{closing}");
    // Open it again on the very next frame: it grows from where it is.
    toggle_left(cx, &shell);
    frame(cx, &shell);
    let reversed = slot_width(cx, &shell).unwrap();
    assert!(
        (reversed - closing).abs() < 1.0,
        "{reversed} after {closing}"
    );
    // And it keeps opening from there.
    wait(cx, 20);
    frame(cx, &shell);
    assert!(slot_width(cx, &shell).unwrap() > reversed, "it grows");
}

#[gpui_kit::test]
fn a_layout_is_drawn_in_place_on_the_first_frame(cx: &mut TestAppContext) {
    let (_shell, full) = open_with_files(cx);
    assert!(
        full > 1.0,
        "a shown slot is at full width on the first frame"
    );
    let hidden = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"] },\n\
         { weight = 0.8, editor = true },\n]\n");
    let shell = open_shell_motion(cx, 1000., &hidden, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    assert_eq!(slot_width(cx, &shell), None, "a hidden slot is not drawn");
}

#[gpui_kit::test]
fn a_window_resize_during_a_motion_ends_it(cx: &mut TestAppContext) {
    let (shell, full) = open_with_files(cx);
    close_left_and_rest(cx, &shell);
    toggle_left(cx, &shell);
    frame(cx, &shell);
    wait(cx, 30);
    frame(cx, &shell);
    assert!(
        slot_width(cx, &shell).unwrap() < full - 1.0,
        "still opening"
    );
    resize(cx, &shell, 900.);
    frame(cx, &shell);
    let after = slot_width(cx, &shell).unwrap();
    wait(cx, 400);
    frame(cx, &shell);
    let rest = slot_width(cx, &shell).unwrap();
    assert!(
        (after - rest).abs() < 0.5,
        "{after} after the resize, {rest} at rest"
    );
}

#[gpui_kit::test]
fn leaving_zoom_draws_the_other_slots_in_place(cx: &mut TestAppContext) {
    let shell = open_stacked(cx);
    let find = files_over_find_height(cx, &shell, DrawerView::Find).expect("FIND is drawn");
    let files = left_slot(&shell, cx);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.focus_slot(files, window, cx);
            w.toggle_zoom(window, cx);
        })
    })
    .unwrap();
    frame(cx, &shell);
    wait(cx, 170);
    frame(cx, &shell);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| w.leave_zoom(window, cx))
    })
    .unwrap();
    frame(cx, &shell);
    let back = files_over_find_height(cx, &shell, DrawerView::Find).expect("FIND is drawn again");
    assert!(
        (back - find).abs() < 0.5,
        "{back} after zoom, {find} before"
    );
}

#[gpui_kit::test]
fn a_narrow_window_shows_no_motion_for_a_collapsed_slot(cx: &mut TestAppContext) {
    let (shell, _) = open_with_files(cx);
    close_left_and_rest(cx, &shell);
    resize(cx, &shell, 420.);
    frame(cx, &shell);
    toggle_left(cx, &shell);
    for millis in [0, 100, 400] {
        wait(cx, millis);
        frame(cx, &shell);
        assert_eq!(
            slot_width(cx, &shell),
            None,
            "a collapsed slot at {millis} ms"
        );
    }
}

#[test]
fn the_frame_facts_name_what_changed() {
    let a = size(px(1000.), px(480.));
    let b = size(px(900.), px(480.));
    let left = RailPosition::Left;
    let mut motion = Motion::default();
    let first = motion.frame_facts(a, None, false, left);
    assert!(first.first_frame && !first.viewport_changed && !first.zoom_changed);
    let rest = motion.frame_facts(a, None, false, left);
    assert_eq!(rest, SnapFacts::default());
    assert!(motion.frame_facts(b, None, false, left).viewport_changed);
    assert!(
        motion
            .frame_facts(b, Some(Leaf::Editor), false, left)
            .zoom_changed
    );
    assert!(
        motion
            .frame_facts(b, Some(Leaf::Editor), true, left)
            .divider_drag
    );
    assert!(motion.frame_facts(b, None, false, left).zoom_changed);
    assert!(
        motion
            .frame_facts(b, None, false, RailPosition::Right)
            .rail_moved,
        "a changed rail position is a fact"
    );
    assert!(
        !motion
            .frame_facts(b, None, false, RailPosition::Right)
            .rail_moved
    );
}

#[gpui_kit::test]
fn a_closing_frame_eases_out_and_does_not_slam_shut(cx: &mut TestAppContext) {
    let (shell, full) = open_with_files(cx);
    toggle_left(cx, &shell);
    frame(cx, &shell);
    // The content fades for the first 80 ms and the frame stays whole.
    wait(cx, 40);
    frame(cx, &shell);
    assert!((slot_width(cx, &shell).unwrap() - full).abs() < 0.5);
    // Then the frame closes: fast at first, soft at the end.
    wait(cx, 60);
    frame(cx, &shell);
    let at_100 = slot_width(cx, &shell).unwrap();
    assert!(
        at_100 > 0.4 * full && at_100 < 0.8 * full,
        "{at_100} at 100 ms"
    );
    wait(cx, 40);
    frame(cx, &shell);
    let at_140 = slot_width(cx, &shell).unwrap();
    assert!(at_140 < 0.3 * full, "{at_140} at 140 ms: nearly closed");
}

// ---- Whole-tree changes and moved views (review of slice one).

use super::palette::PaletteRun;
use super::presets::PresetCommand;

/// FILES left, the editor, OUTLINE right, both shown, with motion on.
fn open_files_and_outline(cx: &mut TestAppContext) -> Shell {
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"], shown = \"outline\" },\n]\n");
    let shell = open_shell_motion(cx, 1200., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    shell
}

fn is_drawn(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> bool {
    cx.update_window(shell.handle, |_, window, _| {
        window.try_find(slot_id(view)).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn resetting_the_layout_leaves_no_ghost_of_the_old_one(cx: &mut TestAppContext) {
    let shell = open_files_and_outline(cx);
    assert!(is_drawn(cx, &shell, DrawerView::Outline));
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.run_palette_row(PaletteRun::Preset(PresetCommand::ResetDefault), window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    // The default layout has no OUTLINE. The new layout is drawn at once.
    assert!(
        !is_drawn(cx, &shell, DrawerView::Outline),
        "a ghost OUTLINE"
    );
    wait(cx, 50);
    frame(cx, &shell);
    assert!(
        !is_drawn(cx, &shell, DrawerView::Outline),
        "a ghost OUTLINE"
    );
}

#[gpui_kit::test]
fn a_moved_view_is_not_drawn_in_its_old_slot_too(cx: &mut TestAppContext) {
    // FILES shown left, OUTLINE placed right but hidden: the right slot
    // shows nothing, and the left one is the last on its side.
    let layout = gui("[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, views = [\"files\"], shown = \"files\" },\n\
         { weight = 0.6, editor = true },\n\
         { weight = 0.2, views = [\"outline\"] },\n]\n");
    let shell = open_shell_motion(cx, 1200., &layout, "", false, LayoutStore::Off, false);
    frame(cx, &shell);
    let slots = cx.read(|cx| shell.workspace.read(cx).layout.slots());
    let (left, right) = (slots[0], slots[1]);
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.move_into(DrawerView::Files, right, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    // The left slot stays (last on its side) and shows nothing. One view is
    // one element: the old slot does not draw FILES again as it leaves.
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        assert_eq!(w.layout.view_slot(&DrawerView::Files), Some(right));
        assert_eq!(w.layout.shown(right), Some(DrawerView::Files));
        assert!(!w.leaf_present(Leaf::Slot(left)), "FILES drawn twice");
    });
}

/// (cell width, content width) of `view`'s slot, `None` while not drawn.
fn widths(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> Option<(f32, f32)> {
    cx.update_window(shell.handle, |_, window, _| {
        let cell = window.try_find(slot_id(view))?;
        let content = window.try_find(slot_content_id(view))?;
        Some((
            f32::from(cell.bounds().size.width),
            f32::from(content.bounds().size.width),
        ))
    })
    .unwrap()
}

fn toggle_right(cx: &mut TestAppContext, shell: &Shell) {
    cx.update_window(shell.handle, |_, window, cx| {
        shell
            .workspace
            .update(cx, |w, cx| w.toggle_side(Side::Right, window, cx))
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_slot_that_is_not_moving_is_never_wider_than_its_cell(cx: &mut TestAppContext) {
    // Weights are relative: when the right slot closes, FILES ends wider
    // than it is now. FILES is not moving itself, so its content follows
    // its cell and is never cut at the right border.
    let shell = open_files_and_outline(cx);
    toggle_right(cx, &shell);
    frame(cx, &shell);
    for millis in [20, 40, 40, 40, 60] {
        wait(cx, millis);
        frame(cx, &shell);
        let (cell, content) = widths(cx, &shell, DrawerView::Files).expect("FILES is drawn");
        assert!(
            content <= cell + 0.5,
            "FILES content {content} in a cell of {cell}"
        );
    }
    // And back: opening the right slot shrinks FILES, still never cut.
    toggle_right(cx, &shell);
    frame(cx, &shell);
    for millis in [20, 40, 40, 40, 60] {
        wait(cx, millis);
        frame(cx, &shell);
        let (cell, content) = widths(cx, &shell, DrawerView::Files).expect("FILES is drawn");
        assert!(
            content <= cell + 0.5,
            "FILES content {content} in a cell of {cell}"
        );
    }
}

#[gpui_kit::test]
fn a_resting_slot_fills_its_cell_exactly_at_any_scale(cx: &mut TestAppContext) {
    // At rest nothing is pinned: the content is the cell, to the pixel, so
    // the surface's right border is never cut off.
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        for width in [1000.0_f32, 1001.0, 1157.0, 1333.0] {
            let shell = open_animated(cx);
            cx.simulate_window_scale_factor_change(shell.handle, scale);
            resize(cx, &shell, width);
            frame(cx, &shell);
            wait(cx, 400);
            frame(cx, &shell);
            cx.update_window(shell.handle, |_, window, _| {
                let cell = window.find(slot_id(DrawerView::Files)).bounds();
                let content = window.find(slot_content_id(DrawerView::Files)).bounds();
                let gap = f32::from(cell.right()) - f32::from(content.right());
                assert!(
                    gap.abs() < 0.01,
                    "scale {scale}, width {width}: the content ends {gap} px from the cell"
                );
            })
            .unwrap();
        }
    }
}

// ---- Keys beyond slots (slice two).

fn first_tab(shell: &Shell, cx: &TestAppContext) -> kimun_notes::logic::editor_region::TabId {
    cx.read(|cx| shell.workspace.read(cx).layout.region().tabs()[0])
}

#[gpui_kit::test]
fn a_new_key_can_start_from_zero(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    let tab = MotionKey::Tab(first_tab(&shell, cx));
    let rig = Rig::keyed(cx, tab, true, Enter::FromZero, Speed::Quick);
    assert_eq!(rig.frame(cx, true), 0.0, "a new key starts from 0");
    wait(cx, 60);
    let middle = rig.frame(cx, true);
    assert!(middle > 0.0 && middle < 1.0, "after 60 ms: {middle}");
    wait(cx, 200);
    assert_eq!(rig.frame(cx, true), 1.0);
    let rest = Rig::keyed(cx, tab, true, Enter::AtRest, Speed::Quick);
    assert_eq!(
        rest.frame(cx, true),
        1.0,
        "at rest a new key adopts its target"
    );
}

#[gpui_kit::test]
fn a_snapping_frame_ignores_from_zero(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    let tab = MotionKey::Tab(first_tab(&shell, cx));
    let rig = Rig::keyed(cx, tab, true, Enter::FromZero, Speed::Quick);
    rig.motion.borrow_mut().begin_frame(SnapFacts {
        viewport_changed: true,
        ..SnapFacts::default()
    });
    assert_eq!(rig.frame(cx, true), 1.0);
}

#[gpui_kit::test]
fn snap_next_frame_snaps_once(cx: &mut TestAppContext) {
    let rig = Rig::keyed(cx, MotionKey::Rail, false, Enter::AtRest, Speed::Standard);
    assert_eq!(rig.frame(cx, false), 0.0);
    rig.motion.borrow_mut().snap_next_frame();
    rig.motion.borrow_mut().begin_frame(SnapFacts::default());
    assert_eq!(rig.frame(cx, true), 1.0, "adopted at once");
    rig.motion.borrow_mut().begin_frame(SnapFacts::default());
    assert_eq!(rig.frame(cx, false), 1.0, "the next change moves again");
    wait(cx, 300);
    assert!(rig.frame(cx, false) < 1.0);
}

// ---- The generic store and one sampling call (consolidation).

/// A view that samples a list of specs in its render, with one call.
struct SpecProbe {
    motion: Rc<RefCell<Motion>>,
    specs: Rc<RefCell<Vec<Spec>>>,
}

impl Render for SpecProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.motion
            .borrow_mut()
            .sample_all(&self.specs.borrow(), window, cx);
        div()
    }
}

struct SpecRig {
    handle: AnyWindowHandle,
    motion: Rc<RefCell<Motion>>,
    specs: Rc<RefCell<Vec<Spec>>>,
}

impl SpecRig {
    fn new(cx: &mut TestAppContext) -> SpecRig {
        let motion = Rc::new(RefCell::new(Motion::default()));
        let specs = Rc::new(RefCell::new(Vec::new()));
        let probe = SpecProbe {
            motion: motion.clone(),
            specs: specs.clone(),
        };
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| probe))
            })
            .expect("open the probe window");
        SpecRig {
            handle,
            motion,
            specs,
        }
    }

    /// Draw one frame sampling `specs`.
    fn frame(&self, cx: &mut TestAppContext, specs: Vec<Spec>) {
        *self.specs.borrow_mut() = specs;
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
}

fn spec(key: MotionKey, present: bool, speed: Speed, enter: Enter) -> Spec {
    Spec {
        key,
        present,
        speed,
        enter,
    }
}

#[test]
fn a_snapshot_round_trips_by_type() {
    let mut motion = Motion::default();
    motion.remember(MotionKey::Rail, 7_u32);
    assert_eq!(motion.recall::<u32>(MotionKey::Rail), Some(7));
    assert_eq!(
        motion.recall::<String>(MotionKey::Rail),
        None,
        "another type"
    );
    assert_eq!(
        motion.recall::<u32>(MotionKey::TabStrip),
        None,
        "another key"
    );
}

#[gpui_kit::test]
fn an_unspecified_lingering_key_leaves_and_is_dropped(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    let tab = MotionKey::Tab(first_tab(&shell, cx));
    let rig = SpecRig::new(cx);
    rig.frame(cx, vec![spec(tab, true, Speed::Quick, Enter::AtRest)]);
    assert_eq!(rig.motion.borrow().presence(tab).frame, 1.0);
    rig.frame(cx, vec![]);
    wait(cx, 60);
    rig.frame(cx, vec![]);
    let frame = rig.motion.borrow().presence(tab).frame;
    assert!(frame > 0.0 && frame < 1.0, "leaving: {frame}");
    assert!(rig.motion.borrow().keys().contains(&tab), "still held");
    wait(cx, 300);
    rig.frame(cx, vec![]);
    assert!(rig.motion.borrow().keys().is_empty(), "gone with its frame");
}

#[gpui_kit::test]
fn an_unspecified_slot_is_dropped_at_once(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    let slot = MotionKey::Slot(left_slot(&shell, cx));
    let rig = SpecRig::new(cx);
    rig.frame(cx, vec![spec(slot, true, Speed::Standard, Enter::AtRest)]);
    assert!(rig.motion.borrow().keys().contains(&slot));
    rig.frame(cx, vec![]);
    assert!(rig.motion.borrow().keys().is_empty());
}

#[gpui_kit::test]
fn after_parent_starts_from_zero_only_when_the_parent_rests(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    let tab = MotionKey::Tab(first_tab(&shell, cx));
    let label = spec(
        tab,
        true,
        Speed::Quick,
        Enter::AfterParent(MotionKey::TabStrip),
    );
    // The strip is at rest (shown) and the label is new: it starts from 0.
    let rig = SpecRig::new(cx);
    rig.frame(
        cx,
        vec![spec(
            MotionKey::TabStrip,
            true,
            Speed::Standard,
            Enter::AtRest,
        )],
    );
    rig.frame(
        cx,
        vec![
            spec(MotionKey::TabStrip, true, Speed::Standard, Enter::AtRest),
            label.clone(),
        ],
    );
    assert_eq!(rig.motion.borrow().presence(tab).frame, 0.0);
    // The strip is not at rest (hidden): the label adopts its target.
    let rig = SpecRig::new(cx);
    rig.frame(
        cx,
        vec![
            spec(MotionKey::TabStrip, false, Speed::Standard, Enter::AtRest),
            label,
        ],
    );
    assert_eq!(rig.motion.borrow().presence(tab).frame, 1.0);
}

#[gpui_kit::test]
fn forget_drops_what_the_predicate_names(cx: &mut TestAppContext) {
    let shell = open_animated(cx);
    let slot = MotionKey::Slot(left_slot(&shell, cx));
    let tab = MotionKey::Tab(first_tab(&shell, cx));
    let rig = SpecRig::new(cx);
    rig.frame(
        cx,
        vec![
            spec(slot, true, Speed::Standard, Enter::AtRest),
            spec(tab, true, Speed::Quick, Enter::AtRest),
        ],
    );
    rig.motion
        .borrow_mut()
        .forget(|key| matches!(key, MotionKey::Slot(_)));
    assert_eq!(rig.motion.borrow().keys(), vec![tab]);
}
