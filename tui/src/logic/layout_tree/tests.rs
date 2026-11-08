use std::collections::BTreeMap;

use super::*;

type Tree = LayoutTree<&'static str>;

fn tree() -> Tree {
    LayoutTree::default_layout("files")
}

fn unit() -> UnitRect {
    UnitRect::new(0.0, 0.0, 1.0, 1.0)
}

/// Left and right slot of the default layout.
fn sides(t: &Tree) -> (SlotId, SlotId) {
    let s = t.slots();
    assert_eq!(s.len(), 2);
    (s[0], s[1])
}

#[test]
fn default_layout_is_left_files_editor_empty_right() {
    let t = tree();
    let (left, right) = sides(&t);
    let g = GroupId::new(0);
    let leaves: Vec<Leaf> = t.node().leaves().into_iter().copied().collect();
    assert_eq!(
        leaves,
        vec![Leaf::Slot(left), Leaf::Group(g), Leaf::Slot(right)]
    );
    assert_eq!(t.slot_views(left), vec!["files"]);
    assert_eq!(t.shown(left), Some("files"));
    assert!(t.slot_views(right).is_empty());
    assert_eq!(t.shown(right), None);
    assert_eq!(t.groups(), vec![g]);
    assert_eq!(t.active_group(), g);
    assert_eq!(t.focused(), Focus::Leaf(Leaf::Group(g)));
    let widths: Vec<f64> = t
        .node()
        .layout(unit())
        .iter()
        .map(|(_, r)| r.width())
        .collect();
    for (w, want) in widths.iter().zip([0.2, 0.6, 0.2]) {
        assert!((w - want).abs() < 1e-9, "{widths:?}");
    }
}

#[test]
fn show_view_hides_the_view_sharing_its_slot_but_keeps_its_placement() {
    let mut t = tree();
    let (left, _) = sides(&t);
    assert!(t.place("tags", left, false));
    assert_eq!(t.show_view(&"tags", false), Some(left));
    assert_eq!(t.shown(left), Some("tags"));
    assert_eq!(t.view_slot(&"files"), Some(left));
    assert_eq!(t.slot_views(left), vec!["files", "tags"]);
}

#[test]
fn hidden_view_returns_to_its_slot_when_shown() {
    let mut t = tree();
    let (_, right) = sides(&t);
    assert!(t.place("links", right, true));
    assert_eq!(t.hide_view(&"links"), Some(right));
    assert_eq!(t.shown(right), None);
    assert_eq!(t.view_slot(&"links"), Some(right));
    assert_eq!(t.hide_view(&"links"), None, "already hidden");
    // home_right is irrelevant for a placed view.
    assert_eq!(t.show_view(&"links", false), Some(right));
    assert_eq!(t.shown(right), Some("links"));
}

#[test]
fn unplaced_view_opens_in_first_slot_or_last_slot_when_home_right() {
    let mut t = tree();
    let (left, right) = sides(&t);
    assert_eq!(t.show_view(&"tags", false), Some(left));
    assert_eq!(t.shown(left), Some("tags"));
    assert_eq!(t.view_slot(&"files"), Some(left), "files stays placed");
    assert_eq!(t.show_view(&"ask", true), Some(right));
    assert_eq!(t.shown(right), Some("ask"));
    // A tree with no slot at all has nowhere to put it.
    let mut bare: Tree =
        LayoutTree::from_parts(Node::leaf(Leaf::Group(GroupId::new(0))), BTreeMap::new()).unwrap();
    assert_eq!(bare.show_view(&"tags", false), None);
    assert_eq!(bare.view_slot(&"tags"), None);
}

#[test]
fn move_view_replaces_shown_and_keeps_old_hidden_there() {
    let mut t = tree();
    let (left, right) = sides(&t);
    assert!(t.place("links", right, true));
    assert!(t.move_view("files", right));
    assert_eq!(t.shown(right), Some("files"));
    assert_eq!(t.view_slot(&"links"), Some(right));
    assert_eq!(t.slot_views(right), vec!["links", "files"]);
    assert!(t.slot_views(left).is_empty());
    assert_eq!(t.shown(left), None);
    // An unplaced view can be moved in too; an absent slot refuses.
    assert!(t.move_view("tags", left));
    assert_eq!(t.shown(left), Some("tags"));
    assert!(!t.move_view("tags", SlotId::new(99)));
    assert_eq!(t.view_slot(&"tags"), Some(left));
}

#[test]
fn split_slot_adds_an_empty_slot_after() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let new = t.split_slot(left, Axis::Vertical).unwrap();
    assert_eq!(t.slots(), vec![left, new, right]);
    assert!(t.slot_views(new).is_empty());
    assert_eq!(t.shown(left), Some("files"));
    assert_ne!(new, left);
    assert_ne!(new, right);
    assert_eq!(
        t.split_slot(SlotId::new(99), Axis::Vertical),
        Err(TreeError::NoSuchSlot)
    );
    // Ids stay unique even after a close.
    t.close_slot(new).unwrap();
    let again = t.split_slot(left, Axis::Vertical).unwrap();
    assert!(again != new && again != left && again != right);
}

#[test]
fn close_slot_merges_views_hidden_into_absorber() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let below = t.split_slot(left, Axis::Vertical).unwrap();
    assert!(t.place("tags", below, true));
    assert!(t.place("links", below, false));
    // `below` is the last child of the left column: `left` takes its area.
    assert_eq!(t.close_slot(below), Ok(left));
    assert_eq!(t.slots(), vec![left, right]);
    assert_eq!(t.slot_views(left), vec!["files", "tags", "links"]);
    assert_eq!(t.shown(left), Some("files"), "merged views arrive hidden");
    assert_eq!(t.view_slot(&"tags"), Some(left));
    // The editor takes the left slot's area; its views go to the nearest
    // remaining slot on that side, hidden.
    assert!(t.place("ask", right, true));
    assert_eq!(t.close_slot(left), Ok(right));
    assert_eq!(t.slot_views(right), vec!["ask", "files", "tags", "links"]);
    assert_eq!(t.shown(right), Some("ask"));
    assert_eq!(t.close_slot(left), Err(TreeError::NoSuchSlot));
}

#[test]
fn close_slot_reports_absorber_for_focus() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let below = t.split_slot(left, Axis::Vertical).unwrap();
    assert!(t.focus(Focus::Leaf(Leaf::Slot(below))));
    let absorber = t.close_slot(below).unwrap();
    assert_eq!(absorber, left);
    assert_eq!(t.focused(), Focus::Leaf(Leaf::Slot(left)));
    // Closing an unfocused slot leaves focus alone.
    assert_eq!(t.close_slot(right), Ok(left));
    assert_eq!(t.focused(), Focus::Leaf(Leaf::Slot(left)));
    // The only remaining slot cannot close: its views would have nowhere to go.
    assert_eq!(t.close_slot(left), Err(TreeError::LastSlot));
    assert_eq!(t.slots(), vec![left]);
    assert_eq!(t.slot_views(left), vec!["files"]);
}

#[test]
fn close_last_group_is_refused() {
    let mut t = tree();
    let g = t.active_group();
    assert_eq!(t.close_group(g), Err(TreeError::LastGroup));
    let g2 = t.split_group(g, Axis::Horizontal).unwrap();
    assert_eq!(t.groups(), vec![g, g2]);
    assert!(t.focus(Focus::Leaf(Leaf::Group(g2))));
    assert_eq!(t.active_group(), g2);
    assert_eq!(t.close_group(g2), Ok(()));
    assert_eq!(t.groups(), vec![g]);
    assert_eq!(t.active_group(), g);
    assert_eq!(t.focused(), Focus::Leaf(Leaf::Group(g)));
    assert_eq!(t.close_group(g2), Err(TreeError::NoSuchGroup));
    assert_eq!(t.close_group(g), Err(TreeError::LastGroup));
    assert_eq!(
        t.split_group(g2, Axis::Vertical),
        Err(TreeError::NoSuchGroup)
    );
}

#[test]
fn a_view_is_in_at_most_one_slot() {
    let mut t = tree();
    let (left, right) = sides(&t);
    assert!(!t.place("files", right, false), "already placed");
    assert!(!t.place("tags", left, true), "left already shows a view");
    assert!(!t.place("tags", SlotId::new(99), false));
    assert!(t.move_view("files", right));
    assert_eq!(t.slot_views(left), Vec::<&str>::new());
    assert_eq!(t.slot_views(right), vec!["files"]);

    let root = || {
        Node::split(
            Axis::Horizontal,
            vec![
                (1.0, Node::leaf(Leaf::Slot(SlotId::new(0)))),
                (1.0, Node::leaf(Leaf::Group(GroupId::new(0)))),
                (1.0, Node::leaf(Leaf::Slot(SlotId::new(1)))),
            ],
        )
    };
    let parts = |a: Vec<&'static str>, b: Vec<&'static str>| {
        BTreeMap::from([(SlotId::new(0), (a, None)), (SlotId::new(1), (b, None))])
    };
    assert_eq!(
        Tree::from_parts(root(), parts(vec!["files"], vec!["files"])).err(),
        Some(TreeError::DuplicateView)
    );
    assert_eq!(
        Tree::from_parts(root(), parts(vec!["tags", "tags"], vec![])).err(),
        Some(TreeError::DuplicateView)
    );
}

#[test]
fn from_parts_validates_its_parts() {
    let s0 = SlotId::new(0);
    let g0 = GroupId::new(0);
    let row = Node::split(
        Axis::Horizontal,
        vec![
            (1.0, Node::leaf(Leaf::Slot(s0))),
            (1.0, Node::leaf(Leaf::Group(g0))),
        ],
    );
    let ok = Tree::from_parts(
        row.clone(),
        BTreeMap::from([(s0, (vec!["files", "tags"], Some("tags")))]),
    )
    .unwrap();
    assert_eq!(ok.shown(s0), Some("tags"));
    assert_eq!(ok.focused(), Focus::Leaf(Leaf::Group(g0)));

    let no_group: Node<Leaf> = Node::leaf(Leaf::Slot(s0));
    assert_eq!(
        Tree::from_parts(no_group, BTreeMap::from([(s0, (vec![], None))])).err(),
        Some(TreeError::NoGroup)
    );
    assert_eq!(
        Tree::from_parts(row.clone(), BTreeMap::new()).err(),
        Some(TreeError::MissingSlot)
    );
    assert_eq!(
        Tree::from_parts(
            row.clone(),
            BTreeMap::from([(s0, (vec![], None)), (SlotId::new(7), (vec![], None))])
        )
        .err(),
        Some(TreeError::NoSuchSlot)
    );
    assert_eq!(
        Tree::from_parts(
            row.clone(),
            BTreeMap::from([(s0, (vec!["files"], Some("tags")))])
        )
        .err(),
        Some(TreeError::NoSuchView)
    );
    let twice = Node::split(
        Axis::Horizontal,
        vec![
            (1.0, Node::leaf(Leaf::Group(g0))),
            (1.0, Node::leaf(Leaf::Group(g0))),
        ],
    );
    assert_eq!(
        Tree::from_parts(twice, BTreeMap::new()).err(),
        Some(TreeError::DuplicateLeaf)
    );
}

#[test]
fn new_ids_never_collide_with_given_ones() {
    let root = Node::split(
        Axis::Horizontal,
        vec![
            (1.0, Node::leaf(Leaf::Group(GroupId::new(3)))),
            (1.0, Node::leaf(Leaf::Slot(SlotId::new(5)))),
        ],
    );
    let mut t = Tree::from_parts(root, BTreeMap::from([(SlotId::new(5), (vec![], None))])).unwrap();
    let g = t.split_group(GroupId::new(3), Axis::Vertical).unwrap();
    assert_ne!(g, GroupId::new(3));
    let s = t.split_slot(SlotId::new(5), Axis::Vertical).unwrap();
    assert_ne!(s, SlotId::new(5));
    assert!(t.resize_after(Leaf::Group(GroupId::new(3)), 0.3));
}
