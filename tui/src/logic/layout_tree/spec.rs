//! Layout **specs**: the authored, serializable form of a [`LayoutTree`]
//! (ADR-0052 §5). A preset file holds named specs as nested TOML; a node is
//! a split (`split` axis + `children`), an editor group (`editor = true`) or
//! a slot (`views`, optional `shown`). Every child may carry a relative
//! `weight` (default 1). The "research" preset:
//!
//! ```toml
//! [preset.research]
//! split = "horizontal"
//! children = [
//!     { weight = 0.2, views = ["files"], shown = "files" },
//!     { weight = 0.6, editor = true },
//!     { weight = 0.2, split = "vertical", children = [
//!         { views = ["ask"], shown = "ask" },
//!         { views = ["outline", "links"] },
//!     ] },
//! ]
//! ```
//!
//! [`build`] validates a spec against the view registry and turns it into a
//! tree; [`to_spec`] goes the other way. Specs never carry slot or group
//! ids: the builder numbers them itself, in reading order.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{Axis, GroupId, LayoutTree, Leaf, Node, RailPosition, SlotId};
use crate::logic::nav::DrawerView;
use crate::logic::rag_caps::Caps;
use crate::logic::view_registry;

/// A split's axis as written (`"horizontal"` or `"vertical"`). Kept as text
/// so an unknown name is a [`SpecError::UnknownAxis`] with its path rather
/// than an opaque parse failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AxisName(pub String);

impl AxisName {
    pub fn of(axis: Axis) -> AxisName {
        AxisName(
            match axis {
                Axis::Horizontal => "horizontal",
                Axis::Vertical => "vertical",
            }
            .to_string(),
        )
    }

    pub fn axis(&self) -> Option<Axis> {
        match self.0.as_str() {
            "horizontal" => Some(Axis::Horizontal),
            "vertical" => Some(Axis::Vertical),
            _ => None,
        }
    }
}

/// One node of a spec, without its weight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum NodeSpec {
    Split {
        split: AxisName,
        children: Vec<ChildSpec>,
    },
    Editor {
        #[serde(deserialize_with = "must_be_true", serialize_with = "always_true")]
        editor: bool,
    },
    Slot {
        views: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shown: Option<String>,
    },
}

/// `editor = false` is not a node: reject it so it cannot pass for a group.
fn must_be_true<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    if bool::deserialize(d)? {
        Ok(true)
    } else {
        Err(serde::de::Error::custom("`editor` must be true"))
    }
}

fn always_true<S: Serializer>(_: &bool, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_bool(true)
}

/// A node with its relative weight among its siblings (default 1; ignored
/// at the root).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(flatten)]
    pub node: NodeSpec,
}

/// A presets file: `[preset.<name>]` tables, each a root [`ChildSpec`],
/// and optionally a `rail` key on the root table (ADR-0055 §2) that is not
/// part of the tree: [`parse_presets`] lifts it into [`PresetFile::rails`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PresetFile {
    #[serde(default)]
    pub preset: BTreeMap<String, ChildSpec>,
    /// The `rail` of each preset that sets one, by preset name.
    #[serde(skip)]
    pub rails: BTreeMap<String, RailPosition>,
}

/// Why a spec does not build. Every variant but [`SpecError::NoEditorGroup`]
/// carries the `path` of the offending node, `""` for the root, else like
/// `children[1].children[0]`.
#[derive(Debug, Clone, PartialEq)]
pub enum SpecError {
    /// A view id the registry does not know.
    UnknownView { path: String, id: String },
    /// A view placed a second time (`path` is the later slot).
    DuplicateView { path: String, id: String },
    /// A slot's `shown` is not one of its `views`.
    ShownNotInViews { path: String, shown: String },
    /// The tree has no `editor = true` leaf.
    NoEditorGroup,
    /// A weight that is zero, negative, NaN or infinite.
    BadWeight { path: String, weight: f64 },
    /// A split whose children's weights add up to more than a finite
    /// number (each finite, e.g. `1e308` twice); `path` is the split.
    WeightSumOverflow { path: String },
    /// An `editor = true` leaf after the first (`path` is the later one):
    /// a layout holds one editor group until group splitting exists.
    MultipleEditorGroups { path: String },
    /// A split with fewer than two children.
    EmptySplit { path: String },
    /// A split axis other than `horizontal` / `vertical`.
    UnknownAxis { path: String, axis: String },
}

impl SpecError {
    /// The offending node's path; `None` for a whole-tree error.
    pub fn path(&self) -> Option<&str> {
        match self {
            SpecError::UnknownView { path, .. }
            | SpecError::DuplicateView { path, .. }
            | SpecError::ShownNotInViews { path, .. }
            | SpecError::BadWeight { path, .. }
            | SpecError::WeightSumOverflow { path }
            | SpecError::MultipleEditorGroups { path }
            | SpecError::EmptySplit { path }
            | SpecError::UnknownAxis { path, .. } => Some(path),
            SpecError::NoEditorGroup => None,
        }
    }
}

/// `the root node` or `the node at <path>`.
fn at(path: &str) -> String {
    if path.is_empty() {
        "the root node".to_string()
    } else {
        format!("the node at {path}")
    }
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpecError::UnknownView { path, id } => {
                write!(f, "Unknown view \"{id}\" in {}.", at(path))
            }
            SpecError::DuplicateView { path, id } => write!(
                f,
                "View \"{id}\" in {} is already placed in another slot.",
                at(path)
            ),
            SpecError::ShownNotInViews { path, shown } => write!(
                f,
                "Shown view \"{shown}\" in {} is not one of its views.",
                at(path)
            ),
            SpecError::NoEditorGroup => {
                write!(f, "The layout has no editor group (`editor = true`).")
            }
            SpecError::BadWeight { path, weight } => write!(
                f,
                "Weight {weight} of {} must be a positive finite number.",
                at(path)
            ),
            SpecError::WeightSumOverflow { path } => write!(
                f,
                "The weights of the split at {} add up to more than a finite number.",
                if path.is_empty() { "the root" } else { path }
            ),
            SpecError::MultipleEditorGroups { path } => write!(
                f,
                "The editor group in {} is a second one; a layout holds one editor group until group splitting exists.",
                at(path)
            ),
            SpecError::EmptySplit { path } => write!(
                f,
                "The split at {} needs at least two children.",
                if path.is_empty() { "the root" } else { path }
            ),
            SpecError::UnknownAxis { path, axis } => write!(
                f,
                "Unknown split axis \"{axis}\" in {} (use \"horizontal\" or \"vertical\").",
                at(path)
            ),
        }
    }
}

impl std::error::Error for SpecError {}

/// Parse a presets file. Text that is not TOML gives the TOML parser's
/// message (with its line and column). TOML whose nodes are not nodes (a
/// misspelt key, a leaf that is two kinds at once, a value of the wrong
/// type) names each problem with its preset and node path instead of
/// serde's "did not match any variant".
pub fn parse_presets(text: &str) -> Result<PresetFile, String> {
    let mut table: toml::Table =
        toml::from_str(text).map_err(|e: toml::de::Error| e.to_string())?;
    // The `rail` key belongs to the preset, not to its tree: lift it out
    // before the tree is read (a node rejects keys it does not know).
    let rails = take_rails(&mut table)?;
    let mut file: PresetFile =
        toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e: toml::de::Error| {
                let problems = shape_problems(&table);
                if problems.is_empty() {
                    e.to_string()
                } else {
                    format!("{} {NODE_HELP}", list_problems(&problems, MAX_LISTED))
                }
            })?;
    file.rails = rails;
    Ok(file)
}

/// Remove the `rail` key from every preset's root table and read it.
fn take_rails(file: &mut toml::Table) -> Result<BTreeMap<String, RailPosition>, String> {
    let mut rails = BTreeMap::new();
    let Some(toml::Value::Table(presets)) = file.get_mut("preset") else {
        return Ok(rails);
    };
    for (name, node) in presets.iter_mut() {
        let toml::Value::Table(root) = node else {
            continue;
        };
        let Some(value) = root.remove("rail") else {
            continue;
        };
        match value.as_str().and_then(RailPosition::parse) {
            Some(rail) => {
                rails.insert(name.clone(), rail);
            }
            None => {
                return Err(format!(
                    "The `rail` of preset \"{name}\" must be \"left\", \"right\", \
                     \"top\" or \"bottom\", got {value}."
                ));
            }
        }
    }
    Ok(rails)
}

/// How many problems a message lists before it counts the rest.
pub const MAX_LISTED: usize = 3;

/// `problems` as one message: the first `max`, then how many more.
pub fn list_problems(problems: &[String], max: usize) -> String {
    let mut text = problems
        .iter()
        .take(max)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if problems.len() > max {
        text.push_str(&format!(" (and {} more)", problems.len() - max));
    }
    text
}

/// The keys a node may hold.
const NODE_KEYS: [&str; 6] = ["split", "children", "editor", "views", "shown", "weight"];

/// What a node is, for the messages.
const NODE_HELP: &str = "(A node is a `split` with `children`, `editor = true`, or `views` \
                         with an optional `shown`; any node may carry a `weight`.)";

/// Every structural problem of a parsed presets file, in reading order.
fn shape_problems(file: &toml::Table) -> Vec<String> {
    let mut problems = Vec::new();
    match file.get("preset") {
        None => {}
        Some(toml::Value::Table(presets)) => {
            for (name, node) in presets {
                node_problems(name, node, "", &mut problems);
            }
        }
        Some(_) => problems.push("`preset` must be a table of named presets.".to_string()),
    }
    problems
}

/// `text` with its first letter in upper case.
fn capitalized(text: String) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

/// `the root node of preset "x"` or `the node at <path> of preset "x"`.
fn where_in(name: &str, path: &str) -> String {
    format!("{} of preset \"{name}\"", at(path))
}

fn node_problems(name: &str, node: &toml::Value, path: &str, out: &mut Vec<String>) {
    use toml::Value;
    let here = || where_in(name, path);
    let Value::Table(table) = node else {
        out.push(format!("{} is not a table.", here()));
        return;
    };
    for key in table.keys() {
        if !NODE_KEYS.contains(&key.as_str()) {
            out.push(format!("Unknown key \"{key}\" in {}.", here()));
        }
    }
    let kinds: Vec<&str> = ["split", "editor", "views"]
        .into_iter()
        .filter(|k| table.contains_key(*k))
        .collect();
    match kinds.as_slice() {
        // An unknown key already says what is wrong with such a node.
        [] if table.keys().all(|k| NODE_KEYS.contains(&k.as_str())) => out.push(format!(
            "{} has no `split`, `editor` or `views`.",
            capitalized(here())
        )),
        [] | [_] => {}
        many => out.push(format!(
            "{} mixes {}.",
            capitalized(here()),
            many.iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(" and ")
        )),
    }
    let misplaced = |key: &str, owner: &str, out: &mut Vec<String>| {
        if table.contains_key(key) && !table.contains_key(owner) {
            out.push(format!("\"{key}\" in {} needs `{owner}`.", here()));
        }
    };
    if kinds.len() == 1 {
        misplaced("children", "split", out);
        misplaced("shown", "views", out);
    }
    if let Some(split) = table.get("split")
        && !split.is_str()
    {
        out.push(format!(
            "\"split\" in {} must be \"horizontal\" or \"vertical\".",
            here()
        ));
    }
    match table.get("editor") {
        None | Some(Value::Boolean(true)) => {}
        Some(_) => out.push(format!("`editor` in {} must be true.", here())),
    }
    match table.get("views") {
        None => {}
        Some(Value::Array(ids)) if ids.iter().all(Value::is_str) => {}
        Some(_) => out.push(format!(
            "\"views\" in {} must be a list of view ids.",
            here()
        )),
    }
    if let Some(shown) = table.get("shown")
        && !shown.is_str()
    {
        out.push(format!("\"shown\" in {} must be a view id.", here()));
    }
    if let Some(weight) = table.get("weight")
        && !(weight.is_float() || weight.is_integer())
    {
        out.push(format!("\"weight\" in {} must be a number.", here()));
    }
    match table.get("children") {
        None => {
            if table.contains_key("split") {
                out.push(format!(
                    "{} is a split with no `children`.",
                    capitalized(here())
                ));
            }
        }
        Some(Value::Array(children)) => {
            for (i, child) in children.iter().enumerate() {
                let child_path = if path.is_empty() {
                    format!("children[{i}]")
                } else {
                    format!("{path}.children[{i}]")
                };
                node_problems(name, child, &child_path, out);
            }
        }
        Some(_) => out.push(format!(
            "\"children\" in {} must be a list of nodes.",
            here()
        )),
    }
}

/// Validate `spec` and build its tree. Views the registry says are
/// unavailable with `caps` are skipped with a warning (a `shown` naming one
/// is cleared); every other problem is an error, all of them collected. An
/// editor-only tree, with no slot, is valid. Slot and group ids are
/// assigned here, sequentially in reading order.
pub fn build(
    spec: &ChildSpec,
    caps: Caps,
) -> Result<(LayoutTree<DrawerView>, Vec<String>), Vec<SpecError>> {
    let mut b = Builder {
        caps,
        errors: Vec::new(),
        warnings: Vec::new(),
        seen: Vec::new(),
        slots: BTreeMap::new(),
        next_slot: 0,
        next_group: 0,
    };
    let root = b.node(&spec.node, "");
    if b.next_group == 0 {
        b.errors.push(SpecError::NoEditorGroup);
    }
    match root {
        Some(root) if b.errors.is_empty() => {
            // Every invariant `from_parts` checks was validated above (the
            // ids are ours and distinct), so a refusal is a builder bug.
            let tree = LayoutTree::from_parts(root, b.slots).expect("a validated spec builds");
            Ok((tree, b.warnings))
        }
        _ => Err(b.errors),
    }
}

/// State of one [`build`] walk.
struct Builder {
    caps: Caps,
    errors: Vec<SpecError>,
    warnings: Vec<String>,
    /// Every view id placed so far (available or not), for duplicates.
    seen: Vec<String>,
    slots: BTreeMap<SlotId, (Vec<DrawerView>, Option<DrawerView>)>,
    next_slot: u32,
    next_group: u32,
}

impl Builder {
    /// Build the node at `path`, recording problems. `None` when the
    /// subtree has an error (the walk still visits all of it).
    fn node(&mut self, spec: &NodeSpec, path: &str) -> Option<Node<Leaf>> {
        match spec {
            NodeSpec::Editor { .. } => {
                if self.next_group > 0 {
                    self.errors.push(SpecError::MultipleEditorGroups {
                        path: path.to_string(),
                    });
                    return None;
                }
                let g = GroupId::new(self.next_group);
                self.next_group += 1;
                Some(Node::leaf(Leaf::Group(g)))
            }
            NodeSpec::Slot { views, shown } => self.slot(views, shown.as_deref(), path),
            NodeSpec::Split { split, children } => {
                let axis = split.axis();
                if axis.is_none() {
                    self.errors.push(SpecError::UnknownAxis {
                        path: path.to_string(),
                        axis: split.0.clone(),
                    });
                }
                if children.len() < 2 {
                    self.errors.push(SpecError::EmptySplit {
                        path: path.to_string(),
                    });
                }
                let mut built = Vec::new();
                let mut failed = false;
                for (i, child) in children.iter().enumerate() {
                    let child_path = if path.is_empty() {
                        format!("children[{i}]")
                    } else {
                        format!("{path}.children[{i}]")
                    };
                    let weight = child.weight.unwrap_or(1.0);
                    if !(weight.is_finite() && weight > 0.0) {
                        self.errors.push(SpecError::BadWeight {
                            path: child_path.clone(),
                            weight,
                        });
                        failed = true;
                    }
                    match self.node(&child.node, &child_path) {
                        Some(n) => built.push((weight, n)),
                        None => failed = true,
                    }
                }
                if !failed && !built.iter().map(|(w, _)| w).sum::<f64>().is_finite() {
                    self.errors.push(SpecError::WeightSumOverflow {
                        path: path.to_string(),
                    });
                    failed = true;
                }
                match axis {
                    Some(axis) if !failed && built.len() >= 2 => Some(Node::split(axis, built)),
                    _ => None,
                }
            }
        }
    }

    fn slot(&mut self, views: &[String], shown: Option<&str>, path: &str) -> Option<Node<Leaf>> {
        let before = self.errors.len();
        let mut placed = Vec::new();
        let mut skipped: Vec<&str> = Vec::new();
        for id in views {
            let Some(spec) = view_registry::by_id(id) else {
                self.errors.push(SpecError::UnknownView {
                    path: path.to_string(),
                    id: id.clone(),
                });
                continue;
            };
            if self.seen.contains(id) {
                self.errors.push(SpecError::DuplicateView {
                    path: path.to_string(),
                    id: id.clone(),
                });
                continue;
            }
            self.seen.push(id.clone());
            if view_registry::available(spec, self.caps) {
                placed.push(spec.view);
            } else {
                skipped.push(id);
                self.warnings.push(format!(
                    "View \"{id}\" in {} is not available on this server and was skipped.",
                    at(path)
                ));
            }
        }
        let shown = match shown {
            None => None,
            Some(s) if !views.iter().any(|v| v == s) => {
                self.errors.push(SpecError::ShownNotInViews {
                    path: path.to_string(),
                    shown: s.to_string(),
                });
                None
            }
            Some(s) if skipped.contains(&s) => None,
            Some(s) => view_registry::by_id(s).map(|spec| spec.view),
        };
        let id = SlotId::new(self.next_slot);
        self.next_slot += 1;
        if self.errors.len() > before {
            return None;
        }
        self.slots.insert(id, (placed, shown));
        Some(Node::leaf(Leaf::Slot(id)))
    }
}

/// The spec of `tree`'s arrangement: axes, weights (normalized per split)
/// and placements. Focus, zoom and ids are not part of it. The root has no
/// weight.
pub fn to_spec(tree: &LayoutTree<DrawerView>) -> ChildSpec {
    ChildSpec {
        weight: None,
        node: node_spec(tree, tree.node()),
    }
}

fn node_spec(tree: &LayoutTree<DrawerView>, node: &Node<Leaf>) -> NodeSpec {
    if let Some((axis, children)) = node.split_children() {
        return NodeSpec::Split {
            split: AxisName::of(axis),
            children: children
                .into_iter()
                .map(|(weight, child)| ChildSpec {
                    weight: Some(weight),
                    node: node_spec(tree, child),
                })
                .collect(),
        };
    }
    match node.as_leaf().expect("a node is a leaf or a split") {
        Leaf::Group(_) => NodeSpec::Editor { editor: true },
        Leaf::Slot(s) => NodeSpec::Slot {
            views: tree
                .slot_views(*s)
                .iter()
                .map(|v| v.id().to_string())
                .collect(),
            shown: tree.shown(*s).map(|v| v.id().to_string()),
        },
    }
}
