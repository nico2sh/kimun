use super::*;
use crate::logic::nav::DrawerView;

const L: Panel = Panel::Drawer(Side::Left);
const R: Panel = Panel::Drawer(Side::Right);

fn layout() -> PanelLayout<DrawerView> {
    PanelLayout::new(DrawerView::Files, 34.0)
}

fn slot(side: Side, index: usize) -> Region {
    Region::Slot { side, index }
}

// ── Moved from `PanelOrder` (tui/src/app_screen/panel_set.rs) ──────────────

#[test]
fn default_focus_is_editor() {
    let order = layout();
    assert_eq!(order.focused(), Panel::Editor);
}

#[test]
fn focus_cycle_wraps_over_visible_panels() {
    let mut order = layout();
    // Focus = editor (right end); next wraps to the rail.
    assert_eq!(order.next_panel(), Some(Panel::Rail));
    assert_eq!(order.prev_panel(), Some(L));

    order.focus(Panel::Rail);
    assert_eq!(order.prev_panel(), Some(Panel::Editor)); // wrap left
    assert_eq!(order.next_panel(), Some(L));
}

#[test]
fn focus_cycle_skips_hidden_drawer() {
    let mut order = layout();
    order.hide(L);
    assert_eq!(order.next_panel(), Some(Panel::Rail));
    order.focus(Panel::Rail);
    assert_eq!(order.next_panel(), Some(Panel::Editor));
}

#[test]
fn show_hide_toggles_visibility_except_rail_and_editor() {
    let mut order = layout();
    assert!(order.is_visible(L));

    order.hide(L);
    assert!(!order.is_visible(L));
    order.show(L);
    assert!(order.is_visible(L));

    // The rail and the editor cannot be hidden.
    order.hide(Panel::Editor);
    assert!(order.is_visible(Panel::Editor));
    order.hide(Panel::Rail);
    assert!(order.is_visible(Panel::Rail));
}

#[test]
fn hiding_focused_panel_moves_focus_to_visible() {
    let mut order = layout();
    order.focus(L);
    order.hide(L);
    // Focus cannot stay on a hidden panel.
    assert!(order.is_visible(order.focused()));
}

#[test]
fn a_hidden_drawer_can_be_shown_again() {
    let mut order = layout();
    order.hide(L);
    assert!(!order.is_visible(L));
    order.show(L);
    assert!(order.is_visible(L));
}

#[test]
fn visible_in_order_skips_hidden() {
    let mut order = layout();
    assert_eq!(order.visible_panels(), vec![Panel::Rail, L, Panel::Editor]);
    order.hide(L);
    assert_eq!(order.visible_panels(), vec![Panel::Rail, Panel::Editor]);
}

// ── Defaults ───────────────────────────────────────────────────────────

#[test]
fn default_is_one_left_slot_one_group_and_an_empty_hidden_right_drawer() {
    let order = layout();
    assert_eq!(order.slot_views(Side::Left), vec![DrawerView::Files]);
    assert_eq!(order.focused_slot(Side::Left), Some(0));
    assert!(order.slot_views(Side::Right).is_empty());
    assert_eq!(order.focused_slot(Side::Right), None);
    assert!(!order.is_visible(R));
    assert_eq!(order.groups().len(), 1);
    assert_eq!(order.focused_region(), Region::Group(order.active_group()));
    assert_eq!(order.drawer_width(Side::Left), 34.0);
}

#[test]
fn an_empty_drawer_cannot_be_shown_or_focused() {
    let mut order = layout();
    order.show(R);
    assert!(!order.is_visible(R));
    order.focus(R);
    assert_eq!(order.focused(), Panel::Editor);
}

// ── Two drawers ────────────────────────────────────────────────────────

#[test]
fn two_drawers_order_rail_left_editor_right() {
    let mut order = layout();
    order.move_view(DrawerView::Links, Side::Right);
    assert_eq!(
        order.visible_panels(),
        vec![Panel::Rail, L, Panel::Editor, R]
    );
    let xs: Vec<(Panel, f64)> = order
        .regions()
        .into_iter()
        .map(|(r, rect)| (r.panel(), rect.x()))
        .collect();
    assert_eq!(
        xs,
        vec![(Panel::Rail, 0.0), (L, 1.0), (Panel::Editor, 2.0), (R, 3.0)]
    );
    // Focus on the editor: next goes to the right drawer, then wraps.
    assert_eq!(order.next_panel(), Some(R));
    order.focus(R);
    assert_eq!(order.next_panel(), Some(Panel::Rail));
}

#[test]
fn moving_a_view_moves_it_and_never_duplicates() {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    assert_eq!(
        order.move_view(DrawerView::Links, Side::Right),
        (Side::Right, 0)
    );
    assert_eq!(order.slot_views(Side::Left), vec![DrawerView::Files]);
    assert_eq!(order.slot_views(Side::Right), vec![DrawerView::Links]);
    assert_eq!(
        order.view_location(&DrawerView::Links),
        Some((Side::Right, 0))
    );
    assert!(order.is_visible(R));

    // Placing it into another slot moves it again.
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    assert_eq!(
        order.slot_views(Side::Left),
        vec![DrawerView::Files, DrawerView::Links]
    );
    assert!(order.slot_views(Side::Right).is_empty());
}

#[test]
fn moving_the_last_view_out_hides_and_empties_its_drawer() {
    let mut order = layout();
    order.move_view(DrawerView::Files, Side::Right);
    assert!(order.slot_views(Side::Left).is_empty());
    assert!(!order.is_visible(L));
    assert!(order.is_visible(R));
}

#[test]
fn focus_follows_a_moved_view() {
    let mut order = layout();
    order.focus(L);
    order.move_view(DrawerView::Files, Side::Right);
    assert_eq!(order.focused_region(), slot(Side::Right, 0));
}

#[test]
fn moving_into_a_full_drawer_replaces_its_focused_slot() {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    order.focus_region(slot(Side::Left, 1));
    order.move_view(DrawerView::Tags, Side::Right);
    assert_eq!(
        order.move_view(DrawerView::Tags, Side::Left),
        (Side::Left, 1)
    );
    assert_eq!(
        order.slot_views(Side::Left),
        vec![DrawerView::Files, DrawerView::Tags]
    );
    assert_eq!(order.view_location(&DrawerView::Links), None, "evicted");
}

#[test]
fn a_drawer_holds_at_most_two_slots() {
    let mut order = layout();
    assert_eq!(order.split_slot(Side::Left, DrawerView::Links), Ok(1));
    assert_eq!(
        order.split_slot(Side::Left, DrawerView::Tags),
        Err(LayoutError::SlotCap)
    );
    assert_eq!(
        order.slot_views(Side::Left),
        vec![DrawerView::Files, DrawerView::Links]
    );
    assert_eq!(order.view_location(&DrawerView::Tags), None);
}

#[test]
fn set_slot_view_replaces_and_moves() {
    let mut order = layout();
    assert_eq!(
        order.set_slot_view(Side::Left, 3, DrawerView::Tags),
        Err(LayoutError::NoSuchSlot)
    );
    order
        .set_slot_view(Side::Left, 0, DrawerView::Tags)
        .unwrap();
    assert_eq!(order.slot_views(Side::Left), vec![DrawerView::Tags]);
    assert_eq!(order.view_location(&DrawerView::Files), None);
}

#[test]
fn place_view_keeps_a_placed_view_and_never_reveals() {
    let mut order = layout();
    order.hide(L);
    assert_eq!(order.place_view(DrawerView::Ask), (Side::Left, 0));
    assert_eq!(order.slot_views(Side::Left), vec![DrawerView::Ask]);
    assert!(!order.is_visible(L), "switching must not force-reveal");

    order.move_view(DrawerView::Links, Side::Right);
    assert_eq!(order.place_view(DrawerView::Links), (Side::Right, 0));
}

#[test]
fn open_view_reveals_the_drawer_the_view_lives_in() {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    order.hide(L);
    assert_eq!(order.open_view(DrawerView::Links), (Side::Left, 1));
    assert!(order.is_visible(L));
    assert_eq!(order.focused_slot(Side::Left), Some(1));
    assert_eq!(order.focused(), Panel::Editor, "panel focus unchanged");
}

#[test]
fn closing_a_slot_keeps_focus_on_the_surviving_view() {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    order.focus_region(slot(Side::Left, 1));
    assert_eq!(order.close_slot(Side::Left, 0), Ok(DrawerView::Files));
    assert_eq!(order.focused_region(), slot(Side::Left, 0));
    assert_eq!(
        order.close_slot(Side::Left, 5),
        Err(LayoutError::NoSuchSlot)
    );
}

#[test]
fn split_ratio_exists_only_with_two_slots() {
    let mut order = layout();
    assert_eq!(order.split_ratio(Side::Left), None);
    assert!(!order.set_split_ratio(Side::Left, 0.3));
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    assert_eq!(order.split_ratio(Side::Left), Some(0.5));
    assert!(order.set_split_ratio(Side::Left, 0.3));
    assert_eq!(
        order.split_ratio(Side::Left),
        Some(0.3),
        "exact, for saving"
    );
    order.set_split_ratio(Side::Left, 1.0 / 3.0);
    assert_eq!(
        order.split_ratio(Side::Left),
        Some(0.3333),
        "rounded to RATIO_DECIMALS"
    );
}

// ── Right drawer hide/show and focus fallbacks ────────────────────────

#[test]
fn hiding_the_focused_right_drawer_falls_back_to_the_editor() {
    let mut order = layout();
    order.move_view(DrawerView::Links, Side::Right);
    order.focus(R);
    order.hide(R);
    assert!(!order.is_visible(R));
    assert_eq!(order.focused(), Panel::Editor);
    order.show(R);
    assert!(order.is_visible(R));
    assert_eq!(order.slot_views(Side::Right), vec![DrawerView::Links]);
}

#[test]
fn closing_the_last_right_slot_falls_back_to_the_editor() {
    let mut order = layout();
    order.move_view(DrawerView::Links, Side::Right);
    order.focus(R);
    order.close_slot(Side::Right, 0).unwrap();
    assert!(!order.is_visible(R));
    assert_eq!(order.focused(), Panel::Editor);
}

#[test]
fn hiding_the_left_drawer_keeps_today_s_fallback_to_the_rail() {
    let mut order = layout();
    order.focus(L);
    order.hide(L);
    assert_eq!(order.focused(), Panel::Rail);
}

// ── Widths ─────────────────────────────────────────────────────────────

const LIMITS: WidthLimits = WidthLimits {
    drawer_min: 20.0,
    editor_min: 20.0,
    available: 113.0,
};

#[test]
fn drawer_width_clamps_to_the_front_end_minimums() {
    let mut order = layout();
    assert_eq!(order.set_drawer_width(Side::Left, 54.0, LIMITS), 54.0);
    assert_eq!(order.set_drawer_width(Side::Left, -5.0, LIMITS), 20.0);
    assert_eq!(order.set_drawer_width(Side::Left, 500.0, LIMITS), 93.0);
    assert_eq!(order.resize_drawer(Side::Left, -4.0, LIMITS), 89.0);
    assert_eq!(order.drawer_width(Side::Left), 89.0);
}

#[test]
fn drawer_width_leaves_room_for_a_visible_other_drawer() {
    let mut order = layout();
    order.move_view(DrawerView::Links, Side::Right);
    // Right drawer is 34 wide and visible: 113 - 20 - 34.
    assert_eq!(order.set_drawer_width(Side::Left, 500.0, LIMITS), 59.0);
}

#[test]
fn drawer_width_never_drops_below_the_drawer_minimum() {
    let mut order = layout();
    let tiny = WidthLimits {
        available: 10.0,
        ..LIMITS
    };
    assert_eq!(order.set_drawer_width(Side::Left, 50.0, tiny), 20.0);
}

#[test]
fn the_clamped_width_keeps_the_stored_preference() {
    let mut order = layout();
    order.set_drawer_width(Side::Left, 80.0, LIMITS);
    let narrow = WidthLimits {
        available: 70.0,
        ..LIMITS
    };
    // 70 - 20 editor = 50 drawn; 80 still stored.
    assert_eq!(order.clamped_drawer_width(Side::Left, narrow), 50.0);
    assert_eq!(order.clamped_drawer_width(Side::Left, LIMITS), 80.0);
    assert_eq!(order.drawer_width(Side::Left), 80.0);
}

// ── Directional focus ──────────────────────────────────────────────────

#[test]
fn directional_focus_in_the_default_layout() {
    let mut order = layout();
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(slot(Side::Left, 0))
    );
    assert_eq!(order.focused(), L);
    assert_eq!(order.focus_dir(Dir::Left), FocusMove::Moved(Region::Rail));
    assert_eq!(
        order.focus_dir(Dir::Right),
        FocusMove::Moved(slot(Side::Left, 0))
    );
    assert_eq!(
        order.focus_dir(Dir::Right),
        FocusMove::Moved(Region::Group(order.active_group()))
    );
}

#[test]
fn directional_focus_does_not_wrap_at_edges() {
    let mut order = layout();
    assert_eq!(order.focus_dir(Dir::Right), FocusMove::AtEdge);
    assert_eq!(order.focused(), Panel::Editor);
    order.focus(Panel::Rail);
    assert_eq!(order.focus_dir(Dir::Left), FocusMove::AtEdge);
    assert_eq!(order.focused(), Panel::Rail);
}

#[test]
fn directional_focus_reaches_a_drawer_on_the_right() {
    let mut order = layout();
    order.move_view(DrawerView::Files, Side::Right);
    assert_eq!(
        order.focus_dir(Dir::Right),
        FocusMove::Moved(slot(Side::Right, 0))
    );
    assert_eq!(order.focus_dir(Dir::Right), FocusMove::AtEdge);
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(Region::Group(order.active_group()))
    );
    // The left drawer is empty and hidden: next stop is the rail.
    assert_eq!(order.focus_dir(Dir::Left), FocusMove::Moved(Region::Rail));
}

#[test]
fn directional_focus_skips_a_hidden_drawer() {
    let mut order = layout();
    order.hide(L);
    assert_eq!(order.focus_dir(Dir::Left), FocusMove::Moved(Region::Rail));
    assert_eq!(
        order.focus_dir(Dir::Right),
        FocusMove::Moved(Region::Group(order.active_group()))
    );
}

#[test]
fn up_and_down_move_between_stacked_slots() {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    order.focus_region(slot(Side::Left, 0));
    assert_eq!(
        order.focus_dir(Dir::Down),
        FocusMove::Moved(slot(Side::Left, 1))
    );
    assert_eq!(order.focus_dir(Dir::Down), FocusMove::AtEdge, "no wrap");
    assert_eq!(
        order.focus_dir(Dir::Up),
        FocusMove::Moved(slot(Side::Left, 0))
    );
    assert_eq!(order.focus_dir(Dir::Up), FocusMove::AtEdge);
}

#[test]
fn entering_a_stacked_drawer_sideways_lands_on_its_focused_slot() {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    order.focus_region(slot(Side::Left, 1));
    order.focus(Panel::Editor);
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(slot(Side::Left, 1))
    );
}

#[test]
fn up_and_down_in_a_single_column_are_not_handled() {
    let mut order = layout();
    assert_eq!(order.focus_dir(Dir::Up), FocusMove::NotHandled);
    assert_eq!(order.focus_dir(Dir::Down), FocusMove::NotHandled);
    order.focus(L);
    assert_eq!(order.focus_dir(Dir::Down), FocusMove::NotHandled);
    order.focus(Panel::Rail);
    assert_eq!(order.focus_dir(Dir::Up), FocusMove::NotHandled);
    assert_eq!(order.focused(), Panel::Rail);
}

#[test]
fn directional_focus_moves_between_stacked_editor_groups() {
    let mut order = layout();
    let top = order.active_group();
    let bottom = order.split_group(top, Axis::Vertical).unwrap();
    assert_eq!(order.groups(), vec![top, bottom]);
    assert_eq!(
        order.active_group(),
        top,
        "splitting keeps the active group"
    );
    assert_eq!(
        order.focus_dir(Dir::Down),
        FocusMove::Moved(Region::Group(bottom))
    );
    assert_eq!(order.active_group(), bottom);
    assert_eq!(order.focus_dir(Dir::Down), FocusMove::AtEdge);
    assert_eq!(
        order.focus_dir(Dir::Up),
        FocusMove::Moved(Region::Group(top))
    );
}

#[test]
fn directional_focus_moves_between_side_by_side_editor_groups() {
    let mut order = layout();
    let first = order.active_group();
    let second = order.split_group(first, Axis::Horizontal).unwrap();
    assert_eq!(
        order.focus_dir(Dir::Right),
        FocusMove::Moved(Region::Group(second))
    );
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(Region::Group(first))
    );
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(slot(Side::Left, 0))
    );
    assert_eq!(order.focus_dir(Dir::Up), FocusMove::NotHandled);
}

#[test]
fn closing_the_active_group_activates_its_predecessor() {
    let mut order = layout();
    let a = order.active_group();
    let b = order.split_group(a, Axis::Horizontal).unwrap();
    let c = order.split_group(b, Axis::Horizontal).unwrap();
    order.focus_region(Region::Group(c));
    assert!(order.close_group(c));
    assert_eq!(order.active_group(), b);
    assert!(order.close_group(a));
    assert!(!order.close_group(b), "the last group stays");
    assert_eq!(order.groups(), vec![b]);
    assert_eq!(order.split_group(a, Axis::Vertical), None);
}

#[test]
fn focus_region_rejects_missing_regions() {
    let mut order = layout();
    assert!(!order.focus_region(slot(Side::Left, 1)));
    assert!(!order.focus_region(slot(Side::Right, 0)));
    assert_eq!(order.focused(), Panel::Editor);
    assert!(order.focus_region(Region::Rail));
    assert_eq!(order.focused(), Panel::Rail);
}

// ── Fix wave: home sides, focus with set_slot_view, split refusal ────────

#[test]
fn an_unplaced_view_opens_on_its_home_side_default_left() {
    let mut order = layout();
    assert_eq!(order.home_side(&DrawerView::Tags), Side::Left);
    assert_eq!(order.open_view(DrawerView::Tags), (Side::Left, 0));
}

#[test]
fn a_view_pushed_out_of_the_right_drawer_reopens_there() {
    let mut order = layout();
    order.move_view(DrawerView::Links, Side::Right);
    order.split_slot(Side::Right, DrawerView::Tags).unwrap();
    order.focus_region(slot(Side::Right, 0));
    // Full drawer: Outline replaces the focused Links.
    order.move_view(DrawerView::Outline, Side::Right);
    assert_eq!(order.view_location(&DrawerView::Links), None);
    assert_eq!(order.home_side(&DrawerView::Links), Side::Right);
    assert_eq!(order.open_view(DrawerView::Links), (Side::Right, 0));
    assert_eq!(
        order.slot_views(Side::Right),
        vec![DrawerView::Links, DrawerView::Tags]
    );
    assert_eq!(order.home_side(&DrawerView::Outline), Side::Right);
}

#[test]
fn moving_a_view_back_updates_its_home() {
    let mut order = layout();
    order.move_view(DrawerView::Links, Side::Right);
    assert_eq!(order.home_side(&DrawerView::Links), Side::Right);
    order.move_view(DrawerView::Links, Side::Left);
    assert_eq!(order.home_side(&DrawerView::Links), Side::Left);
    let (_, i) = order.view_location(&DrawerView::Links).unwrap();
    order.close_slot(Side::Left, i).unwrap();
    assert_eq!(order.open_view(DrawerView::Links).0, Side::Left);
}

#[test]
fn home_side_can_be_set_for_persistence() {
    let mut order = layout();
    order.set_home_side(DrawerView::Tags, Side::Right);
    assert_eq!(order.home_side(&DrawerView::Tags), Side::Right);
    assert_eq!(order.open_view(DrawerView::Tags), (Side::Right, 0));
    assert!(order.is_visible(R));
    // Setting the home of a placed view records it without moving the view.
    order.set_home_side(DrawerView::Tags, Side::Left);
    assert_eq!(
        order.view_location(&DrawerView::Tags),
        Some((Side::Right, 0))
    );
    assert_eq!(order.home_side(&DrawerView::Tags), Side::Left);
}

#[test]
fn place_view_into_an_empty_home_drawer_keeps_it_hidden() {
    let mut order = layout();
    order.set_home_side(DrawerView::Tags, Side::Right);
    assert_eq!(order.place_view(DrawerView::Tags), (Side::Right, 0));
    assert!(!order.is_visible(R));
    assert_eq!(order.slot_views(Side::Left), vec![DrawerView::Files]);
}

#[test]
fn set_slot_view_moves_focus_with_the_view() {
    let mut order = layout();
    order.move_view(DrawerView::Links, Side::Right);
    order.focus_region(slot(Side::Right, 0));
    order
        .set_slot_view(Side::Left, 0, DrawerView::Links)
        .unwrap();
    assert_eq!(order.slot_views(Side::Left), vec![DrawerView::Links]);
    assert!(!order.is_visible(R));
    assert_eq!(order.focused_region(), slot(Side::Left, 0));
    assert_eq!(order.home_side(&DrawerView::Links), Side::Left);
}

#[test]
fn split_slot_refuses_a_view_already_in_that_drawer() {
    let mut order = layout();
    assert_eq!(
        order.split_slot(Side::Left, DrawerView::Files),
        Err(LayoutError::AlreadyInDrawer)
    );
    assert_eq!(order.slot_views(Side::Left), vec![DrawerView::Files]);
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    assert_eq!(
        order.split_slot(Side::Left, DrawerView::Files),
        Err(LayoutError::AlreadyInDrawer),
        "refused before the cap check, order unchanged"
    );
    assert_eq!(
        order.slot_views(Side::Left),
        vec![DrawerView::Files, DrawerView::Links]
    );
}

// ── Uneven weights and tie-breaks ──────────────────────────────────────

/// Left drawer split 0.3/0.7 beside editor groups split 0.5/0.5:
///
/// ```text
/// 0.0 ┌──────┬──────┐
///     │ s0   │ top  │
/// 0.3 ├──────┤      │
///     │ s1   │      │
/// 0.5 │      ├──────┤
///     │      │bottom│
/// 1.0 └──────┴──────┘
/// ```
fn uneven() -> (PanelLayout<DrawerView>, GroupId, GroupId) {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    order.set_split_ratio(Side::Left, 0.3);
    let top = order.active_group();
    let bottom = order.split_group(top, Axis::Vertical).unwrap();
    (order, top, bottom)
}

#[test]
fn uneven_weights_only_reach_overlapping_neighbours() {
    let (mut order, _, bottom) = uneven();
    order.focus_region(Region::Group(bottom));
    // Only s1 (0.3..1.0) overlaps the bottom group (0.5..1.0).
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(slot(Side::Left, 1))
    );
    // From s1, bottom overlaps 0.5 and top 0.2: bottom.
    assert_eq!(
        order.focus_dir(Dir::Right),
        FocusMove::Moved(Region::Group(bottom))
    );
}

/// Tie-break order, pinned: nearest edge, then the **larger overlap**, then
/// the region its panel last focused, then top-left.
#[test]
fn larger_overlap_beats_the_last_focused_region() {
    let (mut order, top, _) = uneven();
    order.focus_region(slot(Side::Left, 1)); // the drawer remembers s1
    order.focus_region(Region::Group(top));
    // From top (0..0.5): s0 overlaps 0.3, s1 overlaps 0.2 → s0, though s1 was
    // the last focused slot.
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(slot(Side::Left, 0))
    );
    // From s0 (0..0.3) only top overlaps; the active group is top anyway.
    // From s1 with top remembered: bottom overlaps more (0.5 vs 0.2) and wins.
    order.focus_region(slot(Side::Left, 1));
    assert_eq!(order.active_group(), top);
    assert_eq!(
        order.focus_dir(Dir::Right),
        FocusMove::Moved(Region::Group(order.groups()[1]))
    );
}

#[test]
fn equal_overlap_goes_to_the_last_focused_region() {
    let mut order = layout();
    order.split_slot(Side::Left, DrawerView::Links).unwrap();
    // Both slots overlap the single full-height group equally (0.5 each).
    order.focus_region(slot(Side::Left, 1));
    order.focus(Panel::Editor);
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(slot(Side::Left, 1))
    );
    order.focus_region(slot(Side::Left, 0));
    order.focus(Panel::Editor);
    assert_eq!(
        order.focus_dir(Dir::Left),
        FocusMove::Moved(slot(Side::Left, 0))
    );
}
