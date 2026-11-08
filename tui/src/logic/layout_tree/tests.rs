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

// ── Regions, geometric focus and zoom ─────────────────────────────────

/// slot0 (0.2) | group0 (0.6) | [slot1 over slot2] (0.2)
fn nested() -> (Tree, [SlotId; 3], GroupId) {
    let s = [SlotId::new(0), SlotId::new(1), SlotId::new(2)];
    let g = GroupId::new(0);
    let root = Node::split(
        Axis::Horizontal,
        vec![
            (0.2, Node::leaf(Leaf::Slot(s[0]))),
            (0.6, Node::leaf(Leaf::Group(g))),
            (
                0.2,
                Node::split(
                    Axis::Vertical,
                    vec![
                        (1.0, Node::leaf(Leaf::Slot(s[1]))),
                        (1.0, Node::leaf(Leaf::Slot(s[2]))),
                    ],
                ),
            ),
        ],
    );
    let slots = BTreeMap::from([
        (s[0], (vec!["files"], Some("files"))),
        (s[1], (Vec::new(), None)),
        (s[2], (Vec::new(), None)),
    ]);
    (Tree::from_parts(root, slots).unwrap(), s, g)
}

fn leaf(l: Leaf) -> Focus {
    Focus::Leaf(l)
}

#[test]
fn regions_cover_the_unit_square_without_the_rail() {
    let (t, s, g) = nested();
    let r = t.regions();
    let leaves: Vec<Leaf> = r.iter().map(|(l, _)| *l).collect();
    assert_eq!(
        leaves,
        vec![
            Leaf::Slot(s[0]),
            Leaf::Group(g),
            Leaf::Slot(s[1]),
            Leaf::Slot(s[2])
        ]
    );
    let area: f64 = r.iter().map(|(_, c)| c.width() * c.height()).sum();
    assert!((area - 1.0).abs() < 1e-9);
    assert!((r[3].1.y() - 0.5).abs() < 1e-9);
}

#[test]
fn focus_dir_follows_geometry_between_nested_leaves() {
    let (mut t, s, g) = nested();
    let m = |l| TreeFocusMove::Moved(leaf(l));
    // Two equal candidates on the right: the top-left one wins.
    assert_eq!(t.focus_dir(Dir::Right), m(Leaf::Slot(s[1])));
    assert_eq!(t.focus_dir(Dir::Down), m(Leaf::Slot(s[2])));
    assert_eq!(t.focus_dir(Dir::Left), m(Leaf::Group(g)));
    // The remembered leaf (last focused slot) breaks the tie.
    assert_eq!(t.focus_dir(Dir::Right), m(Leaf::Slot(s[2])));
    assert_eq!(t.focus_dir(Dir::Left), m(Leaf::Group(g)));
    assert_eq!(t.focus_dir(Dir::Left), m(Leaf::Slot(s[0])));
    assert_eq!(t.focused(), leaf(Leaf::Slot(s[0])));
    // Back right into the group, which stays active.
    assert_eq!(t.focus_dir(Dir::Right), m(Leaf::Group(g)));
    assert_eq!(t.active_group(), g);
}

#[test]
fn focus_dir_does_not_wrap_at_edges() {
    let (mut t, s, _) = nested();
    assert!(t.focus(leaf(Leaf::Slot(s[2]))));
    assert_eq!(t.focus_dir(Dir::Right), TreeFocusMove::AtEdge);
    assert_eq!(t.focus_dir(Dir::Down), TreeFocusMove::AtEdge);
    assert!(t.focus(leaf(Leaf::Slot(s[1]))));
    // slot2 is stacked in the same column: Up is handled, no wrap.
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::AtEdge);
    assert_eq!(t.focused(), leaf(Leaf::Slot(s[1])));
    // From the rail, Left is the edge.
    assert!(t.focus(Focus::Rail));
    assert_eq!(t.focus_dir(Dir::Left), TreeFocusMove::AtEdge);
    assert_eq!(t.focused(), Focus::Rail);
}

#[test]
fn up_down_with_nothing_stacked_is_not_handled() {
    let mut t = tree();
    let g = t.active_group();
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Down), TreeFocusMove::NotHandled);
    assert_eq!(t.focused(), leaf(Leaf::Group(g)));
    assert!(t.focus(Focus::Rail));
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Down), TreeFocusMove::NotHandled);
}

#[test]
fn left_from_leftmost_leaf_reaches_the_rail() {
    let mut t = tree();
    let (left, _) = sides(&t);
    assert_eq!(
        t.focus_dir(Dir::Left),
        TreeFocusMove::Moved(leaf(Leaf::Slot(left)))
    );
    assert_eq!(t.focus_dir(Dir::Left), TreeFocusMove::Moved(Focus::Rail));
    assert_eq!(t.focused(), Focus::Rail);
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(leaf(Leaf::Slot(left)))
    );

    // The first leaf is leftmost, then topmost.
    let (mut t, s, _) = nested();
    let new = t.split_slot(s[0], Axis::Vertical).unwrap();
    assert!(t.focus(leaf(Leaf::Slot(new))));
    assert_eq!(t.focus_dir(Dir::Left), TreeFocusMove::Moved(Focus::Rail));
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(leaf(Leaf::Slot(s[0])))
    );
}

#[test]
fn zoom_keeps_every_placement() {
    let (mut t, s, g) = nested();
    assert!(t.place("tags", s[1], true));
    let before = t.clone();
    assert_eq!(t.zoomed(), None);
    t.zoom(Some(Leaf::Slot(s[1])));
    assert_eq!(t.zoomed(), Some(Leaf::Slot(s[1])));
    assert_eq!(t.regions(), before.regions());
    for slot in t.slots() {
        assert_eq!(t.slot_views(slot), before.slot_views(slot));
        assert_eq!(t.shown(slot), before.shown(slot));
    }
    // Focusing elsewhere keeps the zoom; a missing leaf is ignored.
    assert!(t.focus(leaf(Leaf::Group(g))));
    assert_eq!(t.zoomed(), Some(Leaf::Slot(s[1])));
    t.zoom(Some(Leaf::Slot(SlotId::new(99))));
    assert_eq!(t.zoomed(), Some(Leaf::Slot(s[1])));
    t.zoom(None);
    assert_eq!(t.zoomed(), None);
    assert_eq!(t, before);
}

#[test]
fn zoom_clears_when_zoomed_slot_closes() {
    let (mut t, s, g) = nested();
    t.zoom(Some(Leaf::Slot(s[1])));
    t.close_slot(s[2]).unwrap();
    assert_eq!(t.zoomed(), Some(Leaf::Slot(s[1])));
    t.close_slot(s[1]).unwrap();
    assert_eq!(t.zoomed(), None);

    let g2 = t.split_group(g, Axis::Horizontal).unwrap();
    t.zoom(Some(Leaf::Group(g2)));
    t.close_group(g2).unwrap();
    assert_eq!(t.zoomed(), None);
}

#[test]
fn closing_focused_slot_moves_focus_to_absorber() {
    let (mut t, s, _) = nested();
    assert!(t.focus(leaf(Leaf::Slot(s[2]))));
    assert_eq!(t.close_slot(s[2]), Ok(s[1]));
    assert_eq!(t.focused(), leaf(Leaf::Slot(s[1])));
}

#[test]
fn move_view_dir_targets_the_neighbour_slot() {
    let (mut t, s, _) = nested();
    assert!(t.place("tags", s[1], true));
    assert!(t.place("links", s[2], true));
    assert!(t.move_view_dir(&"tags", Dir::Down));
    assert_eq!(t.shown(s[2]), Some("tags"));
    assert_eq!(t.slot_views(s[2]), vec!["links", "tags"]);
    assert_eq!(t.view_slot(&"tags"), Some(s[2]));
    assert!(t.slot_views(s[1]).is_empty());
    // Groups are not candidates: Left skips the editor to slot0.
    assert!(t.move_view_dir(&"tags", Dir::Left));
    assert_eq!(t.shown(s[0]), Some("tags"));
    assert_eq!(t.slot_views(s[0]), vec!["files", "tags"]);
    // No slot further left; an unplaced view does not move.
    assert!(!t.move_view_dir(&"tags", Dir::Left));
    assert!(!t.move_view_dir(&"outline", Dir::Right));
    assert_eq!(t.view_slot(&"tags"), Some(s[0]));
}

#[test]
fn focus_dir_while_zoomed_only_sees_the_zoomed_leaf_and_the_rail() {
    let (mut t, s, g) = nested();
    t.zoom(Some(Leaf::Group(g)));
    assert_eq!(t.focus_dir(Dir::Right), TreeFocusMove::AtEdge);
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Down), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Left), TreeFocusMove::Moved(Focus::Rail));
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(leaf(Leaf::Group(g)))
    );
    assert_eq!(t.zoomed(), Some(Leaf::Group(g)));

    // A zoomed slot below another: nothing stacked is visible either.
    t.zoom(Some(Leaf::Slot(s[2])));
    assert!(t.focus(leaf(Leaf::Slot(s[2]))));
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Left), TreeFocusMove::Moved(Focus::Rail));
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(leaf(Leaf::Slot(s[2])))
    );
}

#[test]
fn move_view_dir_is_refused_while_zoomed() {
    let (mut t, s, _) = nested();
    assert!(t.place("tags", s[1], true));
    t.zoom(Some(Leaf::Slot(s[1])));
    assert!(!t.move_view_dir(&"tags", Dir::Down));
    assert_eq!(t.view_slot(&"tags"), Some(s[1]));
    assert_eq!(t.shown(s[1]), Some("tags"));
}

#[test]
fn leaving_zoom_restores_normal_geometry() {
    let (mut t, s, g) = nested();
    assert!(t.place("tags", s[1], true));
    t.zoom(Some(Leaf::Group(g)));
    t.zoom(None);
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(leaf(Leaf::Slot(s[1])))
    );
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::AtEdge);
    assert!(t.move_view_dir(&"tags", Dir::Down));
    assert_eq!(t.view_slot(&"tags"), Some(s[2]));
}

// ── Layout specs (Task 5) ──────────────────────────────────────────────

mod spec_tests {
    use super::super::spec::*;
    use super::super::*;
    use crate::logic::nav::DrawerView;
    use crate::logic::rag_caps::Caps;

    const ALL: Caps = Caps {
        semantic: true,
        ask: true,
    };

    fn slot(views: &[&str], shown: Option<&str>) -> ChildSpec {
        ChildSpec {
            weight: None,
            node: NodeSpec::Slot {
                views: views.iter().map(|v| v.to_string()).collect(),
                shown: shown.map(str::to_string),
            },
        }
    }

    fn editor() -> ChildSpec {
        ChildSpec {
            weight: None,
            node: NodeSpec::Editor { editor: true },
        }
    }

    fn split(axis: &str, children: Vec<ChildSpec>) -> ChildSpec {
        ChildSpec {
            weight: None,
            node: NodeSpec::Split {
                split: AxisName(axis.to_string()),
                children,
            },
        }
    }

    fn w(mut c: ChildSpec, weight: f64) -> ChildSpec {
        c.weight = Some(weight);
        c
    }

    /// The arrangement as a comparable string: axes, weights (rounded),
    /// leaf kinds and slot contents, ignoring ids and session state.
    fn shape(t: &LayoutTree<DrawerView>) -> String {
        fn go(t: &LayoutTree<DrawerView>, n: &Node<Leaf>, out: &mut String) {
            match n.split_children() {
                Some((axis, children)) => {
                    out.push_str(&format!("{axis:?}("));
                    for (wt, c) in children {
                        out.push_str(&format!("{:.6}:", wt));
                        go(t, c, out);
                        out.push(',');
                    }
                    out.push(')');
                }
                None => match n.as_leaf().expect("leaf") {
                    Leaf::Group(_) => out.push('E'),
                    Leaf::Slot(s) => {
                        out.push_str(&format!("S{:?}/{:?}", t.slot_views(*s), t.shown(*s)))
                    }
                },
            }
        }
        let mut out = String::new();
        go(t, t.node(), &mut out);
        out
    }

    fn ok(spec: &ChildSpec) -> LayoutTree<DrawerView> {
        let (t, warnings) = build(spec, ALL).expect("valid spec");
        assert!(warnings.is_empty(), "{warnings:?}");
        t
    }

    fn errs(spec: &ChildSpec) -> Vec<SpecError> {
        build(spec, ALL).expect_err("invalid spec")
    }

    #[test]
    fn round_trips_the_default_layout_through_toml() {
        let tree = LayoutTree::default_layout(DrawerView::Files);
        let file = PresetFile {
            preset: [("default".to_string(), to_spec(&tree))].into(),
        };
        let text = toml::to_string(&file).expect("serializes");
        let back = parse_presets(&text).expect("parses");
        assert_eq!(back, file, "{text}");
        let rebuilt = ok(&back.preset["default"]);
        assert_eq!(shape(&rebuilt), shape(&tree));
        assert_eq!(rebuilt, tree, "fresh trees compare equal, ids included");

        // A tree rearranged by operations round-trips too.
        let mut t = tree.clone();
        let (left, right) = (t.slots()[0], t.slots()[1]);
        let below = t.split_slot(right, Axis::Vertical).unwrap();
        assert!(t.move_view(DrawerView::Ask, below));
        assert!(t.place(DrawerView::Links, right, false));
        t.split_group(t.groups()[0], Axis::Vertical).unwrap();
        t.close_slot(left).unwrap();
        let text = toml::to_string(&to_spec(&t)).expect("serializes");
        let spec: ChildSpec = toml::from_str(&text).expect("parses");
        assert_eq!(shape(&ok(&spec)), shape(&t), "{text}");
    }

    #[test]
    fn unknown_view_id_is_reported_with_its_path() {
        let spec = split(
            "horizontal",
            vec![
                editor(),
                split(
                    "vertical",
                    vec![slot(&["nope"], None), slot(&["files", "bogus"], None)],
                ),
            ],
        );
        let e = errs(&spec);
        assert_eq!(
            e,
            vec![
                SpecError::UnknownView {
                    path: "children[1].children[0]".into(),
                    id: "nope".into()
                },
                SpecError::UnknownView {
                    path: "children[1].children[1]".into(),
                    id: "bogus".into()
                },
            ]
        );
        assert_eq!(e[0].path(), Some("children[1].children[0]"));
        assert_eq!(
            e[0].to_string(),
            "Unknown view \"nope\" in the node at children[1].children[0]."
        );
    }

    #[test]
    fn view_in_two_slots_is_duplicate() {
        let spec = split(
            "horizontal",
            vec![
                slot(&["files"], None),
                editor(),
                slot(&["tags", "files"], None),
            ],
        );
        assert_eq!(
            errs(&spec),
            vec![SpecError::DuplicateView {
                path: "children[2]".into(),
                id: "files".into()
            }]
        );
        let twice_in_one = split("horizontal", vec![slot(&["tags", "tags"], None), editor()]);
        assert_eq!(
            errs(&twice_in_one),
            vec![SpecError::DuplicateView {
                path: "children[0]".into(),
                id: "tags".into()
            }]
        );
    }

    #[test]
    fn shown_not_in_views_is_rejected() {
        let spec = split("horizontal", vec![slot(&["files"], Some("tags")), editor()]);
        let e = errs(&spec);
        assert_eq!(
            e,
            vec![SpecError::ShownNotInViews {
                path: "children[0]".into(),
                shown: "tags".into()
            }]
        );
        assert!(!e[0].to_string().is_empty());
    }

    #[test]
    fn tree_without_editor_is_rejected() {
        let spec = split("horizontal", vec![slot(&["files"], None), slot(&[], None)]);
        let e = errs(&spec);
        assert_eq!(e, vec![SpecError::NoEditorGroup]);
        assert_eq!(e[0].path(), None);
        // Every problem is reported, not just the first.
        let many = split(
            "diagonal",
            vec![
                w(slot(&["nope"], None), 0.0),
                split("vertical", vec![slot(&[], None)]),
            ],
        );
        let e = errs(&many);
        assert!(e.contains(&SpecError::UnknownAxis {
            path: "".into(),
            axis: "diagonal".into()
        }));
        assert!(e.contains(&SpecError::BadWeight {
            path: "children[0]".into(),
            weight: 0.0
        }));
        assert!(e.contains(&SpecError::UnknownView {
            path: "children[0]".into(),
            id: "nope".into()
        }));
        assert!(e.contains(&SpecError::EmptySplit {
            path: "children[1]".into()
        }));
        assert!(e.contains(&SpecError::NoEditorGroup));
        assert_eq!(e.len(), 5, "{e:?}");
        // An editor-only tree is valid.
        let t = ok(&editor());
        assert!(t.slots().is_empty());
        assert_eq!(t.groups().len(), 1);
    }

    #[test]
    fn weight_zero_negative_or_nan_is_rejected() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let spec = split("horizontal", vec![w(editor(), bad), slot(&[], None)]);
            let e = errs(&spec);
            assert_eq!(e.len(), 1, "{bad}: {e:?}");
            match &e[0] {
                SpecError::BadWeight { path, weight } => {
                    assert_eq!(path, "children[0]");
                    assert!(weight.to_bits() == bad.to_bits(), "{bad}");
                }
                other => panic!("{bad}: {other:?}"),
            }
        }
    }

    #[test]
    fn unavailable_view_is_skipped_with_a_warning_not_an_error() {
        let spec = split(
            "horizontal",
            vec![
                slot(&["files", "semantic"], Some("files")),
                editor(),
                slot(&["ask", "outline"], Some("ask")),
            ],
        );
        let caps = Caps {
            semantic: false,
            ask: false,
        };
        let (t, warnings) = build(&spec, caps).expect("skips, does not fail");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("semantic")));
        assert!(warnings.iter().any(|w| w.contains("ask")));
        let s = t.slots();
        assert_eq!(t.slot_views(s[0]), vec![DrawerView::Files]);
        assert_eq!(t.shown(s[0]), Some(DrawerView::Files));
        assert_eq!(t.slot_views(s[1]), vec![DrawerView::Outline]);
        assert_eq!(t.shown(s[1]), None, "shown named a skipped view");
    }

    #[test]
    fn parse_presets_reads_nested_children() {
        let text = r#"
[preset.research]
split = "horizontal"
children = [
    { weight = 0.2, views = ["files"], shown = "files" },
    { weight = 0.6, editor = true },
    { weight = 0.2, split = "vertical", children = [
        { views = ["ask"], shown = "ask" },
        { views = ["outline", "links"] },
    ] },
]

[preset.focus]
editor = true
"#;
        let file = parse_presets(text).expect("parses");
        assert_eq!(file.preset.len(), 2);
        let expected = split(
            "horizontal",
            vec![
                w(slot(&["files"], Some("files")), 0.2),
                w(editor(), 0.6),
                w(
                    split(
                        "vertical",
                        vec![
                            slot(&["ask"], Some("ask")),
                            slot(&["outline", "links"], None),
                        ],
                    ),
                    0.2,
                ),
            ],
        );
        assert_eq!(file.preset["research"], expected);
        assert_eq!(file.preset["focus"], editor());
        let t = ok(&file.preset["research"]);
        let s = t.slots();
        assert_eq!(s.len(), 3);
        assert_eq!(t.shown(s[1]), Some(DrawerView::Ask));
        assert_eq!(
            t.slot_views(s[2]),
            vec![DrawerView::Outline, DrawerView::Links]
        );
        // Ids are small and sequential in reading order.
        assert_eq!(s, vec![SlotId::new(0), SlotId::new(1), SlotId::new(2)]);
        assert!(parse_presets("[preset.x]\neditor = false\n").is_err());
        assert!(parse_presets("[preset.x]\nviews = []\neditor = true\n").is_err());
    }

    #[test]
    fn missing_weight_defaults_to_one() {
        let spec = split(
            "horizontal",
            vec![slot(&[], None), w(editor(), 2.0), slot(&[], None)],
        );
        let t = ok(&spec);
        let (_, children) = t.node().split_children().expect("split");
        let weights: Vec<f64> = children.iter().map(|(wt, _)| *wt).collect();
        for (got, want) in weights.iter().zip([0.25, 0.5, 0.25]) {
            assert!((got - want).abs() < 1e-9, "{weights:?}");
        }
    }
}

#[test]
fn shape_mirrors_the_tree_with_weights_in_reading_order() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let g = GroupId::new(0);
    let low = t.split_slot(left, Axis::Vertical).unwrap();
    let TreeShape::Split { axis, children } = t.shape() else {
        panic!("the default root is a split");
    };
    assert_eq!(axis, Axis::Horizontal);
    let weights: Vec<f64> = children.iter().map(|(w, _)| *w).collect();
    for (w, want) in weights.iter().zip([0.2, 0.6, 0.2]) {
        assert!((w - want).abs() < 1e-9, "{weights:?}");
    }
    assert_eq!(
        children[0].1,
        TreeShape::Split {
            axis: Axis::Vertical,
            children: vec![
                (0.5, TreeShape::Leaf(Leaf::Slot(left))),
                (0.5, TreeShape::Leaf(Leaf::Slot(low))),
            ],
        }
    );
    assert_eq!(children[1].1, TreeShape::Leaf(Leaf::Group(g)));
    assert_eq!(children[2].1, TreeShape::Leaf(Leaf::Slot(right)));
}

#[test]
fn focus_dir_among_skips_leaves_that_are_not_visible() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let g = Leaf::Group(GroupId::new(0));
    let not_right = move |l: Leaf| l != Leaf::Slot(right);
    // The hidden right slot is no target: Right from the editor is the edge.
    assert_eq!(
        t.focus_dir_among(Dir::Right, &not_right),
        TreeFocusMove::AtEdge
    );
    assert_eq!(t.focused(), Focus::Leaf(g));
    // Every leaf visible: the plain rule reaches it.
    assert_eq!(
        t.focus_dir_among(Dir::Right, &|_| true),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(right)))
    );
    // A hidden left slot: Left from the editor reaches the rail.
    t.focus(Focus::Leaf(g));
    let not_left = move |l: Leaf| l != Leaf::Slot(left);
    assert_eq!(
        t.focus_dir_among(Dir::Left, &not_left),
        TreeFocusMove::Moved(Focus::Rail)
    );
    // And Right from the rail skips it too.
    assert_eq!(
        t.focus_dir_among(Dir::Right, &not_left),
        TreeFocusMove::Moved(Focus::Leaf(g))
    );
}

// ── Fit ───────────────────────────────────────────────────────────────

/// The default layout at 35 units wide with every leaf 10 wide and the
/// group 20: the trailing slot collapses, the group never does, and a leaf
/// that is not drawn is left out before fitting (its weight goes to the
/// others, it is not reported as collapsed).
#[test]
fn fit_collapses_trailing_slots_skips_undrawn_and_keeps_groups() {
    let t = tree();
    let (left, right) = sides(&t);
    let g = Leaf::Group(GroupId::new(0));
    let min = |leaf: Leaf| match leaf {
        Leaf::Group(_) => (20.0, 0.0),
        Leaf::Slot(_) => (10.0, 6.0),
    };
    let area = UnitRect::new(0.0, 0.0, 35.0, 30.0);
    let fit = t.fit(area, &min, &|_| true);
    assert_eq!(fit.collapsed, vec![Leaf::Slot(right)]);
    let ids: Vec<Leaf> = fit.rects.iter().map(|(l, _)| *l).collect();
    assert_eq!(ids, vec![Leaf::Slot(left), g]);
    // The survivors fill the area by weight (0.2 : 0.6).
    assert!((fit.rects[0].1.width() - 8.75).abs() < 1e-9);

    // Undrawn right slot: nothing collapses, it simply has no rectangle.
    let fit = t.fit(area, &min, &|leaf| leaf != Leaf::Slot(right));
    assert!(fit.collapsed.is_empty());
    assert_eq!(fit.rects.len(), 2);

    // Narrower than every minimum: only the group is left.
    let fit = t.fit(UnitRect::new(0.0, 0.0, 15.0, 30.0), &min, &|_| true);
    assert_eq!(fit.collapsed, vec![Leaf::Slot(right), Leaf::Slot(left)]);
    assert_eq!(fit.rects.len(), 1);
    assert_eq!(fit.rects[0].0, g);
    assert!((fit.rects[0].1.width() - 15.0).abs() < 1e-9);
}
