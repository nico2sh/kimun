//! The recursive layout node shared by drawer slots and editor groups: a
//! **leaf** holding one id, or a **split** laying its children out along an
//! axis by relative weight. Toolkit-free: geometry is in abstract
//! [`UnitRect`]s, which the front end scales to columns or pixels.

/// The direction a split lays its children out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Children side by side, left → right.
    Horizontal,
    /// Children stacked, top → bottom.
    Vertical,
}

/// An axis-aligned rectangle in abstract layout units — not terminal cells,
/// not pixels. Only relative position matters to the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl UnitRect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn width(&self) -> f64 {
        self.w
    }
    pub fn height(&self) -> f64 {
        self.h
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Kind<L> {
    Leaf(L),
    Split { axis: Axis, children: Vec<Child<L>> },
}

#[derive(Debug, Clone, PartialEq)]
struct Child<L> {
    weight: f64,
    node: Node<L>,
}

/// A layout tree whose leaves are distinct ids (`L`). A split always has at
/// least two children; removing a leaf collapses a split left with one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node<L> {
    kind: Kind<L>,
}

/// Smallest share a ratio setter leaves either side of a split.
const MIN_RATIO: f64 = 0.1;

impl<L: Clone + PartialEq> Node<L> {
    /// A single leaf.
    pub(crate) fn leaf(id: L) -> Self {
        Self {
            kind: Kind::Leaf(id),
        }
    }

    /// The id when this node is a leaf.
    pub(crate) fn as_leaf(&self) -> Option<&L> {
        match &self.kind {
            Kind::Leaf(id) => Some(id),
            Kind::Split { .. } => None,
        }
    }

    /// The split axis, `None` for a leaf.
    #[cfg(test)]
    pub(crate) fn axis(&self) -> Option<Axis> {
        match &self.kind {
            Kind::Leaf(_) => None,
            Kind::Split { axis, .. } => Some(*axis),
        }
    }

    /// Every leaf in reading order (left → right, top → bottom).
    pub(crate) fn leaves(&self) -> Vec<&L> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves<'a>(&'a self, out: &mut Vec<&'a L>) {
        match &self.kind {
            Kind::Leaf(id) => out.push(id),
            Kind::Split { children, .. } => {
                for c in children {
                    c.node.collect_leaves(out);
                }
            }
        }
    }

    /// Number of leaves.
    pub(crate) fn leaf_count(&self) -> usize {
        match &self.kind {
            Kind::Leaf(_) => 1,
            Kind::Split { children, .. } => children.iter().map(|c| c.node.leaf_count()).sum(),
        }
    }

    /// Whether `id` is one of the leaves.
    pub(crate) fn contains(&self, id: &L) -> bool {
        self.leaves().into_iter().any(|l| l == id)
    }

    /// Insert `new` right after (or below) the leaf `target` along `axis`.
    /// When `target` already sits in a split along `axis` the new leaf joins
    /// that split as a sibling; otherwise `target` becomes a two-way split.
    /// `target` gives up half its weight. False when `target` is absent.
    pub(crate) fn split_leaf(&mut self, target: &L, axis: Axis, new: L) -> bool {
        match &mut self.kind {
            Kind::Leaf(id) => {
                if id != target {
                    return false;
                }
                let old = Node::leaf(id.clone());
                self.kind = Kind::Split {
                    axis,
                    children: vec![
                        Child {
                            weight: 1.0,
                            node: old,
                        },
                        Child {
                            weight: 1.0,
                            node: Node::leaf(new),
                        },
                    ],
                };
                true
            }
            Kind::Split {
                axis: own,
                children,
            } => {
                let own = *own;
                for i in 0..children.len() {
                    let is_target = children[i].node.as_leaf() == Some(target);
                    if is_target && own == axis {
                        let half = children[i].weight / 2.0;
                        children[i].weight = half;
                        children.insert(
                            i + 1,
                            Child {
                                weight: half,
                                node: Node::leaf(new),
                            },
                        );
                        return true;
                    }
                    if children[i].node.contains(target) {
                        return children[i].node.split_leaf(target, axis, new);
                    }
                }
                false
            }
        }
    }

    /// Remove the leaf `target`. A split left with one child collapses into
    /// it. The root leaf cannot be removed (the owner drops the whole node).
    /// False when `target` is absent or is the root leaf.
    pub(crate) fn remove_leaf(&mut self, target: &L) -> bool {
        let Kind::Split { children, .. } = &mut self.kind else {
            return false;
        };
        let Some(i) = children.iter().position(|c| c.node.contains(target)) else {
            return false;
        };
        if children[i].node.as_leaf().is_some() {
            children.remove(i);
        } else if !children[i].node.remove_leaf(target) {
            return false;
        }
        if children.len() == 1 {
            let only = children.remove(0).node;
            *self = only;
        }
        true
    }

    /// Swap the leaf `old` for `new` in place. False when `old` is absent.
    pub(crate) fn replace_leaf(&mut self, old: &L, new: L) -> bool {
        match &mut self.kind {
            Kind::Leaf(id) if id == old => {
                *id = new;
                true
            }
            Kind::Leaf(_) => false,
            Kind::Split { children, .. } => children
                .iter_mut()
                .find(|c| c.node.contains(old))
                .is_some_and(|c| c.node.replace_leaf(old, new)),
        }
    }

    /// The root split's children as fractions summing to 1; empty for a leaf.
    pub(crate) fn ratios(&self) -> Vec<f64> {
        match &self.kind {
            Kind::Leaf(_) => Vec::new(),
            Kind::Split { children, .. } => {
                let total: f64 = children.iter().map(|c| c.weight).sum();
                children.iter().map(|c| c.weight / total).collect()
            }
        }
    }

    /// Give the root split's first child `ratio` of the space (clamped so
    /// every side keeps a usable share); the others share the rest in their
    /// current proportion. False for a leaf.
    pub(crate) fn set_first_ratio(&mut self, ratio: f64) -> bool {
        let Kind::Split { children, .. } = &mut self.kind else {
            return false;
        };
        let ratio = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        let rest: f64 = children.iter().skip(1).map(|c| c.weight).sum();
        children[0].weight = ratio;
        for c in children.iter_mut().skip(1) {
            c.weight = (1.0 - ratio) * c.weight / rest;
        }
        true
    }

    /// Lay the leaves out over `area`, splitting by weight along each axis.
    pub(crate) fn layout(&self, area: UnitRect) -> Vec<(L, UnitRect)> {
        let mut out = Vec::new();
        self.layout_into(area, &mut out);
        out
    }

    fn layout_into(&self, area: UnitRect, out: &mut Vec<(L, UnitRect)>) {
        match &self.kind {
            Kind::Leaf(id) => out.push((id.clone(), area)),
            Kind::Split { axis, children } => {
                let total: f64 = children.iter().map(|c| c.weight).sum();
                let mut offset = 0.0;
                for c in children {
                    let share = c.weight / total;
                    let rect = match axis {
                        Axis::Horizontal => {
                            UnitRect::new(area.x + offset * area.w, area.y, share * area.w, area.h)
                        }
                        Axis::Vertical => {
                            UnitRect::new(area.x, area.y + offset * area.h, area.w, share * area.h)
                        }
                    };
                    c.node.layout_into(rect, out);
                    offset += share;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> UnitRect {
        UnitRect::new(0.0, 0.0, 1.0, 1.0)
    }

    #[test]
    fn a_leaf_is_one_region_covering_the_area() {
        let n = Node::leaf('a');
        assert_eq!(n.leaves(), vec![&'a']);
        assert_eq!(n.leaf_count(), 1);
        assert_eq!(n.axis(), None);
        assert_eq!(n.layout(unit()), vec![('a', unit())]);
    }

    #[test]
    fn splitting_a_leaf_halves_it_along_the_axis() {
        let mut n = Node::leaf('a');
        assert!(n.split_leaf(&'a', Axis::Vertical, 'b'));
        assert_eq!(n.axis(), Some(Axis::Vertical));
        assert_eq!(n.leaves(), vec![&'a', &'b']);
        assert_eq!(n.ratios(), vec![0.5, 0.5]);
        assert_eq!(
            n.layout(unit()),
            vec![
                ('a', UnitRect::new(0.0, 0.0, 1.0, 0.5)),
                ('b', UnitRect::new(0.0, 0.5, 1.0, 0.5)),
            ]
        );
    }

    #[test]
    fn splitting_along_the_parent_axis_adds_a_sibling() {
        let mut n = Node::leaf('a');
        n.split_leaf(&'a', Axis::Horizontal, 'b');
        n.split_leaf(&'a', Axis::Horizontal, 'c');
        assert_eq!(n.leaves(), vec![&'a', &'c', &'b']);
        assert_eq!(n.ratios(), vec![0.25, 0.25, 0.5]);
    }

    #[test]
    fn splitting_across_the_parent_axis_nests() {
        let mut n = Node::leaf('a');
        n.split_leaf(&'a', Axis::Horizontal, 'b');
        n.split_leaf(&'b', Axis::Vertical, 'c');
        assert_eq!(n.leaves(), vec![&'a', &'b', &'c']);
        let rects = n.layout(unit());
        assert_eq!(rects[0], ('a', UnitRect::new(0.0, 0.0, 0.5, 1.0)));
        assert_eq!(rects[1], ('b', UnitRect::new(0.5, 0.0, 0.5, 0.5)));
        assert_eq!(rects[2], ('c', UnitRect::new(0.5, 0.5, 0.5, 0.5)));
    }

    #[test]
    fn splitting_a_missing_leaf_changes_nothing() {
        let mut n = Node::leaf('a');
        assert!(!n.split_leaf(&'z', Axis::Vertical, 'b'));
        assert_eq!(n, Node::leaf('a'));
    }

    #[test]
    fn removing_a_leaf_collapses_a_single_child_split() {
        let mut n = Node::leaf('a');
        n.split_leaf(&'a', Axis::Horizontal, 'b');
        n.split_leaf(&'b', Axis::Vertical, 'c');
        assert!(n.remove_leaf(&'c'));
        assert_eq!(n.leaves(), vec![&'a', &'b']);
        assert_eq!(n.axis(), Some(Axis::Horizontal));
        assert!(n.remove_leaf(&'a'));
        assert_eq!(n, Node::leaf('b'));
    }

    #[test]
    fn the_root_leaf_cannot_be_removed() {
        let mut n = Node::leaf('a');
        assert!(!n.remove_leaf(&'a'));
        assert!(!n.remove_leaf(&'z'));
        assert_eq!(n, Node::leaf('a'));
    }

    #[test]
    fn replace_swaps_a_leaf_in_place() {
        let mut n = Node::leaf('a');
        n.split_leaf(&'a', Axis::Vertical, 'b');
        assert!(n.replace_leaf(&'b', 'x'));
        assert_eq!(n.leaves(), vec![&'a', &'x']);
        assert!(!n.replace_leaf(&'z', 'y'));
    }

    #[test]
    fn first_ratio_is_clamped_and_moves_the_boundary() {
        let mut n = Node::leaf('a');
        assert!(!n.set_first_ratio(0.3), "a leaf has no ratio");
        n.split_leaf(&'a', Axis::Vertical, 'b');
        assert!(n.set_first_ratio(0.25));
        assert_eq!(n.ratios(), vec![0.25, 0.75]);
        assert_eq!(n.layout(unit())[1].1, UnitRect::new(0.0, 0.25, 1.0, 0.75));
        n.set_first_ratio(0.0);
        assert!((n.ratios()[0] - MIN_RATIO).abs() < 1e-5);
        n.set_first_ratio(1.0);
        assert!((n.ratios()[0] - (1.0 - MIN_RATIO)).abs() < 1e-5);
    }
}
