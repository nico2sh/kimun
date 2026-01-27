use crate::document::KimunChunk;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use log::debug;
use std::sync::{Arc, Mutex};

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

    /// Rerank search results based on query relevance
    /// Returns the top_k results sorted by relevance score
    ///
    /// This operation is CPU-intensive and runs in a blocking thread pool
    /// to avoid blocking the async runtime.
    pub async fn rerank(
        &self,
        query: &str,
        results: Vec<(f64, KimunChunk)>,
        top_k: usize,
    ) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
        if results.is_empty() {
            return Ok(results);
        }

        // Prepare documents for reranking
        let documents: Vec<String> = results
            .iter()
            .map(|(_, chunk)| format!("{}\n{}", chunk.metadata.title, chunk.content))
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
        let mut scored_results: Vec<(f64, KimunChunk)> = rerank_results
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

/// Apply reranking if enabled in config
pub async fn apply_reranking(
    query: &str,
    results: Vec<(f64, KimunChunk)>,
    reranker: Option<&CrossEncoderReranker>,
    top_k: usize,
) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
    if let Some(reranker) = reranker {
        debug!("Reranking the results with the TOP {}", top_k);
        reranker.rerank(query, results, top_k).await
    } else {
        // No reranking, just return top_k
        debug!("No Reranking, returning the TOP {}", top_k);
        let mut results = results;
        results.truncate(top_k);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::KimunMetadata;

    #[tokio::test]
    async fn test_reranker_basic() {
        let reranker = CrossEncoderReranker::new().unwrap();

        let chunks = vec![
            (
                0.8,
                KimunChunk {
                    content: "Python is a programming language".to_string(),
                    metadata: KimunMetadata {
                        source_path: "doc1.md".to_string(),
                        title: "Python".to_string(),
                        date: None,
                        hash: "1234".to_string(),
                    },
                },
            ),
            (
                0.7,
                KimunChunk {
                    content: "Snakes are reptiles".to_string(),
                    metadata: KimunMetadata {
                        source_path: "doc2.md".to_string(),
                        title: "Animals".to_string(),
                        date: None,
                        hash: "5678".to_string(),
                    },
                },
            ),
        ];

        let results = reranker
            .rerank("programming languages", chunks, 2)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        // The programming-related chunk should rank higher
        assert!(results[0].1.content.contains("programming"));
    }
}
