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

/// Push a vault's documents. `vault_id` selects the collection.
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
    /// `reranker.top_k` from config is used. Only meaningful under the
    /// `fixed` context cut — the adaptive cuts size results from the pool's
    /// score shape on both surfaces and ignore it.
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

/// One prior Q&A exchange sent as conversation history. The client strips
/// citation markers before sending; the server passes pairs through verbatim.
#[derive(Debug, Deserialize)]
pub struct HistoryTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub vault_id: String,
    pub query: String,
    #[serde(default)]
    pub context_size: Option<ContextSize>,
    #[serde(default)]
    pub history: Vec<HistoryTurn>,
    /// The client's local date, so "yesterday" means the user's yesterday.
    /// Clamped by `resolve_today`.
    #[serde(default)]
    pub today: Option<chrono::NaiveDate>,
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
    /// The 1-based position this chunk holds in its result list — the `[n]`
    /// citation number for an answer's sources, or the rank position for a
    /// search hit. Assigned by the caller (never re-derived from vec order on
    /// either side of the wire), so the citation↔source pairing survives any
    /// later reorder. Always populated by this server; a `#[serde(default)]` on
    /// the client covers an older server that omits it.
    pub ordinal: usize,
}

impl ChunkResult {
    /// Materialize a wire chunk with its already-assigned `ordinal` — the one
    /// numbering site (answer: the `[n]` the prompt used; search: rank
    /// position) owns the value, this only carries it.
    fn from_scored(ordinal: usize, (score, chunk): ScoredChunk) -> Self {
        ChunkResult {
            path: chunk.doc_path.clone(),
            title: chunk.title.clone(),
            date: chunk.get_date_string(),
            hash: chunk.doc_hash.clone(),
            content: chunk.text,
            similarity_score: score,
            ordinal,
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
    /// How far an indexing job has got, once it knows its amount of work.
    /// Absent for other jobs and for a job that has not started embedding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<crate::server_state::JobProgress>,
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

        // Perform indexing. Progress is best effort: the callback is sync, so it
        // skips an update when the tracker is busy instead of waiting for it.
        let tracker = state_clone.job_tracker.clone();
        let on_progress = move |p: crate::IndexProgress| {
            if let Ok(mut tracker) = tracker.try_lock() {
                tracker.set_progress(
                    &job_id,
                    crate::server_state::JobProgress {
                        done: p.done,
                        total: p.total,
                    },
                );
            }
        };
        match rag
            .index_with_progress(&collection, &request.docs, &on_progress)
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
) -> Result<Json<EmbeddingsResponse>, RagError> {
    let collection = CollectionKey::parse(&request.vault_id)?;
    let top_k = resolve_top_k(request.context_size, state.config.reranker.top_k);

    let started = std::time::Instant::now();
    let results = state
        .rag()?
        .search(&collection, &request.query, top_k)
        .await?;
    let query_time_ms = started.elapsed().as_millis() as u64;

    // Search ordinal = 1-based rank position (best hit first).
    let chunks: Vec<ChunkResult> = results
        .into_iter()
        .enumerate()
        .map(|(i, scored)| ChunkResult::from_scored(i + 1, scored))
        .collect();

    Ok(Json(EmbeddingsResponse {
        chunks,
        query_time_ms,
    }))
}

/// How far a client-supplied date may sit from the server's date and still be
/// trusted, in days either side.
const TODAY_WINDOW_DAYS: i64 = 400;

/// The date that dates the prompt: the client's when it is within
/// [`TODAY_WINDOW_DAYS`] of the server's, else the server's. A wild value is
/// ignored rather than rejected, and never reaches date arithmetic.
fn resolve_today(
    client: Option<chrono::NaiveDate>,
    server_today: chrono::NaiveDate,
) -> chrono::NaiveDate {
    match client {
        Some(day) if (day - server_today).num_days().abs() <= TODAY_WINDOW_DAYS => day,
        Some(day) => {
            log::debug!("ignoring client date {day}: outside the window around {server_today}");
            server_today
        }
        None => server_today,
    }
}

/// The JSON a completed answer job stores as its result.
fn answer_result_json(answer: crate::Answer) -> String {
    // Ordinal is the `[n]` the prompt numbered each source with, assigned once
    // in `answer()` — carried straight through, never re-derived from this
    // vec's order.
    let sources: Vec<ChunkResult> = answer
        .sources
        .into_iter()
        .map(|(ordinal, scored)| ChunkResult::from_scored(ordinal, scored))
        .collect();
    let trail: Vec<String> = answer.trail.into_iter().map(|s| s.text).collect();
    serde_json::json!({
        "answer": answer.text,
        "sources": sources,
        "trail": trail,
    })
    .to_string()
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
    // a job that can only fail. The client already gates on
    // /health.llm_provider; this is the belt-and-suspenders path. The pipeline
    // is the truth here, not the config — it holds the actually-constructed LLM.
    if !rag.can_answer() {
        return Err(RagError::SemanticOnly);
    }
    let job_id = Uuid::new_v4();
    let top_k = resolve_top_k(request.context_size, state.config.reranker.top_k);
    let history: Vec<(String, String)> = request
        .history
        .into_iter()
        .map(|t| (t.question, t.answer))
        .collect();

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

        let today = resolve_today(request.today, chrono::Local::now().date_naive());
        match rag
            .answer(
                &collection,
                &request.query,
                &history,
                top_k,
                Some(today),
                &|_| {},
            )
            .await
        {
            Ok(answer) => {
                let result = answer_result_json(answer);
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

/// Answer, streamed - the same request as [`answer_handler`], answered as
/// server-sent events instead of a polled job: one `step` event per trail
/// step as the loop takes it (`{"text": "..."}`), then exactly one final
/// event, `answer` (the job result JSON) or `error` (`{"error": "..."}`), then
/// the stream closes. Validation and the semantic-only check reject with the
/// usual status BEFORE the stream starts.
///
/// The loop runs in a spawned task and feeds an unbounded channel (the step
/// callback is sync and must not block). If the client disconnects, the
/// receiver drops and the loop is cancelled at its next await: the task ends
/// without sending anything and makes no more model calls.
pub async fn answer_stream_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnswerRequest>,
) -> Result<impl IntoResponse, RagError> {
    use axum::response::sse::{Event, Sse};
    use tokio_stream::StreamExt;

    let collection = CollectionKey::parse(&request.vault_id)?;
    let rag = state.rag()?.clone();
    if !rag.can_answer() {
        return Err(RagError::SemanticOnly);
    }
    let top_k = resolve_top_k(request.context_size, state.config.reranker.top_k);
    let history: Vec<(String, String)> = request
        .history
        .into_iter()
        .map(|t| (t.question, t.answer))
        .collect();
    let today = resolve_today(request.today, chrono::Local::now().date_naive());

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    tokio::spawn(async move {
        let step_tx = tx.clone();
        let on_step = move |step: &crate::agent::TrailStep| {
            let data = serde_json::json!({ "text": step.text }).to_string();
            let _ = step_tx.send(Event::default().event("step").data(data));
        };
        let answered = rag.answer(
            &collection,
            &request.query,
            &history,
            top_k,
            Some(today),
            &on_step,
        );
        let result = tokio::select! {
            result = answered => result,
            // The client is gone: drop the loop, nobody reads the answer.
            () = tx.closed() => return,
        };
        let last = match result {
            Ok(answer) => Event::default()
                .event("answer")
                .data(answer_result_json(answer)),
            Err(e) => Event::default()
                .event("error")
                .data(serde_json::json!({ "error": e.to_string() }).to_string()),
        };
        let _ = tx.send(last);
    });

    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
        .map(Ok::<Event, std::convert::Infallible>);
    // A comment line every few seconds keeps proxies from closing a stream
    // while the model is slow; the client parser skips comments.
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
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
/// push/delete only the differences.
pub async fn collection_hashes_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(vault_id): axum::extract::Path<String>,
) -> Result<Json<HashMap<String, String>>, RagError> {
    let collection = CollectionKey::parse(&vault_id)?;
    Ok(Json(state.rag()?.note_hashes(&collection).await?))
}

/// The body of `PUT /api/collections/{vault_id}/label`.
#[derive(Debug, Deserialize)]
pub struct VaultLabelRequest {
    pub label: String,
}

/// Records the **vault label** the app registers for a vault, so the web UI
/// can show which collection is which. Answers 204 No Content.
pub async fn collection_label_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(vault_id): axum::extract::Path<String>,
    Json(request): Json<VaultLabelRequest>,
) -> Result<StatusCode, RagError> {
    let collection = CollectionKey::parse(&vault_id)?;
    state
        .rag()?
        .set_vault_label(&collection, &request.label)
        .await?;
    Ok(StatusCode::NO_CONTENT)
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

    Ok(Json(job_response(job_id_str, job)))
}

/// The status body for one job: its result or error once it ended, and its
/// progress while it is known.
fn job_response(job_id: String, job: &crate::server_state::Job) -> JobStatusResponse {
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

    JobStatusResponse {
        job_id,
        status: job.status.as_str().to_string(),
        result,
        error,
        progress: job.progress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pairing contract on the wire: whatever ordinal `answer()` assigned
    /// to each source is the ordinal that reaches the client — even when the
    /// sources vec is in a different order than the ordinals (a stand-in for any
    /// future reorder). Mirrors the exact mapping the answer handler runs.
    #[test]
    fn answer_sources_serialize_the_assigned_ordinal_not_vec_position() {
        use crate::document::FlattenedChunk;
        let flat = |path: &str| FlattenedChunk {
            doc_path: path.to_string(),
            doc_hash: "h".to_string(),
            title: "t".to_string(),
            text: "body".to_string(),
            meta: Default::default(),
        };
        // Deliberately shuffled: vec position 0 carries ordinal 3, etc.
        let sources: Vec<(usize, ScoredChunk)> = vec![
            (3, (0.9, flat("/c.md"))),
            (1, (0.8, flat("/a.md"))),
            (2, (0.7, flat("/b.md"))),
        ];
        let chunks: Vec<ChunkResult> = sources
            .into_iter()
            .map(|(ordinal, scored)| ChunkResult::from_scored(ordinal, scored))
            .collect();
        let json = serde_json::to_value(&chunks).unwrap();
        // Ordinal travels with the chunk, not with its slice position.
        assert_eq!(json[0]["path"], "/c.md");
        assert_eq!(json[0]["ordinal"], 3);
        assert_eq!(json[1]["path"], "/a.md");
        assert_eq!(json[1]["ordinal"], 1);
        assert_eq!(json[2]["path"], "/b.md");
        assert_eq!(json[2]["ordinal"], 2);
    }

    #[test]
    fn answer_request_parses_without_history() {
        let r: AnswerRequest = serde_json::from_str(r#"{"vault_id":"v1","query":"q"}"#).unwrap();
        assert!(r.history.is_empty());
    }

    #[test]
    fn answer_request_parses_today() {
        let r: AnswerRequest =
            serde_json::from_str(r#"{"vault_id":"v1","query":"q","today":"2026-11-25"}"#).unwrap();
        assert_eq!(r.today, chrono::NaiveDate::from_ymd_opt(2026, 11, 25));
        let r: AnswerRequest = serde_json::from_str(r#"{"vault_id":"v1","query":"q"}"#).unwrap();
        assert_eq!(r.today, None);
    }

    fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn resolve_today_trusts_only_a_near_client_date() {
        let server = day(2026, 11, 25);
        assert_eq!(resolve_today(None, server), server);
        assert_eq!(
            resolve_today(Some(day(2026, 11, 24)), server),
            day(2026, 11, 24)
        );
        assert_eq!(
            resolve_today(Some(day(2026, 11, 26)), server),
            day(2026, 11, 26)
        );
        assert_eq!(resolve_today(Some(day(2020, 1, 1)), server), server);
        assert_eq!(resolve_today(Some(day(2040, 1, 1)), server), server);
        assert_eq!(resolve_today(Some(chrono::NaiveDate::MIN), server), server);
        assert_eq!(resolve_today(Some(chrono::NaiveDate::MAX), server), server);
    }

    #[test]
    fn job_response_carries_progress_while_a_job_runs() {
        let mut tracker = crate::server_state::JobTracker::new();
        let id = Uuid::new_v4();
        tracker.create(id, JobStatus::Processing);
        tracker.set_progress(
            &id,
            crate::server_state::JobProgress {
                done: 100,
                total: 250,
            },
        );
        let body =
            serde_json::to_value(job_response(id.to_string(), tracker.get(&id).unwrap())).unwrap();
        assert_eq!(body["status"], "processing");
        assert_eq!(body["progress"]["done"], 100);
        assert_eq!(body["progress"]["total"], 250);
    }

    #[test]
    fn job_response_omits_progress_when_there_is_none() {
        let mut tracker = crate::server_state::JobTracker::new();
        let id = Uuid::new_v4();
        tracker.create(id, JobStatus::Queued);
        let body =
            serde_json::to_value(job_response(id.to_string(), tracker.get(&id).unwrap())).unwrap();
        assert!(body.get("progress").is_none());
    }

    #[test]
    fn answer_result_json_carries_the_trail() {
        let answer = crate::Answer {
            text: "hi".into(),
            sources: vec![],
            trail: vec![crate::agent::TrailStep {
                text: "Searched".into(),
            }],
        };
        let json: serde_json::Value = serde_json::from_str(&answer_result_json(answer)).unwrap();
        assert_eq!(json["answer"], "hi");
        assert_eq!(json["sources"], serde_json::json!([]));
        assert_eq!(json["trail"], serde_json::json!(["Searched"]));
    }

    #[test]
    fn answer_request_parses_history_pairs() {
        let r: AnswerRequest = serde_json::from_str(
            r#"{"vault_id":"v1","query":"q","history":[{"question":"q1","answer":"a1"}]}"#,
        )
        .unwrap();
        assert_eq!(r.history.len(), 1);
        assert_eq!(r.history[0].question, "q1");
        assert_eq!(r.history[0].answer, "a1");
    }

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
        // before doing any work. /health and /api/job stay live.
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
                history: vec![],
                today: None,
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
        // fingerprint) — a vault id there would collide with the
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
