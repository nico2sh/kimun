use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::document::KimunChunk;

use super::Embedder;

pub struct FastEmbedder {
    model: TextEmbedding,
}

impl FastEmbedder {
    pub fn new() -> anyhow::Result<Self> {
        let model = TextEmbedding::try_new(
            // InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
            InitOptions::new(EmbeddingModel::BGELargeENV15).with_show_download_progress(true),
        )?;
        Ok(Self { model })
    }
}

impl Embedder for FastEmbedder {
    async fn generate_embeddings(
        &self,
        documents: &Vec<KimunChunk>,
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let embeds = self.model.embed(
            documents
                .iter()
                .map(|chunk| format!("passage: {} - {}", chunk.metadata.title, chunk.content))
                .collect(),
            None,
        )?;
        Ok(embeds)
    }

    async fn prompt_embedding<S: AsRef<str>>(&self, query: S) -> anyhow::Result<Vec<f32>> {
        let embed = self
            .model
            .embed(vec![format!("query: {}", query.as_ref())], None)?;
        Ok(embed.first().unwrap().clone())
    }
}
