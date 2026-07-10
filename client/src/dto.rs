//! Wire types mirroring the RAG server's JSON. Defined here (not shared with the
//! server crate) so the client stays independent of the server build.

use serde::{Deserialize, Serialize};

/// Body of `POST /api/index/docs`.
#[derive(Debug, Serialize)]
pub struct IndexDocsRequest {
    pub vault_id: String,
    pub docs: Vec<WireDoc>,
}

/// A note pushed to the server: its path, content hash, and heading sections.
#[derive(Debug, Clone, Serialize)]
pub struct WireDoc {
    pub path: String,
    pub hash: String,
    pub sections: Vec<WireSection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WireSection {
    pub title: String,
    pub text: String,
}

/// Body of `POST /api/index/delete`.
#[derive(Debug, Serialize)]
pub struct DeleteRequest {
    pub vault_id: String,
    pub paths: Vec<String>,
}

/// Body of `POST /api/embeddings` and `POST /api/answer`.
#[derive(Debug, Serialize)]
pub struct QueryRequest {
    pub vault_id: String,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_size: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsResponse {
    pub chunks: Vec<ChunkResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkResult {
    pub path: String,
    pub title: String,
    pub date: Option<String>,
    pub content: String,
    pub hash: String,
    pub similarity_score: f64,
}

/// `GET /health` capability probe.
#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub status: String,
    #[serde(default)]
    pub reranker: bool,
    #[serde(default)]
    pub llm_provider: String,
    #[serde(default)]
    pub auth_required: bool,
}

/// Response to any job-creating endpoint.
#[derive(Debug, Deserialize)]
pub struct JobAccepted {
    pub job_id: String,
}

/// `GET /api/job/{id}`.
#[derive(Debug, Deserialize)]
pub struct JobStatus {
    pub status: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
