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
    /// The configured LLM provider, or `None` on a semantic-only server (no LLM
    /// → search works, question-answering does not). Must be optional: the
    /// server sends an explicit `null` here, which a plain `String` field —
    /// even with `#[serde(default)]` — fails to deserialize, marking a healthy
    /// semantic-only server as offline.
    #[serde(default)]
    pub llm_provider: Option<String>,
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

/// The `result` payload of a completed answer job.
#[derive(Debug, Deserialize)]
pub struct AnswerResult {
    pub answer: String,
    pub sources: Vec<ChunkResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_parses_semantic_only_null_llm_provider() {
        // A semantic-only server sends `llm_provider: null`. The probe must still
        // parse (server reachable → online), so search stays available even with
        // no LLM configured.
        let json = r#"{"status":"ok","reranker":true,"llm_provider":null,"auth_required":false}"#;
        let health: Health = serde_json::from_str(json).expect("must parse null llm_provider");
        assert_eq!(health.status, "ok");
        assert!(health.llm_provider.is_none());
    }

    #[test]
    fn health_parses_configured_llm_provider() {
        let json = r#"{"status":"ok","reranker":false,"llm_provider":"gemini","auth_required":true}"#;
        let health: Health = serde_json::from_str(json).unwrap();
        assert_eq!(health.llm_provider.as_deref(), Some("gemini"));
        assert!(health.auth_required);
    }
}
