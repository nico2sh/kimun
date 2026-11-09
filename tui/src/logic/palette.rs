//! The **command palette**'s rows: every leader leaf flattened into a
//! fuzzy-searchable list, labelled with its group and its full key sequence.
//! The palette is a labelled door onto the same leaves the leader fires
//! (`logic::leader_route`), never a second implementation. Each frontend
//! draws the rows and runs the chosen row's route.

use crate::keys::leader::{LeaderAction, LeaderNode};
use crate::logic::list::{ListRow, fuzzy_rank};

/// One palette row: a leader leaf with its full key sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteRow {
    /// `group label · leaf label`, e.g. `+find · files`.
    pub label: String,
    /// The key sequence, e.g. `Ctrl+G f f`.
    pub keys: String,
    /// `label + keys`, so the fuzzy filter matches either.
    haystack: String,
    pub action: LeaderAction,
}

impl ListRow for PaletteRow {
    fn match_text(&self) -> Option<&str> {
        Some(&self.haystack)
    }
}

/// Flatten a leader tree into palette rows — the single keymap source.
/// `gateway` labels the leader gateway at the head of every key sequence.
pub fn palette_rows(tree: &LeaderNode, gateway: &str) -> Vec<PaletteRow> {
    fn walk(node: &LeaderNode, group: &str, keys: &str, out: &mut Vec<PaletteRow>) {
        for (key, child) in node.children() {
            let child_keys = format!("{keys} {key}");
            match child {
                // The palette never lists itself — selecting it would just
                // close and reopen the palette.
                LeaderNode::Leaf { action, .. } if *action == LeaderAction::Palette => {}
                // The nine jump leaves stay out of the palette: they're a
                // single fuzzy-searchable concept ("pinned notes"), not nine
                // near-identical rows differing only by digit.
                LeaderNode::Leaf {
                    action: LeaderAction::PinnedJump(_),
                    ..
                } => {}
                LeaderNode::Leaf { label, action } => {
                    let label = if group.is_empty() {
                        (*label).to_string()
                    } else {
                        format!("{group} · {label}")
                    };
                    out.push(PaletteRow {
                        haystack: format!("{label} {child_keys}"),
                        label,
                        keys: child_keys,
                        action: *action,
                    });
                }
                LeaderNode::Group { label, .. } => walk(child, label, &child_keys, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, "", gateway, &mut out);
    out
}

/// The rows matching `query`, as indices into `rows`, best first — the TUI
/// palette's fuzzy filter (a blank query keeps every row, in tree order).
pub fn filter_rows(rows: &[PaletteRow], query: &str) -> Vec<usize> {
    fuzzy_rank(rows, query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::leader::leader_tree;

    #[test]
    fn rows_cover_every_leader_leaf() {
        let rows = palette_rows(&leader_tree(), "Ctrl+G");
        // Spot-check shape and coverage.
        assert!(rows.len() > 20);
        assert!(
            rows.iter()
                .any(|e| e.keys == "Ctrl+G o f" && e.label.contains("files"))
        );
        assert!(
            rows.iter()
                .any(|e| e.action == LeaderAction::Help && e.keys == "Ctrl+G ?")
        );
        assert!(
            !rows
                .iter()
                .any(|e| matches!(e.action, LeaderAction::PinnedJump(_))),
            "the nine jump leaves stay out of the palette"
        );
        assert!(!rows.iter().any(|e| e.action == LeaderAction::Palette));
        assert!(rows.iter().any(|e| e.action == LeaderAction::FindPinned));
        assert!(rows.iter().any(|e| e.action == LeaderAction::NoteTogglePin));
    }

    #[test]
    fn a_blank_query_keeps_every_row_in_order() {
        let rows = palette_rows(&leader_tree(), "Ctrl+B");
        let all: Vec<usize> = (0..rows.len()).collect();
        assert_eq!(filter_rows(&rows, ""), all);
        assert_eq!(filter_rows(&rows, "   "), all);
    }

    /// Fuzzy, not substring: `wrt` finds "write" through its gaps, and the
    /// key sequence is searchable too.
    #[test]
    fn the_filter_is_fuzzy_over_label_and_keys() {
        let rows = palette_rows(&leader_tree(), "Ctrl+B");
        let hits = filter_rows(&rows, "nsav");
        assert!(!hits.is_empty());
        assert!(
            hits.iter()
                .any(|&i| rows[i].action == LeaderAction::NoteSave),
            "{:?}",
            hits.iter().map(|&i| &rows[i].label).collect::<Vec<_>>()
        );
        let by_keys = filter_rows(&rows, "o f");
        assert!(
            by_keys.iter().any(|&i| rows[i].action
                == LeaderAction::OpenDrawer(crate::logic::nav::DrawerView::Files))
        );
        assert!(filter_rows(&rows, "zzzzqqq").is_empty());
    }
}
