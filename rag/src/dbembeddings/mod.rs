use crate::document::{FlattenedChunk, KimunDoc};
use async_trait::async_trait;
use std::{collections::HashMap, fmt::Display};

pub mod embedder;

pub mod vecqdrant;
pub mod vecsqlite;

/// Information about an indexed note
#[derive(Debug, Clone)]
pub struct IndexedNote {
    pub path: String,
    pub content_hash: String,
    pub last_indexed: i64, // Unix timestamp
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
    async fn remove_indexed_note(&self, collection: &str, path: &str) -> anyhow::Result<()>;
}
