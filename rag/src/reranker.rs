use crate::document::KimunChunk;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use log::debug;
use std::sync::Mutex;

/// Reranker for improving search result quality using cross-encoder models
pub struct CrossEncoderReranker {
    model: Mutex<TextRerank>,
}

impl CrossEncoderReranker {
    /// Create a new reranker with the default model (BGE Reranker Base)
    pub fn new() -> anyhow::Result<Self> {
        let model = TextRerank::try_new(
            RerankInitOptions::new(RerankerModel::BGERerankerBase)
                .with_show_download_progress(true),
        )?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }

    /// Rerank search results based on query relevance
    /// Returns the top_k results sorted by relevance score
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

        // Convert to &str references for the API
        let doc_refs: Vec<&str> = documents.iter().map(|s| s.as_str()).collect();

        // Perform reranking
        let mut model = self.model.lock().unwrap();
        let rerank_results = model.rerank(query, doc_refs, true, None)?;

        // Sort by score and take top_k
        let mut scored_results: Vec<(f64, KimunChunk)> = rerank_results
            .into_iter()
            .map(|result| {
                let original_chunk = &results[result.index].1;
                (result.score as f64, original_chunk.clone())
            })
            .collect();

        // Sort by score descending
        scored_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

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
