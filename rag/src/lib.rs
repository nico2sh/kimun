use std::fmt::Display;
use std::sync::Arc;

use dbembeddings::Embeddings;
// use kimun_core::NoteVault;
use llmclients::LLMClient;
use log::debug;

use crate::document::FlattenedChunk;

// Re-export commonly used types and functions
use crate::reranker::CrossEncoderReranker;
pub use document::{KimunDoc, KimunSection, split_chunks_for_rag};

pub mod dbembeddings;
pub mod document;
pub mod llmclients;
pub mod reranker;

// Public modules for server
pub mod auth;
pub mod config;
pub mod handlers;
pub mod server_state;
pub mod webui;

pub struct KimunRag {
    embeddings: Arc<dyn Embeddings + Send + Sync>,
    llm_client: Arc<dyn LLMClient + Send + Sync>,
    reranker: Option<Arc<CrossEncoderReranker>>,
}

impl KimunRag {
    /// Create a new KimunRag instance with provided embeddings and LLM client
    pub fn new(
        embeddings: Arc<dyn Embeddings + Send + Sync>,
        llm_client: Arc<dyn LLMClient + Send + Sync>,
    ) -> Self {
        Self {
            embeddings,
            llm_client,
            reranker: None,
        }
    }

    /// Get a clone of the LLM client
    pub fn get_llm_client(&self) -> Arc<dyn LLMClient + Send + Sync> {
        self.llm_client.clone()
    }

    /// Clone the embeddings handle so a caller can do slow I/O (network scroll,
    /// external embed) without holding the shared `KimunRag` lock.
    pub fn embeddings(&self) -> Arc<dyn Embeddings + Send + Sync> {
        self.embeddings.clone()
    }

    /// Enable reranking with the given top_k parameter
    pub fn with_reranking(mut self) -> anyhow::Result<Self> {
        let reranker = CrossEncoderReranker::new()?;
        self.reranker = Some(Arc::new(reranker));
        Ok(self)
    }

    /// Initialize the embeddings database
    pub async fn init(&self) -> anyhow::Result<()> {
        self.embeddings.init().await?;
        tracing::debug!("KimunRag initialized (using lazy initialization)");
        Ok(())
    }

    /// Query embeddings without reranking (fast - just vector search)
    /// Use this when you need to minimize lock time
    pub async fn query_embeddings_raw(
        &self,
        collection: &str,
        query: &str,
    ) -> anyhow::Result<Vec<(f64, document::FlattenedChunk)>> {
        self.embeddings.query_embedding(collection, query).await
    }

    /// Get reranker if enabled (returns Arc so it can be used without lock)
    pub fn get_reranker(&self) -> Option<Arc<CrossEncoderReranker>> {
        self.reranker.as_ref().map(|r| r.clone())
    }

    /// Apply reranking to results
    /// This is CPU-intensive and should be called without holding locks
    pub async fn apply_reranking(
        &self,
        query: &str,
        results: Vec<(f64, document::FlattenedChunk)>,
        top_k: usize,
    ) -> anyhow::Result<Vec<(f64, document::FlattenedChunk)>> {
        if let Some(reranker) = &self.reranker {
            debug!("Reranking the results to {}", top_k);
            reranker.rerank(query, results, top_k).await
        } else {
            debug!("No reranking needed");
            Ok(results.into_iter().take(top_k).collect())
        }
    }

    /// Query embeddings and return raw results (without LLM)
    /// Applies reranking if enabled
    pub async fn query_embeddings(
        &self,
        collection: &str,
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<(f64, document::FlattenedChunk)>> {
        let results = self.embeddings.query_embedding(collection, query).await?;

        // Apply reranking if enabled
        if let Some(reranker) = &self.reranker {
            debug!("Reranking the results to {}", top_k);
            reranker.rerank(query, results, top_k).await
        } else {
            debug!("No Reranking the results");
            Ok(results.into_iter().take(top_k).collect())
        }
    }

    /// Query the RAG system with a question and get an LLM answer
    /// Uses reranked results if reranking is enabled
    pub async fn ask(
        &self,
        collection: &str,
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<(String, Vec<(f64, FlattenedChunk)>)> {
        self.ask_with_llm(collection, query, self.llm_client.clone(), top_k)
            .await
    }

    pub async fn ask_with_llm(
        &self,
        collection: &str,
        query: &str,
        llm: Arc<dyn LLMClient + Send + Sync>,
        top_k: usize,
    ) -> anyhow::Result<(String, Vec<(f64, FlattenedChunk)>)> {
        let context = self.query_embeddings(collection, query, top_k).await?;
        let answer = llm.ask(query, &context).await?;
        Ok((answer, context))
    }
}

/// Statistics from indexing operation
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub indexed: usize,
    pub skipped: usize,
    pub updated: usize,
    pub removed: usize,
    pub errors: usize,
}

impl Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Index Stats: ")?;
        writeln!(f, "  > Indexed: {}", self.indexed)?;
        writeln!(f, "  > Skipped: {}", self.skipped)?;
        writeln!(f, "  > Updated: {}", self.updated)?;
        writeln!(f, "  > Removed: {}", self.removed)?;
        writeln!(f, "  > Errors: {}", self.errors)
    }
}
