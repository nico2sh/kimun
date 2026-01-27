use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use document::ChunkLoader;

use dbembeddings::Embeddings;
use dbembeddings::vecsqlite::VecSQLite;
// use kimun_core::NoteVault;
use llmclients::{LLMClient, gemini::GeminiClient};
use log::debug;

use crate::document::KimunChunk;
use crate::reranker::CrossEncoderReranker;

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
    reranker_top_k: usize,
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
            reranker_top_k: 20,
        }
    }

    /// Get a clone of the LLM client
    pub fn get_llm_client(&self) -> Arc<dyn LLMClient + Send + Sync> {
        self.llm_client.clone()
    }

    /// Enable reranking with the given top_k parameter
    pub fn with_reranking(mut self, top_k: usize) -> anyhow::Result<Self> {
        let reranker = CrossEncoderReranker::new()?;
        self.reranker = Some(Arc::new(reranker));
        self.reranker_top_k = top_k;
        Ok(self)
    }

    /// Helper to create with SQLite and Gemini (for backward compatibility)
    pub fn sqlite<P: AsRef<Path>>(path: P) -> Self {
        Self {
            embeddings: Arc::new(VecSQLite::new(path)),
            llm_client: Arc::new(GeminiClient::new("gemini-2.5-flash")),
            reranker: None,
            reranker_top_k: 20,
        }
    }

    /// Initialize the embeddings database
    pub async fn init(&self) -> anyhow::Result<()> {
        self.embeddings.init().await?;
        tracing::debug!("KimunRag initialized (using lazy initialization)");
        Ok(())
    }

    /// Store embeddings for all notes in the vault
    pub async fn store_embeddings(&self, db_path: PathBuf) -> anyhow::Result<()> {
        let chunk_loader = ChunkLoader::new(db_path);
        let chunks = chunk_loader.load_notes()?;

        self.embeddings.store_embeddings(&chunks).await?;
        Ok(())
    }

    /// Query embeddings without reranking (fast - just vector search)
    /// Use this when you need to minimize lock time
    pub async fn query_embeddings_raw(
        &self,
        query: &str,
    ) -> anyhow::Result<Vec<(f64, document::KimunChunk)>> {
        self.embeddings.query_embedding(query).await
    }

    /// Get reranker if enabled (returns Arc so it can be used without lock)
    pub fn get_reranker(&self) -> Option<(Arc<CrossEncoderReranker>, usize)> {
        self.reranker
            .as_ref()
            .map(|r| (r.clone(), self.reranker_top_k))
    }

    /// Apply reranking to results
    /// This is CPU-intensive and should be called without holding locks
    pub async fn apply_reranking(
        &self,
        query: &str,
        results: Vec<(f64, document::KimunChunk)>,
    ) -> anyhow::Result<Vec<(f64, document::KimunChunk)>> {
        if let Some(reranker) = &self.reranker {
            debug!("Reranking the results to {}", self.reranker_top_k);
            reranker.rerank(query, results, self.reranker_top_k).await
        } else {
            debug!("No reranking needed");
            Ok(results)
        }
    }

    /// Query embeddings and return raw results (without LLM)
    /// Applies reranking if enabled
    pub async fn query_embeddings(
        &self,
        query: &str,
    ) -> anyhow::Result<Vec<(f64, document::KimunChunk)>> {
        let results = self.embeddings.query_embedding(query).await?;

        // Apply reranking if enabled
        if let Some(reranker) = &self.reranker {
            debug!("Reranking the results to {}", self.reranker_top_k);
            reranker.rerank(query, results, self.reranker_top_k).await
        } else {
            debug!("No Reranking the results");
            Ok(results)
        }
    }

    /// Query the RAG system with a question and get an LLM answer
    /// Uses reranked results if reranking is enabled
    pub async fn ask(&self, query: &str) -> anyhow::Result<(String, Vec<(f64, KimunChunk)>)> {
        self.ask_with_llm(query, self.llm_client.clone()).await
    }

    pub async fn ask_with_llm(
        &self,
        query: &str,
        llm: Arc<dyn LLMClient + Send + Sync>,
    ) -> anyhow::Result<(String, Vec<(f64, KimunChunk)>)> {
        let context = self.query_embeddings(query).await?;
        let answer = llm.ask(query, &context).await?;
        Ok((answer, context))
    }

    /// Store embeddings with incremental indexing (only index changed notes)
    pub async fn store_embeddings_incremental(
        &self,
        db_path: PathBuf,
    ) -> anyhow::Result<IndexStats> {
        let chunk_loader = ChunkLoader::new(db_path);
        let chunks = chunk_loader.load_notes()?;

        // Get currently indexed notes
        let mut indexed_notes = self.embeddings.get_indexed_notes().await?;

        // Group chunks by path and compute hashes
        let mut path_chunks: std::collections::HashMap<String, Vec<&document::KimunChunk>> =
            std::collections::HashMap::new();
        for chunk in &chunks {
            path_chunks
                .entry(chunk.metadata.source_path.clone())
                .or_insert_with(Vec::new)
                .push(chunk);
        }

        let mut indexed_count = 0;
        let mut skipped_count = 0;
        let mut updated_count = 0;

        for (path, path_chunks_vec) in path_chunks {
            let content_hash = if path_chunks_vec
                .windows(2)
                .all(|w| w[0].metadata.hash == w[1].metadata.hash)
            {
                path_chunks_vec
                    .first()
                    .map(|f| f.metadata.hash.clone())
                    .unwrap_or_default()
            } else {
                "".to_string()
            };

            // Check if we need to reindex
            let needs_indexing = if let Some(indexed) = indexed_notes.remove(&path) {
                let update = indexed.content_hash != content_hash;
                if update {
                    updated_count += 1;
                } else {
                    skipped_count += 1;
                }
                update
            } else {
                indexed_count += 1;
                true
            };

            if needs_indexing {
                // Index these chunks
                let chunks_to_index: Vec<document::KimunChunk> =
                    path_chunks_vec.iter().map(|c| (*c).clone()).collect();

                self.embeddings.store_embeddings(&chunks_to_index).await?;
                self.embeddings
                    .mark_as_indexed(&path, &content_hash)
                    .await?;
            }
        }

        let missing = indexed_notes.keys().collect::<Vec<&String>>();
        let removed_count = missing.len();
        self.embeddings.delete_embeddings(missing).await?;

        Ok(IndexStats {
            indexed: indexed_count,
            skipped: skipped_count,
            updated: updated_count,
            errors: 0,
            removed: removed_count,
        })
    }

    /// Store a single note (replacing all existing chunks for that path)
    pub async fn store_single_note(&self, db_path: PathBuf, note_path: &str) -> anyhow::Result<()> {
        let chunk_loader = ChunkLoader::new(db_path);
        let all_chunks = chunk_loader.load_notes()?;

        // Filter to only the chunks for this path
        let chunks: Vec<document::KimunChunk> = all_chunks
            .into_iter()
            .filter(|c| c.metadata.source_path == note_path)
            .collect();

        if chunks.is_empty() {
            // If no chunks, remove from index
            self.embeddings.remove_indexed_note(note_path).await?;
            return Ok(());
        }

        // Compute hash
        let content_hash = if chunks
            .windows(2)
            .all(|w| w[0].metadata.hash == w[1].metadata.hash)
        {
            chunks
                .first()
                .map(|f| f.metadata.hash.clone())
                .unwrap_or_default()
        } else {
            "".to_string()
        };

        // Store embeddings
        self.embeddings.store_embeddings(&chunks).await?;
        self.embeddings
            .mark_as_indexed(note_path, &content_hash)
            .await?;

        Ok(())
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
