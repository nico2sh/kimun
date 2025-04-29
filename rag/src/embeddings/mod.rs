use crate::document::KimunChunk;

mod embedder;
pub mod vecsqlite;

pub trait Embeddings {
    fn init(&mut self) -> anyhow::Result<()>;
    async fn store_embeddings(&self, content: &[KimunChunk]) -> anyhow::Result<()>;
    async fn query_embedding<S: AsRef<str>>(
        &self,
        content: S,
    ) -> anyhow::Result<Vec<(f64, KimunChunk)>>;
}
