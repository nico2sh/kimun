//! The recursive layout node shared by slots and editor groups: a
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

/// The result of `Node::layout_fit`: rectangles for the leaves that fit,
/// and the leaves collapsed (given no rectangle) to make the rest fit.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit<L> {
    pub rects: Vec<(L, UnitRect)>,
    pub collapsed: Vec<L>,
}

/// A split's children with their normalized weights, in order.
pub(crate) type WeightedChildren<'a, L> = Vec<(f64, &'a Node<L>)>;

/// Smallest share a ratio setter leaves either side of a split.
pub(crate) const MIN_RATIO: f64 = 0.1;

/// The least length `Node::make_room` leaves a cell it takes from, so a
/// cell needing nothing keeps a weight above zero.
const MIN_SPARE: f64 = 1e-6;

/// The part of `size` (width, height) along `axis`.
fn along(axis: Axis, size: (f64, f64)) -> f64 {
    match axis {
        Axis::Horizontal => size.0,
        Axis::Vertical => size.1,
    }
}

/// The rectangle of each `active` child of a split over `area` along
/// `axis`: by weight among the active ones, in order. The one place that
/// does this arithmetic for [`Node::layout_fit`] and the walk of
/// [`Node::make_room`], so what `make_room` predicts is what `layout_fit`
/// lays out.
fn active_rects<L>(
    axis: Axis,
    area: UnitRect,
    children: &[Child<L>],
    active: &[usize],
) -> Vec<(usize, UnitRect)> {
    let total: f64 = active.iter().map(|&i| children[i].weight).sum();
    let mut offset = 0.0;
    active
        .iter()
        .map(|&i| {
            let share = children[i].weight / total;
            let rect = match axis {
                Axis::Horizontal => {
                    UnitRect::new(area.x + offset * area.w, area.y, share * area.w, area.h)
                }
                Axis::Vertical => {
                    UnitRect::new(area.x, area.y + offset * area.h, area.w, share * area.h)
                }
            };
            offset += share;
            (i, rect)
        })
        .collect()
}

/// One split on the way to a leaf, as `Node::make_room` measures it.
struct PathSplit {
    /// The child indices from the root down to this split.
    at: Vec<usize>,
    axis: Axis,
    /// The split's length along `axis`.
    len: f64,
    /// Its children holding a drawn leaf, in order.
    drawn: Vec<usize>,
    /// Of those, the ones that do not collapse, in order.
    active: Vec<usize>,
    /// Each child's need along `axis` (drawn leaves only).
    needs: Vec<f64>,
    weights: Vec<f64>,
    /// The child on the way to the leaf.
    toward: usize,
}

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

    /// A split's axis and its children with their weights (normalized to
    /// sum 1), in order; `None` for a leaf. Read-only, for spec export.
    pub(crate) fn split_children(&self) -> Option<(Axis, WeightedChildren<'_, L>)> {
        match &self.kind {
            Kind::Leaf(_) => None,
            Kind::Split { axis, children } => {
                let total: f64 = children.iter().map(|c| c.weight).sum();
                Some((
                    *axis,
                    children
                        .iter()
                        .map(|c| (c.weight / total, &c.node))
                        .collect(),
                ))
            }
        }
    }

    /// Number of leaves.
    #[cfg(test)]
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
        self.split_leaf_at(target, axis, new, false)
    }

    /// [`Self::split_leaf`] with the new leaf before (above, left of)
    /// `target` when `before`, after it otherwise.
    pub(crate) fn split_leaf_at(&mut self, target: &L, axis: Axis, new: L, before: bool) -> bool {
        match &mut self.kind {
            Kind::Leaf(id) => {
                if id != target {
                    return false;
                }
                let old = Child {
                    weight: 1.0,
                    node: Node::leaf(id.clone()),
                };
                let new = Child {
                    weight: 1.0,
                    node: Node::leaf(new),
                };
                self.kind = Kind::Split {
                    axis,
                    children: if before {
                        vec![new, old]
                    } else {
                        vec![old, new]
                    },
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
                            if before { i } else { i + 1 },
                            Child {
                                weight: half,
                                node: Node::leaf(new),
                            },
                        );
                        return true;
                    }
                    if children[i].node.contains(target) {
                        return children[i].node.split_leaf_at(target, axis, new, before);
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

    /// The root split's children as fractions summing to 1; empty for a leaf.
    #[cfg(test)]
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

    /// A split of `children` along `axis`, weights normalized to sum 1. Fewer
    /// than two children collapse to the single child.
    ///
    /// # Panics
    /// With no children (a layout node always holds a leaf).
    pub(crate) fn split(axis: Axis, mut children: Vec<(f64, Node<L>)>) -> Self {
        if children.len() == 1 {
            return children.remove(0).1;
        }
        assert!(!children.is_empty(), "a split needs at least one child");
        let weights = normalized(children.iter().map(|(w, _)| *w).collect());
        Self {
            kind: Kind::Split {
                axis,
                children: children
                    .into_iter()
                    .zip(weights)
                    .map(|((_, node), weight)| Child { weight, node })
                    .collect(),
            },
        }
    }

    /// Move the boundary after the branch holding `leaf`: in the innermost
    /// split where that branch has a following sibling, the branch takes
    /// `share` of (branch + next sibling), clamped to [`MIN_RATIO`]. False
    /// when no such split exists.
    pub(crate) fn resize_after(&mut self, leaf: &L, share: f64) -> bool {
        self.resize_after_visible(leaf, share, &|_| true)
    }

    /// [`Self::resize_after`] over what is drawn: the sibling the boundary
    /// is with is the next one holding a `visible` leaf, so two drawn
    /// branches with hidden siblings between them still share a boundary.
    /// Only those two branches' weights change; the hidden siblings keep
    /// theirs. False when no visible sibling follows at any level.
    pub(crate) fn resize_after_visible(
        &mut self,
        leaf: &L,
        share: f64,
        visible: &dyn Fn(&L) -> bool,
    ) -> bool {
        let Kind::Split { children, .. } = &mut self.kind else {
            return false;
        };
        let Some(i) = children.iter().position(|c| c.node.contains(leaf)) else {
            return false;
        };
        if children[i].node.resize_after_visible(leaf, share, visible) {
            return true;
        }
        if !share.is_finite() {
            return false;
        }
        let Some(j) =
            (i + 1..children.len()).find(|&j| children[j].node.leaves().into_iter().any(visible))
        else {
            return false;
        };
        let pair = children[i].weight + children[j].weight;
        let w = share.clamp(MIN_RATIO, 1.0 - MIN_RATIO) * pair;
        children[i].weight = w;
        children[j].weight = pair - w;
        true
    }

    /// The leaf that takes over `leaf`'s area if it closes: the nearest leaf
    /// of the adjacent sibling subtree (the previous sibling when `leaf` is
    /// last, else the next). `None` for the root leaf or an absent id.
    pub(crate) fn absorber(&self, leaf: &L) -> Option<L> {
        let Kind::Split { children, .. } = &self.kind else {
            return None;
        };
        let i = children.iter().position(|c| c.node.contains(leaf))?;
        if children[i].node.as_leaf().is_none() {
            return children[i].node.absorber(leaf);
        }
        if i + 1 == children.len() {
            children[i - 1].node.leaves().last().map(|l| (*l).clone())
        } else {
            children[i + 1].node.leaves().first().map(|l| (*l).clone())
        }
    }

    /// Minimum (width, height) of this node when nothing collapses.
    fn min_size(&self, min: &dyn Fn(&L) -> (f64, f64)) -> (f64, f64) {
        match &self.kind {
            Kind::Leaf(id) => min(id),
            Kind::Split { axis, children } => {
                let sizes = children.iter().map(|c| c.node.min_size(min));
                match axis {
                    Axis::Horizontal => {
                        sizes.fold((0.0, 0.0), |(w, h), (cw, ch)| (w + cw, h.max(ch)))
                    }
                    Axis::Vertical => {
                        sizes.fold((0.0, 0.0), |(w, h), (cw, ch)| (w.max(cw), h + ch))
                    }
                }
            }
        }
    }

    /// Like [`Self::layout`], but honouring minimum sizes. A split whose
    /// children's minimums along its own axis exceed its area collapses
    /// children last → first until the rest fit, never collapsing a subtree
    /// holding a `keep` leaf. Collapsed leaves get no rectangle and their
    /// weight goes to the survivors. Nested splits are checked in turn.
    pub(crate) fn layout_fit(
        &self,
        area: UnitRect,
        min: &dyn Fn(&L) -> (f64, f64),
        keep: &dyn Fn(&L) -> bool,
    ) -> Fit<L> {
        let mut fit = Fit {
            rects: Vec::new(),
            collapsed: Vec::new(),
        };
        self.fit_into(area, min, keep, &mut fit);
        fit
    }

    fn fit_into(
        &self,
        area: UnitRect,
        min: &dyn Fn(&L) -> (f64, f64),
        keep: &dyn Fn(&L) -> bool,
        fit: &mut Fit<L>,
    ) {
        let (axis, children) = match &self.kind {
            Kind::Leaf(id) => {
                fit.rects.push((id.clone(), area));
                return;
            }
            Kind::Split { axis, children } => (*axis, children),
        };
        let len = along(axis, (area.w, area.h));
        let needs: Vec<f64> = children
            .iter()
            .map(|c| along(axis, c.node.min_size(min)))
            .collect();
        let (active, gone) = collapse(len, (0..children.len()).collect(), &needs, &|i: usize| {
            children[i].node.leaves().into_iter().any(keep)
        });
        for i in gone {
            fit.collapsed
                .extend(children[i].node.leaves().into_iter().rev().cloned());
        }
        for (i, rect) in active_rects(axis, area, children, &active) {
            children[i].node.fit_into(rect, min, keep, fit);
        }
    }

    /// Grow, by weight, the room `targets` get until [`Self::layout_fit`]
    /// (with the leaves `drawn` rejects left out) collapses none of them.
    /// Each round takes the first target still collapsed and the first
    /// split on the way to it that collapses it, and widens the cell
    /// holding that split, in the nearest split above it along the same
    /// axis, to the split's full need. The room comes from the cell's
    /// surviving siblings, each giving only what it has above its own
    /// need, so none of them collapses there or (a `keep` one) goes under
    /// its minimum; the split itself then fits whole.
    ///
    /// Bounded: a round leaves the splits above the widened one as they
    /// were, only adds length inside the widened cell (where collapse can
    /// only shrink), and fixes one collapsing split, so normally each round
    /// removes a collapse and adds none. A new one can only appear in a
    /// split nested inside a sibling that gave room, so the rounds are
    /// also capped at one per leaf (plus one) for each target; at the cap
    /// it gives up. True when no target collapses (already, or now);
    /// false, the node unchanged, when a target is absent or not drawn,
    /// collapses in a split with nothing above it along that axis that can
    /// give, or the cap is reached.
    pub(crate) fn make_room(
        &mut self,
        targets: &[L],
        area: UnitRect,
        min: &dyn Fn(&L) -> (f64, f64),
        keep: &dyn Fn(&L) -> bool,
        drawn: &dyn Fn(&L) -> bool,
    ) -> bool {
        let mut grown = self.clone();
        let rounds = (self.leaves().len() + 1) * targets.len();
        for round in 0..=rounds {
            let mut collapsed = None;
            for target in targets {
                let Some((splits, collapsing)) = grown.path_to(target, area, min, keep, drawn)
                else {
                    return false;
                };
                if let Some(s) = collapsing {
                    collapsed = Some((splits, s));
                    break;
                }
            }
            let Some((splits, s)) = collapsed else {
                *self = grown;
                return true;
            };
            if round == rounds {
                // Out of rounds: the last widen was checked above.
                return false;
            }
            let short = &splits[s];
            let need: f64 = short.drawn.iter().map(|&i| short.needs[i]).sum();
            let Some(above) = splits[..s].iter().rev().find(|p| p.axis == short.axis) else {
                return false;
            };
            if !grown.widen(above, need - short.len) {
                return false;
            }
        }
        false
    }

    /// The splits on the way from this node to `target` as
    /// [`Self::layout_fit`] meets them with the leaves `drawn` rejects
    /// left out (so a split with fewer than two drawn children is passed
    /// through), and the index of the one that collapses `target`, if
    /// any. `None` when `target` is absent or not drawn.
    fn path_to(
        &self,
        target: &L,
        area: UnitRect,
        min: &dyn Fn(&L) -> (f64, f64),
        keep: &dyn Fn(&L) -> bool,
        drawn: &dyn Fn(&L) -> bool,
    ) -> Option<(Vec<PathSplit>, Option<usize>)> {
        if !drawn(target) {
            return None;
        }
        let drawn_min = |leaf: &L| if drawn(leaf) { min(leaf) } else { (0.0, 0.0) };
        let mut splits = Vec::new();
        let (mut node, mut area, mut at) = (self, area, Vec::new());
        loop {
            let (axis, children) = match &node.kind {
                Kind::Leaf(id) => return (id == target).then_some((splits, None)),
                Kind::Split { axis, children } => (*axis, children),
            };
            let toward = children.iter().position(|c| c.node.contains(target))?;
            let shown: Vec<usize> = (0..children.len())
                .filter(|&i| children[i].node.leaves().into_iter().any(drawn))
                .collect();
            if shown.len() >= 2 {
                let len = along(axis, (area.w, area.h));
                let needs: Vec<f64> = children
                    .iter()
                    .map(|c| along(axis, c.node.min_size(&drawn_min)))
                    .collect();
                let kept = |i: usize| {
                    children[i]
                        .node
                        .leaves()
                        .into_iter()
                        .any(|l| drawn(l) && keep(l))
                };
                let (active, _) = collapse(len, shown.clone(), &needs, &kept);
                let toward_rect = active_rects(axis, area, children, &active)
                    .into_iter()
                    .find(|&(i, _)| i == toward)
                    .map(|(_, rect)| rect);
                splits.push(PathSplit {
                    at: at.clone(),
                    axis,
                    len,
                    drawn: shown,
                    active,
                    needs,
                    weights: children.iter().map(|c| c.weight).collect(),
                    toward,
                });
                // Not among the active children: collapsed.
                let Some(rect) = toward_rect else {
                    let s = splits.len() - 1;
                    return Some((splits, Some(s)));
                };
                area = rect;
            }
            at.push(toward);
            node = &children[toward].node;
        }
    }

    /// Give the child of `split` on the way to the leaf `more` of the
    /// split's length, taken from its other surviving children in
    /// proportion to what each has above its need (never below it, nor
    /// to nothing). Undrawn and collapsed children keep their weights.
    /// False when the others have nothing to give.
    fn widen(&mut self, split: &PathSplit, more: f64) -> bool {
        let total: f64 = split.active.iter().map(|&i| split.weights[i]).sum();
        let spare = |i: usize| {
            let len = split.weights[i] / total * split.len;
            (len - split.needs[i].max(MIN_SPARE)).max(0.0)
        };
        let givers: Vec<usize> = split
            .active
            .iter()
            .copied()
            .filter(|&i| i != split.toward)
            .collect();
        let slack: f64 = givers.iter().map(|&i| spare(i)).sum();
        if slack <= 0.0 || more.is_nan() || more <= 0.0 {
            return false;
        }
        let gain = more.min(slack);
        let per_len = total / split.len;
        let Some(children) = self.children_at_mut(&split.at) else {
            return false;
        };
        children[split.toward].weight += gain * per_len;
        for i in givers {
            children[i].weight -= gain * spare(i) / slack * per_len;
        }
        true
    }

    /// The children of the split reached from this node by the child
    /// indices `path`; `None` when that is no split.
    fn children_at_mut(&mut self, path: &[usize]) -> Option<&mut Vec<Child<L>>> {
        let Kind::Split { children, .. } = &mut self.kind else {
            return None;
        };
        match path.split_first() {
            None => Some(children),
            Some((&i, rest)) => children.get_mut(i)?.node.children_at_mut(rest),
        }
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

/// The collapse rule of `Node::layout_fit` for one split `len` long: of
/// the children `candidates` (in order, `needs[i]` long each along the
/// split), drop the last one not `kept` while their needs exceed `len`.
/// The survivors in order, and the dropped ones in the order they went.
fn collapse(
    len: f64,
    mut candidates: Vec<usize>,
    needs: &[f64],
    kept: &dyn Fn(usize) -> bool,
) -> (Vec<usize>, Vec<usize>) {
    let mut gone = Vec::new();
    let mut sum: f64 = candidates.iter().map(|&i| needs[i]).sum();
    while sum > len + 1e-9 {
        let Some(pos) = candidates.iter().rposition(|&i| !kept(i)) else {
            break;
        };
        let i = candidates.remove(pos);
        sum -= needs[i];
        gone.push(i);
    }
    (candidates, gone)
}

/// `weights` scaled to sum 1. Positive finite weights whose sum overflows
/// are scaled down first; any zero, negative or non-finite weight gives
/// equal shares, so a split never lays out NaN rectangles.
fn normalized(weights: Vec<f64>) -> Vec<f64> {
    let n = weights.len() as f64;
    if !weights.iter().all(|w| w.is_finite() && *w > 0.0) {
        return vec![1.0 / n; weights.len()];
    }
    let total: f64 = weights.iter().sum();
    if total.is_finite() {
        return weights.iter().map(|w| w / total).collect();
    }
    let max = weights.iter().copied().fold(0.0, f64::max);
    let total: f64 = weights.iter().map(|w| w / max).sum();
    weights.iter().map(|w| w / max / total).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_with_bad_or_overflowing_weights_still_lays_out() {
        let huge = Node::split(
            Axis::Horizontal,
            vec![(1e308, Node::leaf('a')), (1e308, Node::leaf('b'))],
        );
        assert_eq!(huge.ratios(), vec![0.5, 0.5]);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let n = Node::split(
                Axis::Horizontal,
                vec![(bad, Node::leaf('a')), (1.0, Node::leaf('b'))],
            );
            assert_eq!(n.ratios(), vec![0.5, 0.5], "{bad}");
            assert!(n.layout(unit()).iter().all(|(_, r)| r.w.is_finite()));
        }
    }

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

    fn ids<L: Clone>(v: &[(L, UnitRect)]) -> Vec<L> {
        v.iter().map(|(l, _)| l.clone()).collect()
    }

    #[test]
    fn split_builds_normalized_weights() {
        let n = Node::split(
            Axis::Horizontal,
            vec![(1.0, Node::leaf('a')), (3.0, Node::leaf('b'))],
        );
        assert_eq!(n.axis(), Some(Axis::Horizontal));
        assert_eq!(n.ratios(), vec![0.25, 0.75]);
        let one = Node::split(Axis::Vertical, vec![(5.0, Node::leaf('a'))]);
        assert_eq!(one, Node::leaf('a'));
    }

    #[test]
    fn resize_after_moves_nested_boundary() {
        let col = Node::split(
            Axis::Vertical,
            vec![(1.0, Node::leaf('b')), (1.0, Node::leaf('c'))],
        );
        let mut n = Node::split(Axis::Horizontal, vec![(1.0, Node::leaf('a')), (1.0, col)]);
        assert!(n.resize_after(&'b', 0.7));
        let r = n.layout(unit());
        assert_eq!(r[0].1, UnitRect::new(0.0, 0.0, 0.5, 1.0));
        assert!((r[1].1.height() - 0.7).abs() < 1e-9);
        assert!((r[2].1.height() - 0.3).abs() < 1e-9);
        assert!(!n.resize_after(&'c', 0.5));
        assert!(n.resize_after(&'a', 0.2));
        assert!((n.layout(unit())[0].1.width() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn resize_after_visible_skips_hidden_siblings() {
        // a | b (hidden) | c | d: the boundary after `a` is with `c`.
        let mut n = Node::split(
            Axis::Horizontal,
            vec![
                (0.25, Node::leaf('a')),
                (0.25, Node::leaf('b')),
                (0.25, Node::leaf('c')),
                (0.25, Node::leaf('d')),
            ],
        );
        let visible = |l: &char| *l != 'b';
        assert!(n.resize_after_visible(&'a', 0.8, &visible));
        let r = n.ratios();
        assert!((r[0] - 0.4).abs() < 1e-9, "{r:?}");
        assert!((r[1] - 0.25).abs() < 1e-9, "b keeps its weight: {r:?}");
        assert!((r[2] - 0.1).abs() < 1e-9, "{r:?}");
        assert!((r[3] - 0.25).abs() < 1e-9, "d keeps its weight: {r:?}");
        // Nothing visible after `d`, nor after `c` once `d` hides.
        assert!(!n.resize_after_visible(&'d', 0.5, &visible));
        assert!(!n.resize_after_visible(&'c', 0.5, &|l: &char| *l != 'd'));
        // A nested branch whose following sibling is hidden defers to the
        // outer split.
        let col = Node::split(
            Axis::Vertical,
            vec![(1.0, Node::leaf('x')), (1.0, Node::leaf('y'))],
        );
        let mut n = Node::split(Axis::Horizontal, vec![(1.0, col), (1.0, Node::leaf('z'))]);
        assert!(n.resize_after_visible(&'x', 0.3, &|l: &char| *l != 'y'));
        assert!((n.ratios()[0] - 0.3).abs() < 1e-9, "{:?}", n.ratios());
    }

    #[test]
    fn absorber_prefers_previous_sibling_for_last_child_else_next() {
        let col = Node::split(
            Axis::Vertical,
            vec![(1.0, Node::leaf('b')), (1.0, Node::leaf('c'))],
        );
        let n = Node::split(
            Axis::Horizontal,
            vec![(1.0, Node::leaf('a')), (1.0, col), (1.0, Node::leaf('d'))],
        );
        assert_eq!(n.absorber(&'d'), Some('c'));
        assert_eq!(n.absorber(&'a'), Some('b'));
        assert_eq!(n.absorber(&'b'), Some('c'));
        assert_eq!(n.absorber(&'c'), Some('b'));
    }

    #[test]
    fn absorber_of_root_leaf_is_none() {
        assert_eq!(Node::leaf('a').absorber(&'a'), None);
    }

    fn row3() -> Node<char> {
        Node::split(
            Axis::Horizontal,
            vec![
                (1.0, Node::leaf('a')),
                (1.0, Node::leaf('b')),
                (1.0, Node::leaf('c')),
            ],
        )
    }

    #[test]
    fn fit_without_overflow_matches_layout() {
        let n = row3();
        let area = UnitRect::new(0.0, 0.0, 60.0, 10.0);
        let f = n.layout_fit(area, &|_| (12.0, 1.0), &|_| false);
        assert!(f.collapsed.is_empty());
        assert_eq!(f.rects, n.layout(area));
    }

    #[test]
    fn fit_collapses_trailing_children_of_first_overflowing_split() {
        let n = row3();
        let area = UnitRect::new(0.0, 0.0, 30.0, 10.0);
        let f = n.layout_fit(area, &|_| (12.0, 1.0), &|l| *l == 'a');
        assert_eq!(f.collapsed, vec!['c']);
        assert_eq!(ids(&f.rects), vec!['a', 'b']);
        assert_eq!(f.rects[0].1, UnitRect::new(0.0, 0.0, 15.0, 10.0));
        assert_eq!(f.rects[1].1, UnitRect::new(15.0, 0.0, 15.0, 10.0));
        let area = UnitRect::new(0.0, 0.0, 20.0, 10.0);
        let f = n.layout_fit(area, &|_| (12.0, 1.0), &|l| *l == 'a');
        assert_eq!(f.collapsed, vec!['c', 'b']);
        assert_eq!(f.rects, vec![('a', area)]);
    }

    #[test]
    fn fit_never_collapses_kept_subtree() {
        let col = Node::split(
            Axis::Vertical,
            vec![(1.0, Node::leaf('b')), (1.0, Node::leaf('c'))],
        );
        let n = Node::split(Axis::Horizontal, vec![(1.0, Node::leaf('a')), (1.0, col)]);
        let area = UnitRect::new(0.0, 0.0, 20.0, 10.0);
        let f = n.layout_fit(area, &|_| (12.0, 1.0), &|l| *l == 'c');
        assert_eq!(f.collapsed, vec!['a']);
        assert_eq!(ids(&f.rects), vec!['b', 'c']);
        // Everything kept: nothing collapses even though it overflows.
        let f = n.layout_fit(area, &|_| (12.0, 1.0), &|_| true);
        assert!(f.collapsed.is_empty());
        assert_eq!(f.rects.len(), 3);
    }

    /// `make_room` predicts collapse with `path_to`; it must say what
    /// `layout_fit` does over the tree with the undrawn leaves removed.
    #[test]
    fn path_to_agrees_with_layout_fit() {
        let col = |a, b, c| {
            Node::split(
                Axis::Vertical,
                vec![
                    (1.0, Node::leaf(a)),
                    (2.0, Node::leaf(b)),
                    (1.0, Node::leaf(c)),
                ],
            )
        };
        let row = Node::split(
            Axis::Horizontal,
            vec![(1.0, Node::leaf('e')), (3.0, col('f', 'g', 'h'))],
        );
        let tree = Node::split(
            Axis::Horizontal,
            vec![
                (1.0, col('a', 'b', 'c')),
                (2.0, Node::leaf('d')),
                (1.0, row),
                (1.0, Node::leaf('i')),
            ],
        );
        let min = |_: &char| (12.0, 3.0);
        let keep = |l: &char| *l == 'd';
        let undrawn_sets: [&[char]; 4] = [&[], &['b'], &['a', 'g', 'i'], &['e', 'f', 'h']];
        for width in [20.0, 40.0, 70.0, 120.0] {
            for height in [4.0, 9.0, 30.0] {
                let area = UnitRect::new(0.0, 0.0, width, height);
                for undrawn in undrawn_sets {
                    let drawn = |l: &char| !undrawn.contains(l);
                    let mut pruned = tree.clone();
                    for l in undrawn {
                        pruned.remove_leaf(l);
                    }
                    let fit = pruned.layout_fit(area, &min, &keep);
                    for leaf in tree.leaves().into_iter().filter(|l| drawn(l)) {
                        let predicted = tree
                            .path_to(leaf, area, &min, &keep, &drawn)
                            .expect("drawn leaf is in the tree")
                            .1
                            .is_some();
                        assert_eq!(
                            predicted,
                            fit.collapsed.contains(leaf),
                            "{leaf} in {width}x{height} without {undrawn:?}"
                        );
                    }
                }
            }
        }
    }
}
