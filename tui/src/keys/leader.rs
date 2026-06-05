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
    // help
    Help,
}

/// One node of the leader tree.
pub enum LeaderNode {
    Group {
        label: &'static str,
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
    pub fn label(&self) -> &'static str {
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
        label: "leader — pick a group",
        children: vec![
            (
                'f',
                Group {
                    label: "+find",
                    children: vec![
                        ('f', leaf("files", A::FindFiles)),
                        ('g', leaf("grep/query", A::FindGrep)),
                        ('t', leaf("tags", A::FindTags)),
                        ('b', leaf("backlinks", A::FindBacklinks)),
                        ('r', leaf("recent", A::FindRecent)),
                        ('h', leaf("headings", A::FindHeadings)),
                    ],
                },
            ),
            (
                'n',
                Group {
                    label: "+note",
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
                    label: "+links",
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
                    label: "+open drawer",
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
                    label: "+git/sync",
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
                    label: "+vault",
                    children: vec![
                        ('s', leaf("switch vault", A::VaultSwitch)),
                        ('r', leaf("reindex", A::VaultReindex)),
                        ('c', leaf("config", A::VaultConfig)),
                    ],
                },
            ),
            (
                'w',
                Group {
                    label: "+window",
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
                    label: "+this note",
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
            ('?', leaf("help / cheatsheet", A::Help)),
        ],
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
        Self {
            tree: leader_tree(),
            path: Vec::new(),
            since: None,
        }
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
        assert_eq!(groups, vec!['f', 'n', 'l', 'o', 'g', 'v', 'w', 'm', '?']);
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
