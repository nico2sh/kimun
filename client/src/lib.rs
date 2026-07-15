//! `kimun_server_client` — the component inside Kimün that owns every dealing with
//! the RAG server: connection/capability probing, pushing note changes, and
//! hash-diff reconciliation (see CONTEXT.md "RAG client", adr/0018–0021). Core
//! stays network-free; it feeds this crate only through the [`observer`] seam.

use std::collections::HashMap;
use std::time::Duration;

pub mod dto;
pub mod observer;
pub mod reconcile;

pub mod sync;

use async_trait::async_trait;
use dto::{
    AnswerResult, DeleteRequest, EmbeddingsResponse, Health, IndexDocsRequest, JobAccepted,
    JobStatus, QueryRequest, WireDoc,
};

pub use dto::{ChunkResult, WireSection};
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

/// The subset of server operations the sync orchestration depends on, behind a
/// trait so it can be exercised with a fake in tests. [`RagClient`] is the real
/// implementation.
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

/// HTTP client for one vault's collection on a RAG server.
#[derive(Clone)]
pub struct RagClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
    vault_id: String,
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
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            // Only fails on broken TLS backend/system config; no meaningful
            // recovery, and it would fail identically for every request.
            .expect("build HTTP client");
        Self {
            http,
            base_url,
            token,
            vault_id: vault_id.into(),
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
    /// because it is always a UUID (from `.kimun/vault-id`, adr/0020) and thus
    /// URL-safe. If that ever changes, percent-encode the segment here.
    pub async fn server_hashes(&self) -> Result<HashMap<String, String>, RagError> {
        let path = format!("/api/collections/{}/hashes", self.vault_id);
        let resp = self.auth(self.http.get(self.url(&path))).send().await?;
        Ok(Self::ok(resp)
            .await?
            .json::<HashMap<String, String>>()
            .await?)
    }

    /// Semantic search: returns the matching chunks (no LLM). `context_size`
    /// omitted uses the server's configured default.
    pub async fn search(
        &self,
        query: &str,
        context_size: Option<ContextSize>,
    ) -> Result<Vec<ChunkResult>, RagError> {
        let body = QueryRequest {
            vault_id: self.vault_id.clone(),
            query: query.to_string(),
            context_size: context_size.map(|c| c.as_str().to_string()),
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
        context_size: Option<ContextSize>,
    ) -> Result<AnswerResult, RagError> {
        let body = QueryRequest {
            vault_id: self.vault_id.clone(),
            query: query.to_string(),
            context_size: context_size.map(|c| c.as_str().to_string()),
        };
        let resp = self
            .auth(self.http.post(self.url("/api/answer")).json(&body))
            .send()
            .await?;
        let job_id = Self::ok(resp).await?.json::<JobAccepted>().await?.job_id;
        self.poll_answer(&job_id).await
    }

    /// Polls `/api/job/{id}` until the answer job completes or fails. The ceiling
    /// tracks the server's job-retention window (~5 min) so a slow LLM (large
    /// context, local model) isn't cut off with a false timeout while its result
    /// is still coming.
    async fn poll_answer(&self, job_id: &str) -> Result<AnswerResult, RagError> {
        let path = format!("/api/job/{job_id}");
        let mut consecutive_errors = 0u32;
        for _ in 0..300 {
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
