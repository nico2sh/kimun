//! The **keyboard reference**'s rows (F1 in the desktop app): every action
//! with the chords a frontend binds it to, grouped under
//! [`ActionShortcuts::group`]'s headings, then every leader sequence read off
//! the configured leader tree. Toolkit-free: the frontend hands in the labels
//! and the chords as it writes them (its effective keymap), and draws the
//! rows; the grouping, the leader walk and the filter's order live here.
//!
//! **Filter.** Fuzzy over each row's label and keys (the shared list engine's
//! [`fuzzy_rank`]), the matches then kept under their headings: a heading's
//! rows best first, the headings in [`HelpGroup`] order.

use crate::keys::leader::{LeaderAction, LeaderNode};
use crate::logic::actions::{ActionGroup, ActionShortcuts};
use crate::logic::list::{ListRow, fuzzy_rank};

/// What a row's keys say when its action has no chord.
pub const NOT_BOUND: &str = "not bound";

/// A heading of the reference, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HelpGroup {
    WindowAndPanels,
    Editor,
    FilesAndNotes,
    SearchAndAsk,
    /// The leader tree's sequences (keys after the gateway).
    LeaderTree,
    Other,
    /// Actions the frontend could bind but has no chord for now (a
    /// `[gui.keys]` override unbound them).
    Unbound,
}

impl HelpGroup {
    /// Every heading, in display order.
    pub const ALL: [HelpGroup; 7] = [
        HelpGroup::WindowAndPanels,
        HelpGroup::Editor,
        HelpGroup::FilesAndNotes,
        HelpGroup::SearchAndAsk,
        HelpGroup::LeaderTree,
        HelpGroup::Other,
        HelpGroup::Unbound,
    ];

    /// The heading's text.
    pub fn title(self) -> &'static str {
        match self {
            HelpGroup::WindowAndPanels => "Window & panels",
            HelpGroup::Editor => "Editor",
            HelpGroup::FilesAndNotes => "Files & notes",
            HelpGroup::SearchAndAsk => "Search & Ask",
            HelpGroup::LeaderTree => "Leader tree",
            HelpGroup::Other => "Other",
            HelpGroup::Unbound => "Unbound",
        }
    }
}

impl From<ActionGroup> for HelpGroup {
    fn from(group: ActionGroup) -> Self {
        match group {
            ActionGroup::WindowAndPanels => HelpGroup::WindowAndPanels,
            ActionGroup::Editor => HelpGroup::Editor,
            ActionGroup::FilesAndNotes => HelpGroup::FilesAndNotes,
            ActionGroup::SearchAndAsk => HelpGroup::SearchAndAsk,
            ActionGroup::Other => HelpGroup::Other,
        }
    }
}

/// What a row describes.
#[derive(Debug, Clone, PartialEq)]
pub enum HelpTarget {
    /// A keymap action.
    Action(ActionShortcuts),
    /// A leader leaf.
    Leaf(LeaderAction),
}

/// One row of the reference: `label — keys`, under its heading.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpRow {
    pub group: HelpGroup,
    pub label: String,
    /// The chords joined with ` / `, a leader sequence (`v p`), or
    /// [`NOT_BOUND`].
    pub keys: String,
    /// A short qualifier shown after the keys (`coming soon`, where vim
    /// keeps a chord).
    pub note: Option<String>,
    pub target: HelpTarget,
    /// `label keys`, so the filter matches either.
    haystack: String,
}

impl ListRow for HelpRow {
    fn match_text(&self) -> Option<&str> {
        Some(&self.haystack)
    }
}

impl HelpRow {
    fn new(group: HelpGroup, label: String, keys: String, target: HelpTarget) -> Self {
        Self {
            haystack: format!("{label} {keys}"),
            group,
            label,
            keys,
            note: None,
            target,
        }
    }

    /// `action`'s row: under its heading with `chords` (each as the
    /// frontend writes it), or under [`HelpGroup::Unbound`] saying
    /// [`NOT_BOUND`] when there are none.
    pub fn action(action: ActionShortcuts, label: impl Into<String>, chords: &[String]) -> Self {
        let (group, keys) = if chords.is_empty() {
            (HelpGroup::Unbound, NOT_BOUND.to_string())
        } else {
            (action.group().into(), chords.join(" / "))
        };
        Self::new(group, label.into(), keys, HelpTarget::Action(action))
    }

    /// The same row with `note` after its keys.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// Every leaf of `tree` as a [`HelpGroup::LeaderTree`] row, in tree order:
/// the keys after the gateway (`v p`; the gateway is the heading's, shown
/// once) and the leaf's label under its group's (`+vault · preferences`, as
/// the command palette writes it). Every leaf is listed, the palette's own
/// and the pinned-note digits included: this is the whole tree.
pub fn leader_rows(tree: &LeaderNode) -> Vec<HelpRow> {
    fn walk(node: &LeaderNode, group: &str, keys: &str, out: &mut Vec<HelpRow>) {
        for (key, child) in node.children() {
            let child_keys = if keys.is_empty() {
                key.to_string()
            } else {
                format!("{keys} {key}")
            };
            match child {
                LeaderNode::Leaf { label, action } => {
                    let label = if group.is_empty() {
                        (*label).to_string()
                    } else {
                        format!("{group} · {label}")
                    };
                    out.push(HelpRow::new(
                        HelpGroup::LeaderTree,
                        label,
                        child_keys,
                        HelpTarget::Leaf(*action),
                    ));
                }
                LeaderNode::Group { label, .. } => walk(child, label, &child_keys, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, "", "", &mut out);
    out
}

/// The rows of `rows` that `query` keeps, as indices: every row in its
/// heading for a blank query, else the fuzzy matches, each heading's best
/// first, the headings in [`HelpGroup`] order.
pub fn rank(rows: &[HelpRow], query: &str) -> Vec<usize> {
    let mut ranked = fuzzy_rank(rows, query.trim());
    // Stable: within a heading the fuzzy order (or the row order) stays.
    ranked.sort_by_key(|&i| rows[i].group);
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::leader::leader_tree;

    #[test]
    fn an_action_row_joins_its_chords_under_its_heading() {
        let row = HelpRow::action(
            ActionShortcuts::OpenPreferences,
            "Preferences",
            &["F4".to_string(), "Ctrl+,".to_string()],
        );
        assert_eq!(row.group, HelpGroup::Other);
        assert_eq!(row.keys, "F4 / Ctrl+,");
        assert_eq!(
            row.target,
            HelpTarget::Action(ActionShortcuts::OpenPreferences)
        );
        let unbound = HelpRow::action(ActionShortcuts::FindInBuffer, "Find in note", &[]);
        assert_eq!(unbound.group, HelpGroup::Unbound);
        assert_eq!(unbound.keys, NOT_BOUND);
    }

    #[test]
    fn leader_rows_list_every_leaf_without_the_gateway() {
        let rows = leader_rows(&leader_tree());
        let find = |keys: &str| rows.iter().find(|r| r.keys == keys).cloned();
        let prefs = find("v p").expect("v p is listed");
        assert_eq!(prefs.label, "+vault · preferences");
        assert_eq!(prefs.group, HelpGroup::LeaderTree);
        assert_eq!(
            prefs.target,
            HelpTarget::Leaf(LeaderAction::VaultPreferences)
        );
        assert_eq!(
            find("?").unwrap().target,
            HelpTarget::Leaf(LeaderAction::Help)
        );
        assert_eq!(
            find("p").unwrap().target,
            HelpTarget::Leaf(LeaderAction::Palette)
        );
        assert!(find("1").is_some(), "the pinned digits too");
        // Every leaf once: as many rows as leaves in the tree.
        fn leaves(node: &LeaderNode) -> usize {
            node.children()
                .iter()
                .map(|(_, c)| match c {
                    LeaderNode::Leaf { .. } => 1,
                    LeaderNode::Group { .. } => leaves(c),
                })
                .sum()
        }
        assert_eq!(rows.len(), leaves(&leader_tree()));
    }

    #[test]
    fn rank_keeps_the_headings_in_order_and_filters_fuzzily() {
        let mut rows = vec![
            HelpRow::action(ActionShortcuts::Quit, "Quit", &["Ctrl+Q".to_string()]),
            HelpRow::action(
                ActionShortcuts::ToggleSidebar,
                "Toggle left drawer",
                &["Ctrl+T".to_string()],
            ),
            HelpRow::action(ActionShortcuts::FindInBuffer, "Find in note", &[]),
        ];
        rows.extend(leader_rows(&leader_tree()));
        let all = rank(&rows, "");
        assert_eq!(all.len(), rows.len());
        let groups: Vec<HelpGroup> = all.iter().map(|&i| rows[i].group).collect();
        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted, "headings in order");
        assert_eq!(rows[all[0]].label, "Toggle left drawer");

        let quit = rank(&rows, "quit");
        assert!(quit.len() < rows.len());
        assert!(
            quit.iter()
                .all(|&i| rows[i].label.to_lowercase().contains("quit"))
        );
        // The keys match too.
        let by_keys = rank(&rows, "ctrl+t");
        assert_eq!(rows[by_keys[0]].label, "Toggle left drawer");
        assert!(rank(&rows, "zzzzqqq").is_empty());
    }

    #[test]
    fn every_heading_has_a_title() {
        let titles: Vec<&str> = HelpGroup::ALL.iter().map(|g| g.title()).collect();
        assert_eq!(
            titles,
            [
                "Window & panels",
                "Editor",
                "Files & notes",
                "Search & Ask",
                "Leader tree",
                "Other",
                "Unbound"
            ]
        );
    }
}
