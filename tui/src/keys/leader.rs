//! The **leader engine** — the non-modal key-sequence state machine behind
//! the leader gateway (Ctrl-G; spec §8 says Ctrl-K, which stays the note
//! browser here). The gateway starts a sequence in every context; subsequent
//! keys walk the leader tree until a leaf fires, `Esc` cancels, or
//! `Backspace` steps up a level. The which-key overlay (phase 06) renders
//! the pending node; this module is pure input logic.

use std::time::Instant;

use crate::components::drawer::DrawerView;
use crate::components::drawer_views::LinksTab;

/// What a leader leaf does. Executed by the editor screen, which owns every
/// surface the actions touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderAction {
    // +open drawer
    OpenDrawer(DrawerView),
    // +find — these route to the existing pickers/drawers until the
    // telescope modal (phase 08) takes the list-style leaves over.
    FindFiles,
    FindGrep,
    FindTags,
    FindBacklinks,
    FindRecent,
    FindSaved,
    FindHeadings,
    // +note
    NoteNew,
    NoteDaily,
    NoteFromTemplate,
    NoteRename,
    NoteMove,
    NoteDelete,
    // +links (for the open note)
    LinksTab(LinksTab),
    LinksGraph,
    // +git/sync
    GitStatus,
    GitSync,
    GitLog,
    GitDiff,
    // +vault
    VaultSwitch,
    VaultReindex,
    VaultConfig,
    VaultTheme,
    VaultPreferences,
    // +window
    WindowZen,
    WindowSplit,
    WindowGrowDrawer,
    WindowShrinkDrawer,
    // +this note (m)
    NoteToggleTodo,
    NotePreview,
    NoteCopyWikilink,
    NoteExport,
    NoteYankPath,
    /// Open the command palette.
    Palette,
    // help
    Help,
}

impl LeaderAction {
    /// Stable identifier for config files (`[leader]` overrides) and docs.
    /// Renaming one breaks user configs — treat as public API.
    pub fn id(&self) -> &'static str {
        match self {
            LeaderAction::OpenDrawer(DrawerView::Files) => "drawer.files",
            LeaderAction::OpenDrawer(DrawerView::Find) => "drawer.find",
            LeaderAction::OpenDrawer(DrawerView::Tags) => "drawer.tags",
            LeaderAction::OpenDrawer(DrawerView::Links) => "drawer.links",
            LeaderAction::OpenDrawer(DrawerView::Outline) => "drawer.outline",
            LeaderAction::OpenDrawer(DrawerView::Config) => "drawer.config",
            LeaderAction::FindFiles => "find.files",
            LeaderAction::FindGrep => "find.grep",
            LeaderAction::FindTags => "find.tags",
            LeaderAction::FindBacklinks => "find.backlinks",
            LeaderAction::FindRecent => "find.recent",
            LeaderAction::FindSaved => "find.saved",
            LeaderAction::FindHeadings => "find.headings",
            LeaderAction::NoteNew => "note.new",
            LeaderAction::NoteDaily => "note.daily",
            LeaderAction::NoteFromTemplate => "note.template",
            LeaderAction::NoteRename => "note.rename",
            LeaderAction::NoteMove => "note.move",
            LeaderAction::NoteDelete => "note.delete",
            LeaderAction::LinksTab(LinksTab::Backlinks) => "links.backlinks",
            LeaderAction::LinksTab(LinksTab::Outgoing) => "links.outgoing",
            LeaderAction::LinksTab(LinksTab::Unlinked) => "links.unlinked",
            LeaderAction::LinksGraph => "links.graph",
            LeaderAction::GitStatus => "git.status",
            LeaderAction::GitSync => "git.sync",
            LeaderAction::GitLog => "git.log",
            LeaderAction::GitDiff => "git.diff",
            LeaderAction::VaultSwitch => "vault.switch",
            LeaderAction::VaultReindex => "vault.reindex",
            LeaderAction::VaultConfig => "vault.config",
            LeaderAction::VaultTheme => "vault.theme",
            LeaderAction::VaultPreferences => "vault.settings",
            LeaderAction::WindowZen => "window.zen",
            LeaderAction::WindowSplit => "window.split",
            LeaderAction::WindowGrowDrawer => "window.grow",
            LeaderAction::WindowShrinkDrawer => "window.shrink",
            LeaderAction::NoteToggleTodo => "this.todo",
            LeaderAction::NotePreview => "this.preview",
            LeaderAction::NoteCopyWikilink => "this.copy-link",
            LeaderAction::NoteExport => "this.export",
            LeaderAction::NoteYankPath => "this.yank-path",
            LeaderAction::Palette => "palette",
            LeaderAction::Help => "help",
        }
    }

    /// Every action, for id lookup and docs.
    pub const ALL: [LeaderAction; 42] = [
        LeaderAction::OpenDrawer(DrawerView::Files),
        LeaderAction::OpenDrawer(DrawerView::Find),
        LeaderAction::OpenDrawer(DrawerView::Tags),
        LeaderAction::OpenDrawer(DrawerView::Links),
        LeaderAction::OpenDrawer(DrawerView::Outline),
        LeaderAction::OpenDrawer(DrawerView::Config),
        LeaderAction::FindFiles,
        LeaderAction::FindGrep,
        LeaderAction::FindTags,
        LeaderAction::FindBacklinks,
        LeaderAction::FindRecent,
        LeaderAction::FindSaved,
        LeaderAction::FindHeadings,
        LeaderAction::NoteNew,
        LeaderAction::NoteDaily,
        LeaderAction::NoteFromTemplate,
        LeaderAction::NoteRename,
        LeaderAction::NoteMove,
        LeaderAction::NoteDelete,
        LeaderAction::LinksTab(LinksTab::Backlinks),
        LeaderAction::LinksTab(LinksTab::Outgoing),
        LeaderAction::LinksTab(LinksTab::Unlinked),
        LeaderAction::LinksGraph,
        LeaderAction::GitStatus,
        LeaderAction::GitSync,
        LeaderAction::GitLog,
        LeaderAction::GitDiff,
        LeaderAction::VaultSwitch,
        LeaderAction::VaultReindex,
        LeaderAction::VaultConfig,
        LeaderAction::VaultTheme,
        LeaderAction::VaultPreferences,
        LeaderAction::WindowZen,
        LeaderAction::WindowSplit,
        LeaderAction::WindowGrowDrawer,
        LeaderAction::WindowShrinkDrawer,
        LeaderAction::NoteToggleTodo,
        LeaderAction::NotePreview,
        LeaderAction::NoteCopyWikilink,
        LeaderAction::NoteExport,
        LeaderAction::NoteYankPath,
        LeaderAction::Palette,
    ];

    /// Look an action up by its config id. `Help` is included via ALL? It is
    /// not — `help` resolves explicitly so ALL's length stays the leaf count.
    pub fn from_id(id: &str) -> Option<LeaderAction> {
        if id == "help" {
            return Some(LeaderAction::Help);
        }
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// Default display label for config-added leaves (the built-in tree
    /// carries hand-written labels; an override that adds an action somewhere
    /// new falls back to this).
    pub fn default_label(&self) -> &'static str {
        match self {
            LeaderAction::OpenDrawer(_) => "open drawer",
            LeaderAction::FindFiles => "files",
            LeaderAction::FindGrep => "grep/query",
            LeaderAction::FindTags => "tags",
            LeaderAction::FindBacklinks => "backlinks",
            LeaderAction::FindRecent => "recent",
            LeaderAction::FindSaved => "saved searches",
            LeaderAction::FindHeadings => "headings",
            LeaderAction::NoteNew => "new note",
            LeaderAction::NoteDaily => "daily",
            LeaderAction::NoteFromTemplate => "from template",
            LeaderAction::NoteRename => "rename",
            LeaderAction::NoteMove => "move",
            LeaderAction::NoteDelete => "delete",
            LeaderAction::LinksTab(_) => "links",
            LeaderAction::LinksGraph => "local graph",
            LeaderAction::GitStatus => "git status",
            LeaderAction::GitSync => "git sync",
            LeaderAction::GitLog => "git log",
            LeaderAction::GitDiff => "git diff",
            LeaderAction::VaultSwitch => "switch vault",
            LeaderAction::VaultReindex => "reindex",
            LeaderAction::VaultConfig => "config",
            LeaderAction::VaultTheme => "theme picker",
            LeaderAction::VaultPreferences => "preferences",
            LeaderAction::WindowZen => "zen",
            LeaderAction::WindowSplit => "split",
            LeaderAction::WindowGrowDrawer => "grow drawer",
            LeaderAction::WindowShrinkDrawer => "shrink drawer",
            LeaderAction::NoteToggleTodo => "toggle todo",
            LeaderAction::NotePreview => "preview",
            LeaderAction::NoteCopyWikilink => "copy wikilink",
            LeaderAction::NoteExport => "export",
            LeaderAction::NoteYankPath => "yank note path",
            LeaderAction::Palette => "command palette",
            LeaderAction::Help => "help / cheatsheet",
        }
    }
}

/// One node of the leader tree.
pub enum LeaderNode {
    Group {
        /// Owned-or-static so config can rename groups (`[leader.labels]`).
        label: std::borrow::Cow<'static, str>,
        children: Vec<(char, LeaderNode)>,
    },
    Leaf {
        label: &'static str,
        action: LeaderAction,
    },
}

impl LeaderNode {
    fn child(&self, key: char) -> Option<&LeaderNode> {
        match self {
            LeaderNode::Group { children, .. } => children
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, node)| node),
            LeaderNode::Leaf { .. } => None,
        }
    }

    /// The node's display label (group caption or leaf description).
    pub fn label(&self) -> &str {
        match self {
            LeaderNode::Group { label, .. } => label,
            LeaderNode::Leaf { label, .. } => label,
        }
    }

    /// Children of a group node, for the which-key overlay. Empty for leaves.
    pub fn children(&self) -> &[(char, LeaderNode)] {
        match self {
            LeaderNode::Group { children, .. } => children,
            LeaderNode::Leaf { .. } => &[],
        }
    }
}

/// The leader tree per spec §8c (gateway key deviations noted in the module
/// docs). Group letters: f n l o g v w m, plus `?` for help.
pub fn leader_tree() -> LeaderNode {
    use DrawerView as DV;
    use LeaderAction as A;
    use LeaderNode::{Group, Leaf};

    fn leaf(label: &'static str, action: LeaderAction) -> LeaderNode {
        Leaf { label, action }
    }

    Group {
        label: "leader — pick a group".into(),
        children: vec![
            (
                'f',
                Group {
                    label: "+find".into(),
                    children: vec![
                        ('f', leaf("files", A::FindFiles)),
                        ('g', leaf("grep/query", A::FindGrep)),
                        ('t', leaf("tags", A::FindTags)),
                        ('b', leaf("backlinks", A::FindBacklinks)),
                        ('r', leaf("recent", A::FindRecent)),
                        ('s', leaf("saved searches", A::FindSaved)),
                        ('h', leaf("headings", A::FindHeadings)),
                    ],
                },
            ),
            (
                'n',
                Group {
                    label: "+note".into(),
                    children: vec![
                        ('n', leaf("new", A::NoteNew)),
                        ('d', leaf("daily", A::NoteDaily)),
                        ('t', leaf("from template", A::NoteFromTemplate)),
                        ('r', leaf("rename", A::NoteRename)),
                        ('m', leaf("move", A::NoteMove)),
                        ('D', leaf("delete", A::NoteDelete)),
                    ],
                },
            ),
            (
                'l',
                Group {
                    label: "+links".into(),
                    children: vec![
                        ('b', leaf("backlinks", A::LinksTab(LinksTab::Backlinks))),
                        ('o', leaf("outgoing", A::LinksTab(LinksTab::Outgoing))),
                        ('u', leaf("unlinked", A::LinksTab(LinksTab::Unlinked))),
                        ('g', leaf("local graph", A::LinksGraph)),
                    ],
                },
            ),
            (
                'o',
                Group {
                    label: "+open drawer".into(),
                    children: vec![
                        ('f', leaf("files", A::OpenDrawer(DV::Files))),
                        ('q', leaf("find", A::OpenDrawer(DV::Find))),
                        ('t', leaf("tags", A::OpenDrawer(DV::Tags))),
                        ('k', leaf("links", A::OpenDrawer(DV::Links))),
                        ('l', leaf("outline", A::OpenDrawer(DV::Outline))),
                    ],
                },
            ),
            (
                'g',
                Group {
                    label: "+git/sync".into(),
                    children: vec![
                        ('s', leaf("status", A::GitStatus)),
                        ('p', leaf("sync/push", A::GitSync)),
                        ('l', leaf("log", A::GitLog)),
                        ('d', leaf("diff", A::GitDiff)),
                    ],
                },
            ),
            (
                'v',
                Group {
                    label: "+vault".into(),
                    children: vec![
                        ('s', leaf("switch vault", A::VaultSwitch)),
                        ('r', leaf("reindex", A::VaultReindex)),
                        ('c', leaf("config", A::VaultConfig)),
                        ('t', leaf("theme picker", A::VaultTheme)),
                        ('p', leaf("preferences", A::VaultPreferences)),
                    ],
                },
            ),
            (
                'w',
                Group {
                    label: "+window".into(),
                    children: vec![
                        ('z', leaf("zen", A::WindowZen)),
                        ('v', leaf("split (soon)", A::WindowSplit)),
                        ('l', leaf("grow drawer", A::WindowGrowDrawer)),
                        ('h', leaf("shrink drawer", A::WindowShrinkDrawer)),
                    ],
                },
            ),
            (
                'm',
                Group {
                    label: "+this note".into(),
                    children: vec![
                        ('t', leaf("toggle todo", A::NoteToggleTodo)),
                        ('p', leaf("preview", A::NotePreview)),
                        ('c', leaf("copy wikilink", A::NoteCopyWikilink)),
                        ('e', leaf("export (soon)", A::NoteExport)),
                        // Same dialog as `n r` — every rename rewrites
                        // backlinks (core LinkRewrite), so the labels match.
                        ('r', leaf("rename", A::NoteRename)),
                        ('y', leaf("yank note path", A::NoteYankPath)),
                    ],
                },
            ),
            ('p', leaf("command palette", A::Palette)),
            ('?', leaf("help / cheatsheet", A::Help)),
        ],
    }
}

/// Apply config overrides onto the default tree: each entry maps a key
/// sequence (space-separated keys after the gateway, e.g. `"o f"` or `"x"`)
/// to an action id — or `"none"` to remove the binding. Unknown ids and
/// empty sequences are skipped with a warning; intermediate groups are
/// created on demand (labelled `+<key>`).
pub fn apply_overrides<'a, I>(mut tree: LeaderNode, overrides: I) -> LeaderNode
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    for (seq, action_id) in overrides {
        let keys: Vec<char> = seq
            .split_whitespace()
            .filter_map(|t| {
                let mut chars = t.chars();
                let c = chars.next()?;
                chars.next().is_none().then_some(c)
            })
            .collect();
        if keys.is_empty() || keys.len() != seq.split_whitespace().count() {
            tracing::warn!("[leader] ignoring invalid sequence {seq:?} (single-char keys only)");
            continue;
        }
        if action_id.eq_ignore_ascii_case("none") {
            remove_at(&mut tree, &keys);
            continue;
        }
        let Some(action) = LeaderAction::from_id(action_id) else {
            tracing::warn!("[leader] ignoring unknown action id {action_id:?} for {seq:?}");
            continue;
        };
        insert_at(&mut tree, &keys, action);
    }
    tree
}

/// Caption for on-demand groups created by overrides; `[leader.labels]`
/// renames them (and any built-in group).
fn synth_group_label(key: char) -> std::borrow::Cow<'static, str> {
    std::borrow::Cow::Owned(format!("+{key}"))
}

/// Apply `[leader.labels]` overrides: each entry maps the key sequence of a
/// GROUP (e.g. `"f"`, or `"y z"` for a nested one) to its caption. Unknown
/// sequences and leaves are skipped with a warning.
pub fn apply_labels<'a, I>(mut tree: LeaderNode, labels: I) -> LeaderNode
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    for (seq, label) in labels {
        let keys: Vec<char> = seq
            .split_whitespace()
            .filter_map(|t| {
                let mut chars = t.chars();
                let c = chars.next()?;
                chars.next().is_none().then_some(c)
            })
            .collect();
        if keys.is_empty() || keys.len() != seq.split_whitespace().count() {
            tracing::warn!("[leader.labels] ignoring invalid sequence {seq:?}");
            continue;
        }
        let mut node = Some(&mut tree);
        for key in &keys {
            node = node.and_then(|n| match n {
                LeaderNode::Group { children, .. } => children
                    .iter_mut()
                    .find(|(k, _)| k == key)
                    .map(|(_, child)| child),
                LeaderNode::Leaf { .. } => None,
            });
        }
        match node {
            Some(LeaderNode::Group { label: slot, .. }) => {
                *slot = std::borrow::Cow::Owned(label.to_string());
            }
            _ => tracing::warn!("[leader.labels] {seq:?} is not a group; ignored"),
        }
    }
    tree
}

fn insert_at(node: &mut LeaderNode, keys: &[char], action: LeaderAction) {
    let LeaderNode::Group { children, .. } = node else {
        return; // a leaf can't be descended into; overrides target groups
    };
    let (head, rest) = (keys[0], &keys[1..]);
    if rest.is_empty() {
        let leaf = LeaderNode::Leaf {
            label: action.default_label(),
            action,
        };
        if let Some((_, child)) = children.iter_mut().find(|(k, _)| *k == head) {
            if matches!(child, LeaderNode::Group { .. }) {
                // Loud: a one-key override replacing a whole group is more
                // often a typo (`"f"` for `"f f"`) than an intent.
                tracing::warn!(
                    "[leader.bind] key {head:?} replaces an entire group with \
                     a single action — its sub-bindings are gone"
                );
            }
            *child = leaf;
        } else {
            children.push((head, leaf));
        }
        return;
    }
    // Descend, creating (or replacing a leaf with) a group as needed.
    let needs_group = !matches!(
        children.iter().find(|(k, _)| *k == head),
        Some((_, LeaderNode::Group { .. }))
    );
    if needs_group {
        let group = LeaderNode::Group {
            label: synth_group_label(head),
            children: Vec::new(),
        };
        if let Some((_, child)) = children.iter_mut().find(|(k, _)| *k == head) {
            *child = group;
        } else {
            children.push((head, group));
        }
    }
    let (_, child) = children
        .iter_mut()
        .find(|(k, _)| *k == head)
        .expect("just ensured");
    insert_at(child, rest, action);
}

fn remove_at(node: &mut LeaderNode, keys: &[char]) {
    let LeaderNode::Group { children, .. } = node else {
        return;
    };
    let (head, rest) = (keys[0], &keys[1..]);
    if rest.is_empty() {
        children.retain(|(k, _)| *k != head);
        return;
    }
    if let Some((_, child)) = children.iter_mut().find(|(k, _)| *k == head) {
        remove_at(child, rest);
        // Drop a group emptied by the removal.
        if matches!(child, LeaderNode::Group { children, .. } if children.is_empty()) {
            children.retain(|(k, _)| *k != head);
        }
    }
}

/// What feeding a key into a pending sequence produced.
#[derive(Debug, PartialEq, Eq)]
pub enum LeaderOutcome {
    /// Stepped into a group; sequence still pending.
    Descended,
    /// A leaf fired.
    Fired(LeaderAction),
    /// The key matched nothing; sequence stays where it was (gentle no-op).
    Invalid,
    /// Sequence cancelled (Esc).
    Cancelled,
    /// Stepped up one level (Backspace); pending unless at the root… in
    /// which case it cancels.
    SteppedUp,
}

/// The pending-sequence state machine. `start()` arms it; `feed()` walks the
/// tree. Not pending = idle, all keys flow normally.
pub struct LeaderEngine {
    tree: LeaderNode,
    /// Keys pressed since the gateway, in order. Empty = at the root.
    path: Vec<char>,
    /// When the sequence was last advanced — the which-key overlay reveals
    /// itself when `now - since > timeout` (phase 06).
    since: Option<Instant>,
}

impl LeaderEngine {
    pub fn new() -> Self {
        Self::with_tree(leader_tree())
    }

    /// Build the engine over a configured tree (defaults + `[leader]`
    /// overrides) — the same tree the which-key overlay, the cheatsheet,
    /// and the command palette must read.
    pub fn with_tree(tree: LeaderNode) -> Self {
        Self {
            tree,
            path: Vec::new(),
            since: None,
        }
    }

    /// The tree the engine walks — single source for every surface that
    /// documents it.
    pub fn tree(&self) -> &LeaderNode {
        &self.tree
    }

    pub fn is_pending(&self) -> bool {
        self.since.is_some()
    }

    /// The keys pressed since the gateway (for the which-key header).
    pub fn path(&self) -> &[char] {
        &self.path
    }

    /// When the pending sequence last advanced, for hesitation detection.
    pub fn pending_since(&self) -> Option<Instant> {
        self.since
    }

    /// The node the sequence currently sits on (root when just started).
    pub fn current_node(&self) -> &LeaderNode {
        let mut node = &self.tree;
        for key in &self.path {
            match node.child(*key) {
                Some(next) => node = next,
                None => break,
            }
        }
        node
    }

    /// Arm the engine: the gateway was pressed.
    pub fn start(&mut self) {
        self.path.clear();
        self.since = Some(Instant::now());
    }

    /// Disarm without firing.
    pub fn cancel(&mut self) {
        self.path.clear();
        self.since = None;
    }

    /// Feed a printable key into the pending sequence.
    pub fn feed(&mut self, key: char) -> LeaderOutcome {
        debug_assert!(self.is_pending());
        match self.current_node().child(key) {
            Some(LeaderNode::Leaf { action, .. }) => {
                let action = *action;
                self.cancel();
                LeaderOutcome::Fired(action)
            }
            Some(LeaderNode::Group { .. }) => {
                self.path.push(key);
                self.since = Some(Instant::now());
                LeaderOutcome::Descended
            }
            None => {
                // The user is clearly hesitating — restart the reveal timer
                // so the which-key overlay (phase 06) can help.
                self.since = Some(Instant::now());
                LeaderOutcome::Invalid
            }
        }
    }

    /// Step up one level (Backspace). At the root this cancels.
    pub fn step_up(&mut self) -> LeaderOutcome {
        if self.path.pop().is_some() {
            self.since = Some(Instant::now());
            LeaderOutcome::SteppedUp
        } else {
            self.cancel();
            LeaderOutcome::Cancelled
        }
    }
}

impl Default for LeaderEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_sequence_fires_leaf() {
        let mut e = LeaderEngine::new();
        e.start();
        assert_eq!(e.feed('o'), LeaderOutcome::Descended);
        assert_eq!(
            e.feed('f'),
            LeaderOutcome::Fired(LeaderAction::OpenDrawer(DrawerView::Files))
        );
        assert!(!e.is_pending());
    }

    #[test]
    fn invalid_key_keeps_sequence_pending() {
        let mut e = LeaderEngine::new();
        e.start();
        assert_eq!(e.feed('x'), LeaderOutcome::Invalid);
        assert!(e.is_pending());
        assert_eq!(e.feed('o'), LeaderOutcome::Descended);
    }

    #[test]
    fn backspace_steps_up_then_cancels() {
        let mut e = LeaderEngine::new();
        e.start();
        e.feed('f');
        assert_eq!(e.step_up(), LeaderOutcome::SteppedUp);
        assert!(e.is_pending());
        assert_eq!(e.step_up(), LeaderOutcome::Cancelled);
        assert!(!e.is_pending());
    }

    #[test]
    fn cancel_disarms() {
        let mut e = LeaderEngine::new();
        e.start();
        e.feed('n');
        e.cancel();
        assert!(!e.is_pending());
        assert!(e.path().is_empty());
    }

    #[test]
    fn tree_matches_spec_groups() {
        let tree = leader_tree();
        let groups: Vec<char> = tree.children().iter().map(|(k, _)| *k).collect();
        assert_eq!(
            groups,
            vec!['f', 'n', 'l', 'o', 'g', 'v', 'w', 'm', 'p', '?']
        );
        // Doubled letters fire the group's most-common action.
        let mut e = LeaderEngine::new();
        e.start();
        e.feed('f');
        assert_eq!(e.feed('f'), LeaderOutcome::Fired(LeaderAction::FindFiles));
        e.start();
        e.feed('n');
        assert_eq!(e.feed('n'), LeaderOutcome::Fired(LeaderAction::NoteNew));
    }

    #[test]
    fn overrides_remap_add_and_remove() {
        let tree = apply_overrides(
            leader_tree(),
            [
                ("o f", "find.files"),    // remap an existing leaf
                ("x", "note.daily"),      // add a new top-level leaf
                ("y z", "vault.theme"),   // add under a new on-demand group
                ("g p", "none"),          // remove a leaf
                ("bad seq!", "note.new"), // invalid (multi-char key) → skipped
                ("q", "no.such.action"),  // unknown id → skipped
            ],
        );
        let mut e = LeaderEngine::with_tree(tree);

        e.start();
        e.feed('o');
        assert_eq!(e.feed('f'), LeaderOutcome::Fired(LeaderAction::FindFiles));

        e.start();
        assert_eq!(e.feed('x'), LeaderOutcome::Fired(LeaderAction::NoteDaily));

        e.start();
        assert_eq!(e.feed('y'), LeaderOutcome::Descended);
        assert_eq!(e.feed('z'), LeaderOutcome::Fired(LeaderAction::VaultTheme));

        e.start();
        e.feed('g');
        assert_eq!(e.feed('p'), LeaderOutcome::Invalid); // removed

        e.start();
        assert_eq!(e.feed('q'), LeaderOutcome::Invalid); // unknown id skipped
    }

    #[test]
    fn labels_rename_groups_including_synth_ones() {
        let tree = apply_overrides(leader_tree(), [("y z", "vault.theme")]);
        let tree = apply_labels(
            tree,
            [
                ("f", "+search"), // rename a built-in group
                ("y", "+mine"),   // rename an override-created group
                ("n n", "+nope"), // a leaf → warned + ignored
                ("zz", "+bad"),   // invalid sequence → ignored
            ],
        );
        let find = tree.children().iter().find(|(k, _)| *k == 'f').unwrap();
        assert_eq!(find.1.label(), "+search");
        let mine = tree.children().iter().find(|(k, _)| *k == 'y').unwrap();
        assert_eq!(mine.1.label(), "+mine");
        // Leaf labels untouched.
        let note = tree.children().iter().find(|(k, _)| *k == 'n').unwrap();
        let nn = note.1.children().iter().find(|(k, _)| *k == 'n').unwrap();
        assert_eq!(nn.1.label(), "new");
    }

    /// Every action reachable through the default tree must resolve through
    /// `from_id` — catches a new leaf variant missing its `ALL` entry, which
    /// would silently break `[leader.bind]` overrides for it.
    #[test]
    fn every_tree_leaf_is_id_addressable() {
        fn walk(node: &LeaderNode, out: &mut Vec<LeaderAction>) {
            for (_, child) in node.children() {
                match child {
                    LeaderNode::Leaf { action, .. } => out.push(*action),
                    LeaderNode::Group { .. } => walk(child, out),
                }
            }
        }
        let mut leaves = Vec::new();
        walk(&leader_tree(), &mut leaves);
        for action in leaves {
            assert_eq!(
                LeaderAction::from_id(action.id()),
                Some(action),
                "{action:?} (id {:?}) missing from LeaderAction::ALL",
                action.id()
            );
        }
    }

    #[test]
    fn action_ids_round_trip() {
        for action in LeaderAction::ALL {
            assert_eq!(
                LeaderAction::from_id(action.id()),
                Some(action),
                "id round-trip failed for {action:?}"
            );
        }
        assert_eq!(LeaderAction::from_id("help"), Some(LeaderAction::Help));
        assert_eq!(LeaderAction::from_id("nope"), None);
    }

    #[test]
    fn capital_letters_are_distinct_keys() {
        let mut e = LeaderEngine::new();
        e.start();
        e.feed('n');
        assert_eq!(e.feed('d'), LeaderOutcome::Fired(LeaderAction::NoteDaily));
        e.start();
        e.feed('n');
        assert_eq!(e.feed('D'), LeaderOutcome::Fired(LeaderAction::NoteDelete));
    }
}
