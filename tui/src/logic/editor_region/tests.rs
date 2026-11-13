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

// ── Record and restore ───────────────────────────────────────────────────

use crate::settings::session::{AxisRecord, GroupTreeRecord, SessionRecord, TabRecord};
use kimun_core::nfs::VaultPath;

fn note(path: &str) -> VaultPath {
    VaultPath::new(path)
}

fn leaf_record(path: Option<&str>) -> GroupTreeRecord {
    GroupTreeRecord::Leaf {
        note: path.map(VaultPath::new),
    }
}

/// Two tabs: `a.md` alone; then `b.md` beside a column of an empty group
/// over `c.md`, at 0.25 / 0.75, the shown tab, with `c.md` active.
fn two_tab_region() -> (EditorRegion, Vec<(GroupId, VaultPath)>) {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    let (_, b) = r.new_tab();
    let empty = r.split_group(b, Axis::Horizontal).unwrap();
    let c = r.split_group(empty, Axis::Vertical).unwrap();
    r.resize_after(b, 0.25);
    r.set_active_group(c);
    (
        r,
        vec![(a, note("a.md")), (b, note("b.md")), (c, note("c.md"))],
    )
}

fn note_lookup(notes: &[(GroupId, VaultPath)]) -> impl Fn(GroupId) -> Option<VaultPath> + '_ {
    move |g| notes.iter().find(|(h, _)| *h == g).map(|(_, p)| p.clone())
}

/// The shape with every group replaced by the note it holds, so two regions
/// with different ids compare by arrangement.
fn shape_with_notes(r: &EditorRegion, tab: TabId, notes: &[(GroupId, VaultPath)]) -> String {
    let mut shown = r.clone();
    shown.show(tab);
    let mut text = format!("{:?}", shown.shape());
    for g in shown.groups(tab) {
        let name = notes
            .iter()
            .find(|(h, _)| *h == g)
            .map_or_else(|| "empty".to_string(), |(_, p)| p.to_string());
        text = text.replace(&format!("Group({g:?})"), &name);
    }
    text
}

#[test]
fn record_then_restore_gives_the_same_arrangement() {
    let (r, notes) = two_tab_region();
    let record = r.to_record(note_lookup(&notes));
    let (back, opened) = EditorRegion::from_record(&record, |_| true);
    assert_valid(&back);
    assert_eq!(back.tabs().len(), 2);
    for (was, now) in r.tabs().into_iter().zip(back.tabs()) {
        assert_eq!(
            shape_with_notes(&r, was, &notes),
            shape_with_notes(&back, now, &opened),
            "every tab's splits, weights and notes come back"
        );
    }
    assert_eq!(back.to_record(note_lookup(&opened)), record);
    assert_eq!(opened.len(), 3, "the empty group opens nothing");
}

#[test]
fn record_keeps_the_weights_exactly() {
    let mut r = EditorRegion::new();
    let a = r.active_group();
    let b = r.split_group(a, Axis::Horizontal).unwrap();
    r.resize_after(a, 0.3);
    let notes = vec![(a, note("a.md")), (b, note("b.md"))];
    let record = r.to_record(note_lookup(&notes));
    let GroupTreeRecord::Split { axis, weights, .. } = &record.tabs[0].tree else {
        panic!("a split is recorded as one: {record:?}");
    };
    assert_eq!(*axis, AxisRecord::Horizontal);
    assert_eq!(weights, &vec![0.3, 0.7]);
    let (back, _) = EditorRegion::from_record(&record, |_| true);
    assert_eq!(back.shape(), r.shape(), "the same weights, to the bit");
}

#[test]
fn restore_rebuilds_nested_splits() {
    let record = SessionRecord {
        tabs: vec![TabRecord {
            tree: GroupTreeRecord::Split {
                axis: AxisRecord::Vertical,
                weights: vec![0.5, 0.5],
                children: vec![
                    leaf_record(Some("top.md")),
                    GroupTreeRecord::Split {
                        axis: AxisRecord::Horizontal,
                        weights: vec![0.25, 0.25, 0.5],
                        children: vec![
                            leaf_record(Some("l.md")),
                            leaf_record(None),
                            leaf_record(Some("r.md")),
                        ],
                    },
                ],
            },
            active: 3,
        }],
        shown: 0,
    };
    let (r, opened) = EditorRegion::from_record(&record, |_| true);
    assert_valid(&r);
    let groups = r.groups(r.shown());
    assert_eq!(groups.len(), 4);
    assert_eq!(r.active_group(), groups[3]);
    let GroupShape::Split { axis, children } = r.shape() else {
        panic!("a split");
    };
    assert_eq!(axis, Axis::Vertical);
    assert_eq!(children[0], (0.5, GroupShape::Group(groups[0])));
    assert_eq!(
        children[1],
        (
            0.5,
            GroupShape::Split {
                axis: Axis::Horizontal,
                children: vec![
                    (0.25, GroupShape::Group(groups[1])),
                    (0.25, GroupShape::Group(groups[2])),
                    (0.5, GroupShape::Group(groups[3])),
                ],
            }
        )
    );
    assert_eq!(
        opened,
        vec![
            (groups[0], note("top.md")),
            (groups[1], note("l.md")),
            (groups[3], note("r.md")),
        ]
    );
    assert_eq!(r.to_record(note_lookup(&opened)), record);
}

#[test]
fn restore_drops_a_missing_note_and_keeps_the_rest() {
    let (r, notes) = two_tab_region();
    let record = r.to_record(note_lookup(&notes));
    let (back, opened) = EditorRegion::from_record(&record, |p| *p != note("b.md"));
    assert_valid(&back);
    assert_eq!(back.tabs().len(), 2, "the tab of the missing note stays");
    assert_eq!(back.all_groups().len(), 4, "its group stays, empty");
    let names: Vec<String> = opened.iter().map(|(_, p)| p.to_string()).collect();
    assert_eq!(
        names,
        vec![note("a.md").to_string(), note("c.md").to_string()]
    );
    let second = back.tabs()[1];
    let b_group = back.groups(second)[0];
    assert!(opened.iter().all(|(g, _)| *g != b_group));
}

#[test]
fn restore_with_every_note_missing_gives_one_empty_group() {
    let (r, notes) = two_tab_region();
    let record = r.to_record(note_lookup(&notes));
    let (back, opened) = EditorRegion::from_record(&record, |_| false);
    assert_valid(&back);
    assert_eq!(back.tabs().len(), 1);
    assert_eq!(back.all_groups().len(), 1);
    assert!(opened.is_empty());
    // A record of empty groups only is a fresh start too.
    let (back, opened) = EditorRegion::from_record(&r.to_record(|_| None), |_| true);
    assert_eq!(back.all_groups().len(), 1);
    assert!(opened.is_empty());
}

#[test]
fn restore_treats_an_empty_note_path_as_missing() {
    // An empty string loads as `VaultPath::new("")`: no note at all.
    let record = SessionRecord {
        tabs: vec![TabRecord {
            tree: GroupTreeRecord::Split {
                axis: AxisRecord::Horizontal,
                weights: vec![0.5, 0.5],
                children: vec![leaf_record(Some("")), leaf_record(Some("kept.md"))],
            },
            active: 0,
        }],
        shown: 0,
    };
    let (back, opened) = EditorRegion::from_record(&record, |_| true);
    assert_eq!(back.all_groups().len(), 2);
    assert_eq!(opened, vec![(back.all_groups()[1], note("kept.md"))]);
}

#[test]
fn restore_ignores_a_duplicate_note_and_keeps_the_first() {
    let record = SessionRecord {
        tabs: vec![
            TabRecord {
                tree: leaf_record(Some("same.md")),
                active: 0,
            },
            TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Horizontal,
                    weights: vec![0.5, 0.5],
                    // The same note again, spelled with other case.
                    children: vec![leaf_record(Some("SAME.md")), leaf_record(Some("other.md"))],
                },
                active: 0,
            },
        ],
        shown: 1,
    };
    let (back, opened) = EditorRegion::from_record(&record, |_| true);
    assert_valid(&back);
    let first = back.groups(back.tabs()[0])[0];
    let second_tab = back.groups(back.tabs()[1]);
    assert_eq!(
        opened,
        vec![(first, note("same.md")), (second_tab[1], note("other.md"))],
        "a note opens in one group only: the first that held it"
    );
    assert_eq!(second_tab.len(), 2, "the duplicate's group stays, empty");
}

#[test]
fn restore_shows_the_recorded_tab_and_active_group() {
    let record = SessionRecord {
        tabs: vec![
            TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Horizontal,
                    weights: vec![0.5, 0.5],
                    children: vec![leaf_record(Some("a.md")), leaf_record(Some("b.md"))],
                },
                active: 1,
            },
            TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Vertical,
                    weights: vec![0.5, 0.5],
                    children: vec![leaf_record(Some("c.md")), leaf_record(Some("d.md"))],
                },
                active: 0,
            },
        ],
        shown: 0,
    };
    let (back, _) = EditorRegion::from_record(&record, |_| true);
    let tabs = back.tabs();
    assert_eq!(back.shown(), tabs[0]);
    assert_eq!(back.active_group(), back.groups(tabs[0])[1]);
    assert_eq!(
        back.active_group_of(tabs[1]),
        Some(back.groups(tabs[1])[0]),
        "a hidden tab keeps its own active group"
    );
    // New ids never collide with the restored ones.
    let mut back = back;
    let before = back.all_groups();
    let (_, fresh) = back.new_tab();
    assert!(!before.contains(&fresh));
}
