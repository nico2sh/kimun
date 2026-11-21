//! UI tests for the **Facet view**'s undo of cleared picks and for its
//! focus ring. Each test draws a `FacetView` alone in a headless window
//! (see `facet_tests` for the helpers).
#![cfg(test)]

use std::time::Duration;

use gpui_kit::TestAppContext;
use kimun_notes::settings::themes::{Theme, ThemeColor};

use super::facet::{CLEAR_ID, CLEARED_ID, ListKind, UNDO_ID, menu_id, pill_id};
use super::facet_tests::{
    bounds, cell, click, focus_view, has_focus, key, label, open, pick_from_menu, picks, press,
    shows, tab_to, tag, view_focused,
};
use super::facet_ui::{ring_id, ring_radius};
use crate::keymap::{self, Keymap, Platform};
use crate::theme::{self, contrast};

const NOTE: &str = "# Head\n\nsome #a\n\nmore #b\n";

fn undo_depth(
    view: &gpui_kit::Entity<super::facet::FacetView>,
    cx: &mut gpui_kit::VisualTestContext,
) -> usize {
    view.read_with(cx, |view, _| view.undo_depth())
}

// ── undo ─────────────────────────────────────────────────────────────────

#[gpui_kit::test]
fn clear_pushes_the_removed_picks_and_ctrl_z_restores_them(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a"), tag("b")]);
    click(cx, CLEAR_ID);
    assert!(picks(&view, cx).is_empty());
    assert_eq!(undo_depth(&view, cx), 1);
    focus_view(&view, cx);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
    assert_eq!(undo_depth(&view, cx), 0, "the set left the stack");
    // Nothing left to undo: Ctrl+Z changes nothing.
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
}

#[gpui_kit::test]
fn escape_clears_and_ctrl_z_restores(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    focus_view(&view, cx);
    key(cx, "escape");
    assert!(picks(&view, cx).is_empty());
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a")]);
}

/// With the macOS table the undo chord is Cmd+Z, the keymap's `primary`,
/// whatever OS runs the test: Ctrl+Z does nothing there.
#[gpui_kit::test]
fn the_macos_table_undoes_on_cmd_z(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    cx.update(|_, cx| keymap::init(cx, Keymap::defaults(Platform::MacOs)));
    focus_view(&view, cx);
    key(cx, "escape");
    key(cx, "ctrl-z");
    assert!(picks(&view, cx).is_empty(), "Ctrl+Z is not the Mac undo");
    key(cx, "cmd-z");
    assert_eq!(picks(&view, cx), [tag("a")]);
}

#[gpui_kit::test]
fn two_clears_in_a_row_keep_both_sets_on_the_stack(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    pick_from_menu(&view, cx, &tag("b"));
    click(cx, CLEAR_ID);
    assert_eq!(undo_depth(&view, cx), 2);
    focus_view(&view, cx);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("b")]);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a")]);
}

#[gpui_kit::test]
fn a_new_pick_after_a_clear_removes_the_undo_button(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    assert!(shows(cx, UNDO_ID));
    assert_eq!(label(cx, CLEARED_ID).as_deref(), Some("Picks cleared"));
    pick_from_menu(&view, cx, &tag("b"));
    assert!(!shows(cx, UNDO_ID));
    assert!(!shows(cx, CLEARED_ID));
    assert_eq!(undo_depth(&view, cx), 1, "a pick leaves the stack alone");
}

#[gpui_kit::test]
fn the_undo_button_goes_after_eight_seconds(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    assert!(shows(cx, UNDO_ID));
    cx.executor().advance_clock(Duration::from_secs(7));
    cx.run_until_parked();
    assert!(shows(cx, UNDO_ID), "still there after 7 seconds");
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(!shows(cx, UNDO_ID));
    assert!(!shows(cx, CLEARED_ID));
    assert_eq!(undo_depth(&view, cx), 1, "Ctrl+Z still restores");
}

#[gpui_kit::test]
fn the_undo_stack_holds_at_most_twenty_sets(cx: &mut TestAppContext) {
    let text: String = (0..25).map(|i| format!("#t{i:02} ")).collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![]);
    for i in 0..25 {
        view.update(cx, |view, cx| view.toggle(tag(&format!("t{i:02}")), cx));
        click(cx, CLEAR_ID);
    }
    assert_eq!(undo_depth(&view, cx), 20);
    let oldest = view.read_with(cx, |view, _| view.undo_sets()[0].clone());
    assert_eq!(oldest, [tag("t05")], "the five oldest sets went");
    focus_view(&view, cx);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("t24")]);
    assert_eq!(undo_depth(&view, cx), 19);
    // Ctrl+Z with picks puts them on the stack, which stays at 20.
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("t23")]);
    assert_eq!(undo_depth(&view, cx), 19);
    click(cx, CLEAR_ID);
    assert_eq!(undo_depth(&view, cx), 20);
    view.update(cx, |view, cx| view.toggle(tag("t00"), cx));
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("t23")]);
    assert_eq!(undo_depth(&view, cx), 20, "the pushed picks keep the cap");
}

/// Ctrl+Z after new picks puts them on the stack before it restores: a
/// second Ctrl+Z brings them back.
#[gpui_kit::test]
fn ctrl_z_after_new_picks_keeps_them_on_the_stack(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "#a #b #c\n", vec![tag("a"), tag("b")]);
    click(cx, CLEAR_ID);
    pick_from_menu(&view, cx, &tag("c"));
    focus_view(&view, cx);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("c")]);
}

/// Enter on `Clear` hands the keyboard to `Undo`. Enter on `Undo` restores
/// the picks and hands the keyboard to the view.
#[gpui_kit::test]
fn enter_on_clear_moves_the_keyboard_to_undo(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    focus_view(&view, cx);
    tab_to(cx, CLEAR_ID, 20);
    key(cx, "enter");
    assert!(picks(&view, cx).is_empty());
    assert!(has_focus(cx, UNDO_ID), "the keyboard is on Undo");
    assert_eq!(label(cx, UNDO_ID).as_deref(), Some("Undo clear"));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("a")]);
    assert!(view_focused(&view, cx), "the keyboard is on the view");
    // Space on `Clear` does the same.
    tab_to(cx, CLEAR_ID, 20);
    key(cx, "space");
    assert!(has_focus(cx, UNDO_ID));
}

/// Ctrl+Z works while a piece has the keyboard.
#[gpui_kit::test]
fn ctrl_z_in_a_piece_restores_the_picks(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    // "some #a" is row 2; col 1 is plain text.
    let at = cell(&view, 0, 1, 2, cx);
    press(cx, at, false);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a")]);
}

/// Ctrl+Z works while a chip has the keyboard.
#[gpui_kit::test]
fn ctrl_z_on_a_chip_restores_the_picks(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    focus_view(&view, cx);
    tab_to(cx, menu_id(ListKind::Sections), 20);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a")]);
}

/// Escape out of a piece, Escape again to clear, then Ctrl+Z restores.
#[gpui_kit::test]
fn escape_from_a_piece_clears_and_ctrl_z_restores(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    let at = cell(&view, 0, 1, 0, cx);
    press(cx, at, false);
    key(cx, "escape");
    assert_eq!(
        picks(&view, cx),
        [tag("a")],
        "the first Escape leaves the piece"
    );
    key(cx, "escape");
    assert!(picks(&view, cx).is_empty());
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a")]);
}

#[gpui_kit::test]
fn the_undo_button_is_a_tab_stop_and_restores(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a"), tag("b")]);
    click(cx, CLEAR_ID);
    focus_view(&view, cx);
    tab_to(cx, UNDO_ID, 20);
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
    assert!(!shows(cx, UNDO_ID));
    // A click does the same.
    click(cx, CLEAR_ID);
    click(cx, UNDO_ID);
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
}

/// A restored set drops the picks the note lost since the clear.
#[gpui_kit::test]
fn a_restored_set_is_pruned_against_the_note(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a"), tag("b")]);
    click(cx, CLEAR_ID);
    view.update(cx, |view, cx| view.set_text("only #b\n".into(), cx));
    focus_view(&view, cx);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("b")]);
}

/// While the list's filter field has the keyboard, Ctrl+Z is the field's.
#[gpui_kit::test]
fn ctrl_z_in_the_filter_field_leaves_the_picks_alone(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    click(cx, super::facet::menu_id(super::facet::ListKind::Tags));
    key(cx, "ctrl-z");
    assert!(picks(&view, cx).is_empty(), "{:?}", picks(&view, cx));
    assert_eq!(undo_depth(&view, cx), 1);
}

// ── the focus ring ───────────────────────────────────────────────────────

#[gpui_kit::test]
fn the_focus_ring_is_two_pixels_and_outside_a_picked_pill(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    focus_view(&view, cx);
    let pill = pill_id(&tag("a"));
    tab_to(cx, pill.clone(), 10);
    assert!(has_focus(cx, pill.clone()));
    let inner = bounds(cx, pill.clone());
    let ring = bounds(cx, ring_id(&pill));
    // A 2 px stroke after a 1 px gap: 3 px out on every side.
    let out = gpui_kit::px(3.);
    assert_eq!(ring.origin.x, inner.origin.x - out, "{ring:?} {inner:?}");
    assert_eq!(ring.origin.y, inner.origin.y - out, "{ring:?} {inner:?}");
    assert_eq!(ring.size.width, inner.size.width + out * 2., "{ring:?}");
    assert_eq!(ring.size.height, inner.size.height + out * 2., "{ring:?}");
    // The pill keeps its place: the ring takes no room.
    key(cx, "tab");
    assert_eq!(bounds(cx, pill.clone()), inner);
    assert!(!shows(cx, ring_id(&pill)), "no ring without the keyboard");
}

/// The ring's corners follow the stop's own (`rounded_md`, 0.375 rem),
/// pushed out by the ring's 3 px.
#[test]
fn the_ring_radius_follows_the_rem_size() {
    assert_eq!(ring_radius(gpui_kit::px(16.)), gpui_kit::px(9.));
    assert_eq!(ring_radius(gpui_kit::px(20.)), gpui_kit::px(10.5));
}

#[test]
fn the_focus_ring_has_three_to_one_contrast_in_every_builtin_theme() {
    for t in Theme::builtins() {
        let ratio = contrast(theme::focus_ring(&t), theme::background(&t));
        assert!(ratio >= 3.0, "{}: {ratio:.2}", t.name);
    }
}

/// A theme whose focus colour already reads keeps it.
#[test]
fn the_focus_ring_keeps_a_focus_colour_that_reads() {
    for t in Theme::builtins() {
        let own = theme::focus_border(&t);
        if contrast(own, theme::background(&t)) >= 3.0 {
            assert_eq!(theme::focus_ring(&t), own, "{}", t.name);
        }
    }
}

/// A focus colour that is the page itself falls back to one that reads.
#[test]
fn a_faint_focus_colour_falls_back_to_one_that_reads() {
    let t = Theme {
        bg: ThemeColor::Rgb(30, 30, 30),
        fg: ThemeColor::Rgb(220, 220, 220),
        focus_border: ThemeColor::Rgb(32, 32, 32),
        ..Theme::default()
    };
    let ratio = contrast(theme::focus_ring(&t), theme::background(&t));
    assert!(ratio >= 3.0, "{ratio:.2}");
}

/// An undo that brings back the very picks shown now does not put them on
/// the stack again: the next Ctrl+Z is not wasted on the same set.
#[gpui_kit::test]
fn undo_does_not_push_picks_equal_to_the_set_it_brings_back(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("b")]);
    click(cx, CLEAR_ID);
    pick_from_menu(&view, cx, &tag("a"));
    click(cx, CLEAR_ID);
    assert_eq!(
        view.read_with(cx, |view, _| view.undo_sets().to_vec()),
        [vec![tag("b")], vec![tag("a")]]
    );
    // Pick `a` again by hand: the same set as the top of the stack.
    pick_from_menu(&view, cx, &tag("a"));
    focus_view(&view, cx);
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("a")]);
    assert_eq!(
        view.read_with(cx, |view, _| view.undo_sets().to_vec()),
        [vec![tag("b")]],
        "the equal set was not pushed"
    );
    key(cx, "ctrl-z");
    assert_eq!(picks(&view, cx), [tag("b")], "the next Ctrl+Z goes on");
}
