//! The component inside Kimün that owns every dealing with the RAG server:
//! connection/capability probing, pushing note changes, and hash-diff
//! reconciliation (see CONTEXT.md, "Server client"). Core stays network-free;
//! it feeds this module only through the [`observer`] seam, which reports a
//! path, a content hash and upsert-or-delete, and knows nothing of RAG.
//!
//! Published as the `kimun_server_client` crate until kimun-notes went to
//! crates.io — which forced every path dependency of a published crate to be
//! published too, and this one was on crates.io for no reason of its own. Like
//! the `ropetext` module it stays self-contained, so it can go back out if a
//! second consumer (a GUI) ever wants it: nothing here may name `crate::`
//! outside `crate::server_client::`, checked in CI. `kimun_core` is the one
//! kimün dependency it keeps, and it would keep that as a crate too.

use std::collections::HashMap;
use std::time::Duration;

pub mod dto;
pub mod observer;
pub mod reconcile;
pub mod sse;

pub mod sync;

use async_trait::async_trait;
use dto::{
    DeleteRequest, EmbeddingsResponse, Health, HistoryTurn, IndexDocsRequest, JobAccepted,
    JobStatus, LabelRequest, QueryRequest, WireDoc,
};

pub use dto::{AnswerEvent, AnswerResult, ChunkResult, SearchFilter, WireSection};
pub use observer::{DirtyOp, DirtySet, RagObserver};
pub use reconcile::{ReconcilePlan, diff as reconcile_diff};

#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned {status}: {body}")]
    Status { status: u16, body: String },
    /// A well-formed HTTP exchange that violated the expected protocol (job
    /// failed/timed out, unparseable result).
    #[error("{0}")]
    Protocol(String),
}

impl RagError {
    /// Whether this is an authentication/authorization rejection (401/403) —
    /// a token problem, not an unreachable server. Callers should surface it
    /// as such instead of folding it into a generic "offline".
    pub fn is_auth(&self) -> bool {
        matches!(
            self,
            RagError::Status {
                status: 401 | 403,
                ..
            }
        )
    }
}

/// The subset of server operations the sync orchestration depends on, behind a
/// trait so it can be exercised with a fake in tests. [`RagClient`] is the real
/// implementation.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait RagTransport: Send + Sync {
    async fn push_docs(&self, docs: Vec<WireDoc>) -> Result<(), RagError>;
    async fn delete_paths(&self, paths: Vec<String>) -> Result<(), RagError>;
    async fn server_hashes(&self) -> Result<HashMap<String, String>, RagError>;
}

#[async_trait]
impl RagTransport for RagClient {
    async fn push_docs(&self, docs: Vec<WireDoc>) -> Result<(), RagError> {
        // The server indexes in the background; an accepted push is enough for
        // the drain path (reconciliation catches any server-side failure).
        RagClient::push_docs(self, docs).await.map(|_job_id| ())
    }
    async fn delete_paths(&self, paths: Vec<String>) -> Result<(), RagError> {
        RagClient::delete_paths(self, paths).await
    }
    async fn server_hashes(&self) -> Result<HashMap<String, String>, RagError> {
        RagClient::server_hashes(self).await
    }
}

/// The query half of the server API — semantic `search` and `ask` — behind a
/// trait so a test (in this crate or another, such as the GUI) can stand in for
/// the server. A [`RagClient`] built with [`RagClient::with_transport`] answers
/// those two calls from the stub instead of over HTTP; every other call on such
/// a client fails (it has no base URL).
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait QueryTransport: Send + Sync {
    async fn search(
        &self,
        query: &str,
        context_size: Option<ContextSize>,
    ) -> Result<Vec<ChunkResult>, RagError>;
    async fn ask(
        &self,
        query: &str,
        history: &[(String, String)],
        context_size: Option<ContextSize>,
    ) -> Result<AnswerResult, RagError>;

    /// Streamed variant of [`QueryTransport::ask`]. The default runs `ask` and
    /// emits one `Answer` or `Error` event, so a stub need not implement it.
    async fn ask_stream(
        &self,
        query: &str,
        history: &[(String, String)],
        context_size: Option<ContextSize>,
    ) -> Result<tokio::sync::mpsc::Receiver<AnswerEvent>, RagError> {
        let event = match self.ask(query, history, context_size).await {
            Ok(answer) => AnswerEvent::Answer(answer),
            Err(e) => AnswerEvent::Error(e.to_string()),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let _ = tx.send(event).await;
        Ok(rx)
    }
}

/// The single place a note's content hash is turned into its wire/reconcile
/// string. Both the pushed [`WireDoc::hash`](dto::WireDoc) and the reconcile
/// `local` hash set MUST go through this — string equality in
/// [`reconcile::diff`] only holds if the two are byte-identical.
pub fn hash_string(hash: u64) -> String {
    hash.to_string()
}

/// Number of results to request. Maps to the server's `context_size` variants,
/// so a caller can't send an invalid string (which the server rejects with 400).
#[derive(Debug, Clone, Copy)]
pub enum ContextSize {
    Small,
    Medium,
    Large,
}

impl ContextSize {
    fn as_str(self) -> &'static str {
        match self {
            ContextSize::Small => "small",
            ContextSize::Medium => "medium",
            ContextSize::Large => "large",
        }
    }
}

/// Bound on establishing a TCP/TLS connection. Without it, a black-holing
/// host (firewalled port, sleeping machine) hangs a probe for the OS default
/// (minutes) instead of failing over to "offline" promptly.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Bound on a whole request/response exchange. reqwest has NO default here —
/// a server that accepts the connection but never answers would hang forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Looser bound for [`RagClient::push_docs`]: a first sync of a large vault
/// uploads every document in one request, which can legitimately outlast
/// [`REQUEST_TIMEOUT`] on a slow link.
const PUSH_TIMEOUT: Duration = Duration::from_secs(120);

/// One process-wide HTTP client, so every [`RagClient`] — however short-lived —
/// shares the same connection pool and keep-alive connections instead of
/// paying a fresh TCP+TLS handshake per construction.
fn shared_http() -> reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            // Only fails on broken TLS backend/system config; no meaningful
            // recovery, and it would fail identically for every request.
            .expect("build HTTP client")
    })
    .clone()
}

/// HTTP client for one vault's collection on a RAG server.
#[derive(Clone)]
pub struct RagClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
    vault_id: String,
    /// A stand-in for the server's query endpoints (`search`, `ask`); `None`
    /// for every real client. See [`RagClient::with_transport`].
    stub: Option<std::sync::Arc<dyn QueryTransport>>,
}

impl RagClient {
    /// `base_url` like `http://host:7573`; `token` is the bearer token if the
    /// server requires one; `vault_id` selects this vault's collection.
    pub fn new(
        base_url: impl Into<String>,
        token: Option<String>,
        vault_id: impl Into<String>,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            http: shared_http(),
            base_url,
            token,
            vault_id: vault_id.into(),
            stub: None,
        }
    }

    /// A client whose `search` and `ask` are answered by `transport` instead of
    /// a server — for tests of the Ask and semantic-search surfaces, in this
    /// crate or another (`cfg(test)` items are not visible across crates). It
    /// has no base URL, so any other call (health, push, delete, hashes) fails
    /// with an HTTP error.
    pub fn with_transport(transport: std::sync::Arc<dyn QueryTransport>) -> Self {
        Self {
            http: shared_http(),
            base_url: String::new(),
            token: None,
            vault_id: String::new(),
            stub: Some(transport),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Attaches the bearer token when configured.
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    /// Turns a non-2xx response into a [`RagError::Status`], else yields the
    /// response for JSON decoding.
    async fn ok(resp: reqwest::Response) -> Result<reqwest::Response, RagError> {
        if resp.status().is_success() {
            Ok(resp)
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(RagError::Status { status, body })
        }
    }

    /// Probes `GET /health` for reachability + capabilities.
    pub async fn health(&self) -> Result<Health, RagError> {
        let resp = self.auth(self.http.get(self.url("/health"))).send().await?;
        Ok(Self::ok(resp).await?.json::<Health>().await?)
    }

    /// Pushes documents to this vault's collection; returns the server's job id.
    pub async fn push_docs(&self, docs: Vec<WireDoc>) -> Result<String, RagError> {
        let body = IndexDocsRequest {
            vault_id: self.vault_id.clone(),
            docs,
        };
        let resp = self
            .auth(self.http.post(self.url("/api/index/docs")).json(&body))
            .timeout(PUSH_TIMEOUT)
            .send()
            .await?;
        Ok(Self::ok(resp).await?.json::<JobAccepted>().await?.job_id)
    }

    /// Deletes notes by path from this vault's collection.
    pub async fn delete_paths(&self, paths: Vec<String>) -> Result<(), RagError> {
        let body = DeleteRequest {
            vault_id: self.vault_id.clone(),
            paths,
        };
        let resp = self
            .auth(self.http.post(self.url("/api/index/delete")).json(&body))
            .send()
            .await?;
        Self::ok(resp).await?;
        Ok(())
    }

    /// The server's `{note-path → hash}` set for this vault (reconcile input).
    ///
    /// `vault_id` is interpolated into the URL path un-encoded; this is safe
    /// because it is always a UUID (from `.kimun/vault-id`) and thus
    /// URL-safe. If that ever changes, percent-encode the segment here.
    pub async fn server_hashes(&self) -> Result<HashMap<String, String>, RagError> {
        let path = format!("/api/collections/{}/hashes", self.vault_id);
        let resp = self.auth(self.http.get(self.url(&path))).send().await?;
        Ok(Self::ok(resp)
            .await?
            .json::<HashMap<String, String>>()
            .await?)
    }

    /// Registers this vault's **vault label** (the workspace name) with the
    /// server, so its web UI can show which collection is which. An older
    /// server without the route answers 404 or 405. That counts as done,
    /// because a retry cannot succeed.
    ///
    /// `vault_id` is interpolated into the URL path un-encoded, as in
    /// [`Self::server_hashes`].
    pub async fn put_collection_label(&self, label: &str) -> Result<(), RagError> {
        let (path, body) = label_request(&self.vault_id, label);
        let resp = self
            .auth(self.http.put(self.url(&path)).json(&body))
            .send()
            .await?;
        let status = resp.status().as_u16();
        if resp.status().is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        label_status(status, body)
    }

    /// Semantic search: returns the matching chunks (no LLM). `context_size`
    /// omitted uses the server's configured default.
    pub async fn search(
        &self,
        query: &str,
        context_size: Option<ContextSize>,
    ) -> Result<Vec<ChunkResult>, RagError> {
        self.search_filtered(query, &SearchFilter::default(), context_size)
            .await
    }

    /// Semantic search narrowed by `filter`. A non-empty filter first probes
    /// `GET /health` and refuses, without sending the search, when the server
    /// does not report `search_filters`. An empty filter sends the same
    /// request as an older client.
    pub async fn search_filtered(
        &self,
        query: &str,
        filter: &SearchFilter,
        context_size: Option<ContextSize>,
    ) -> Result<Vec<ChunkResult>, RagError> {
        if let Some(stub) = &self.stub {
            if !filter.is_empty() {
                return Err(RagError::Protocol(
                    "This search stand-in cannot filter a search.".into(),
                ));
            }
            return stub.search(query, context_size).await;
        }
        if !filter.is_empty() && !self.health().await?.search_filters {
            return Err(RagError::Protocol(
                "This server is too old to filter a search. Update the server.".into(),
            ));
        }
        let body = QueryRequest {
            vault_id: self.vault_id.clone(),
            query: query.to_string(),
            context_size: context_size.map(|c| c.as_str().to_string()),
            history: vec![],
            today: None,
            filter: filter.clone(),
        };
        let resp = self
            .auth(self.http.post(self.url("/api/embeddings")).json(&body))
            .send()
            .await?;
        Ok(Self::ok(resp)
            .await?
            .json::<EmbeddingsResponse>()
            .await?
            .chunks)
    }

    /// Submits a question, polls the job to completion, and returns the LLM
    /// answer plus its cited source chunks.
    pub async fn ask(
        &self,
        query: &str,
        history: &[(String, String)],
        context_size: Option<ContextSize>,
    ) -> Result<AnswerResult, RagError> {
        if let Some(stub) = &self.stub {
            return stub.ask(query, history, context_size).await;
        }
        let body = answer_request(
            &self.vault_id,
            query,
            history,
            context_size,
            chrono::Local::now().date_naive(),
        );
        let resp = self
            .auth(self.http.post(self.url("/api/answer")).json(&body))
            .send()
            .await?;
        let job_id = Self::ok(resp).await?.json::<JobAccepted>().await?.job_id;
        self.poll_answer(&job_id).await
    }

    /// Streams a question's answer over `POST /api/answer/stream`: the events
    /// arrive on the returned channel (poll it with `try_recv`/`recv`) as the
    /// server produces them — `Step`s, then exactly one `Answer` or `Error`,
    /// after which the channel closes. A rejection before the stream starts
    /// (bad vault, semantic-only server, auth) is an `Err`, as for [`Self::ask`].
    /// Dropping the receiver stops the reader; the server finishes the answer
    /// on its own.
    ///
    /// The request carries its own overall timeout of `Self::ANSWER_POLL_ATTEMPTS`
    /// seconds (the same ~12 minute budget `ask` polls for), not the 30 second
    /// `REQUEST_TIMEOUT` of the short calls: reqwest's timeout covers reading
    /// the body, and an answer can take minutes. A stream that ends without a
    /// final event (dropped connection, timeout) yields an `Error` event.
    pub async fn ask_stream(
        &self,
        query: &str,
        history: &[(String, String)],
        context_size: Option<ContextSize>,
    ) -> Result<tokio::sync::mpsc::Receiver<AnswerEvent>, RagError> {
        if let Some(stub) = &self.stub {
            return stub.ask_stream(query, history, context_size).await;
        }
        let body = answer_request(
            &self.vault_id,
            query,
            history,
            context_size,
            chrono::Local::now().date_naive(),
        );
        let resp = self
            .auth(self.http.post(self.url("/api/answer/stream")).json(&body))
            .timeout(Duration::from_secs(u64::from(Self::ANSWER_POLL_ATTEMPTS)))
            .send()
            .await?;
        let resp = Self::ok(resp).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(read_answer_stream(resp, tx));
        Ok(rx)
    }

    /// Poll iterations before giving up on an answer job, at ~1s each: ~12
    /// minutes, safely inside the server's 15-minute job retention (see
    /// `server_state.rs`) so a slow LLM (large context, CPU-only local model
    /// — legitimately several minutes) isn't cut off with a false timeout
    /// while its result is still coming. Keep this below the retention window:
    /// past it the job is swept and polling can never succeed.
    const ANSWER_POLL_ATTEMPTS: u32 = 720;

    /// Polls `/api/job/{id}` until the answer job completes or fails.
    async fn poll_answer(&self, job_id: &str) -> Result<AnswerResult, RagError> {
        let path = format!("/api/job/{job_id}");
        let mut consecutive_errors = 0u32;
        for _ in 0..Self::ANSWER_POLL_ATTEMPTS {
            // A transient poll error (network blip, server briefly busy) must not
            // abort an answer that is still being generated — retry, and only
            // give up after a run of consecutive failures.
            let status = match self.poll_once(&path).await {
                Ok(s) => {
                    consecutive_errors = 0;
                    s
                }
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= 15 {
                        return Err(e);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            match status.status.as_str() {
                "completed" => {
                    let result = status
                        .result
                        .ok_or_else(|| RagError::Protocol("completed job had no result".into()))?;
                    return serde_json::from_value::<AnswerResult>(result)
                        .map_err(|e| RagError::Protocol(format!("bad answer result: {e}")));
                }
                "failed" => {
                    return Err(RagError::Protocol(
                        status.error.unwrap_or_else(|| "answer job failed".into()),
                    ));
                }
                _ => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            }
        }
        Err(RagError::Protocol("answer job timed out".into()))
    }

    /// One poll of the job status endpoint.
    async fn poll_once(&self, path: &str) -> Result<JobStatus, RagError> {
        let resp = self.auth(self.http.get(self.url(path))).send().await?;
        Ok(Self::ok(resp).await?.json::<JobStatus>().await?)
    }
}

/// Reads the SSE body chunk by chunk and forwards each event; stops after the
/// final event or when the receiver is gone. A body that ends (or fails)
/// before a final event is reported as an `Error` event.
async fn read_answer_stream(
    mut resp: reqwest::Response,
    tx: tokio::sync::mpsc::Sender<AnswerEvent>,
) {
    let mut parser = sse::SseParser::default();
    loop {
        let events = match resp.chunk().await {
            Ok(Some(chunk)) => parser.push(&chunk),
            Ok(None) => {
                let rest = parser.finish();
                if !forward(&tx, rest).await {
                    return;
                }
                let _ = tx
                    .send(AnswerEvent::Error(
                        "the answer stream ended before an answer".into(),
                    ))
                    .await;
                return;
            }
            Err(e) => {
                let _ = tx
                    .send(AnswerEvent::Error(format!("answer stream failed: {e}")))
                    .await;
                return;
            }
        };
        if !forward(&tx, events).await {
            return;
        }
    }
}

/// Sends the mapped events; `false` once the stream is over (a final event
/// went out, or the receiver was dropped).
async fn forward(tx: &tokio::sync::mpsc::Sender<AnswerEvent>, events: Vec<sse::SseEvent>) -> bool {
    for event in events {
        let Some(mapped) = sse::to_answer_event(&event) else {
            continue;
        };
        let last = matches!(mapped, AnswerEvent::Answer(_) | AnswerEvent::Error(_));
        if tx.send(mapped).await.is_err() || last {
            return false;
        }
    }
    true
}

/// The path and body of the vault label request.
fn label_request(vault_id: &str, label: &str) -> (String, LabelRequest) {
    (
        format!("/api/collections/{vault_id}/label"),
        LabelRequest {
            label: label.to_string(),
        },
    )
}

/// The result of a vault label request with this status. A 2xx is done. A 404
/// or 405 comes from an older server without the route, and is done too. Every
/// other status is a [`RagError::Status`].
fn label_status(status: u16, body: String) -> Result<(), RagError> {
    match status {
        200..=299 | 404 | 405 => Ok(()),
        _ => Err(RagError::Status { status, body }),
    }
}

/// The `/api/answer` request body: the question, its history, and the
/// client's local date so "yesterday" means the user's yesterday.
fn answer_request(
    vault_id: &str,
    query: &str,
    history: &[(String, String)],
    context_size: Option<ContextSize>,
    today: chrono::NaiveDate,
) -> QueryRequest {
    QueryRequest {
        vault_id: vault_id.to_string(),
        query: query.to_string(),
        context_size: context_size.map(|c| c.as_str().to_string()),
        history: history
            .iter()
            .map(|(q, a)| HistoryTurn {
                question: q.clone(),
                answer: a.clone(),
            })
            .collect(),
        today: Some(today.format("%Y-%m-%d").to_string()),
        filter: SearchFilter::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_request_carries_the_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        let body = answer_request("v", "q", &[("q1".into(), "a1".into())], None, today);
        assert_eq!(body.today.as_deref(), Some("2026-11-25"));
        assert_eq!(body.history.len(), 1);
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains(r#""today":"2026-11-25""#), "{json}");
    }

    #[test]
    fn label_request_puts_the_label_on_the_vault_path() {
        let (path, body) = label_request("vault-1", "kimun-notes");
        assert_eq!(path, "/api/collections/vault-1/label");
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"label":"kimun-notes"}"#
        );
    }

    #[test]
    fn label_status_treats_a_server_without_the_route_as_done() {
        assert!(label_status(204, String::new()).is_ok());
        assert!(label_status(200, String::new()).is_ok());
        assert!(label_status(404, String::new()).is_ok(), "older server");
        assert!(label_status(405, String::new()).is_ok(), "older server");
        let auth = label_status(401, "no".into()).unwrap_err();
        assert!(auth.is_auth(), "{auth}");
        let err = label_status(500, "boom".into()).unwrap_err();
        assert!(
            matches!(err, RagError::Status { status: 500, ref body } if body == "boom"),
            "{err}"
        );
        assert!(label_status(400, String::new()).is_err());
    }

    #[test]
    fn is_auth_matches_only_credential_rejections() {
        let status = |status| RagError::Status {
            status,
            body: String::new(),
        };
        assert!(status(401).is_auth());
        assert!(status(403).is_auth());
        assert!(!status(500).is_auth());
        assert!(!status(404).is_auth());
        assert!(!RagError::Protocol("boom".into()).is_auth());
    }

    /// A one-thread HTTP server for the client tests. It answers `GET /health`
    /// with `health` and `POST /api/embeddings` with no chunks, and records
    /// each request as `"<METHOD> <path> <body>"`.
    fn fake_server(
        health: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if body.len() >= len || n == 0 {
                            break;
                        }
                    } else if n == 0 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let method = first.next().unwrap_or("");
                let path = first.next().unwrap_or("");
                log.lock().unwrap().push(format!("{method} {path} {body}"));
                let reply = if path == "/health" {
                    health
                } else {
                    r#"{"chunks":[]}"#
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.len(),
                    reply
                );
            }
        });
        (url, seen)
    }

    fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[tokio::test]
    async fn search_filtered_sends_fields() {
        let (url, seen) = fake_server(r#"{"status":"ok","search_filters":true}"#);
        let client = RagClient::new(url, None, "v");
        let filter = SearchFilter {
            date_from: Some(day(2026, 11, 1)),
            date_to: Some(day(2026, 11, 30)),
            date_key: Some("due".into()),
            folder: Some("work".into()),
            tags: vec!["a".into(), "b".into()],
        };
        client.search_filtered("q", &filter, None).await.unwrap();
        let log = seen.lock().unwrap();
        assert!(log[0].starts_with("GET /health"), "{log:?}");
        let post = log
            .iter()
            .find(|l| l.starts_with("POST /api/embeddings"))
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(post.strip_prefix("POST /api/embeddings ").unwrap()).unwrap();
        assert_eq!(body["date_from"], "2026-11-01");
        assert_eq!(body["date_to"], "2026-11-30");
        assert_eq!(body["date_key"], "due");
        assert_eq!(body["folder"], "work");
        assert_eq!(body["tags"], serde_json::json!(["a", "b"]));
        assert_eq!(body["query"], "q");
    }

    #[tokio::test]
    async fn search_filtered_refuses_on_old_server() {
        let (url, seen) = fake_server(r#"{"status":"ok"}"#);
        let client = RagClient::new(url, None, "v");
        let filter = SearchFilter {
            folder: Some("work".into()),
            ..Default::default()
        };
        let err = client
            .search_filtered("q", &filter, None)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "This server is too old to filter a search. Update the server."
        );
        let log = seen.lock().unwrap();
        assert!(
            log.iter().all(|l| !l.starts_with("POST")),
            "the search must not be sent: {log:?}"
        );
    }

    #[tokio::test]
    async fn search_with_empty_filter_sends_no_filter_fields() {
        let (url, seen) = fake_server(r#"{"status":"ok"}"#);
        let client = RagClient::new(url, None, "v");
        client.search("q", None).await.unwrap();
        let log = seen.lock().unwrap();
        assert_eq!(log.len(), 1, "no health probe for an empty filter: {log:?}");
        let body = log[0].strip_prefix("POST /api/embeddings ").unwrap();
        for field in ["date_from", "date_to", "date_key", "folder", "tags"] {
            assert!(!body.contains(field), "{field} leaked: {body}");
        }
    }
}
