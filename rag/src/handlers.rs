use axum::{Json, extract::State, http::StatusCode};
use log::debug;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::Hash, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    IndexStats, KimunRag,
    config::RagConfig,
    dbembeddings::IndexedNote,
    document::{ChunkPayload, KimunChunk, KimunMetadata},
    server_state::{AppState, JobStatus},
};

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct IndexSingleRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ChunkData {
    pub content: String,
    pub title: String,
    pub date: Option<String>, // Format: YYYY-MM-DD
}

#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    pub query: String,
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub query: String,
    pub llm_provider: Option<String>, // "claude", "openai", "gemini", "mistral"
    pub llm_model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IndexResponse {
    pub job_id: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub job_id: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingsResponse {
    pub chunks: Vec<ChunkResult>,
}

#[derive(Debug, Serialize)]
pub struct ChunkResult {
    pub path: String,
    pub title: String,
    pub date: Option<String>,
    pub content: String,
    pub hash: String,
    pub similarity_score: f64,
}

#[derive(Debug, Serialize)]
pub struct AnswerResponse {
    pub answer: String,
    pub sources: Vec<ChunkResult>,
}

#[derive(Debug, Serialize)]
pub struct JobStatusResponse {
    pub job_id: String,
    pub status: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// Index All - Parse vault and create/store embeddings
pub async fn index_all_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<IndexResponse>, (StatusCode, Json<ErrorResponse>)> {
    let job_id = Uuid::new_v4();

    // Mark job as queued
    state
        .job_tracker
        .lock()
        .await
        .create(job_id, JobStatus::Queued);

    // Spawn background task
    let state_clone = state.clone();
    tokio::spawn(async move {
        // Mark as processing
        state_clone
            .job_tracker
            .lock()
            .await
            .update_status(&job_id, JobStatus::Processing);

        // Perform indexing
        match index_all_impl(state_clone.rag.clone(), &state_clone.config).await {
            Ok(stats) => {
                let result = serde_json::json!({
                    "indexed": stats.indexed,
                    "skipped": stats.skipped,
                    "updated": stats.updated,
                    "errors": stats.errors,
                })
                .to_string();
                state_clone
                    .job_tracker
                    .lock()
                    .await
                    .set_result(&job_id, result);
            }
            Err(e) => {
                state_clone
                    .job_tracker
                    .lock()
                    .await
                    .set_error(&job_id, e.to_string());
            }
        }
    });

    Ok(Json(IndexResponse {
        job_id: job_id.to_string(),
        message: "Indexing job started".to_string(),
    }))
}

/// Index Single Entry - Receive text chunks + path, replace all chunks for that path
pub async fn index_single_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<IndexSingleRequest>,
) -> Result<Json<IndexResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Store synchronously (as per user requirement)
    match store_single_note_impl(&request.path, state.rag.clone(), &state.config).await {
        Ok(()) => Ok(Json(IndexResponse {
            job_id: Uuid::new_v4().to_string(),
            message: format!("Successfully indexed path {}", request.path),
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to index: {}", e),
            }),
        )),
    }
}

/// Get Embeddings - Query text → return top X chunks with path, title, similarity scores
pub async fn get_embeddings_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<QueryRequest>,
) -> Result<Json<EmbeddingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let rag = state.rag.lock().await;

    match rag.query_embeddings(&request.query).await {
        Ok(results) => {
            let chunks: Vec<ChunkResult> = results
                .into_iter()
                .map(|(score, chunk)| {
                    let source_path = chunk.metadata.source_path.clone();
                    let title = chunk.metadata.title.clone();
                    let date = chunk.metadata.get_date_string();
                    let hash = chunk.metadata.hash.clone();
                    ChunkResult {
                        path: source_path,
                        title,
                        date,
                        content: chunk.content,
                        similarity_score: score,
                        hash,
                    }
                })
                .collect();

            Ok(Json(EmbeddingsResponse { chunks }))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Query failed: {}", e),
            }),
        )),
    }
}

/// Answer - Query text → LLM answer with context (queued)
/// Supports dynamic LLM selection via request body and X-API-Key header
pub async fn answer_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    let job_id = Uuid::new_v4();

    // Extract API key from header
    let api_key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Mark job as queued
    state
        .job_tracker
        .lock()
        .await
        .create(job_id, JobStatus::Queued);

    // Spawn background task
    let state_clone = state.clone();

    tokio::spawn(async move {
        // Mark as processing
        state_clone
            .job_tracker
            .lock()
            .await
            .update_status(&job_id, JobStatus::Processing);

        // Perform query and answer with dynamic LLM
        match answer_impl_with_llm(
            state_clone.rag.clone(),
            &request.query,
            request.llm_provider.as_deref(),
            request.llm_model.as_deref(),
            api_key.as_deref(),
        )
        .await
        {
            Ok((answer, sources)) => {
                let result = serde_json::json!({
                    "answer": answer,
                    "sources": sources,
                })
                .to_string();
                state_clone
                    .job_tracker
                    .lock()
                    .await
                    .set_result(&job_id, result);
            }
            Err(e) => {
                state_clone
                    .job_tracker
                    .lock()
                    .await
                    .set_error(&job_id, e.to_string());
            }
        }
    });

    Ok(Json(QueryResponse {
        job_id: job_id.to_string(),
        message: "Query job started".to_string(),
    }))
}

/// Job Status - Get status of a job
pub async fn job_status_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(job_id_str): axum::extract::Path<String>,
) -> Result<Json<JobStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    let job_id = Uuid::parse_str(&job_id_str).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid job ID format".to_string(),
            }),
        )
    })?;

    let tracker = state.job_tracker.lock().await;

    if let Some(job) = tracker.get(&job_id) {
        let (status_str, result, error) = match &job.status {
            JobStatus::Queued => ("queued".to_string(), None, None),
            JobStatus::Processing => ("processing".to_string(), None, None),
            JobStatus::Completed => {
                let result = job
                    .result
                    .as_ref()
                    .and_then(|s| serde_json::from_str(s).ok());
                ("completed".to_string(), result, None)
            }
            JobStatus::Failed => ("failed".to_string(), None, job.error.clone()),
        };

        Ok(Json(JobStatusResponse {
            job_id: job_id_str,
            status: status_str,
            result,
            error,
        }))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Job {} not found", job_id_str),
            }),
        ))
    }
}

// ============================================================================
// Implementation Functions
// ============================================================================
async fn store_chunks_impl(
    chunks: &[ChunkPayload],
    indexed_notes: &HashMap<String, IndexedNote>,
    rag: Arc<Mutex<KimunRag>>,
) -> anyhow::Result<IndexStats> {
    let mut indexed_count = 0;
    let mut updated_count = 0;
    let mut skipped_count = 0;

    debug!("Starting to store {} chunks", chunks.len());

    // Read notes from the database in a blocking task (rusqlite is not Send)
    let mut chunk_payloads: HashMap<String, Vec<KimunChunk>> = HashMap::new();
    for chunk in chunks {
        let path = chunk.doc_path.clone();
        chunk_payloads
            .entry(path)
            .or_insert_with(|| vec![chunk.to_owned().into()]);
    }

    let rag_lock = rag.lock().await;
    for (path, chunks) in chunk_payloads {
        // We take the first hash as the valid one
        let content_hash = chunks
            .first()
            .map_or_else(|| "".to_string(), |c| c.metadata.hash.clone());

        let needs_indexing = if let Some(indexed) = indexed_notes.get(&path) {
            let update = indexed.content_hash != content_hash;
            if update {
                // These ones needs to be updated, so we need to remove them first
                debug!("For updating, deleting embeddings for {}", path);
                rag_lock.embeddings.delete_embeddings(vec![&path]).await?;
                updated_count += 1;
            } else {
                debug!("Skipping embeddings for {}", path);
                skipped_count += 1;
            }
            update
        } else {
            debug!("Indexing embeddings for {}", path);
            indexed_count += 1;
            true
        };

        if needs_indexing {
            debug!("Starting storing embeddings");
            rag_lock.embeddings.store_embeddings(&chunks).await?;
            rag_lock
                .embeddings
                .mark_as_indexed(&path, &content_hash)
                .await?;
            debug!("Finished storing embeddings");
        }
    }

    Ok(IndexStats {
        indexed: indexed_count,
        skipped: skipped_count,
        updated: updated_count,
        removed: 0,
        errors: 0,
    })
}

async fn store_single_note_impl(
    path: &str,
    rag: Arc<Mutex<KimunRag>>,
    config: &RagConfig,
) -> anyhow::Result<()> {
    // Open the vault database
    let vault_path = config.vault.path.clone();
    let db_path = vault_path.join("kimun.sqlite");

    if !db_path.exists() {
        anyhow::bail!("Vault database not found at {:?}", db_path);
    }

    let source_path = path.to_string();
    // Read notes from the database in a blocking task (rusqlite is not Send)
    let chunks = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use rusqlite::Connection;
        let conn = Connection::open(&db_path)?;

        let mut stmt = conn.prepare(
            "SELECT n.path, nc.breadCrumb, nc.text, n.hash
             FROM notes n
             JOIN notesContent nc ON n.path = nc.path where n.path = ?1",
        )?;
        let chunks_iter = stmt.query_map([source_path], |row| {
            let path: String = row.get(0)?;
            let breadcrumb: String = row.get(1)?;
            let text: String = row.get(2)?;
            let hash: String = row.get(3)?;

            Ok(ChunkPayload {
                title: breadcrumb,
                text,
                doc_path: path,
                doc_hash: hash,
            })
        })?;

        let mut chunks = vec![];
        for chunk_payload in chunks_iter {
            let chunk_payload = chunk_payload?;
            let chunk = Into::<KimunChunk>::into(chunk_payload);
            chunks.push(chunk);
        }
        Ok(chunks)
    })
    .await??;
    let rag_lock = rag.lock().await;

    if chunks.is_empty() {
        // If no chunks, remove from index
        rag_lock.embeddings.remove_indexed_note(path).await?;
        return Ok(());
    }
    let content_hash = &chunks.first().unwrap().metadata.hash;

    // Store embeddings
    rag_lock.embeddings.store_embeddings(&chunks).await?;
    rag_lock
        .embeddings
        .mark_as_indexed(path, content_hash)
        .await?;

    Ok(())
}

async fn index_all_impl(
    rag: Arc<Mutex<KimunRag>>,
    config: &RagConfig,
) -> anyhow::Result<crate::IndexStats> {
    // Open the vault database
    let vault_path = config.vault.path.clone();
    let db_path = vault_path.join("kimun.sqlite");

    if !db_path.exists() {
        anyhow::bail!("Vault database not found at {:?}", db_path);
    }

    // Read notes from the database in a blocking task (rusqlite is not Send)
    let chunks = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use rusqlite::Connection;
        let conn = Connection::open(&db_path)?;

        let mut stmt = conn.prepare(
            "SELECT n.path, nc.breadCrumb, nc.text, n.hash
             FROM notes n
             JOIN notesContent nc ON n.path = nc.path",
        )?;
        let chunks_iter = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                let breadcrumb: String = row.get(1)?;
                let text: String = row.get(2)?;
                let hash: String = row.get(3)?;

                let chunk = ChunkPayload {
                    title: breadcrumb,
                    text,
                    doc_path: path,
                    doc_hash: hash,
                };

                Ok(chunk)
            })?
            .filter_map(|p| p.ok())
            .collect::<Vec<ChunkPayload>>();

        Ok(chunks_iter)
    })
    .await??;

    let mut indexed_notes = {
        // We get the lock in a separate context, so we don't hold it
        let rag_lock = rag.lock().await;
        let indexed_notes = rag_lock.embeddings.get_indexed_notes().await?;
        indexed_notes
    };
    let mut index_stats = store_chunks_impl(&chunks, &indexed_notes, rag.clone()).await?;

    for chunk in chunks {
        indexed_notes.remove(&chunk.doc_path);
    }

    let missing = indexed_notes.keys().collect::<Vec<&String>>();
    index_stats.removed += missing.len();
    let rag_lock = rag.lock().await;
    rag_lock.embeddings.delete_embeddings(missing).await?;
    debug!("Done indexing: {}", index_stats);

    Ok(index_stats)
}

/// Answer implementation with dynamic LLM selection
async fn answer_impl_with_llm(
    rag: Arc<Mutex<KimunRag>>,
    query: &str,
    llm_provider: Option<&str>,
    llm_model: Option<&str>,
    api_key: Option<&str>,
) -> anyhow::Result<(String, Vec<ChunkResult>)> {
    use crate::llmclients::{
        LLMClient, claude::ClaudeClient, gemini::GeminiClient, mistral::MistralClient,
        openai::OpenAIClient,
    };

    debug!("Answering a question");

    // Step 1: Query embeddings (fast vector search) and get reranker while holding the lock briefly
    let (raw_results, reranker_option) = {
        let rag_lock = rag.lock().await;
        let results = rag_lock.query_embeddings_raw(query).await?;
        let reranker = rag_lock.get_reranker();
        (results, reranker)
    }; // Lock released here

    debug!(
        "Got {} raw results from embeddings query",
        raw_results.len()
    );

    // Step 2: Apply reranking (CPU-intensive) WITHOUT holding the lock
    let results = if let Some((reranker, top_k)) = reranker_option {
        debug!("Reranking results without lock");
        reranker.rerank(query, raw_results, top_k).await?
    } else {
        debug!("No reranking needed");
        raw_results
    };

    debug!("After reranking: {} results", results.len());

    // Step 3: Set up API key if provided (without lock)
    let _env_guard = if let Some(key) = api_key {
        llm_provider
            .as_ref()
            .map(|provider| set_temp_api_key(provider, key))
    } else {
        None
    };

    // Step 4: Create or get LLM client (without holding the lock)
    let llm_client: Arc<dyn LLMClient + Send + Sync> = if let Some(provider) = llm_provider {
        match provider.to_lowercase().as_str() {
            "claude" => {
                let model = llm_model.unwrap_or("claude-3-5-sonnet-20241022");
                Arc::new(ClaudeClient::new(model.to_string()))
            }
            "openai" => {
                let model = llm_model.unwrap_or("gpt-4o-mini");
                Arc::new(OpenAIClient::new(model.to_string()))
            }
            "gemini" => {
                let model = llm_model.unwrap_or("gemini-2.5-flash");
                Arc::new(GeminiClient::new(model))
            }
            "mistral" => {
                let model = llm_model.unwrap_or("mistral-large-latest");
                Arc::new(MistralClient::new(model))
            }
            _ => anyhow::bail!("Unknown LLM provider: {}", provider),
        }
    } else {
        // Get default LLM client (briefly holding lock just to clone the Arc)
        let rag_lock = rag.lock().await;
        let client = rag_lock.get_llm_client();
        drop(rag_lock); // Explicitly release lock before LLM call
        client
    };

    // Step 5: Call LLM without holding any lock - this is the slow operation (can take seconds)
    debug!("Calling LLM without lock");
    let answer = llm_client.ask(query, &results).await?;

    // Format sources
    let sources: Vec<ChunkResult> = results
        .into_iter()
        .map(|(score, chunk)| {
            let source_path = chunk.metadata.source_path.clone();
            let title = chunk.metadata.title.clone();
            let date = chunk.metadata.get_date_string();
            let hash = chunk.metadata.hash.clone();
            ChunkResult {
                path: source_path,
                title,
                date,
                content: chunk.content,
                similarity_score: score,
                hash,
            }
        })
        .collect();

    Ok((answer, sources))
}

/// Helper to temporarily set API key in environment
/// Returns a guard that will restore the original value on drop
fn set_temp_api_key(provider: &str, api_key: &str) -> TempEnvGuard {
    let env_var = match provider {
        "claude" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        _ => {
            return TempEnvGuard {
                var: None,
                original: None,
            };
        }
    };

    let original = std::env::var(env_var).ok();

    // SAFETY: We're modifying environment variables in a controlled way within a single-threaded
    // context (tokio spawn). The guard ensures restoration. This is acceptable for temporary
    // API key injection per request.
    unsafe {
        std::env::set_var(env_var, api_key);
    }

    TempEnvGuard {
        var: Some(env_var.to_string()),
        original,
    }
}

/// Guard that restores environment variable on drop
struct TempEnvGuard {
    var: Option<String>,
    original: Option<String>,
}

impl Drop for TempEnvGuard {
    fn drop(&mut self) {
        if let Some(var) = &self.var {
            // SAFETY: Restoring environment variable state
            unsafe {
                if let Some(original) = &self.original {
                    std::env::set_var(var, original);
                } else {
                    std::env::remove_var(var);
                }
            }
        }
    }
}
