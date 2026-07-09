use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;

use dbembeddings::Embeddings;
use dbembeddings::vecsqlite::VecSQLite;
// use kimun_core::NoteVault;
use llmclients::{LLMClient, gemini::GeminiClient};
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
pub mod config;
pub mod handlers;
pub mod server_state;

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

    /// Enable reranking with the given top_k parameter
    pub fn with_reranking(mut self) -> anyhow::Result<Self> {
        let reranker = CrossEncoderReranker::new()?;
        self.reranker = Some(Arc::new(reranker));
        Ok(self)
    }

    /// Helper to create with local SQLite + fastembed and Gemini.
    pub fn sqlite<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let embedder =
            Arc::new(crate::dbembeddings::embedder::fastembedder::FastEmbedder::new(None)?);
        Ok(Self {
            embeddings: Arc::new(VecSQLite::new(path, embedder)),
            llm_client: Arc::new(GeminiClient::new("gemini-2.5-flash")),
            reranker: None,
        })
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
        query: &str,
    ) -> anyhow::Result<Vec<(f64, document::FlattenedChunk)>> {
        self.embeddings.query_embedding(query).await
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
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<(f64, document::FlattenedChunk)>> {
        let results = self.embeddings.query_embedding(query).await?;

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
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<(String, Vec<(f64, FlattenedChunk)>)> {
        self.ask_with_llm(query, self.llm_client.clone(), top_k)
            .await
    }

    pub async fn ask_with_llm(
        &self,
        query: &str,
        llm: Arc<dyn LLMClient + Send + Sync>,
        top_k: usize,
    ) -> anyhow::Result<(String, Vec<(f64, FlattenedChunk)>)> {
        let context = self.query_embeddings(query, top_k).await?;
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
