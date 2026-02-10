use crate::document::FlattenedChunk;

pub mod fastembedder;

pub trait Embedder {
    async fn generate_embeddings(
        &self,
        content: &[FlattenedChunk],
    ) -> anyhow::Result<Vec<Vec<f32>>>;
    async fn prompt_embedding<S: AsRef<str>>(&self, content: S) -> anyhow::Result<Vec<f32>>;
}
