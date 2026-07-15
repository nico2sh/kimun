use async_trait::async_trait;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use std::sync::{Arc, Mutex};

use crate::config::{RerankerConfig, RerankerProvider};
use crate::document::FlattenedChunk;

pub mod http;

pub use http::HttpReranker;

/// Reorders a retrieved candidate pool by query relevance and keeps the
/// `top_k` best. One implementation runs locally (fastembed cross-encoder);
/// the other calls an external HTTP rerank endpoint. Unlike the [`super::dbembeddings::embedder::Embedder`],
/// this is not an invariant of the stored vectors — swapping rerankers never
/// invalidates the index.
#[async_trait]
pub trait Reranker: Send + Sync {
    /// Reranks `results` against `query`; returns the `top_k` best, sorted by
    /// relevance score descending.
    async fn rerank(
        &self,
        query: &str,
        results: Vec<(f64, FlattenedChunk)>,
        top_k: usize,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>>;
}

/// Builds the configured reranker. Errors (model download failure, unreachable
/// endpoint, missing url) are the caller's to handle — the server treats them
/// as non-fatal and degrades to plain vector ranking.
pub async fn from_config(cfg: &RerankerConfig) -> anyhow::Result<Arc<dyn Reranker>> {
    match cfg.provider {
        RerankerProvider::FastEmbed => Ok(Arc::new(CrossEncoderReranker::new()?)),
        RerankerProvider::Http => {
            let url = cfg
                .url
                .clone()
                .ok_or_else(|| anyhow::anyhow!("[reranker] type = \"http\" needs a url"))?;
            Ok(Arc::new(HttpReranker::new(url, cfg.model.clone(), cfg.api_key.clone()).await?))
        }
    }
}

/// The document text a reranker scores — same title+body shape the embedder
/// indexes, so both backends judge identical inputs.
pub(crate) fn rerank_document(chunk: &FlattenedChunk) -> String {
    format!("{}\n{}", chunk.title, chunk.text)
}

/// Reranker for improving search result quality using cross-encoder models
pub struct CrossEncoderReranker {
    model: Arc<Mutex<TextRerank>>,
}

impl CrossEncoderReranker {
    /// Create a new reranker with the default model (BGE Reranker Base)
    pub fn new() -> anyhow::Result<Self> {
        let model = TextRerank::try_new(
            RerankInitOptions::new(RerankerModel::BGERerankerBase)
                .with_show_download_progress(true),
        )?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }
}

#[async_trait]
impl Reranker for CrossEncoderReranker {
    /// Rerank search results based on query relevance
    /// Returns the top_k results sorted by relevance score
    ///
    /// This operation is CPU-intensive and runs in a blocking thread pool
    /// to avoid blocking the async runtime.
    async fn rerank(
        &self,
        query: &str,
        results: Vec<(f64, FlattenedChunk)>,
        top_k: usize,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        if results.is_empty() {
            return Ok(results);
        }

        // Prepare documents for reranking
        let documents: Vec<String> = results
            .iter()
            .map(|(_, chunk)| rerank_document(chunk))
            .collect();

        let query_owned = query.to_string();
        let model = self.model.clone(); // Clone the Arc, not the model itself

        // Run the CPU-intensive reranking in a blocking task to avoid blocking the async runtime
        let rerank_results = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            // This is a synchronous blocking operation - neural network inference
            let mut model_guard = model.lock().unwrap();

            // Create Vec<&String> which satisfies AsRef<[&String]>
            let doc_refs: Vec<&String> = documents.iter().collect();
            let results = model_guard.rerank(&query_owned, doc_refs, true, None)?;
            Ok(results)
        })
        .await??;

        // Sort by score and take top_k
        let mut scored_results: Vec<(f64, FlattenedChunk)> = rerank_results
            .into_iter()
            .map(|result| {
                let original_chunk = &results[result.index].1;
                (result.score as f64, original_chunk.clone())
            })
            .collect();

        // Sort by score descending
        scored_results.sort_by(|(score_a, _), (score_b, _)| score_b.partial_cmp(score_a).unwrap());

        // Take top_k
        scored_results.truncate(top_k);

        Ok(scored_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_reranker_basic() {
        let reranker = CrossEncoderReranker::new().unwrap();

        let chunks = vec![
            (
                0.8,
                FlattenedChunk {
                    text: "Python is a programming language".to_string(),
                    doc_path: "doc1.md".to_string(),
                    title: "Python".to_string(),
                    date: None,
                    doc_hash: "1234".to_string(),
                },
            ),
            (
                0.7,
                FlattenedChunk {
                    text: "Snakes are reptiles".to_string(),
                    doc_path: "doc2.md".to_string(),
                    title: "Animals".to_string(),
                    date: None,
                    doc_hash: "5678".to_string(),
                },
            ),
        ];

        let results = reranker
            .rerank("programming languages", chunks, 2)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        // The programming-related chunk should rank higher
        assert!(results[0].1.text.contains("programming"));
    }
}
