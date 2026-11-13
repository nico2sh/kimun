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

/// The shape names the shown tab's arrangement with normalized weights;
/// the widest row counts the groups side by side, a stacked split taking
/// its widest child.
#[test]
fn shape_and_widest_row_follow_the_shown_tab() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    assert_eq!(r.shape(), GroupShape::Group(a));
    assert_eq!(r.widest_row(), 1);
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    assert_eq!(
        r.shape(),
        GroupShape::Split {
            axis: Axis::Horizontal,
            children: vec![(0.5, GroupShape::Group(a)), (0.5, GroupShape::Group(b))],
        }
    );
    assert_eq!(r.widest_row(), 2);
    assert_eq!(r.shape().last_group(), b);
    // Stacking under `b` adds no width; splitting the lower one does.
    let c = r.split_group(b, Axis::Vertical).unwrap();
    assert_eq!(r.widest_row(), 2);
    assert_eq!(r.shape().last_group(), c);
    let d = r.split_group(c, Axis::Horizontal).unwrap();
    assert_eq!(r.widest_row(), 3);
    assert_eq!(r.shape().last_group(), d);
    // Another tab has its own shape.
    let (_, e) = r.new_tab();
    assert_eq!(r.shape(), GroupShape::Group(e));
    assert_eq!(r.widest_row(), 1);
}

/// A closed group's area goes to one neighbour, the group before it (the
/// one after it when it is first); every other group keeps its size.
#[test]
fn closing_a_group_gives_its_area_to_the_group_before_it() {
    let width = |r: &EditorRegion| -> Vec<(GroupId, f64)> {
        r.regions()
            .into_iter()
            .map(|(g, rect)| (g, rect.width()))
            .collect()
    };
    let mut r = EditorRegion::new();
    let a = r.active_group();
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    let c = r.split_group(b, Axis::Horizontal).unwrap();
    assert_eq!(width(&r), vec![(a, 0.5), (b, 0.25), (c, 0.25)]);
    assert_eq!(r.close_group(b), Some(CloseOutcome::Group));
    assert_eq!(width(&r), vec![(a, 0.75), (c, 0.25)]);
    // The first group's area goes to the one after it.
    let d = r.split_group(c, Axis::Horizontal).unwrap();
    assert_eq!(width(&r), vec![(a, 0.75), (c, 0.125), (d, 0.125)]);
    assert_eq!(r.close_group(a), Some(CloseOutcome::Group));
    assert_eq!(width(&r), vec![(c, 0.875), (d, 0.125)]);
}

/// Every invariant of the region: each tab holds a group, the shown tab
/// exists, each tab's active group is one of its own, and no group sits in
/// two places.
fn assert_valid(r: &EditorRegion) {
    assert!(!r.tabs().is_empty(), "the region holds a tab");
    assert!(r.tabs().contains(&r.shown()), "the shown tab exists");
    for tab in r.tabs() {
        let groups = r.groups(tab);
        assert!(!groups.is_empty(), "{tab:?} holds a group");
        let active = r.active_group_of(tab).unwrap();
        assert!(
            groups.contains(&active),
            "{tab:?}'s active group is its own"
        );
    }
    let mut all = r.all_groups();
    let n = all.len();
    all.sort_by_key(|g| format!("{g:?}"));
    all.dedup();
    assert_eq!(all.len(), n, "no group sits in two places");
}

#[test]
fn move_group_beside_relocates_a_group_within_its_tab() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    let c = r.split_group(b, Axis::Horizontal).unwrap();
    // c goes above a: a column of [c, a] left of b.
    assert!(r.move_group_beside(c, a, Dir::Up));
    assert_eq!(r.groups(r.shown()), vec![c, a, b], "keeps its id");
    assert_eq!(
        r.shape(),
        GroupShape::Split {
            axis: Axis::Horizontal,
            children: vec![
                (
                    0.5,
                    GroupShape::Split {
                        axis: Axis::Vertical,
                        children: vec![(0.5, GroupShape::Group(c)), (0.5, GroupShape::Group(a))],
                    }
                ),
                (0.5, GroupShape::Group(b)),
            ],
        }
    );
    assert_eq!(r.active_group(), c, "the moved group is active");
    // Each side of the target.
    assert!(r.move_group_beside(c, b, Dir::Right));
    assert_eq!(r.groups(r.shown()), vec![a, b, c]);
    assert!(r.move_group_beside(c, b, Dir::Left));
    assert_eq!(r.groups(r.shown()), vec![a, c, b]);
    assert!(r.move_group_beside(a, b, Dir::Down));
    assert_eq!(r.groups(r.shown()), vec![c, b, a]);
    assert_eq!(r.tabs().len(), 1);
    assert_valid(&r);
}

#[test]
fn move_group_beside_refuses_itself_and_unknown_groups() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    let before = r.clone();
    assert!(!r.move_group_beside(a, a, Dir::Left));
    assert!(!r.move_group_beside(GroupId::new(99), a, Dir::Left));
    assert!(!r.move_group_beside(a, GroupId::new(99), Dir::Left));
    assert_eq!(r, before, "nothing changed");
    let _ = b;
}

#[test]
fn move_group_beside_across_tabs_closes_a_source_tab_left_empty() {
    let mut r = EditorRegion::new();
    let first = r.shown();
    let a = r.active_group();
    let (second, b) = r.new_tab();
    let b2 = r.split_group(b, Axis::Horizontal).unwrap();
    // b2 leaves a tab that keeps b: that tab stays, b active there.
    r.set_active_group(b2);
    assert!(r.move_group_beside(b2, a, Dir::Left));
    assert_eq!(r.groups(first), vec![b2, a]);
    assert_eq!(r.groups(second), vec![b]);
    assert_eq!(r.active_group_of(second), Some(b));
    assert_eq!(r.active_group_of(first), Some(b2));
    assert_eq!(r.shown(), second, "the shown tab is kept");
    assert_valid(&r);
    // b is alone in the shown tab: the tab closes and the target shows.
    assert!(r.move_group_beside(b, a, Dir::Down));
    assert_eq!(r.tabs(), vec![first]);
    assert_eq!(r.shown(), first);
    assert_eq!(r.groups(first), vec![b2, a, b]);
    assert_eq!(r.active_group(), b);
    assert_valid(&r);
    // Ids are never reused after the tab closed.
    let (_, fresh) = r.new_tab();
    assert!(![a, b, b2].contains(&fresh));
}

#[test]
fn swap_groups_trades_places_within_a_tab() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    let c = r.split_group(b, Axis::Vertical).unwrap();
    r.resize_after(a, 0.3);
    assert!(r.swap_groups(a, c));
    assert_eq!(
        r.shape(),
        GroupShape::Split {
            axis: Axis::Horizontal,
            children: vec![
                (0.3, GroupShape::Group(c)),
                (
                    0.7,
                    GroupShape::Split {
                        axis: Axis::Vertical,
                        children: vec![(0.5, GroupShape::Group(b)), (0.5, GroupShape::Group(a))],
                    }
                ),
            ],
        },
        "the places and their sizes stay; the groups trade them"
    );
    assert_eq!(r.active_group(), a, "the active group is kept");
    assert_valid(&r);
}

#[test]
fn swap_groups_works_across_tabs_and_keeps_active_groups_valid() {
    let mut r = EditorRegion::new();
    let first = r.shown();
    let a = r.active_group();
    let a2 = r.split_group(a, Axis::Horizontal).unwrap();
    let (second, b) = r.new_tab();
    assert!(r.swap_groups(a, b));
    assert_eq!(r.groups(first), vec![b, a2]);
    assert_eq!(r.groups(second), vec![a]);
    assert_eq!(
        r.active_group_of(first),
        Some(b),
        "the active place follows"
    );
    assert_eq!(r.active_group_of(second), Some(a));
    assert_eq!(r.shown(), second);
    assert_valid(&r);
    // Refused: itself, unknown.
    let before = r.clone();
    assert!(!r.swap_groups(a, a));
    assert!(!r.swap_groups(a, GroupId::new(99)));
    assert!(!r.swap_groups(GroupId::new(99), a));
    assert_eq!(r, before);
}

#[test]
fn move_group_to_new_tab_takes_the_group_alone_into_a_new_last_tab() {
    let mut r = EditorRegion::new();
    let first = r.shown();
    let a = r.active_group();
    let a2 = r.split_group(a, Axis::Horizontal).unwrap();
    r.set_active_group(a2);
    let tab = r.move_group_to_new_tab(a2).unwrap();
    assert_eq!(r.tabs(), vec![first, tab]);
    assert_eq!(r.groups(tab), vec![a2], "keeps its id");
    assert_eq!(r.active_group_of(tab), Some(a2));
    assert_eq!(r.groups(first), vec![a]);
    assert_eq!(r.shown(), first, "the source tab stays shown");
    assert_eq!(r.active_group(), a);
    assert_valid(&r);
    // Alone in the shown tab: that tab closes, the new one shows.
    let tab2 = r.move_group_to_new_tab(a).unwrap();
    assert_eq!(r.tabs(), vec![tab, tab2]);
    assert_eq!(r.groups(tab2), vec![a]);
    assert_eq!(r.shown(), tab2);
    assert_valid(&r);
    // Alone in the only tab: refused, nothing changes.
    let mut lone = EditorRegion::new();
    let only = lone.active_group();
    let before = lone.clone();
    assert_eq!(lone.move_group_to_new_tab(only), None);
    assert_eq!(lone.move_group_to_new_tab(GroupId::new(99)), None);
    assert_eq!(lone, before);
}
