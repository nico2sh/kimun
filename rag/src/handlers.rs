use axum::{Json, extract::State, http::StatusCode};
use log::debug;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    IndexStats, KimunRag,
    dbembeddings::IndexedNote,
    document::KimunDoc,
    server_state::{AppState, JobStatus},
};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Push a vault's documents (adr/0018). `vault_id` selects the collection.
#[derive(Debug, Deserialize)]
pub struct IndexDocsRequest {
    pub vault_id: String,
    pub docs: Vec<KimunDoc>,
}

/// Delete a vault's notes by path from its collection.
#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub vault_id: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    pub vault_id: String,
    pub query: String,
    /// Overrides the server's default result count when set; otherwise
    /// `reranker.top_k` from config is used.
    #[serde(default)]
    pub context_size: Option<ContextSize>,
}

/// Result count: the per-request `context_size` override, or the server default.
fn resolve_top_k(context_size: Option<ContextSize>, default: usize) -> usize {
    context_size.map(|c| c.to_top_k()).unwrap_or(default)
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub enum ContextSize {
    #[serde(rename = "small")]
    Small,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "large")]
    Large,
}

impl ContextSize {
    pub fn to_top_k(self) -> usize {
        match self {
            ContextSize::Small => 10,
            ContextSize::Medium => 20,
            ContextSize::Large => 40,
        }
    }
}

fn bad_request(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

/// Validates the vault id — the collection key. Rejects blank ids (which would
/// cross-mix every blank-id vault) and any id with characters outside
/// `[A-Za-z0-9._-]`, so it stays a safe, non-colliding collection-name segment
/// (Kimün always sends a UUID; adr/0020).
fn require_vault_id(vault_id: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let ok = !vault_id.is_empty()
        && vault_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(bad_request(
            "vault_id must be non-empty and contain only [A-Za-z0-9._-]",
        ))
    }
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub vault_id: String,
    pub query: String,
    #[serde(default)]
    pub context_size: Option<ContextSize>,
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

pub async fn index_docs_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<IndexDocsRequest>,
) -> Result<Json<IndexResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_vault_id(&request.vault_id)?;
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

        // Serialize against other index writes (store/delete) for the lifetime
        // of this read-modify-write, so two concurrent pushes can't both treat a
        // note as new and double-insert its chunks.
        let _index_guard = state_clone.index_lock.lock().await;

        // Perform indexing
        match index_docs_impl(
            &request.vault_id,
            &request.docs,
            None,
            state_clone.rag.clone(),
        )
        .await
        {
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
        message: "Index chunks job started".to_string(),
    }))
}

/// Get Embeddings - Query text → return top X chunks with path, title, similarity scores
pub async fn get_embeddings_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<QueryRequest>,
) -> Result<Json<EmbeddingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_vault_id(&request.vault_id)?;
    // Take clones of the embeddings + reranker, then release the lock so the
    // (possibly network-bound) embed/search/rerank does not serialize other
    // requests.
    let (embeddings, reranker) = {
        let rag = state.rag.lock().await;
        (rag.embeddings(), rag.get_reranker())
    };
    let top_k = resolve_top_k(request.context_size, state.config.reranker.top_k);

    let fail = |e: anyhow::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Query failed: {}", e),
            }),
        )
    };

    let raw = embeddings
        .query_embedding(&request.vault_id, &request.query)
        .await
        .map_err(&fail)?;
    let raw = deduplicate_chunks(raw);
    // Rank the FULL pool before cutting: semantic search lists NOTES, but a
    // single section-heavy note can otherwise fill every chunk slot and collapse
    // (client-side, one row per note) to a single result. Rerank/sort everything,
    // then keep each note's best chunk and take the top_k NOTES. (`/answer` keeps
    // chunk-level context — this note-dedup is search-only.)
    let pool_size = raw.len();
    let ranked = match reranker {
        Some(reranker) => reranker
            .rerank(&request.query, raw, pool_size)
            .await
            .map_err(&fail)?,
        // `deduplicate_chunks` already sorted best-first.
        None => raw,
    };
    let results = dedupe_by_note(ranked, top_k);

    let chunks: Vec<ChunkResult> = results
        .into_iter()
        .map(|(score, chunk)| ChunkResult {
            path: chunk.doc_path.clone(),
            title: chunk.title.clone(),
            date: chunk.get_date_string(),
            hash: chunk.doc_hash.clone(),
            content: chunk.text,
            similarity_score: score,
        })
        .collect();

    Ok(Json(EmbeddingsResponse { chunks }))
}

/// Answer - Query text → LLM answer with context (queued). The LLM is the one
/// configured on the server (adr: server-owned LLM config); the request carries
/// no provider/model/key.
pub async fn answer_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_vault_id(&request.vault_id)?;
    // Semantic-only server: reject question-answering up front rather than minting
    // a job that can only fail (adr/0022). The client already gates on
    // /health.llm_provider; this is the belt-and-suspenders path.
    if state.config.llm.is_none() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "no LLM configured; this server answers semantic searches only".to_string(),
            }),
        ));
    }
    let job_id = Uuid::new_v4();
    let top_k = resolve_top_k(request.context_size, state.config.reranker.top_k);

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

        match answer_impl(
            state_clone.rag.clone(),
            &request.vault_id,
            &request.query,
            top_k,
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

/// Delete notes by path from a vault's collection (used by the client when a
/// note is removed).
pub async fn index_delete_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DeleteRequest>,
) -> Result<Json<IndexResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_vault_id(&request.vault_id)?;
    let embeddings = { state.rag.lock().await.embeddings() };
    let paths: Vec<&String> = request.paths.iter().collect();
    // Serialize with index writes so a delete can't interleave with a store on
    // the same collection (partial-visibility / lost updates in the store).
    let _index_guard = state.index_lock.lock().await;
    match embeddings.delete_embeddings(&request.vault_id, paths).await {
        Ok(()) => Ok(Json(IndexResponse {
            job_id: Uuid::new_v4().to_string(),
            message: format!("Deleted {} paths", request.paths.len()),
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Delete failed: {}", e),
            }),
        )),
    }
}

/// Reconcile support: the `{note path → content hash}` set the server holds for
/// a vault, so the client can diff it against its own authoritative set and
/// push/delete only the differences (adr/0019).
pub async fn collection_hashes_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(vault_id): axum::extract::Path<String>,
) -> Result<Json<HashMap<String, String>>, (StatusCode, Json<ErrorResponse>)> {
    require_vault_id(&vault_id)?;
    let embeddings = { state.rag.lock().await.embeddings() };
    match embeddings.get_indexed_notes(&vault_id).await {
        Ok(notes) => Ok(Json(
            notes
                .into_iter()
                .map(|(path, note)| (path, note.content_hash))
                .collect(),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to read hashes: {}", e),
            }),
        )),
    }
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

/// Deduplicates embedding results by FlattenedChunk (keeping the highest score
/// per unique chunk) and returns them sorted best-first. Scores are similarities
/// (higher = better) for both backends, so this ordering is what `take(top_k)`
/// relies on when no reranker is present.
/// Collapse ranked chunks to one row per note — the best (first-seen, so
/// highest-ranked) chunk of each `doc_path` — and keep at most `top_k` notes.
/// Semantic search lists notes, so the top_k cut must land on NOTES, not chunks;
/// otherwise a note split into many matching sections crowds every other note
/// out of the results. Input must already be ranked best-first.
fn dedupe_by_note(
    ranked: Vec<(f64, crate::document::FlattenedChunk)>,
    top_k: usize,
) -> Vec<(f64, crate::document::FlattenedChunk)> {
    use std::collections::HashSet;
    if top_k == 0 {
        return Vec::new();
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (score, chunk) in ranked {
        if seen.insert(chunk.doc_path.clone()) {
            out.push((score, chunk));
            if out.len() == top_k {
                break;
            }
        }
    }
    out
}

fn deduplicate_chunks(
    results: Vec<(f64, crate::document::FlattenedChunk)>,
) -> Vec<(f64, crate::document::FlattenedChunk)> {
    use std::collections::HashMap;

    let original_count = results.len();
    let mut dedup_map: HashMap<crate::document::FlattenedChunk, f64> = HashMap::new();

    for (score, chunk) in results {
        // Keep the chunk with the highest score
        dedup_map
            .entry(chunk)
            .and_modify(|existing_score| {
                if score > *existing_score {
                    *existing_score = score;
                }
            })
            .or_insert(score);
    }

    let mut deduplicated: Vec<(f64, crate::document::FlattenedChunk)> = dedup_map
        .into_iter()
        .map(|(chunk, score)| (score, chunk))
        .collect();
    // The HashMap destroyed the query's ordering; restore best-first so a
    // no-reranker `take(top_k)` keeps the actual top matches.
    deduplicated.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    debug!(
        "After deduplication: {} unique results (from {} total)",
        deduplicated.len(),
        original_count
    );

    deduplicated
}

async fn index_docs_impl(
    collection: &str,
    docs: &[KimunDoc],
    indexed_notes: Option<&HashMap<String, IndexedNote>>,
    rag: Arc<Mutex<KimunRag>>,
) -> anyhow::Result<IndexStats> {
    let mut indexed_count = 0;
    let mut updated_count = 0;
    let mut skipped_count = 0;

    debug!("Starting to store {} chunks", docs.len());

    // Clone the embeddings handle and drop the lock so the (network/CPU-heavy)
    // embed + store below doesn't block every concurrent search/answer request.
    let embeddings = { rag.lock().await.embeddings() };
    let indexed_notes = match indexed_notes {
        Some(inotes) => inotes,
        None => &embeddings.get_indexed_notes(collection).await?,
    };
    debug!("Indexed notes: {}", indexed_notes.len());

    let mut current_batch_pos = 0;
    const BATCH_SIZE: usize = 250;
    let mut batch: Vec<KimunDoc> = Vec::with_capacity(BATCH_SIZE);

    for doc in docs {
        let content_hash = doc.hash.clone();
        let needs_indexing = if let Some(indexed) = indexed_notes.get(&doc.path) {
            let update = indexed.content_hash != content_hash;
            if update {
                // These ones needs to be updated, so we need to remove them first
                // debug!("For updating, deleting embeddings for {}", doc.path);
                embeddings
                    .delete_embeddings(collection, vec![&doc.path])
                    .await?;
                updated_count += 1;
            } else {
                // debug!("Skipping embeddings for {}", doc.path);
                skipped_count += 1;
            }
            update
        } else {
            // debug!("Indexing embeddings for {}", doc.path);
            indexed_count += 1;
            true
        };

        if needs_indexing {
            batch.push(doc.to_owned());

            // Store batch when it reaches BATCH_SIZE
            let batch_len = batch.len();
            if batch_len >= BATCH_SIZE {
                debug!(
                    "Storing batch from {} to {} documents",
                    current_batch_pos,
                    current_batch_pos + batch_len
                );
                embeddings.store_embeddings(collection, &batch).await?;
                batch.clear();
                current_batch_pos += batch_len;
            }
        }
    }

    // Store any remaining documents in the batch
    if !batch.is_empty() {
        debug!(
            "Storing final batch from {} to {} documents",
            current_batch_pos,
            current_batch_pos + batch.len()
        );
        embeddings.store_embeddings(collection, &batch).await?;
    }

    Ok(IndexStats {
        indexed: indexed_count,
        skipped: skipped_count,
        updated: updated_count,
        removed: 0,
        errors: 0,
    })
}

/// Answer implementation using the server-configured LLM.
async fn answer_impl(
    rag: Arc<Mutex<KimunRag>>,
    collection: &str,
    query: &str,
    top_k: usize,
) -> anyhow::Result<(String, Vec<ChunkResult>)> {
    debug!("Answering a question");

    // Query embeddings + grab the reranker and LLM client while holding the
    // lock briefly, then release it before any slow work. The LLM is checked
    // first (defense-in-depth: the handler already rejects a semantic-only
    // server, adr/0022) so we don't run a vector search we'd only throw away.
    let (raw_results, reranker_option, llm_client) = {
        let rag_lock = rag.lock().await;
        let llm_client = rag_lock
            .get_llm_client()
            .ok_or_else(|| anyhow::anyhow!("no LLM configured; this server is semantic-only"))?;
        let results = rag_lock.query_embeddings_raw(collection, query).await?;
        (results, rag_lock.get_reranker(), llm_client)
    }; // Lock released here

    let raw_results = deduplicate_chunks(raw_results);

    // Rerank (CPU-intensive) without the lock.
    let results = if let Some(reranker) = reranker_option {
        reranker.rerank(query, raw_results, top_k).await?
    } else {
        raw_results.into_iter().take(top_k).collect()
    };

    // Slow LLM call, no lock held.
    let answer = llm_client.ask(query, &results).await?;

    let sources: Vec<ChunkResult> = results
        .into_iter()
        .map(|(score, chunk)| ChunkResult {
            path: chunk.doc_path.clone(),
            title: chunk.title.clone(),
            date: chunk.get_date_string(),
            hash: chunk.doc_hash.clone(),
            content: chunk.text,
            similarity_score: score,
        })
        .collect();

    Ok((answer, sources))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FlattenedChunk;

    fn chunk(path: &str, section: &str) -> FlattenedChunk {
        FlattenedChunk {
            doc_path: path.to_string(),
            doc_hash: "h".to_string(),
            title: section.to_string(),
            text: format!("{path}#{section}"),
            date: None,
        }
    }

    #[test]
    fn dedupe_by_note_keeps_best_chunk_per_note_and_caps_at_top_k() {
        // Ranked best-first: note A dominates the top with three sections, then B,
        // then C. Chunk-level top_k=2 would return two A chunks → one note; the
        // note-dedup must instead surface A and B (each note's best chunk).
        let ranked = vec![
            (0.99, chunk("/a.md", "intro")),
            (0.98, chunk("/a.md", "body")),
            (0.97, chunk("/a.md", "end")),
            (0.80, chunk("/b.md", "b1")),
            (0.70, chunk("/c.md", "c1")),
        ];
        let out = dedupe_by_note(ranked, 2);
        assert_eq!(out.len(), 2, "top_k counts NOTES, not chunks");
        assert_eq!(out[0].1.doc_path, "/a.md");
        assert_eq!(out[0].1.title, "intro", "keeps the note's highest-ranked chunk");
        assert_eq!(out[1].1.doc_path, "/b.md");
    }

    #[test]
    fn dedupe_by_note_returns_all_distinct_notes_when_under_top_k() {
        let ranked = vec![
            (0.9, chunk("/a.md", "s")),
            (0.8, chunk("/a.md", "s2")),
            (0.7, chunk("/b.md", "s")),
        ];
        let out = dedupe_by_note(ranked, 20);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1.doc_path, "/a.md");
        assert_eq!(out[1].1.doc_path, "/b.md");
    }

    #[test]
    fn dedupe_by_note_top_k_zero_is_empty() {
        let ranked = vec![(0.9, chunk("/a.md", "s"))];
        assert!(dedupe_by_note(ranked, 0).is_empty());
    }
}
