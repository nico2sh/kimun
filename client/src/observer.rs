//! The [`IndexObserver`] the client registers on the vault, and the dirty-set it
//! feeds. Per adr/0019 the dirty-set is best-effort in-memory: a lost entry
//! costs a reconciliation pass, not a lost update.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kimun_core::{IndexObserver, NoteChange, nfs::VaultPath};

/// The pending change for a note. The latest event wins, so an Upsert followed
/// by a Delete on the same path collapses to Delete (and vice-versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyOp {
    Upsert,
    Delete,
}

/// Set of notes changed since the last drain, keyed by path (latest op per path).
#[derive(Debug, Default)]
pub struct DirtySet {
    inner: Mutex<HashMap<VaultPath, DirtyOp>>,
}

impl DirtySet {
    /// Records a change, overwriting any earlier pending op for the same note.
    pub fn record(&self, change: &NoteChange) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match change {
            NoteChange::Upsert { path, .. } => map.insert(path.clone(), DirtyOp::Upsert),
            NoteChange::Delete { path } => map.insert(path.clone(), DirtyOp::Delete),
        };
    }

    /// Takes and clears all pending ops for flushing.
    pub fn drain(&self) -> Vec<(VaultPath, DirtyOp)> {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *map).into_iter().collect()
    }

    /// Puts back ops that failed to flush, without clobbering a newer op that
    /// was recorded for the same note while the flush was in flight.
    pub fn requeue(&self, items: impl IntoIterator<Item = (VaultPath, DirtyOp)>) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        for (path, op) in items {
            map.entry(path).or_insert(op);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Registered on the vault; folds every note change into the shared [`DirtySet`].
#[derive(Debug)]
pub struct RagObserver {
    dirty: Arc<DirtySet>,
}

impl RagObserver {
    pub fn new(dirty: Arc<DirtySet>) -> Self {
        Self { dirty }
    }
}

impl IndexObserver for RagObserver {
    fn on_change(&self, change: &NoteChange) {
        self.dirty.record(change);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upsert(p: &str) -> NoteChange {
        NoteChange::Upsert {
            path: VaultPath::new(p),
            hash: 1,
        }
    }
    fn delete(p: &str) -> NoteChange {
        NoteChange::Delete {
            path: VaultPath::new(p),
        }
    }

    #[test]
    fn latest_op_wins_per_path() {
        let set = DirtySet::default();
        set.record(&upsert("a.md"));
        set.record(&delete("a.md")); // delete supersedes the upsert
        let drained = set.drain();
        assert_eq!(drained, vec![(VaultPath::new("a.md"), DirtyOp::Delete)]);

        set.record(&delete("b.md"));
        set.record(&upsert("b.md")); // recreate supersedes the delete
        assert_eq!(set.drain(), vec![(VaultPath::new("b.md"), DirtyOp::Upsert)]);
    }

    #[test]
    fn drain_clears() {
        let set = DirtySet::default();
        set.record(&upsert("a.md"));
        assert_eq!(set.len(), 1);
        let _ = set.drain();
        assert!(set.is_empty());
    }

    #[test]
    fn requeue_does_not_clobber_newer_op() {
        let set = DirtySet::default();
        // Flush of "a.md" as Upsert failed; meanwhile a Delete arrived.
        set.record(&delete("a.md"));
        set.requeue([(VaultPath::new("a.md"), DirtyOp::Upsert)]);
        // The newer Delete must survive.
        assert_eq!(set.drain(), vec![(VaultPath::new("a.md"), DirtyOp::Delete)]);
    }

    #[test]
    fn observer_records_into_shared_set() {
        let dirty = Arc::new(DirtySet::default());
        let observer = RagObserver::new(dirty.clone());
        observer.on_change(&upsert("n.md"));
        assert_eq!(dirty.len(), 1);
    }
}
