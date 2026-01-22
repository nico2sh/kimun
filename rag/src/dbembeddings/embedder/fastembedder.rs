use std::sync::Mutex;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::document::KimunChunk;

use super::Embedder;

pub struct FastEmbedder {
    model: Mutex<TextEmbedding>,
}

impl FastEmbedder {
    pub fn new() -> anyhow::Result<Self> {
        let model = TextEmbedding::try_new(
            // InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
            InitOptions::new(EmbeddingModel::BGELargeENV15).with_show_download_progress(true),
        )?;
        Ok(Self { model: Mutex::new(model) })
    }
}

impl Embedder for FastEmbedder {
    async fn generate_embeddings(&self, documents: &[KimunChunk]) -> anyhow::Result<Vec<Vec<f32>>> {
        let texts: Vec<String> = documents
            .iter()
            .map(|chunk| format!("passage: {}\n{}", chunk.metadata.title, chunk.content))
            .collect();

        let mut model = self.model.lock().unwrap();
        let embeds = model.embed(texts, None)?;
        Ok(embeds)
    }

    async fn prompt_embedding<S: AsRef<str>>(&self, query: S) -> anyhow::Result<Vec<f32>> {
        let texts = vec![format!("query: {}", query.as_ref())];
        let mut model = self.model.lock().unwrap();
        let embed = model.embed(texts, None)?;
        Ok(embed.into_iter().next().unwrap())
    }
}
