use async_trait::async_trait;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use std::sync::{Arc, Mutex};

use crate::config::{RerankerConfig, RerankerProvider};
use crate::document::FlattenedChunk;

pub mod http;

pub use http::HttpReranker;

/// Scores a retrieved candidate pool by query relevance. One implementation
/// runs locally (fastembed cross-encoder); the other calls an external HTTP
/// rerank endpoint. Unlike the [`super::dbembeddings::embedder::Embedder`],
/// this is not an invariant of the stored vectors — swapping rerankers never
/// invalidates the index.
#[async_trait]
pub trait Reranker: Send + Sync {
    /// Scores `results` against `query`; returns one `(input index, score)`
    /// pair per input, sorted best-first. **Contract: scores are calibrated
    /// relevance in `0.0..=1.0`, higher = better** — the context cuts and the
    /// UI assume that scale; adapters own the conversion from their backend's
    /// native scale (e.g. the cross-encoder sigmoids its raw logits), and the
    /// pipeline sigmoid-normalizes violating batches (zeroing non-finite
    /// scores) with a warning. Returning indices (not
    /// chunks) keeps the hot path clone-free — the pipeline materializes the
    /// reordered pool by moving chunks it already owns. Borrows the pool so a
    /// failed rerank leaves it in the caller's hands — the query pipeline
    /// falls back to plain vector ranking instead of failing the request.
    async fn rerank(
        &self,
        query: &str,
        results: &[(f64, FlattenedChunk)],
    ) -> anyhow::Result<Vec<(usize, f64)>>;
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
            Ok(Arc::new(
                HttpReranker::new(url, cfg.model.clone(), cfg.api_key.clone()).await?,
            ))
        }
    }
}

/// The document text a reranker scores — same title+body shape the embedder
/// indexes, so both backends judge identical inputs.
pub(crate) fn rerank_document(chunk: &FlattenedChunk) -> String {
    format!("{}\n{}", chunk.title, chunk.text)
}

/// Shared back half of every reranker: validates the scored `(index, score)`
/// pairs against the submitted pool size (exactly one score per document,
/// each index in range and unique — a violating backend is a provider bug
/// that must surface, not silently duplicate or drop chunks), then sorts
/// best-first. No chunks are touched — the pipeline materializes the order.
pub(crate) fn validate_scored(
    mut scored: Vec<(usize, f64)>,
    documents: usize,
) -> anyhow::Result<Vec<(usize, f64)>> {
    if scored.len() != documents {
        anyhow::bail!(
            "rerank returned {} scores for a {documents}-document request",
            scored.len(),
        );
    }
    let mut seen = vec![false; documents];
    for &(index, _) in &scored {
        let slot = seen.get_mut(index).ok_or_else(|| {
            anyhow::anyhow!("rerank returned index {index} for a {documents}-document request")
        })?;
        if *slot {
            anyhow::bail!("rerank returned index {index} more than once");
        }
        *slot = true;
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(scored)
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
    /// Rerank search results based on query relevance.
    ///
    /// This operation is CPU-intensive and runs in a blocking thread pool
    /// to avoid blocking the async runtime.
    async fn rerank(
        &self,
        query: &str,
        results: &[(f64, FlattenedChunk)],
    ) -> anyhow::Result<Vec<(usize, f64)>> {
        if results.is_empty() {
            return Ok(Vec::new());
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
            // return_documents = false: the pipeline consumes only
            // (index, score) — echoing every candidate's text back would
            // clone ~80 documents per query for nothing.
            let results = model_guard.rerank(&query_owned, doc_refs, false, None)?;
            Ok(results)
        })
        .await??;

        validate_scored(
            rerank_results
                .into_iter()
                // Raw cross-encoder logits go negative for irrelevant chunks;
                // sigmoid maps them to 0..1 (order-preserving) — the adapter
                // owns the conversion from its backend's native scale to the
                // trait's calibrated 0..1 contract (adr/0029).
                .map(|r| (r.index, sigmoid(r.score as f64)))
                .collect(),
            results.len(),
        )
    }
}

/// Order-preserving squash of a raw logit into `0..1`. Also used by the
/// pipeline to normalize a score batch that violates the trait's 0..1
/// contract — monotonic, so the ranking survives.
pub(crate) fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
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
            .rerank("programming languages", &chunks)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        // The programming-related chunk (input index 0) should rank higher,
        // with a sigmoid-calibrated score in 0..1.
        assert_eq!(results[0].0, 0);
        assert!((0.0..=1.0).contains(&results[0].1));
    }
}
