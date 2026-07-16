use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

use crate::{
    CollectionKey, RagError, ScoredChunk,
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
    /// `reranker.top_k` from config is used. One exception: `answer` with no
    /// active reranker ignores it — the configured context cut sizes the LLM
    /// context from the pool's score shape instead (adr/0027). `search`
    /// always honors it.
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
    /// Wall-clock duration of the whole search pipeline (query embedding +
    /// store search + rerank), in milliseconds.
    pub query_time_ms: u64,
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

impl From<ScoredChunk> for ChunkResult {
    fn from((score, chunk): ScoredChunk) -> Self {
        ChunkResult {
            path: chunk.doc_path.clone(),
            title: chunk.title.clone(),
            date: chunk.get_date_string(),
            hash: chunk.doc_hash.clone(),
            content: chunk.text,
            similarity_score: score,
        }
    }
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

/// The one place a [`RagError`] becomes HTTP: status per variant, body always
/// `{"error": "..."}`. Handlers return `Result<Json<T>, RagError>` and use `?`;
/// they never build status tuples by hand.
impl IntoResponse for RagError {
    fn into_response(self) -> Response {
        let status = match &self {
            RagError::Validation(_) => StatusCode::BAD_REQUEST,
            RagError::NotFound(_) => StatusCode::NOT_FOUND,
            RagError::SemanticOnly | RagError::Unconfigured => StatusCode::SERVICE_UNAVAILABLE,
            RagError::Backend(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(ErrorResponse {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}

// ============================================================================
// Handlers
// ============================================================================

pub async fn index_docs_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<IndexDocsRequest>,
) -> Result<Json<IndexResponse>, RagError> {
    let collection = CollectionKey::parse(&request.vault_id)?;
    let rag = state.rag()?.clone();
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
        match rag.index(&collection, &request.docs).await {
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
) -> Result<Json<EmbeddingsResponse>, RagError> {
    let collection = CollectionKey::parse(&request.vault_id)?;
    let top_k = resolve_top_k(request.context_size, state.config.reranker.top_k);

    let started = std::time::Instant::now();
    let results = state
        .rag()?
        .search(&collection, &request.query, top_k)
        .await?;
    let query_time_ms = started.elapsed().as_millis() as u64;

    let chunks: Vec<ChunkResult> = results.into_iter().map(ChunkResult::from).collect();

    Ok(Json(EmbeddingsResponse {
        chunks,
        query_time_ms,
    }))
}

/// Answer - Query text → LLM answer with context (queued). The LLM is the one
/// configured on the server (adr: server-owned LLM config); the request carries
/// no provider/model/key.
pub async fn answer_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<QueryResponse>, RagError> {
    let collection = CollectionKey::parse(&request.vault_id)?;
    let rag = state.rag()?.clone();
    // Semantic-only server: reject question-answering up front rather than minting
    // a job that can only fail (adr/0022). The client already gates on
    // /health.llm_provider; this is the belt-and-suspenders path. The pipeline
    // is the truth here, not the config — it holds the actually-constructed LLM.
    if !rag.can_answer() {
        return Err(RagError::SemanticOnly);
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

        match rag.answer(&collection, &request.query, top_k).await {
            Ok(answer) => {
                let sources: Vec<ChunkResult> =
                    answer.sources.into_iter().map(ChunkResult::from).collect();
                let result = serde_json::json!({
                    "answer": answer.text,
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
) -> Result<Json<IndexResponse>, RagError> {
    let collection = CollectionKey::parse(&request.vault_id)?;
    // Serialize with index writes so a delete can't interleave with a store on
    // the same collection (partial-visibility / lost updates in the store).
    let _index_guard = state.index_lock.lock().await;
    state
        .rag()?
        .delete_notes(&collection, &request.paths)
        .await?;
    Ok(Json(IndexResponse {
        job_id: Uuid::new_v4().to_string(),
        message: format!("Deleted {} paths", request.paths.len()),
    }))
}

/// Reconcile support: the `{note path → content hash}` set the server holds for
/// a vault, so the client can diff it against its own authoritative set and
/// push/delete only the differences (adr/0019).
pub async fn collection_hashes_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(vault_id): axum::extract::Path<String>,
) -> Result<Json<HashMap<String, String>>, RagError> {
    let collection = CollectionKey::parse(&vault_id)?;
    Ok(Json(state.rag()?.note_hashes(&collection).await?))
}

/// Job Status - Get status of a job
pub async fn job_status_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(job_id_str): axum::extract::Path<String>,
) -> Result<Json<JobStatusResponse>, RagError> {
    let job_id = Uuid::parse_str(&job_id_str)
        .map_err(|_| RagError::Validation("Invalid job ID format".to_string()))?;

    let tracker = state.job_tracker.lock().await;

    let job = tracker
        .get(&job_id)
        .ok_or_else(|| RagError::NotFound(format!("Job {} not found", job_id_str)))?;

    let (result, error) = match &job.status {
        JobStatus::Completed => (
            job.result
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            None,
        ),
        JobStatus::Failed => (None, job.error.clone()),
        JobStatus::Queued | JobStatus::Processing => (None, None),
    };

    Ok(Json(JobStatusResponse {
        job_id: job_id_str,
        status: job.status.as_str().to_string(),
        result,
        error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status table is the wire contract (the client keys on codes, never on
    /// message text): Validation→400, NotFound→404, SemanticOnly→503, Backend→500.
    #[test]
    fn rag_error_maps_to_the_contracted_status_codes() {
        let cases = [
            (RagError::Validation("bad".into()), StatusCode::BAD_REQUEST),
            (RagError::NotFound("gone".into()), StatusCode::NOT_FOUND),
            (RagError::SemanticOnly, StatusCode::SERVICE_UNAVAILABLE),
            (RagError::Unconfigured, StatusCode::SERVICE_UNAVAILABLE),
            (
                RagError::Backend(anyhow::anyhow!("boom")),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.into_response().status(), expected);
        }
    }

    #[tokio::test]
    async fn rag_error_body_is_the_error_json_shape() {
        let resp = RagError::Validation("vault_id must be non-empty".into()).into_response();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"], "vault_id must be non-empty");
    }

    #[tokio::test]
    async fn data_handlers_reject_unconfigured_with_503() {
        // An unconfigured server has no pipeline: every data endpoint rejects
        // before doing any work (adr/0024). /health and /api/job stay live.
        use crate::config::RagConfig;
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\n").unwrap();
        let state = Arc::new(AppState::new(None, config));

        let err = get_embeddings_handler(
            State(state.clone()),
            Json(QueryRequest {
                vault_id: "vault-1".into(),
                query: "q".into(),
                context_size: None,
            }),
        )
        .await
        .expect_err("unconfigured must reject search");
        assert!(matches!(err, RagError::Unconfigured));
        assert_eq!(
            err.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        let err = index_docs_handler(
            State(state.clone()),
            Json(IndexDocsRequest {
                vault_id: "vault-1".into(),
                docs: vec![],
            }),
        )
        .await
        .expect_err("unconfigured must reject indexing");
        assert!(matches!(err, RagError::Unconfigured));

        let err = answer_handler(
            State(state.clone()),
            Json(AnswerRequest {
                vault_id: "vault-1".into(),
                query: "q".into(),
                context_size: None,
            }),
        )
        .await
        .expect_err("unconfigured must reject answering");
        assert!(matches!(err, RagError::Unconfigured));

        let err = index_delete_handler(
            State(state.clone()),
            Json(DeleteRequest {
                vault_id: "vault-1".into(),
                paths: vec!["a.md".into()],
            }),
        )
        .await
        .expect_err("unconfigured must reject deletes");
        assert!(matches!(err, RagError::Unconfigured));

        let err =
            collection_hashes_handler(State(state), axum::extract::Path("vault-1".to_string()))
                .await
                .expect_err("unconfigured must reject hash reads");
        assert!(matches!(err, RagError::Unconfigured));
    }

    #[test]
    fn collection_key_rejects_blank_and_unsafe_ids() {
        assert!(matches!(
            CollectionKey::parse(""),
            Err(RagError::Validation(_))
        ));
        assert!(matches!(
            CollectionKey::parse("../escape"),
            Err(RagError::Validation(_))
        ));
        // "__" is reserved for server metadata collections (the embedder
        // fingerprint, adr/0025) — a vault id there would collide with the
        // qdrant fingerprint collection and be hidden from listings/wipes.
        assert!(matches!(
            CollectionKey::parse("__fingerprint"),
            Err(RagError::Validation(_))
        ));
        assert!(matches!(
            CollectionKey::parse("__x"),
            Err(RagError::Validation(_))
        ));
        // A single leading underscore is still a valid id character.
        assert!(CollectionKey::parse("_vault").is_ok());
        let key = CollectionKey::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(key.as_str(), "550e8400-e29b-41d4-a716-446655440000");
    }
}
