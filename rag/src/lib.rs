use std::path::Path;
use std::sync::Arc;

use document::ChunkLoader;

use dbembeddings::Embeddings;
use dbembeddings::vecsqlite::VecSQLite;
use kimun_core::NoteVault;
use llmclients::mistral::MistralClient;
use llmclients::{LLMClient, gemini::GeminiClient};

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
    reranker: Option<Arc<reranker::CrossEncoderReranker>>,
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

    /// Enable reranking with the given top_k parameter
    pub fn with_reranking(mut self, top_k: usize) -> anyhow::Result<Self> {
        let reranker = reranker::CrossEncoderReranker::new()?;
        self.reranker = Some(Arc::new(reranker));
        self.reranker_top_k = top_k;
        Ok(self)
    }

    /// Helper to create with SQLite and Gemini (for backward compatibility)
    pub fn sqlite<P: AsRef<Path>>(path: P) -> Self {
        Self {
            embeddings: Arc::new(VecSQLite::new(path)),
            llm_client: Arc::new(GeminiClient::new(
                llmclients::gemini::GeminiModel::Gemini25Flash,
            )),
            reranker: None,
            reranker_top_k: 20,
        }
    }

    /// Initialize the embeddings database
    /// Note: With the current trait design using Arc, initialization happens
    /// automatically on first use. This method is kept for API compatibility.
    pub async fn init(&self) -> anyhow::Result<()> {
        tracing::debug!("KimunRag initialized (using lazy initialization)");
        Ok(())
    }

    /// Store embeddings for all notes in the vault
    pub async fn store_embeddings(&self, vault: NoteVault) -> anyhow::Result<()> {
        let chunk_loader = ChunkLoader::new(vault);
        let chunks = chunk_loader.load_notes()?;

        self.embeddings.store_embeddings(&chunks).await?;
        Ok(())
    }

    /// Query embeddings and return raw results (without LLM)
    /// Applies reranking if enabled
    pub async fn query(&self, query: &str) -> anyhow::Result<Vec<(f64, document::KimunChunk)>> {
        let results = self.embeddings.query_embedding(query).await?;

        // Apply reranking if enabled
        if let Some(reranker) = &self.reranker {
            reranker.rerank(query, results, self.reranker_top_k).await
        } else {
            Ok(results)
        }
    }

    /// Query the RAG system with a question and get an LLM answer
    /// Uses reranked results if reranking is enabled
    pub async fn ask(&self, query: &str) -> anyhow::Result<String> {
        let context = self.query(query).await?;
        let answer = self.llm_client.ask(query, context).await?;
        Ok(answer)
    }

    /// Store embeddings with incremental indexing (only index changed notes)
    pub async fn store_embeddings_incremental(
        &self,
        vault: NoteVault,
    ) -> anyhow::Result<IndexStats> {
        let chunk_loader = ChunkLoader::new(vault);
        let chunks = chunk_loader.load_notes()?;

        // Get currently indexed notes
        let indexed_notes = self.embeddings.get_indexed_notes()?;

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

        for (path, path_chunks_vec) in path_chunks {
            // Compute hash of all chunks for this path
            let content: String = path_chunks_vec
                .iter()
                .map(|c| c.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let content_hash = dbembeddings::vecsqlite::compute_content_hash(&content);

            // Check if we need to reindex
            let needs_indexing = if let Some(indexed) = indexed_notes.get(&path) {
                indexed.content_hash != content_hash
            } else {
                true
            };

            if needs_indexing {
                // Index these chunks
                let chunks_to_index: Vec<document::KimunChunk> =
                    path_chunks_vec.iter().map(|c| (*c).clone()).collect();

                self.embeddings.store_embeddings(&chunks_to_index).await?;
                self.embeddings.mark_as_indexed(&path, &content_hash)?;
                indexed_count += 1;
            } else {
                skipped_count += 1;
            }
        }

        Ok(IndexStats {
            indexed: indexed_count,
            skipped: skipped_count,
            updated: 0,
            errors: 0,
        })
    }

    /// Store a single note (replacing all existing chunks for that path)
    pub async fn store_single_note(&self, vault: NoteVault, note_path: &str) -> anyhow::Result<()> {
        let chunk_loader = ChunkLoader::new(vault);
        let all_chunks = chunk_loader.load_notes()?;

        // Filter to only the chunks for this path
        let chunks: Vec<document::KimunChunk> = all_chunks
            .into_iter()
            .filter(|c| c.metadata.source_path == note_path)
            .collect();

        if chunks.is_empty() {
            // If no chunks, remove from index
            self.embeddings.remove_indexed_note(note_path)?;
            return Ok(());
        }

        // Compute hash
        let content: String = chunks
            .iter()
            .map(|c| c.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let content_hash = dbembeddings::vecsqlite::compute_content_hash(&content);

        // Store embeddings
        self.embeddings.store_embeddings(&chunks).await?;
        self.embeddings.mark_as_indexed(note_path, &content_hash)?;

        Ok(())
    }
}

/// Statistics from indexing operation
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub indexed: usize,
    pub skipped: usize,
    pub updated: usize,
    pub errors: usize,
}
