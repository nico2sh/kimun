//! The weights store of GUI motion (ADR-0060): how a layout cell that
//! changes size moves, and what a removed cell leaves behind while it
//! decays. It is the second half of the GPUI motion module, and it names the
//! kit's motion types like its parent does.
//!
//! A cell is a child of a split of the layout tree. Its [`CellKey`] says
//! which one: the leaf at its end, its depth in the tree, and whether the
//! cell is that one leaf (a subtree cell and the leaf at its end are two
//! keys, so a collapse that shifts depths cannot make one take the other's
//! weight). The store keeps no presence: a thing that opens and closes is in
//! the presence store of the parent module.

use std::collections::{HashMap, HashSet};

use gpui_kit::base::motion::{Transition, TransitionState};
use gpui_kit::{App, Window};
use kimun_notes::logic::layout_tree::{Axis, Leaf, TreeShape};

use crate::motion::{Speed, Stage, timing};
use crate::workspace::tree_view::last_leaf;

/// One cell of the layout tree: a child of a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct CellKey {
    /// The leaf at the end of the cell.
    pub leaf: Leaf,
    /// The nesting depth of the cell (the root's children are at 1).
    pub depth: u8,
    /// The cell is that one leaf, not a split that ends in it.
    pub is_leaf: bool,
}

impl CellKey {
    /// The key of the layout cell `shape`, which stands at nesting `depth`.
    /// The layout pass and the motion pass both name a cell this way, and
    /// only this way.
    pub(in crate::workspace) fn of(shape: &TreeShape, depth: u8) -> CellKey {
        CellKey {
            leaf: last_leaf(shape),
            depth,
            is_leaf: matches!(shape, TreeShape::Leaf(_)),
        }
    }
}

/// What one weight is asked to be in one frame: it moves from where it is to
/// `target` over the `speed`.
#[derive(Debug, Clone)]
pub(in crate::workspace) struct ValueSpec {
    pub key: CellKey,
    pub target: f32,
    pub speed: Speed,
    /// Keys a new `key` can start from: the weights of the leaves inside
    /// the cell. A cell that gets wrapped changes its key, and must not jump.
    pub seed_from: Vec<CellKey>,
}

/// The weights next to a split child, as the layout pass last drew it: what a
/// ghost needs to be put back where its cell stood (slot removal).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::workspace) struct Siblings {
    /// The key of the child before it, and of the one after it.
    pub prev: Option<CellKey>,
    pub next: Option<CellKey>,
    /// The cell is one leaf: only that kind of cell leaves a ghost. A subtree
    /// cell that collapsed has no parent split left to be drawn in.
    pub leaf: bool,
    /// The px the cell had along its split when it was last drawn: the ghost
    /// holds that much room and gives it back as it decays. The cell's
    /// weight alone is not enough, as a view's minimum can hold a cell wider
    /// than its weight share.
    pub px: f64,
    /// The axis of the split the cell stood in: a ghost is drawn only in a
    /// split along the same axis, so a split that took the place of its own
    /// (a collapse into a surviving split) does not get a spacer of the wrong
    /// kind.
    pub axis: Axis,
}

/// The weight a removed cell leaves behind while it decays to nothing, so
/// the cells next to it grow into its room over time, not at once.
#[derive(Debug, Clone, Copy)]
pub(in crate::workspace) struct Ghost {
    /// Which cell it was. The layout pass finds a ghost by its neighbors, so
    /// only the tests read it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub key: CellKey,
    pub siblings: Siblings,
    /// Its weight now, and the weight it had when it began to decay: the
    /// share of its room that is left is the first over the second.
    pub weight: f32,
    pub from: f32,
}

#[derive(Default)]
struct Cell {
    /// Where the weight is moving to, and where it is in this frame.
    value: TransitionState<f32>,
    now: Option<f32>,
    /// The weight is no longer asked for and decays to 0: a ghost, and the
    /// weight it had when it began.
    ghost: bool,
    ghost_from: f32,
    /// The speed of the last sample: a ghost decays at it.
    speed: Speed,
    /// When the cell was made: the one order that ghosts can be put in, the
    /// same in every process.
    born: u64,
    /// What the layout pass last drew next to the cell.
    siblings: Option<Siblings>,
}

/// The weight of every cell the layout pass has met.
#[derive(Default)]
pub(in crate::workspace) struct Weights {
    cells: HashMap<CellKey, Cell>,
    /// How many cells were made so far: what [`Cell::born`] counts.
    births: u64,
}

impl Weights {
    /// The cell of `key`, made (and numbered) if it is new.
    fn cell_mut(&mut self, key: CellKey) -> &mut Cell {
        let births = &mut self.births;
        self.cells.entry(key).or_insert_with(|| {
            *births += 1;
            Cell {
                born: *births,
                ..Cell::default()
            }
        })
    }

    /// Move every weight toward its target for this frame. A weight with no
    /// state adopts its target, and so does every weight on a snapping
    /// frame (`snap`: a divider drag follows the pointer, a window resize
    /// reflows at once). A weight that is not in `specs` is dropped, unless
    /// it is a ghost: see below.
    pub(in crate::workspace) fn sample(
        &mut self,
        specs: &[ValueSpec],
        snap: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut used_as_seed: HashSet<CellKey> = HashSet::new();
        for spec in specs {
            // A key that is new starts where the key it replaces was.
            let seed = (!snap && !self.cells.contains_key(&spec.key))
                .then(|| {
                    spec.seed_from.iter().find_map(|key| {
                        let value = self.cells.get(key).and_then(|cell| cell.now)?;
                        Some((*key, value))
                    })
                })
                .flatten();
            let cell = self.cell_mut(spec.key);
            cell.ghost = false;
            if snap {
                cell.value = TransitionState::default();
            }
            cell.speed = spec.speed;
            // On the clock of a slot's frame, so a column that opens and the
            // cell it takes room from move together: the sum of their sizes
            // does not overshoot.
            let policy = Transition::new(timing(spec.speed, Stage::Frame, true).duration);
            if let Some((key, from)) = seed {
                used_as_seed.insert(key);
                cell.value.sample(from, policy.clone(), window, cx);
            }
            cell.now = Some(cell.value.sample(spec.target, policy, window, cx).value);
        }
        let asked: HashSet<CellKey> = specs.iter().map(|spec| spec.key).collect();
        // The leaves that still stand in the tree: each has a cell asked for.
        let standing: HashSet<Leaf> = specs
            .iter()
            .filter(|spec| spec.key.is_leaf)
            .map(|spec| spec.key.leaf)
            .collect();
        // A weight that is no longer asked for is a ghost, if it was a leaf
        // cell that left the tree and no new key took its weight over (a
        // leaf that is still there changed its key, it did not leave). It
        // decays to 0, and the layout pass draws a spacer for it that gives
        // its px back, so the cells next to it grow into its room. A
        // snapping frame keeps no ghost, and drops the ones that exist.
        let leaving: Vec<(CellKey, Speed)> = if snap {
            Vec::new()
        } else {
            self.cells
                .iter()
                .filter(|(key, _)| !asked.contains(key) && !used_as_seed.contains(key))
                .filter(|(key, _)| !standing.contains(&key.leaf))
                .filter(|(_, cell)| cell.now.is_some_and(|value| value > 0.0))
                .filter(|(_, cell)| cell.siblings.is_some_and(|s| s.leaf))
                .map(|(key, cell)| (*key, cell.speed))
                .collect()
        };
        for (key, speed) in leaving {
            let policy = Transition::new(timing(speed, Stage::Frame, true).duration);
            if let Some(cell) = self.cells.get_mut(&key) {
                if !cell.ghost {
                    cell.ghost_from = cell.now.unwrap_or(0.0);
                }
                cell.ghost = true;
                cell.now = Some(cell.value.sample(0.0, policy, window, cx).value);
            }
        }
        self.cells.retain(|key, cell| {
            asked.contains(key)
                || (cell.ghost && !snap && cell.now.is_some_and(|value| value > 0.0))
        });
    }

    /// The ghosts at nesting depth `depth`: the weights of cells removed
    /// from the tree that are still decaying.
    pub(in crate::workspace) fn ghosts(&self, depth: u8) -> Vec<Ghost> {
        let mut ghosts: Vec<(u64, Ghost)> = self
            .cells
            .iter()
            .filter(|(key, cell)| cell.ghost && key.depth == depth)
            .filter_map(|(key, cell)| {
                Some((
                    cell.born,
                    Ghost {
                        key: *key,
                        siblings: cell.siblings?,
                        weight: cell.now?,
                        from: cell.ghost_from,
                    },
                ))
            })
            .collect();
        // The same order in every process.
        ghosts.sort_by_key(|(born, _)| *born);
        ghosts.into_iter().map(|(_, ghost)| ghost).collect()
    }

    /// Whether the store holds the cell `key`.
    pub(in crate::workspace) fn holds(&self, key: CellKey) -> bool {
        self.cells.contains_key(&key)
    }

    /// What a weight was sampled to in this frame.
    pub(in crate::workspace) fn value(&self, key: CellKey) -> Option<f32> {
        self.cells.get(&key)?.now
    }

    /// Remember what stands next to the cell `key` and how many px it has,
    /// for a ghost.
    pub(in crate::workspace) fn remember(&mut self, key: CellKey, siblings: Siblings) {
        self.cell_mut(key).siblings = Some(siblings);
    }

    /// The cells the store holds, the ghosts included.
    #[cfg(test)]
    pub(in crate::workspace) fn keys(&self) -> Vec<CellKey> {
        self.cells.keys().copied().collect()
    }

    /// Forget every cell: the layout tree was replaced, and the new cells
    /// have nothing to move from.
    pub(in crate::workspace) fn forget_all(&mut self) {
        self.cells.clear();
    }
}
