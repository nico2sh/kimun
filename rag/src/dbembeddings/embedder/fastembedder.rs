use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::document::FlattenedChunk;

use super::Embedder;

pub struct FastEmbedder {
    model: Arc<Mutex<TextEmbedding>>,
}

impl FastEmbedder {
    pub fn new() -> anyhow::Result<Self> {
        let model = TextEmbedding::try_new(
            // InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
            InitOptions::new(EmbeddingModel::BGELargeENV15).with_show_download_progress(true),
        )?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }
}

impl Embedder for FastEmbedder {
    async fn generate_embeddings(
        &self,
        chunks: &[FlattenedChunk],
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let texts: Vec<String> = chunks
            .iter()
            .map(|chunk| format!("passage: {}\n{}", chunk.title, chunk.text))
            .collect();

        let model = self.model.clone();
        let embeds = tokio::task::spawn_blocking(move || {
            let mut model_guard = model.blocking_lock();
            model_guard.embed(texts, None)
        })
        .await??;

        Ok(embeds)
    }

    async fn prompt_embedding<S: AsRef<str>>(&self, query: S) -> anyhow::Result<Vec<f32>> {
        let text = format!("query: {}", query.as_ref());
        let model = self.model.clone();

        let embed = tokio::task::spawn_blocking(move || {
            let mut model_guard = model.blocking_lock();
            model_guard.embed(vec![text], None)
        })
        .await??;

        Ok(embed.into_iter().next().unwrap())
    }
}
