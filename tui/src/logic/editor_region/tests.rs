use super::*;

fn unit() -> UnitRect {
    UnitRect::new(0.0, 0.0, 1.0, 1.0)
}

#[test]
fn new_region_has_one_tab_with_one_group() {
    let r = EditorRegion::new();
    assert_eq!(r.tabs().len(), 1);
    assert_eq!(r.shown(), r.tabs()[0]);
    assert_eq!(r.groups(r.shown()), vec![r.active_group()]);
    assert_eq!(r.all_groups().len(), 1);
}

#[test]
fn new_tab_is_shown_and_holds_one_group() {
    let mut r = EditorRegion::new();
    let (tab, group) = r.new_tab();
    assert_eq!(r.tabs().len(), 2);
    assert_eq!(r.tabs()[1], tab);
    assert_eq!(r.shown(), tab);
    assert_eq!(r.groups(tab), vec![group]);
    assert_eq!(r.active_group(), group);
    assert_eq!(r.group_tab(group), Some(tab));
}

#[test]
fn close_last_tab_is_refused() {
    let mut r = EditorRegion::new();
    let only = r.shown();
    assert_eq!(r.close_tab(only), None);
    assert_eq!(r.tabs(), vec![only]);
}

#[test]
fn close_tab_returns_its_groups() {
    let mut r = EditorRegion::new();
    let (tab, first) = r.new_tab();
    let second = r.split_group(first, Axis::Horizontal).unwrap();
    assert_eq!(r.close_tab(tab), Some(vec![first, second]));
    assert_eq!(r.tabs().len(), 1);
    assert_eq!(r.group_tab(first), None);
    assert_eq!(r.close_tab(tab), None);
}

#[test]
fn closing_the_shown_tab_shows_its_neighbor() {
    let mut r = EditorRegion::new();
    let a = r.shown();
    let (b, _) = r.new_tab();
    let (c, _) = r.new_tab();
    r.show(b);
    r.close_tab(b);
    assert_eq!(r.shown(), c, "the next tab takes its place");
    r.close_tab(c);
    assert_eq!(r.shown(), a, "the last tab falls back to the previous");
    // Closing a tab that is not shown leaves the shown one alone.
    let (d, _) = r.new_tab();
    r.show(a);
    r.close_tab(d);
    assert_eq!(r.shown(), a);
}

#[test]
fn next_and_prev_tab_wrap() {
    let mut r = EditorRegion::new();
    let a = r.shown();
    let (b, _) = r.new_tab();
    let (c, _) = r.new_tab();
    r.show(a);
    r.next_tab();
    assert_eq!(r.shown(), b);
    r.next_tab();
    r.next_tab();
    assert_eq!(r.shown(), a);
    r.prev_tab();
    assert_eq!(r.shown(), c);
    r.prev_tab();
    assert_eq!(r.shown(), b);
    r.move_tab(c, 0);
    assert_eq!(r.tabs(), vec![c, a, b]);
    assert_eq!(r.shown(), b);
}

#[test]
fn split_group_adds_a_group_after_it_in_the_same_tab() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    let tab = r.shown();
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    assert_eq!(r.groups(tab), vec![a, b]);
    assert_eq!(r.group_tab(b), Some(tab));
    assert_eq!(r.split_group(GroupId::new(99), Axis::Vertical), None);
}

#[test]
fn split_group_keeps_the_active_group() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    r.split_group(a, Axis::Vertical).unwrap();
    assert_eq!(r.active_group(), a);
}

#[test]
fn close_last_group_of_a_tab_closes_the_tab() {
    let mut r = EditorRegion::new();
    let first = r.shown();
    let (tab, group) = r.new_tab();
    assert_eq!(r.close_group(group), Some(CloseOutcome::Tab(tab)));
    assert_eq!(r.tabs(), vec![first]);
    assert_eq!(r.shown(), first);
    // A group of a tab with others only leaves that group.
    let a = r.active_group();
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    r.set_active_group(b);
    assert_eq!(r.close_group(b), Some(CloseOutcome::Group));
    assert_eq!(r.active_group(), a);
    assert_eq!(r.close_group(b), None);
}

#[test]
fn close_last_group_of_last_tab_reports_emptied_and_keeps_the_group() {
    let mut r = EditorRegion::new();
    let (tab, group) = (r.shown(), r.active_group());
    assert_eq!(r.close_group(group), Some(CloseOutcome::Emptied));
    assert_eq!(r.tabs(), vec![tab]);
    assert_eq!(r.groups(tab), vec![group]);
    assert_eq!(r.active_group(), group);
}

#[test]
fn set_active_group_shows_its_tab() {
    let mut r = EditorRegion::new();
    let first = r.shown();
    let a = r.active_group();
    let (second, b) = r.new_tab();
    assert!(r.set_active_group(a));
    assert_eq!(r.shown(), first);
    assert_eq!(r.active_group(), a);
    assert!(r.set_active_group(b));
    assert_eq!(r.shown(), second);
    assert!(!r.set_active_group(GroupId::new(99)));
    assert_eq!(r.shown(), second);
}

#[test]
fn group_ids_are_never_reused_across_tabs() {
    let mut r = EditorRegion::new();
    let mut seen = r.all_groups();
    let (tab, g) = r.new_tab();
    seen.push(g);
    r.close_tab(tab);
    let (_, g) = r.new_tab();
    assert!(!seen.contains(&g));
    seen.push(g);
    let s = r.split_group(g, Axis::Horizontal).unwrap();
    assert!(!seen.contains(&s));
    seen.push(s);
    r.close_group(s);
    let t = r.split_group(g, Axis::Horizontal).unwrap();
    assert!(!seen.contains(&t));
}

#[test]
fn regions_cover_the_unit_square_for_the_shown_tab() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    assert_eq!(r.regions(), vec![(a, unit())]);
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    let (_, other) = r.new_tab();
    assert_eq!(r.regions(), vec![(other, unit())]);
    r.set_active_group(a);
    assert_eq!(
        r.regions(),
        vec![
            (a, UnitRect::new(0.0, 0.0, 0.5, 1.0)),
            (b, UnitRect::new(0.5, 0.0, 0.5, 1.0)),
        ]
    );
    assert!(r.resize_after(a, 0.25));
    assert_eq!(r.regions()[0].1, UnitRect::new(0.0, 0.0, 0.25, 1.0));
    assert!(!r.resize_after(b, 0.5));
}

#[test]
fn move_group_to_tab_adds_a_split_right_of_the_target_active_group() {
    let mut r = EditorRegion::new();
    let first = r.shown();
    let a = r.active_group();
    let a2 = r.split_group(a, Axis::Horizontal).unwrap();
    let (second, b) = r.new_tab();
    assert!(r.move_group_to_tab(a2, second));
    assert_eq!(r.groups(first), vec![a]);
    assert_eq!(r.groups(second), vec![b, a2], "keeps its id, right of b");
    assert_eq!(r.group_tab(a2), Some(second));
    assert_eq!(r.shown(), second);
    assert_eq!(r.active_group(), a2);
    // Refused: same tab, unknown tab, unknown group.
    assert!(!r.move_group_to_tab(a2, second));
    assert!(!r.move_group_to_tab(a, TabId(99)));
    assert!(!r.move_group_to_tab(GroupId::new(99), first));
    // The source tab's active group moved off the group that left.
    r.set_active_group(a);
    let c = r.split_group(a, Axis::Vertical).unwrap();
    r.set_active_group(c);
    assert!(r.move_group_to_tab(c, second));
    assert_eq!(r.groups(first), vec![a]);
    assert!(r.set_active_group(a));
    assert_eq!(r.active_group(), a);
}

#[test]
fn move_group_that_is_alone_closes_its_old_tab() {
    let mut r = EditorRegion::new();
    let first = r.shown();
    let a = r.active_group();
    let (second, b) = r.new_tab();
    r.show(first);
    assert!(r.move_group_to_tab(a, second));
    assert_eq!(r.tabs(), vec![second]);
    assert_eq!(r.shown(), second, "the target is shown");
    assert_eq!(r.groups(second), vec![b, a]);
    // A hidden tab that empties does not change what is shown.
    let (third, c) = r.new_tab();
    r.show(second);
    assert!(r.move_group_to_tab(c, second));
    assert_eq!(r.tabs(), vec![second]);
    assert_eq!(r.shown(), second);
    assert_eq!(r.group_tab(c), Some(second));
    assert_eq!(r.group_tab(a), Some(second));
    let _ = third;
}

#[test]
fn active_group_of_names_a_hidden_tabs_active_group() {
    let mut r = EditorRegion::new();
    let first_tab = r.shown();
    let first = r.active_group();
    let second = r.split_group(first, Axis::Horizontal).unwrap();
    assert!(r.set_active_group(second));
    let (tab, group) = r.new_tab();
    assert_eq!(r.shown(), tab);
    assert_eq!(r.active_group_of(first_tab), Some(second));
    assert_eq!(r.active_group_of(tab), Some(group));
    r.close_tab(tab);
    assert_eq!(r.active_group_of(tab), None);
}
