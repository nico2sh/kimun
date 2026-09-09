//! NoteLocks — per-note in-process write locks.
//!
//! Concurrent content mutations to the same note (e.g. parallel MCP tool
//! calls) serialize on these so a read-modify-write like `replace` can't lose
//! an update. Cross-process writers are not covered — a local single-user
//! vault rarely sees that, and backups make any clobbered version recoverable.
//!
//! One instance per vault, shared across clones. Grows with the number of
//! distinct notes mutated this process; entries are tiny.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::OwnedMutexGuard;

use crate::nfs::VaultPath;

/// The lock table. Cheap to clone: clones share the same table.
#[derive(Clone, Default, Debug)]
pub(crate) struct NoteLocks {
    map: Arc<std::sync::Mutex<HashMap<VaultPath, Arc<tokio::sync::Mutex<()>>>>>,
}

impl NoteLocks {
    /// Acquires the per-note write lock for `path`, serializing content
    /// mutations to it within this process.
    pub(crate) async fn lock_note(&self, path: &VaultPath) -> OwnedMutexGuard<()> {
        let key = path.flatten();
        let lock = {
            let mut map = self.map.lock().unwrap();
            map.entry(key)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    /// Acquires the per-note locks for several notes at once, in a stable
    /// (sorted, deduped) order so concurrent multi-note operations (e.g. two
    /// renames with overlapping victims) can't deadlock. Hold the returned
    /// guards for the duration of the operation.
    pub(crate) async fn lock_notes<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a VaultPath>,
    ) -> Vec<OwnedMutexGuard<()>> {
        let mut keys: Vec<VaultPath> = paths.into_iter().map(|p| p.flatten()).collect();
        keys.sort();
        keys.dedup();
        let mut guards = Vec::with_capacity(keys.len());
        for key in &keys {
            guards.push(self.lock_note(key).await);
        }
        guards
    }
}

#[cfg(test)]
mod tests {
    use super::NoteLocks;
    use crate::nfs::VaultPath;

    #[tokio::test]
    async fn lock_notes_dedups_and_sorts_so_one_guard_per_distinct_note() {
        let locks = NoteLocks::default();
        let a = VaultPath::new("/a.md");
        let a_again = VaultPath::new("/x/../a.md");
        let b = VaultPath::new("/b.md");
        let guards = locks.lock_notes([&b, &a, &a_again]).await;
        assert_eq!(guards.len(), 2);
    }

    #[tokio::test]
    async fn lock_note_serializes_a_second_locker_until_the_guard_drops() {
        let locks = NoteLocks::default();
        let path = VaultPath::new("/n.md");
        let guard = locks.lock_note(&path).await;
        let second = locks.lock_note(&path);
        tokio::pin!(second);
        assert!(
            futures_util::poll!(second.as_mut()).is_pending(),
            "second locker must wait while the first guard is held"
        );
        drop(guard);
        let _second_guard = second.await;
    }

    #[tokio::test]
    async fn clones_share_the_same_locks() {
        let locks = NoteLocks::default();
        let other = locks.clone();
        let path = VaultPath::new("/n.md");
        let _guard = locks.lock_note(&path).await;
        let second = other.lock_note(&path);
        tokio::pin!(second);
        assert!(futures_util::poll!(second.as_mut()).is_pending());
    }
}
