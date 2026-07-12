use crate::document::{FlattenedChunk, KimunDoc};
use async_trait::async_trait;
use std::{collections::HashMap, fmt::Display};

pub mod embedder;

pub mod veclance;
pub mod vecqdrant;

/// Information about an indexed note
#[derive(Debug, Clone)]
pub struct IndexedNote {
    pub path: String,
    pub content_hash: String,
    pub last_indexed: i64, // Unix timestamp
}

/// One collection's summary for the server admin UI: its name (the vault id)
/// and how many notes it has indexed.
#[derive(Debug, Clone)]
pub struct CollectionInfo {
    pub name: String,
    pub note_count: usize,
}

impl Display for IndexedNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Path: {}, Hash: {}, Last Indexed: {}",
            self.path, self.content_hash, self.last_indexed
        )
    }
}

/// Storage + retrieval of chunk embeddings, scoped per **collection** — one
/// collection per vault, keyed by the vault's id (adr/0020). Every operation
/// takes the collection so one server can serve many vaults in isolation.
#[async_trait]
pub trait Embeddings: Send + Sync {
    async fn init(&self) -> anyhow::Result<()>;
    async fn store_embeddings(&self, collection: &str, content: &[KimunDoc]) -> anyhow::Result<()>;
    async fn delete_embeddings(&self, collection: &str, paths: Vec<&String>) -> anyhow::Result<()>;
    async fn query_embedding(
        &self,
        collection: &str,
        content: &str,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>>;

    /// The `{note path → IndexedNote}` map for one collection — the authoritative
    /// server-side hash set the client reconciles against.
    async fn get_indexed_notes(
        &self,
        collection: &str,
    ) -> anyhow::Result<HashMap<String, IndexedNote>>;

    /// Every collection the store holds, with its indexed-note count. Powers the
    /// server admin UI's collections page. May be O(store) per collection on some
    /// backends — use [`collection_names`](Self::collection_names) when only the
    /// names are needed.
    async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>>;

    /// Just the collection names (vault ids), cheaply — no per-collection scan.
    /// For pickers/dropdowns that don't need counts.
    async fn collection_names(&self) -> anyhow::Result<Vec<String>>;
    /// Removes a single note's hash record. Deletes go through `delete` on the
    /// store (which drops chunks + hash together); this lower-level hook is
    /// currently unused, and the two backends differ in whether it also removes
    /// chunk vectors — wire it through `delete` before relying on it.
    async fn remove_indexed_note(&self, collection: &str, path: &str) -> anyhow::Result<()>;
}
