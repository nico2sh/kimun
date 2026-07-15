//! Hash-diff reconciliation (adr/0019): the correctness backbone. Given the
//! vault's authoritative `{note-path → hash}` and the server's, compute exactly
//! which notes to push and which to delete so the two agree.

use std::collections::HashMap;

/// What a reconciliation pass must do to bring the server in step with the vault.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcilePlan {
    /// Notes present in the vault but missing from the server, or whose hash
    /// differs — must be (re)pushed.
    pub to_push: Vec<String>,
    /// Notes the server holds that no longer exist in the vault — must be deleted.
    pub to_delete: Vec<String>,
}

impl ReconcilePlan {
    pub fn is_empty(&self) -> bool {
        self.to_push.is_empty() && self.to_delete.is_empty()
    }
}

/// Diffs the vault's authoritative hash set against the server's.
pub fn diff(local: &HashMap<String, String>, server: &HashMap<String, String>) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();

    for (path, hash) in local {
        match server.get(path) {
            Some(server_hash) if server_hash == hash => {} // already in sync
            _ => plan.to_push.push(path.clone()),          // missing or changed
        }
    }
    for path in server.keys() {
        if !local.contains_key(path) {
            plan.to_delete.push(path.clone());
        }
    }

    plan.to_push.sort();
    plan.to_delete.sort();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(p, h)| (p.to_string(), h.to_string()))
            .collect()
    }

    #[test]
    fn identical_sets_need_nothing() {
        let plan = diff(
            &map(&[("a", "1"), ("b", "2")]),
            &map(&[("a", "1"), ("b", "2")]),
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn new_and_changed_notes_are_pushed() {
        // "a" changed (1→2), "c" is new, "b" unchanged.
        let local = map(&[("a", "2"), ("b", "2"), ("c", "9")]);
        let server = map(&[("a", "1"), ("b", "2")]);
        let plan = diff(&local, &server);
        assert_eq!(plan.to_push, vec!["a".to_string(), "c".to_string()]);
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn notes_gone_from_the_vault_are_deleted() {
        let local = map(&[("a", "1")]);
        let server = map(&[("a", "1"), ("stale", "7")]);
        let plan = diff(&local, &server);
        assert!(plan.to_push.is_empty());
        assert_eq!(plan.to_delete, vec!["stale".to_string()]);
    }

    #[test]
    fn empty_server_pushes_everything() {
        let plan = diff(&map(&[("a", "1"), ("b", "2")]), &HashMap::new());
        assert_eq!(plan.to_push, vec!["a".to_string(), "b".to_string()]);
    }
}
