use crate::document::{FlattenedChunk, KimunDoc};
use async_trait::async_trait;
use std::{collections::HashMap, fmt::Display};

mod embedder;

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

#[async_trait]
pub trait Embeddings: Send + Sync {
    async fn init(&self) -> anyhow::Result<()>;
    async fn store_embeddings(&self, content: &[KimunDoc]) -> anyhow::Result<()>;
    async fn delete_embeddings(&self, paths: Vec<&String>) -> anyhow::Result<()>;
    async fn query_embedding(&self, content: &str) -> anyhow::Result<Vec<(f64, FlattenedChunk)>>;

    // Index tracking methods
    async fn get_indexed_notes(&self) -> anyhow::Result<HashMap<String, IndexedNote>>;
    async fn mark_as_indexed(&self, path: &str, content_hash: &str) -> anyhow::Result<()>;
    async fn remove_indexed_note(&self, path: &str) -> anyhow::Result<()>;
}
