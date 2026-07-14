//! Orchestration: turn observed changes and the vault's authoritative state into
//! server pushes/deletes. `register` wires the observer; `drain` flushes the
//! dirty-set (the fast path); `reconcile` is the correctness backbone
//! (adr/0019). All server I/O goes through [`RagTransport`], so this logic is
//! tested with a fake against a real vault.

use std::collections::HashMap;
use std::sync::Arc;

use kimun_core::{IndexObserver, NoteVault, error::VaultError, nfs::VaultPath};

use crate::dto::{WireDoc, WireSection};
use crate::{
    DirtyOp, DirtySet, RagClient, RagError, RagObserver, RagTransport, hash_string, reconcile_diff,
};

/// What a reachable server can do, derived from `/health` (adr/0024): search
/// needs an embedder, question-answering needs an embedder AND an LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerCapability {
    /// No embedder configured — nothing works server-side; the client must not
    /// push or reconcile (every call would 503).
    Unconfigured,
    /// Embedder, no LLM: search and sync work, question-answering does not.
    SemanticOnly,
    /// Embedder and LLM: everything works.
    Full,
}

impl ServerCapability {
    /// Derives the capability from a health probe's fields.
    pub fn from_health(health: &crate::dto::Health) -> Self {
        match (health.embedder.is_some(), health.llm_provider.is_some()) {
            (false, _) => ServerCapability::Unconfigured,
            (true, false) => ServerCapability::SemanticOnly,
            (true, true) => ServerCapability::Full,
        }
    }

    /// Whether push/reconcile/search are usable.
    pub fn search_available(self) -> bool {
        !matches!(self, ServerCapability::Unconfigured)
    }

    /// Whether question-answering is usable.
    pub fn llm_available(self) -> bool {
        matches!(self, ServerCapability::Full)
    }
}

/// Bundles a vault, its dirty-set, and the server client, and drives sync. The
/// caller (the TUI) owns the schedule: probe [`online`](RagSync::online) to gate
/// features, and call [`tick`](RagSync::tick) periodically to keep the server in
/// step.
pub struct RagSync {
    vault: Arc<NoteVault>,
    dirty: Arc<DirtySet>,
    /// The exact observer this sync registered, kept so `Drop` can deregister
    /// *only* ours (by identity) and never a newer one that replaced it.
    observer: Arc<dyn IndexObserver>,
    client: RagClient,
}

impl RagSync {
    /// Registers the observer on `vault` and returns a handle over it. Construct
    /// **one** `RagSync` per vault: the observer is zero-or-one, so a second
    /// `RagSync` for the same vault replaces the first's observer and strands its
    /// dirty-set (its `tick` still self-heals via reconcile, but its drain fast
    /// path goes silent).
    pub fn new(vault: Arc<NoteVault>, client: RagClient) -> Self {
        let dirty = Arc::new(DirtySet::default());
        let observer: Arc<dyn IndexObserver> = Arc::new(RagObserver::new(dirty.clone()));
        vault.set_index_observer(observer.clone());
        Self {
            vault,
            dirty,
            observer,
            client,
        }
    }

    /// Whether the server is reachable (drives capability gating).
    pub async fn online(&self) -> bool {
        self.client.health().await.is_ok()
    }

    /// Probe reachability and capability in one `/health` request: `None` =
    /// offline; otherwise the server's [`ServerCapability`] — `Unconfigured`
    /// (no embedder: don't sync), `SemanticOnly` (search, no Q&A), or `Full`.
    pub async fn probe(&self) -> Option<ServerCapability> {
        self.client
            .health()
            .await
            .ok()
            .map(|h| ServerCapability::from_health(&h))
    }

    /// One sync pass: flush pending changes, then reconcile to repair drift.
    pub async fn tick(&self) -> Result<(), RagError> {
        drain(&self.vault, &self.dirty, &self.client).await?;
        reconcile(&self.vault, &self.client).await
    }

    /// Flush pending changes only — the cheap fast path (touches only dirty
    /// notes). Run this often; run [`tick`](Self::tick)/[`reconcile`](Self::reconcile)
    /// occasionally as the safety net.
    pub async fn drain(&self) -> Result<(), RagError> {
        drain(&self.vault, &self.dirty, &self.client).await
    }

    /// Full hash-diff reconciliation only — an index-wide read + a full-collection
    /// hash fetch. The periodic backbone; not needed on every tick.
    pub async fn reconcile(&self) -> Result<(), RagError> {
        reconcile(&self.vault, &self.client).await
    }

    /// The underlying client, for queries (search / ask).
    pub fn client(&self) -> &RagClient {
        &self.client
    }
}

impl Drop for RagSync {
    fn drop(&mut self) {
        // Deregister *our* observer so a superseded/aborted sync doesn't leave
        // the vault feeding a dirty-set nobody drains — but only if it's still
        // ours, so we never wipe a newer sync that has replaced it.
        self.vault.clear_index_observer_if(&self.observer);
    }
}

/// Registers the RAG observer on the vault and returns the dirty-set it feeds.
pub fn register(vault: &NoteVault) -> Arc<DirtySet> {
    let dirty = Arc::new(DirtySet::default());
    vault.set_index_observer(Arc::new(RagObserver::new(dirty.clone())));
    dirty
}

/// Builds the wire document for a note: its canonical path, content hash, and
/// heading sections pulled from the index. Returns `None` when the note has no
/// indexable sections — an empty note is not RAG content, so it is never pushed
/// (this keeps both backends from perpetually re-pushing chunkless notes, since
/// only one of them records a hash for them server-side).
pub async fn build_doc(
    vault: &NoteVault,
    path: &VaultPath,
    hash: u64,
) -> Result<Option<WireDoc>, VaultError> {
    let chunks = vault.get_note_chunks(path).await?;
    let sections: Vec<WireSection> = chunks
        .into_values()
        .flatten()
        .map(|c| WireSection {
            title: c.get_breadcrumb().to_string(),
            text: c.get_text().to_string(),
        })
        .collect();
    if sections.is_empty() {
        return Ok(None);
    }
    Ok(Some(WireDoc {
        path: path.to_string(),
        hash: hash_string(hash),
        sections,
    }))
}

/// Flushes the dirty-set to the server. Failed operations are re-queued so the
/// next drain (or a reconcile) retries them.
pub async fn drain<T: RagTransport>(
    vault: &NoteVault,
    dirty: &DirtySet,
    transport: &T,
) -> Result<(), RagError> {
    let ops = dirty.drain();
    if ops.is_empty() {
        return Ok(());
    }

    let mut upserts: Vec<(VaultPath, u64)> = Vec::new();
    let mut deletes: Vec<String> = Vec::new();
    for (path, op) in ops {
        match op {
            DirtyOp::Upsert(hash) => upserts.push((path, hash)),
            DirtyOp::Delete => deletes.push(path.to_string()),
        }
    }

    // Build the docs for upserts; a note that can't be read right now is
    // re-queued rather than dropped.
    let mut docs = Vec::new();
    let mut built: Vec<(VaultPath, u64)> = Vec::new();
    for (path, hash) in upserts {
        match build_doc(vault, &path, hash).await {
            Ok(Some(doc)) => {
                docs.push(doc);
                built.push((path, hash));
            }
            // An emptied note has no chunks to index — delete it server-side so
            // its old chunks don't linger (and so /hashes stops reporting it).
            Ok(None) => deletes.push(path.to_string()),
            Err(_) => dirty.requeue([(path, DirtyOp::Upsert(hash))]),
        }
    }

    let mut first_err: Option<RagError> = None;
    if !docs.is_empty()
        && let Err(e) = transport.push_docs(docs).await
    {
        dirty.requeue(built.into_iter().map(|(p, h)| (p, DirtyOp::Upsert(h))));
        first_err = Some(e);
    }
    if !deletes.is_empty() {
        let paths_for_requeue: Vec<VaultPath> = deletes.iter().map(VaultPath::new).collect();
        if let Err(e) = transport.delete_paths(deletes).await {
            dirty.requeue(paths_for_requeue.into_iter().map(|p| (p, DirtyOp::Delete)));
            first_err = first_err.or(Some(e));
        }
    }

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Reconciles the server with the vault: diff hash sets, then push/delete only
/// the differences. Self-healing — repairs anything the drain path missed.
pub async fn reconcile<T: RagTransport>(vault: &NoteVault, transport: &T) -> Result<(), RagError> {
    let notes = vault
        .get_all_notes()
        .await
        .map_err(|e| RagError::Protocol(format!("read vault notes: {e}")))?;

    let local_hashes: HashMap<String, u64> = notes
        .into_iter()
        .map(|(entry, content)| (entry.path.to_string(), content.hash))
        .collect();
    let local_str: HashMap<String, String> = local_hashes
        .iter()
        .map(|(p, h)| (p.clone(), hash_string(*h)))
        .collect();

    let server = transport.server_hashes().await?;
    let plan = reconcile_diff(&local_str, &server);

    let mut docs = Vec::new();
    let mut to_delete = plan.to_delete;
    for path_str in &plan.to_push {
        let hash = local_hashes[path_str];
        match build_doc(vault, &VaultPath::new(path_str), hash)
            .await
            .map_err(|e| RagError::Protocol(format!("build doc {path_str}: {e}")))?
        {
            Some(doc) => docs.push(doc),
            // Empty note: it carries no chunks. If the server still has it (it
            // was emptied), delete it; if not, it's already converged (nothing
            // to push). Either way it never becomes a stale server entry.
            None => {
                if server.contains_key(path_str) {
                    to_delete.push(path_str.clone());
                }
            }
        }
    }
    if !docs.is_empty() {
        transport.push_docs(docs).await?;
    }
    if !to_delete.is_empty() {
        transport.delete_paths(to_delete).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[test]
    fn capability_from_health_fields() {
        use crate::dto::Health;
        let h = |embedder: Option<&str>, llm: Option<&str>| Health {
            status: "ok".into(),
            reranker: false,
            embedder: embedder.map(str::to_string),
            llm_provider: llm.map(str::to_string),
            auth_required: false,
        };
        assert_eq!(
            ServerCapability::from_health(&h(None, None)),
            ServerCapability::Unconfigured
        );
        assert_eq!(
            // An LLM without an embedder still can't answer — retrieval is dead.
            ServerCapability::from_health(&h(None, Some("gemini"))),
            ServerCapability::Unconfigured
        );
        assert_eq!(
            ServerCapability::from_health(&h(Some("fastembed"), None)),
            ServerCapability::SemanticOnly
        );
        assert_eq!(
            ServerCapability::from_health(&h(Some("fastembed"), Some("gemini"))),
            ServerCapability::Full
        );
    }
    use kimun_core::VaultConfig;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct FakeTransport {
        pushed: Mutex<Vec<WireDoc>>,
        deleted: Mutex<Vec<String>>,
        server: Mutex<HashMap<String, String>>,
        fail_push: Mutex<bool>,
    }

    #[async_trait]
    impl RagTransport for FakeTransport {
        async fn push_docs(&self, docs: Vec<WireDoc>) -> Result<(), RagError> {
            if *self.fail_push.lock().unwrap() {
                return Err(RagError::Protocol("boom".into()));
            }
            self.pushed.lock().unwrap().extend(docs);
            Ok(())
        }
        async fn delete_paths(&self, paths: Vec<String>) -> Result<(), RagError> {
            self.deleted.lock().unwrap().extend(paths);
            Ok(())
        }
        async fn server_hashes(&self) -> Result<HashMap<String, String>, RagError> {
            Ok(self.server.lock().unwrap().clone())
        }
    }

    async fn vault(dir: &std::path::Path) -> NoteVault {
        NoteVault::new(VaultConfig::new(dir)).await.unwrap()
    }

    #[tokio::test]
    async fn drain_pushes_created_note_and_deletes_removed() {
        let dir = TempDir::new().unwrap();
        let vault = vault(dir.path()).await;
        let dirty = register(&vault);
        let transport = FakeTransport::default();

        vault
            .create_note(&VaultPath::new("a.md"), "# Title\n\nbody")
            .await
            .unwrap();
        drain(&vault, &dirty, &transport).await.unwrap();

        // Block-scoped: clippy's await_holding_lock tracks lexical scope, so an
        // explicit drop() before the awaits below wouldn't silence it.
        {
            let pushed = transport.pushed.lock().unwrap();
            assert_eq!(pushed.len(), 1);
            assert_eq!(pushed[0].path, "/a.md"); // canonical
            assert!(!pushed[0].sections.is_empty());
            assert!(dirty.is_empty());
        }

        vault.delete_note(&VaultPath::new("a.md")).await.unwrap();
        drain(&vault, &dirty, &transport).await.unwrap();
        assert_eq!(
            *transport.deleted.lock().unwrap(),
            vec!["/a.md".to_string()]
        );
    }

    #[tokio::test]
    async fn failed_push_requeues() {
        let dir = TempDir::new().unwrap();
        let vault = vault(dir.path()).await;
        let dirty = register(&vault);
        let transport = FakeTransport::default();
        *transport.fail_push.lock().unwrap() = true;

        vault
            .create_note(&VaultPath::new("a.md"), "body")
            .await
            .unwrap();
        assert!(drain(&vault, &dirty, &transport).await.is_err());
        // The op survived for a later retry.
        assert_eq!(dirty.len(), 1);
    }

    #[tokio::test]
    async fn reconcile_pushes_missing_and_deletes_stale() {
        let dir = TempDir::new().unwrap();
        let vault = vault(dir.path()).await;
        let _dirty = register(&vault);
        let transport = FakeTransport::default();

        vault
            .create_note(&VaultPath::new("keep.md"), "kept")
            .await
            .unwrap();
        // Server already has a stale note the vault no longer contains.
        transport
            .server
            .lock()
            .unwrap()
            .insert("/gone.md".to_string(), "oldhash".to_string());

        reconcile(&vault, &transport).await.unwrap();

        let pushed = transport.pushed.lock().unwrap();
        assert!(pushed.iter().any(|d| d.path == "/keep.md"));
        assert_eq!(
            *transport.deleted.lock().unwrap(),
            vec!["/gone.md".to_string()]
        );
    }
}
