use async_trait::async_trait;

use crate::document::FlattenedChunk;

pub mod fastembedder;
pub mod ollama;
pub mod openai;

/// A text→vector embedder. One implementation runs locally (fastembed); others
/// call an external service (Ollama, any OpenAI-compatible endpoint). The same
/// embedder must serve both indexing and querying — mixing models produces
/// meaningless similarities — so it is a per-server invariant of the stored
/// vectors.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embeds document chunks (the indexing side).
    async fn generate_embeddings(
        &self,
        content: &[FlattenedChunk],
    ) -> anyhow::Result<Vec<Vec<f32>>>;

    /// Embeds a query string (the search side).
    async fn prompt_embedding(&self, content: &str) -> anyhow::Result<Vec<f32>>;

    /// Output vector width. Fixed for a local model, probed at construction for
    /// an external one. The sqlite vec table is created at this width; a stored
    /// width that differs means the embedder or model changed and the
    /// collection must be re-embedded.
    fn dimension(&self) -> usize;
}
