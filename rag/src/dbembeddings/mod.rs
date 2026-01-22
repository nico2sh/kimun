use async_trait::async_trait;
use std::collections::HashMap;
use crate::document::KimunChunk;

mod embedder;

pub mod vecsqlite;
pub mod vecqdrant;

/// Information about an indexed note
#[derive(Debug, Clone)]
pub struct IndexedNote {
    pub path: String,
    pub content_hash: String,
    pub last_indexed: i64, // Unix timestamp
}

#[async_trait]
pub trait Embeddings: Send + Sync {
    fn init(&mut self) -> anyhow::Result<()>;
    async fn store_embeddings(&self, content: &[KimunChunk]) -> anyhow::Result<()>;
    async fn query_embedding(&self, content: &str) -> anyhow::Result<Vec<(f64, KimunChunk)>>;

    // Index tracking methods
    fn get_indexed_notes(&self) -> anyhow::Result<HashMap<String, IndexedNote>>;
    fn mark_as_indexed(&self, path: &str, content_hash: &str) -> anyhow::Result<()>;
    fn remove_indexed_note(&self, path: &str) -> anyhow::Result<()>;
}
