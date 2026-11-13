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
    let leaves: Vec<Leaf> = t.node().leaves().into_iter().copied().collect();
    assert_eq!(
        leaves,
        vec![Leaf::Slot(left), Leaf::Editor, Leaf::Slot(right)]
    );
    assert_eq!(t.slot_views(left), vec!["files"]);
    assert_eq!(t.shown(left), Some("files"));
    assert!(t.slot_views(right).is_empty());
    assert_eq!(t.shown(right), None);
    assert_eq!(t.focused(), Focus::Leaf(Leaf::Editor));
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
fn default_layout_has_one_editor_leaf_and_one_group() {
    let t = tree();
    let editors = t
        .node()
        .leaves()
        .into_iter()
        .filter(|l| **l == Leaf::Editor)
        .count();
    assert_eq!(editors, 1);
    let g = GroupId::new(0);
    assert_eq!(t.region().tabs().len(), 1);
    assert_eq!(t.region().all_groups(), vec![g]);
    assert_eq!(t.active_group(), g);
    assert_eq!(t.region().active_group(), g);
    assert_eq!(t.focused(), Focus::Leaf(Leaf::Editor));
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
    let mut bare: Tree = LayoutTree::from_parts(Node::leaf(Leaf::Editor), BTreeMap::new()).unwrap();
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
                (1.0, Node::leaf(Leaf::Editor)),
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
    let row = Node::split(
        Axis::Horizontal,
        vec![
            (1.0, Node::leaf(Leaf::Slot(s0))),
            (1.0, Node::leaf(Leaf::Editor)),
        ],
    );
    let ok = Tree::from_parts(
        row.clone(),
        BTreeMap::from([(s0, (vec!["files", "tags"], Some("tags")))]),
    )
    .unwrap();
    assert_eq!(ok.shown(s0), Some("tags"));
    assert_eq!(ok.focused(), Focus::Leaf(Leaf::Editor));

    let no_group: Node<Leaf> = Node::leaf(Leaf::Slot(s0));
    assert_eq!(
        Tree::from_parts(no_group, BTreeMap::from([(s0, (vec![], None))])).err(),
        Some(TreeError::NoEditor)
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
            (1.0, Node::leaf(Leaf::Editor)),
            (1.0, Node::leaf(Leaf::Editor)),
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
            (1.0, Node::leaf(Leaf::Editor)),
            (1.0, Node::leaf(Leaf::Slot(SlotId::new(5)))),
        ],
    );
    let mut t = Tree::from_parts(root, BTreeMap::from([(SlotId::new(5), (vec![], None))])).unwrap();
    let s = t.split_slot(SlotId::new(5), Axis::Vertical).unwrap();
    assert_ne!(s, SlotId::new(5));
    assert!(t.resize_after(Leaf::Editor, 0.3));
}

// ── Regions, geometric focus and zoom ─────────────────────────────────

/// slot0 (0.2) | editor (0.6) | [slot1 over slot2] (0.2)
fn nested() -> (Tree, [SlotId; 3], Leaf) {
    let s = [SlotId::new(0), SlotId::new(1), SlotId::new(2)];
    let g = Leaf::Editor;
    let root = Node::split(
        Axis::Horizontal,
        vec![
            (0.2, Node::leaf(Leaf::Slot(s[0]))),
            (0.6, Node::leaf(g)),
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
        vec![Leaf::Slot(s[0]), g, Leaf::Slot(s[1]), Leaf::Slot(s[2])]
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
    assert_eq!(t.focus_dir(Dir::Left), m(g));
    // The remembered leaf (last focused slot) breaks the tie.
    assert_eq!(t.focus_dir(Dir::Right), m(Leaf::Slot(s[2])));
    assert_eq!(t.focus_dir(Dir::Left), m(g));
    assert_eq!(t.focus_dir(Dir::Left), m(Leaf::Slot(s[0])));
    assert_eq!(t.focused(), leaf(Leaf::Slot(s[0])));
    // Back right into the editor, whose group stays active.
    assert_eq!(t.focus_dir(Dir::Right), m(g));
    assert_eq!(t.active_group(), GroupId::new(0));
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
    let g = Leaf::Editor;
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Down), TreeFocusMove::NotHandled);
    assert_eq!(t.focused(), leaf(g));
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
    assert!(t.focus(leaf(g)));
    assert_eq!(t.zoomed(), Some(Leaf::Slot(s[1])));
    t.zoom(Some(Leaf::Slot(SlotId::new(99))));
    assert_eq!(t.zoomed(), Some(Leaf::Slot(s[1])));
    t.zoom(None);
    assert_eq!(t.zoomed(), None);
    assert_eq!(t, before);
}

#[test]
fn zoom_clears_when_zoomed_slot_closes() {
    let (mut t, s, _) = nested();
    t.zoom(Some(Leaf::Slot(s[1])));
    t.close_slot(s[2]).unwrap();
    assert_eq!(t.zoomed(), Some(Leaf::Slot(s[1])));
    t.close_slot(s[1]).unwrap();
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
    t.zoom(Some(g));
    assert_eq!(t.focus_dir(Dir::Right), TreeFocusMove::AtEdge);
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Down), TreeFocusMove::NotHandled);
    assert_eq!(t.focus_dir(Dir::Left), TreeFocusMove::Moved(Focus::Rail));
    assert_eq!(t.focus_dir(Dir::Right), TreeFocusMove::Moved(leaf(g)));
    assert_eq!(t.zoomed(), Some(g));

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
    t.zoom(Some(g));
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

    const WITH_RAIL: &str = r#"
[preset.writing]
rail = "bottom"
split = "horizontal"
children = [{ views = ["files"] }, { editor = true }]

[preset.plain]
split = "horizontal"
children = [{ views = ["files"] }, { editor = true }]
"#;

    #[test]
    fn a_preset_with_rail_parses_and_the_tree_is_unchanged() {
        let file = parse_presets(WITH_RAIL).unwrap();
        assert_eq!(file.rails.get("writing"), Some(&RailPosition::Bottom));
        assert_eq!(file.rails.get("plain"), None);
        assert_eq!(file.preset["writing"], file.preset["plain"]);
    }

    #[test]
    fn a_preset_with_a_bad_rail_names_the_preset() {
        let text = WITH_RAIL.replace("\"bottom\"", "\"diagonal\"");
        let message = parse_presets(&text).unwrap_err();
        assert!(message.contains("preset \"writing\""), "{message}");
        assert!(message.contains("diagonal"), "{message}");
        let message = parse_presets(&WITH_RAIL.replace("\"bottom\"", "3")).unwrap_err();
        assert!(message.contains("preset \"writing\""), "{message}");
    }

    #[test]
    fn rail_on_an_inner_node_is_an_error() {
        let text = r#"
[preset.x]
split = "horizontal"
children = [{ views = ["files"], rail = "left" }, { editor = true }]
"#;
        let message = parse_presets(text).unwrap_err();
        assert!(message.contains("Unknown key \"rail\""), "{message}");
    }

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
                    Leaf::Editor => out.push('E'),
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
            rails: Default::default(),
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
        t.close_slot(left).unwrap();
        let text = toml::to_string(&to_spec(&t)).expect("serializes");
        let spec: ChildSpec = toml::from_str(&text).expect("parses");
        assert_eq!(shape(&ok(&spec)), shape(&t), "{text}");
    }

    /// A layout holds one editor region (ADR-0058); its groups split inside
    /// it. A spec naming a second editor leaf is refused with that leaf's
    /// path.
    #[test]
    fn spec_rejects_a_second_editor_leaf() {
        let spec = split(
            "horizontal",
            vec![
                slot(&["files"], None),
                editor(),
                split("vertical", vec![slot(&["tags"], None), editor()]),
            ],
        );
        let e = errs(&spec);
        assert_eq!(
            e,
            vec![SpecError::MultipleEditorRegions {
                path: "children[2].children[1]".into()
            }]
        );
        assert_eq!(e[0].path(), Some("children[2].children[1]"));
        let text = e[0].to_string();
        assert_eq!(
            text,
            "The editor region in the node at children[2].children[1] is a second one; \
             a layout holds one editor region (split the editor inside it instead)."
        );
        // A tree whose region is split into groups saves as a preset with one
        // editor leaf, which builds.
        let mut t = LayoutTree::default_layout(DrawerView::Files);
        let g = t.active_group();
        t.region_mut().split_group(g, Axis::Vertical).unwrap();
        let rebuilt = ok(&to_spec(&t));
        assert_eq!(shape(&rebuilt), shape(&t));
        assert_eq!(rebuilt.region().all_groups().len(), 1);
    }

    /// Finite weights whose sum is not finite would lay out NaN rectangles:
    /// refused with the split's path.
    #[test]
    fn weights_summing_to_infinity_are_rejected() {
        let spec = split(
            "horizontal",
            vec![w(editor(), 1e308), w(slot(&["files"], None), 1e308)],
        );
        let e = errs(&spec);
        assert_eq!(e, vec![SpecError::WeightSumOverflow { path: "".into() }]);
        assert_eq!(e[0].path(), Some(""));
        assert!(!e[0].to_string().is_empty());
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
        assert_eq!(e, vec![SpecError::NoEditorRegion]);
        assert_eq!(
            e[0].to_string(),
            "The layout has no editor region (`editor = true`)."
        );
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
        assert!(e.contains(&SpecError::NoEditorRegion));
        assert_eq!(e.len(), 5, "{e:?}");
        // An editor-only tree is valid.
        let t = ok(&editor());
        assert!(t.slots().is_empty());
        assert_eq!(t.region().all_groups().len(), 1);
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

    /// A typo in a nested leaf names the preset, the node's path and the
    /// key, not serde's "did not match any variant of untagged enum".
    #[test]
    fn a_typo_in_a_nested_leaf_names_the_preset_path_and_key() {
        let text = "[preset.research]\nsplit = \"horizontal\"\nchildren = [\n\
                    { editor = true },\n\
                    { split = \"vertical\", children = [\n\
                    { views = [\"tags\"] },\n\
                    { veiws = [\"outline\"] },\n\
                    ] },\n]\n";
        let err = parse_presets(text).expect_err("a typo does not parse");
        assert!(!err.contains("untagged"), "{err}");
        assert!(err.contains("\"research\""), "{err}");
        assert!(err.contains("children[1].children[1]"), "{err}");
        assert!(err.contains("\"veiws\""), "{err}");
    }

    /// The other shapes serde only calls "no variant": mixed kinds, no
    /// kind, `editor = false`, values of the wrong type. Each problem is
    /// named with its preset and path; several are all counted.
    #[test]
    fn malformed_nodes_are_named_with_their_path() {
        let err = |text: &str| parse_presets(text).expect_err(text);
        let mixed = err("[preset.a]\nviews = []\neditor = true\n");
        assert!(mixed.contains("\"a\"") && mixed.contains("root"), "{mixed}");
        assert!(
            mixed.contains("editor") && mixed.contains("views"),
            "{mixed}"
        );
        let none = err("[preset.a]\nweight = 1\n");
        assert!(none.contains("\"a\""), "{none}");
        let off = err("[preset.a]\neditor = false\n");
        assert!(off.contains("must be true"), "{off}");
        let not_list = err("[preset.a]\nviews = \"files\"\n");
        assert!(not_list.contains("\"views\""), "{not_list}");
        let not_text =
            err("[preset.a]\nsplit = 3\nchildren = [{ editor = true }, { editor = true }]\n");
        assert!(not_text.contains("\"split\""), "{not_text}");
        // Two bad leaves in two presets: both named.
        let two = err("[preset.a]\nview = []\n\n[preset.b]\nedtor = true\n");
        assert!(
            two.contains("\"view\"") && two.contains("\"edtor\""),
            "{two}"
        );
        // TOML that is not even TOML keeps the parser's line and column.
        let syntax = err("[preset.a\n");
        // What the check allows, serde reads: an integer weight is fine.
        assert!(parse_presets("[preset.a]\nweight = 2\neditor = true\n").is_ok());
        assert!(syntax.contains("line 1"), "{syntax}");
    }

    #[test]
    fn problem_lists_show_the_first_few_and_count_the_rest() {
        let items: Vec<String> = (1..=5).map(|i| format!("P{i}.")).collect();
        assert_eq!(list_problems(&items[..2], 3), "P1. P2.");
        assert_eq!(list_problems(&items, 3), "P1. P2. P3. (and 2 more)");
        assert_eq!(list_problems(&items[..4], 3), "P1. P2. P3. (and 1 more)");
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
    let g = Leaf::Editor;
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
    assert_eq!(children[1].1, TreeShape::Leaf(g));
    assert_eq!(children[2].1, TreeShape::Leaf(Leaf::Slot(right)));
}

#[test]
fn focus_dir_among_skips_leaves_that_are_not_visible() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let g = Leaf::Editor;
    let not_right = move |l: Leaf| l != Leaf::Slot(right);
    // The hidden right slot is no target: Right from the editor is the edge.
    assert_eq!(
        t.focus_dir_in(Dir::Right, &not_right, Some(RailPosition::Left)),
        TreeFocusMove::AtEdge
    );
    assert_eq!(t.focused(), Focus::Leaf(g));
    // Every leaf visible: the plain rule reaches it.
    assert_eq!(
        t.focus_dir_in(Dir::Right, &|_| true, Some(RailPosition::Left)),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(right)))
    );
    // A hidden left slot: Left from the editor reaches the rail.
    t.focus(Focus::Leaf(g));
    let not_left = move |l: Leaf| l != Leaf::Slot(left);
    assert_eq!(
        t.focus_dir_in(Dir::Left, &not_left, Some(RailPosition::Left)),
        TreeFocusMove::Moved(Focus::Rail)
    );
    // And Right from the rail skips it too.
    assert_eq!(
        t.focus_dir_in(Dir::Right, &not_left, Some(RailPosition::Left)),
        TreeFocusMove::Moved(Focus::Leaf(g))
    );
}

#[test]
fn facing_points_from_the_layout_to_the_rail() {
    assert_eq!(RailPosition::Left.facing(), Dir::Left);
    assert_eq!(RailPosition::Right.facing(), Dir::Right);
    assert_eq!(RailPosition::Top.facing(), Dir::Up);
    assert_eq!(RailPosition::Bottom.facing(), Dir::Down);
}

#[test]
fn focus_from_the_rail_goes_toward_the_layout_for_each_position() {
    for rail in RailPosition::ALL {
        let mut t = tree();
        let (left, right) = sides(&t);
        let first = if rail == RailPosition::Right {
            right
        } else {
            left
        };
        t.focus(Focus::Rail);
        assert_eq!(
            t.focus_dir_in(rail.toward(), &|_| true, Some(rail)),
            TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(first))),
            "{rail:?}"
        );
        // Back on the rail: away from the layout is the edge, and the
        // cross-axis keys are not the rail's.
        t.focus(Focus::Rail);
        assert_eq!(
            t.focus_dir_in(rail.facing(), &|_| true, Some(rail)),
            TreeFocusMove::AtEdge,
            "{rail:?}"
        );
        for cross in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
            if cross == rail.toward() || cross == rail.facing() {
                continue;
            }
            assert_eq!(
                t.focus_dir_in(cross, &|_| true, Some(rail)),
                TreeFocusMove::NotHandled,
                "{rail:?} {cross:?}"
            );
        }
    }
}

#[test]
fn focus_toward_the_rail_from_the_layout_edge_reaches_it() {
    let g = Leaf::Editor;
    for rail in [RailPosition::Right, RailPosition::Top, RailPosition::Bottom] {
        let mut t = tree();
        let (left, right) = sides(&t);
        // Only the editor is drawn: nothing lies between it and the rail.
        let only_group = move |l: Leaf| l != Leaf::Slot(left) && l != Leaf::Slot(right);
        t.focus(Focus::Leaf(g));
        assert_eq!(
            t.focus_dir_in(rail.facing(), &only_group, Some(rail)),
            TreeFocusMove::Moved(Focus::Rail),
            "{rail:?}"
        );
    }
}

#[test]
fn focus_toward_a_hidden_rail_stops_at_the_edge() {
    let g = Leaf::Editor;
    let mut t = tree();
    let (left, right) = sides(&t);
    let only_group = move |l: Leaf| l != Leaf::Slot(left) && l != Leaf::Slot(right);
    t.focus(Focus::Leaf(g));
    assert_eq!(
        t.focus_dir_in(Dir::Left, &only_group, None),
        TreeFocusMove::AtEdge
    );
    assert_eq!(t.focused(), Focus::Leaf(g));
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
    let g = Leaf::Editor;
    let min = |leaf: Leaf| match leaf {
        Leaf::Editor => (20.0, 0.0),
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

#[test]
fn side_queries_follow_the_active_group() {
    let t = tree();
    let (left, right) = sides(&t);
    assert_eq!(t.side_slots(Side::Left), vec![left]);
    assert_eq!(t.side_slots(Side::Right), vec![right]);
    assert_eq!(t.slot_side(left), Side::Left);
    assert_eq!(t.slot_side(right), Side::Right);
    assert_eq!(t.slot_beside_rail(RailPosition::Left), Some(left));
    assert!(!t.slot_stacked(left, &|_| true));
}

#[test]
fn slot_stacked_sees_a_slot_in_the_same_column_that_is_drawn() {
    let mut t = tree();
    let (left, _) = sides(&t);
    let below = t.split_slot(left, Axis::Vertical).unwrap();
    assert!(t.slot_stacked(left, &|_| true));
    assert!(!t.slot_stacked(left, &|s| s != below));
    assert_eq!(t.slot_beside_rail(RailPosition::Left), Some(left));
}

#[test]
fn focus_dir_among_measures_the_layout_as_drawn() {
    // Editor | X over Y | P (1) over Q (2). Y hidden: X is drawn full
    // height, so Q (two thirds of it) is the larger overlap. Measured on
    // the full tree X is the top half and P would win.
    let s: Vec<SlotId> = (0..5).map(SlotId::new).collect();
    let (x, y, p, q) = (s[1], s[2], s[3], s[4]);
    let g = Leaf::Editor;
    let col = |a, wa, b, wb| {
        Node::split(
            Axis::Vertical,
            vec![
                (wa, Node::leaf(Leaf::Slot(a))),
                (wb, Node::leaf(Leaf::Slot(b))),
            ],
        )
    };
    let root = Node::split(
        Axis::Horizontal,
        vec![
            (1.0, Node::leaf(g)),
            (1.0, col(x, 1.0, y, 1.0)),
            (1.0, col(p, 1.0, q, 2.0)),
        ],
    );
    let slots = BTreeMap::from([
        (x, (vec!["x"], Some("x"))),
        (y, (vec!["y"], None)),
        (p, (vec!["p"], Some("p"))),
        (q, (vec!["q"], Some("q"))),
    ]);
    let mut t = Tree::from_parts(root, slots).unwrap();
    assert!(t.focus(leaf(Leaf::Slot(x))));
    let visible = move |l: Leaf| l != Leaf::Slot(y);
    assert_eq!(
        t.focus_dir_in(Dir::Right, &visible, Some(RailPosition::Left)),
        TreeFocusMove::Moved(leaf(Leaf::Slot(q)))
    );
}

fn rect_of(t: &Tree, s: SlotId) -> UnitRect {
    t.regions()
        .into_iter()
        .find(|(l, _)| *l == Leaf::Slot(s))
        .map(|(_, r)| r)
        .expect("slot region")
}

/// Slot `right` holding `[x, y]` with `x` shown.
fn right_with_two() -> (Tree, SlotId) {
    let mut t = tree();
    let (_, right) = sides(&t);
    assert!(t.move_view("y", right));
    assert!(t.move_view("x", right));
    (t, right)
}

#[test]
fn place_beside_down_puts_the_view_in_a_new_slot_below() {
    let (mut t, a) = right_with_two();
    let b = t.place_beside("x", a, Dir::Down).unwrap();
    assert_eq!(t.view_slot(&"x"), Some(b));
    assert_eq!(t.shown(b), Some("x"));
    assert_eq!(t.slot_views(a), vec!["y"]);
    let (ra, rb) = (rect_of(&t, a), rect_of(&t, b));
    assert!((rb.y() - ra.bottom()).abs() < 1e-9);
}

#[test]
fn place_beside_up_and_left_put_the_new_slot_first() {
    let (mut t, a) = right_with_two();
    let b = t.place_beside("x", a, Dir::Up).unwrap();
    assert!((rect_of(&t, b).bottom() - rect_of(&t, a).y()).abs() < 1e-9);
    let (mut t, a) = right_with_two();
    let b = t.place_beside("x", a, Dir::Left).unwrap();
    assert!((rect_of(&t, b).right() - rect_of(&t, a).x()).abs() < 1e-9);
    let (mut t, a) = right_with_two();
    let b = t.place_beside("x", a, Dir::Right).unwrap();
    assert!((rect_of(&t, b).x() - rect_of(&t, a).right()).abs() < 1e-9);
}

#[test]
fn place_beside_a_hidden_view_shows_it_in_the_new_slot() {
    let (mut t, a) = right_with_two();
    assert_eq!(t.hide_view(&"x"), Some(a));
    let b = t.place_beside("x", a, Dir::Down).unwrap();
    assert_eq!(t.shown(b), Some("x"));
}

#[test]
fn place_beside_an_absent_slot_changes_nothing() {
    let (mut t, _) = right_with_two();
    let before = t.clone();
    assert_eq!(
        t.place_beside("x", SlotId::new(99), Dir::Down),
        Err(TreeError::NoSuchSlot)
    );
    assert_eq!(t, before);
}

#[test]
fn place_beside_editor_puts_the_new_slot_at_the_editor_edge() {
    let (mut t, _) = right_with_two();
    let b = t.place_beside_editor("x", Dir::Left);
    assert_eq!(t.view_slot(&"x"), Some(b));
    assert_eq!(t.shown(b), Some("x"));
    let editor = t
        .regions()
        .into_iter()
        .find(|(l, _)| *l == Leaf::Editor)
        .map(|(_, r)| r)
        .unwrap();
    assert!((rect_of(&t, b).right() - editor.x()).abs() < 1e-9);
}

#[test]
fn place_beside_editor_right_puts_the_new_slot_after_the_editor() {
    let (mut t, _) = right_with_two();
    let b = t.place_beside_editor("x", Dir::Right);
    let editor = t
        .regions()
        .into_iter()
        .find(|(l, _)| *l == Leaf::Editor)
        .map(|(_, r)| r)
        .unwrap();
    assert!((rect_of(&t, b).x() - editor.right()).abs() < 1e-9);
}

#[test]
fn place_beside_reuses_a_split_along_the_same_axis() {
    let (mut t, a) = right_with_two();
    let c = t.split_slot(a, Axis::Vertical).unwrap();
    let h = rect_of(&t, a).height();
    let b = t.place_beside("x", a, Dir::Down).unwrap();
    let leaves: Vec<Leaf> = t.node().leaves().into_iter().copied().collect();
    let at = leaves.iter().position(|l| *l == Leaf::Slot(a)).unwrap();
    assert_eq!(leaves[at + 1], Leaf::Slot(b));
    assert_eq!(leaves[at + 2], Leaf::Slot(c));
    assert!((rect_of(&t, a).height() - h / 2.0).abs() < 1e-9);
    assert!((rect_of(&t, b).height() - h / 2.0).abs() < 1e-9);
}

#[test]
fn vacate_close_closes_an_empty_slot_and_not_the_last() {
    let (mut t, a) = right_with_two();
    assert!(t.move_view("y", a));
    let b = t.place_beside("x", a, Dir::Down).unwrap();
    assert_eq!(t.place_beside("y", a, Dir::Down).map(|_| ()), Ok(()));
    let absorbed = t.vacate_close(a).expect("empty slot closes");
    assert!(!t.slots().contains(&a));
    assert!(t.slots().contains(&absorbed));
    assert!(t.slots().contains(&b));

    let mut one = tree();
    while one.slots().len() > 1 {
        let s = one.slots()[0];
        one.close_slot(s).unwrap();
    }
    let only = one.slots()[0];
    assert_eq!(one.vacate_close(only), None);
}

#[test]
fn vacate_close_keeps_a_slot_with_hidden_views() {
    let (mut t, a) = right_with_two();
    assert_eq!(t.hide_view(&"x"), Some(a));
    assert!(t.move_view("y", t.slots()[0]));
    assert_eq!(t.slot_views(a), vec!["x"]);
    assert_eq!(t.vacate_close(a), None);
}

#[test]
fn vacate_close_keeps_the_last_slot_on_its_side() {
    // FILES leaves the default left slot for the right one: the left slot
    // is the only one left of the editor, so it stays, empty.
    let mut t = tree();
    let (left, right) = sides(&t);
    assert!(t.move_view("files", right));
    assert!(t.slot_views(left).is_empty());
    assert_eq!(t.vacate_close(left), None);
    assert_eq!(t.slots(), vec![left, right]);
    // The same for the right slot once FILES goes back.
    assert!(t.move_view("files", left));
    assert_eq!(t.vacate_close(right), None);
    assert_eq!(t.slots(), vec![left, right]);
}

#[test]
fn vacate_close_closes_a_slot_with_a_sibling_on_its_side() {
    // Two stacked left slots: the lower one, emptied, closes.
    let mut t = tree();
    let (left, right) = sides(&t);
    assert!(t.move_view("x", left));
    let lower = t.place_beside("x", left, Dir::Down).unwrap();
    assert!(t.move_view("x", right));
    assert_eq!(t.vacate_close(lower), Some(left));
    assert_eq!(t.slots(), vec![left, right]);
    // A slot with views stays whatever its side holds.
    let lower = t.place_beside("files", left, Dir::Down).unwrap();
    assert!(!t.slot_views(lower).is_empty());
    assert_eq!(t.vacate_close(lower), None);
}

// ── Making room ───────────────────────────────────────────────────────

/// FILES (s0) and ASK (s1) side by side over FIND (s2) left, the group, and
/// OUTLINE (s3) right, weighted `left : group : right`.
fn ask_beside_files(left: f64, group: f64, right: f64) -> (Tree, [SlotId; 4], Leaf) {
    let s = [0, 1, 2, 3].map(SlotId::new);
    let g = Leaf::Editor;
    let pair = Node::split(
        Axis::Horizontal,
        vec![
            (1.0, Node::leaf(Leaf::Slot(s[0]))),
            (1.0, Node::leaf(Leaf::Slot(s[1]))),
        ],
    );
    let column = Node::split(
        Axis::Vertical,
        vec![(1.0, pair), (1.0, Node::leaf(Leaf::Slot(s[2])))],
    );
    let root = Node::split(
        Axis::Horizontal,
        vec![
            (left, column),
            (group, Node::leaf(g)),
            (right, Node::leaf(Leaf::Slot(s[3]))),
        ],
    );
    let slots = BTreeMap::from([
        (s[0], (vec!["files"], Some("files"))),
        (s[1], (vec!["ask"], Some("ask"))),
        (s[2], (vec!["find"], Some("find"))),
        (s[3], (vec!["outline"], Some("outline"))),
    ]);
    (Tree::from_parts(root, slots).unwrap(), s, g)
}

/// Group 20 wide, ASK (s1) 18 × 12, every other slot 10 × 6.
fn room_min(leaf: Leaf) -> (f64, f64) {
    match leaf {
        Leaf::Editor => (20.0, 0.0),
        Leaf::Slot(s) if s == SlotId::new(1) => (18.0, 12.0),
        Leaf::Slot(_) => (10.0, 6.0),
    }
}

fn fit_width(fit: &Fit<Leaf>, leaf: Leaf) -> f64 {
    fit.rects
        .iter()
        .find(|(l, _)| *l == leaf)
        .map(|(_, r)| r.width())
        .expect("leaf has a rectangle")
}

/// The left column's 0.2 cannot hold FILES and ASK side by side (28
/// wide), so ASK collapses. Making room grows the column, taking from
/// the group and OUTLINE: ASK fits, nothing else collapses, and neither
/// sibling goes under its minimum.
#[test]
fn make_room_grows_the_column_holding_a_collapsed_slot() {
    let (mut t, s, g) = ask_beside_files(0.2, 0.6, 0.2);
    let area = UnitRect::new(0.0, 0.0, 59.5, 27.5);
    assert_eq!(
        t.fit(area, &room_min, &|_| true).collapsed,
        vec![Leaf::Slot(s[1])]
    );
    assert!(t.make_room(&[Leaf::Slot(s[1])], area, &room_min, &|_| true));
    let fit = t.fit(area, &room_min, &|_| true);
    assert!(fit.collapsed.is_empty(), "{:?}", fit.collapsed);
    let column = fit_width(&fit, Leaf::Slot(s[0])) + fit_width(&fit, Leaf::Slot(s[1]));
    assert!(column >= 28.0 - 1e-9, "{column}");
    assert!(fit_width(&fit, g) >= 20.0 - 1e-9);
    assert!(fit_width(&fit, Leaf::Slot(s[3])) >= 10.0 - 1e-9);
}

/// The group sits just above its minimum and OUTLINE has plenty: the room
/// comes from what each sibling has above its minimum, not from its
/// weight, so the group never goes under its minimum.
#[test]
fn make_room_never_shrinks_the_group_below_its_minimum() {
    let (mut t, s, g) = ask_beside_files(0.2, 0.35, 0.45);
    let area = UnitRect::new(0.0, 0.0, 59.5, 27.5);
    assert!(t.make_room(&[Leaf::Slot(s[1])], area, &room_min, &|_| true));
    let fit = t.fit(area, &room_min, &|_| true);
    assert!(fit.collapsed.is_empty(), "{:?}", fit.collapsed);
    let group = fit_width(&fit, g);
    assert!(group >= 20.0 - 1e-9, "{group}");
}

/// Too narrow for the column next to the group at all: the column
/// collapses in the outermost split, which nothing can grow. False, and
/// the tree is unchanged.
#[test]
fn make_room_without_room_changes_nothing() {
    let (mut t, s, _) = ask_beside_files(0.2, 0.6, 0.2);
    let before = t.clone();
    let area = UnitRect::new(0.0, 0.0, 40.0, 27.5);
    assert!(!t.make_room(&[Leaf::Slot(s[1])], area, &room_min, &|_| true));
    assert_eq!(t, before);
}

/// A leaf that already fits: true, the tree is unchanged.
#[test]
fn make_room_for_a_leaf_that_fits_changes_nothing() {
    let (mut t, s, _) = ask_beside_files(0.4, 0.4, 0.2);
    let before = t.clone();
    let area = UnitRect::new(0.0, 0.0, 100.0, 27.5);
    assert!(t.make_room(&[Leaf::Slot(s[1])], area, &room_min, &|_| true));
    assert_eq!(t, before);
}

/// OUTLINE is not drawn: it gives nothing and keeps its weight (its
/// region, by weight over every leaf, is as wide as before); the room
/// comes from the group.
#[test]
fn make_room_leaves_undrawn_leaves_alone() {
    let (mut t, s, g) = ask_beside_files(0.2, 0.6, 0.2);
    let area = UnitRect::new(0.0, 0.0, 59.5, 27.5);
    let drawn = |leaf: Leaf| leaf != Leaf::Slot(s[3]);
    let outline = rect_of(&t, s[3]).width();
    assert!(t.make_room(&[Leaf::Slot(s[1])], area, &room_min, &drawn));
    let fit = t.fit(area, &room_min, &drawn);
    assert!(fit.collapsed.is_empty(), "{:?}", fit.collapsed);
    assert!(fit_width(&fit, g) >= 20.0 - 1e-9);
    assert!((rect_of(&t, s[3]).width() - outline).abs() < 1e-9);
}

/// X (s1) joins the cell's split between A (s0) and B (s2): the split now
/// collapses B, not X. Making room for both grows the cell in the root
/// split until neither collapses; making room for X alone leaves B
/// collapsed, since X already fits.
#[test]
fn make_room_grows_for_every_leaf_it_is_given() {
    let s = [0, 1, 2].map(SlotId::new);
    let g = Leaf::Editor;
    let cell = Node::split(
        Axis::Horizontal,
        vec![
            (1.0, Node::leaf(Leaf::Slot(s[0]))),
            (1.0, Node::leaf(Leaf::Slot(s[1]))),
            (1.0, Node::leaf(Leaf::Slot(s[2]))),
        ],
    );
    let root = Node::split(Axis::Horizontal, vec![(0.5, cell), (0.5, Node::leaf(g))]);
    let slots = BTreeMap::from([
        (s[0], (vec!["files"], Some("files"))),
        (s[1], (vec!["ask"], Some("ask"))),
        (s[2], (vec!["tags"], Some("tags"))),
    ]);
    let t = Tree::from_parts(root, slots).unwrap();
    let area = UnitRect::new(0.0, 0.0, 59.5, 27.5);
    let (x, b) = (Leaf::Slot(s[1]), Leaf::Slot(s[2]));
    assert_eq!(t.fit(area, &room_min, &|_| true).collapsed, vec![b]);

    let mut alone = t.clone();
    assert!(alone.make_room(&[x], area, &room_min, &|_| true));
    assert_eq!(alone, t, "X fits already");

    let mut both = t.clone();
    assert!(both.make_room(&[x, b], area, &room_min, &|_| true));
    let fit = both.fit(area, &room_min, &|_| true);
    assert!(fit.collapsed.is_empty(), "{:?}", fit.collapsed);
    assert!(fit_width(&fit, g) >= 20.0 - 1e-9);
}

// ── The editor region inside the tree (ADR-0058) ──────────────────────

fn assert_rect(got: UnitRect, want: (f64, f64, f64, f64)) {
    let have = (got.x(), got.y(), got.width(), got.height());
    for (a, b) in [
        (have.0, want.0),
        (have.1, want.1),
        (have.2, want.2),
        (have.3, want.3),
    ] {
        assert!((a - b).abs() < 1e-9, "{have:?} != {want:?}");
    }
}

#[test]
fn regions_split_the_editor_leaf_into_the_shown_tabs_group_rects() {
    let mut t = tree();
    let g0 = t.active_group();
    let g1 = t.region_mut().split_group(g0, Axis::Horizontal).unwrap();
    let g2 = t.region_mut().split_group(g1, Axis::Vertical).unwrap();
    // The outer tree still has one editor leaf, whole.
    let editor: Vec<UnitRect> = t
        .regions()
        .into_iter()
        .filter(|(l, _)| *l == Leaf::Editor)
        .map(|(_, r)| r)
        .collect();
    assert_eq!(editor.len(), 1);
    assert_rect(editor[0], (0.2, 0.0, 0.6, 1.0));
    // Its rectangle is shared out among the shown tab's groups.
    let groups = t.group_regions();
    let ids: Vec<GroupId> = groups.iter().map(|(g, _)| *g).collect();
    assert_eq!(ids, vec![g0, g1, g2]);
    assert_rect(groups[0].1, (0.2, 0.0, 0.3, 1.0));
    assert_rect(groups[1].1, (0.5, 0.0, 0.3, 0.5));
    assert_rect(groups[2].1, (0.5, 0.5, 0.3, 0.5));
    // Another tab shown: only its group, over the whole editor leaf.
    let (tab, g3) = t.region_mut().new_tab();
    assert_eq!(t.region().shown(), tab);
    let groups = t.group_regions();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, g3);
    assert_rect(groups[0].1, (0.2, 0.0, 0.6, 1.0));
}

#[test]
fn focus_dir_moves_between_groups_of_the_shown_tab_before_leaving_the_region() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let g0 = t.active_group();
    let g1 = t.region_mut().split_group(g0, Axis::Horizontal).unwrap();
    let editor = TreeFocusMove::Moved(Focus::Leaf(Leaf::Editor));
    assert_eq!(t.focus_dir(Dir::Right), editor);
    assert_eq!(t.active_group(), g1);
    assert_eq!(t.focused(), Focus::Leaf(Leaf::Editor));
    // At the region's right edge: on to the right slot.
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(right)))
    );
    // Back in, the group last active has the keyboard.
    assert_eq!(t.focus_dir(Dir::Left), editor);
    assert_eq!(t.active_group(), g1);
    assert_eq!(t.focus_dir(Dir::Left), editor);
    assert_eq!(t.active_group(), g0);
    assert_eq!(
        t.focus_dir(Dir::Left),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(left)))
    );
    // Groups of a tab that is not shown are no targets.
    assert_eq!(t.focus_dir(Dir::Right), editor);
    let (_, g2) = t.region_mut().new_tab();
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(right)))
    );
    assert_eq!(t.active_group(), g2);
}

#[test]
fn focus_dir_leaves_the_region_at_its_edge_into_the_neighbor_slot() {
    // slot0 | editor (g0 over g1) | slot1 over slot2
    let (mut t, s, _) = nested();
    let g0 = t.active_group();
    let g1 = t.region_mut().split_group(g0, Axis::Vertical).unwrap();
    // From the top group, Up is the edge: something is stacked in the
    // column (the lower group), so the key is handled.
    assert_eq!(t.focus_dir(Dir::Up), TreeFocusMove::AtEdge);
    assert_eq!(
        t.focus_dir(Dir::Down),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Editor))
    );
    assert_eq!(t.active_group(), g1);
    assert_eq!(t.focus_dir(Dir::Down), TreeFocusMove::AtEdge);
    // Leaving to the right is measured from the lower group, so the lower
    // slot wins (from the whole region, the top-left one would).
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(s[2])))
    );
    assert!(t.focus(Focus::Leaf(Leaf::Editor)));
    assert_eq!(t.active_group(), g1);
    assert!(t.region_mut().set_active_group(g0));
    assert_eq!(
        t.focus_dir(Dir::Right),
        TreeFocusMove::Moved(Focus::Leaf(Leaf::Slot(s[1])))
    );
}

#[test]
fn edge_group_of_a_side_is_the_region_not_a_group() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let g0 = t.active_group();
    let g1 = t.region_mut().split_group(g0, Axis::Horizontal).unwrap();
    // A slot placed at the left edge of the editor lands left of the whole
    // region, never between its groups.
    let b = t.place_beside_editor("x", Dir::Left);
    let groups = t.group_regions();
    assert_eq!(
        groups.iter().map(|(g, _)| *g).collect::<Vec<_>>(),
        vec![g0, g1]
    );
    for (_, r) in &groups {
        assert!(r.x() >= rect_of(&t, b).right() - 1e-9, "{r:?}");
    }
    assert!((rect_of(&t, b).right() - groups[0].1.x()).abs() < 1e-9);
    assert_eq!(t.edge_slots(Side::Left), vec![left, b]);
    assert_eq!(t.side_slots(Side::Left), vec![left, b]);
    // On the right, the same.
    let c = t.place_beside_editor("y", Dir::Right);
    let groups = t.group_regions();
    assert!((rect_of(&t, c).x() - groups[1].1.right()).abs() < 1e-9);
    assert_eq!(t.edge_slots(Side::Right), vec![c, right]);
    assert_eq!(t.region().all_groups(), vec![g0, g1]);
}

#[test]
fn rail_position_reads_and_writes_lowercase() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Holder {
        rail: RailPosition,
    }
    for (text, want) in [
        ("left", RailPosition::Left),
        ("right", RailPosition::Right),
        ("top", RailPosition::Top),
        ("bottom", RailPosition::Bottom),
    ] {
        let parsed: Holder = toml::from_str(&format!("rail = \"{text}\"")).unwrap();
        assert_eq!(parsed.rail, want);
        assert_eq!(
            toml::to_string(&parsed).unwrap().trim(),
            format!("rail = \"{text}\"")
        );
    }
    assert!(toml::from_str::<Holder>("rail = \"diagonal\"").is_err());
    assert_eq!(RailPosition::default(), RailPosition::Left);
}

#[test]
fn toward_points_into_the_layout() {
    assert_eq!(RailPosition::Left.toward(), Dir::Right);
    assert_eq!(RailPosition::Right.toward(), Dir::Left);
    assert_eq!(RailPosition::Top.toward(), Dir::Down);
    assert_eq!(RailPosition::Bottom.toward(), Dir::Up);
}

#[test]
fn place_dir_is_the_axis_pair() {
    for rail in [RailPosition::Left, RailPosition::Right] {
        assert!(!rail.horizontal());
        assert!(rail.place_dir(Dir::Up) && rail.place_dir(Dir::Down));
        assert!(!rail.place_dir(Dir::Left) && !rail.place_dir(Dir::Right));
    }
    for rail in [RailPosition::Top, RailPosition::Bottom] {
        assert!(rail.horizontal());
        assert!(rail.place_dir(Dir::Left) && rail.place_dir(Dir::Right));
        assert!(!rail.place_dir(Dir::Up) && !rail.place_dir(Dir::Down));
    }
}

#[test]
fn slot_beside_rail_follows_the_edge() {
    let mut t = tree();
    let (left, right) = sides(&t);
    let left_below = t.split_slot(left, Axis::Vertical).unwrap();
    let right_below = t.split_slot(right, Axis::Vertical).unwrap();
    assert_eq!(t.slot_beside_rail(RailPosition::Left), Some(left));
    assert_eq!(t.slot_beside_rail(RailPosition::Right), Some(right));
    assert_eq!(t.slot_beside_rail(RailPosition::Top), Some(left));
    assert_eq!(t.slot_beside_rail(RailPosition::Bottom), Some(left_below));
    let _ = right_below;
}

#[test]
fn unplace_takes_a_view_out_and_keeps_the_rest() {
    let mut t = LayoutTree::default_layout("files");
    let left = t.view_slot(&"files").unwrap();
    assert!(t.place("find", left, false));
    assert_eq!(t.unplace(&"files"), Some(left));
    assert_eq!(t.view_slot(&"files"), None);
    assert_eq!(t.shown(left), None);
    assert_eq!(t.slot_views(left), ["find"]);
    assert_eq!(t.unplace(&"files"), None, "already unplaced");
}

#[test]
fn a_slot_beside_the_editor_takes_a_side_columns_share_not_half_of_it() {
    let mut t = LayoutTree::default_layout("files");
    let new = t.place_beside_editor("x", Dir::Right);
    let group = t
        .regions()
        .into_iter()
        .find(|(l, _)| *l == Leaf::Editor)
        .map(|(_, r)| r)
        .unwrap();
    // The default sides are 0.2 wide, the editor 0.6: the new column is as
    // wide as a side, and the editor gave up the difference.
    assert!((rect_of(&t, new).width() - 0.2).abs() < 1e-9);
    assert!((group.width() - 0.4).abs() < 1e-9);
}

#[test]
fn a_slot_above_or_below_the_editor_still_takes_half_of_it() {
    let mut t = LayoutTree::default_layout("files");
    let new = t.place_beside_editor("x", Dir::Down);
    let group = t
        .regions()
        .into_iter()
        .find(|(l, _)| *l == Leaf::Editor)
        .map(|(_, r)| r)
        .unwrap();
    assert!((rect_of(&t, new).height() - group.height()).abs() < 1e-9);
}

#[test]
fn the_last_slot_on_a_side_is_known_and_follows_the_active_group() {
    let t = LayoutTree::default_layout("files");
    let slots = t.slots();
    let (left, right) = (slots[0], slots[1]);
    assert!(t.last_on_side(left));
    assert!(t.last_on_side(right));

    let mut t = t;
    let below = t.split_slot(left, Axis::Vertical).unwrap();
    assert!(!t.last_on_side(left), "two slots on the left");
    assert!(!t.last_on_side(below));
    assert!(t.last_on_side(right));
}
