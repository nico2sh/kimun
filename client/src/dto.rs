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

/// One prior Q&A pair sent as conversation history on `/api/answer`.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryTurn {
    pub question: String,
    pub answer: String,
}

/// Body of `POST /api/embeddings` and `POST /api/answer`.
#[derive(Debug, Serialize)]
pub struct QueryRequest {
    pub vault_id: String,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_size: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<HistoryTurn>,
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
    /// The 1-based ordinal the server assigned this chunk: the `[n]` citation
    /// number for an answer's source, or the rank position for a search hit.
    /// The pairing contract — a consumer keys citations off this, never off vec
    /// position. `0` means the field was absent (an older server that predates
    /// it); the TUI normalizes 0 to the 1-based position at conversion.
    #[serde(default)]
    pub ordinal: usize,
}

/// `GET /health` capability probe.
#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub status: String,
    #[serde(default)]
    pub reranker: bool,
    /// The configured embedder provider, or `None` on an *unconfigured* server
    /// (no embedder → no indexing, no search, adr/0024). Optional for the same
    /// reason as `llm_provider`: the server sends an explicit `null`.
    #[serde(default)]
    pub embedder: Option<String>,
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
    fn query_request_omits_empty_history() {
        let req = QueryRequest {
            vault_id: "v".into(),
            query: "q".into(),
            context_size: None,
            history: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            !json.contains("history"),
            "empty history must not hit the wire: {json}"
        );
    }

    #[test]
    fn query_request_serializes_history_pairs() {
        let req = QueryRequest {
            vault_id: "v".into(),
            query: "q".into(),
            context_size: None,
            history: vec![HistoryTurn {
                question: "q1".into(),
                answer: "a1".into(),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""history":[{"question":"q1","answer":"a1"}]"#));
    }

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
        let json =
            r#"{"status":"ok","reranker":false,"llm_provider":"gemini","auth_required":true}"#;
        let health: Health = serde_json::from_str(json).unwrap();
        assert_eq!(health.llm_provider.as_deref(), Some("gemini"));
        assert!(health.auth_required);
    }

    #[test]
    fn health_parses_unconfigured_null_embedder() {
        // An unconfigured server (no embedder, adr/0024) sends embedder: null.
        let json = r#"{"status":"ok","reranker":true,"embedder":null,"llm_provider":null,"auth_required":false}"#;
        let health: Health = serde_json::from_str(json).expect("must parse null embedder");
        assert!(health.embedder.is_none());
    }

    #[test]
    fn health_parses_configured_embedder() {
        let json = r#"{"status":"ok","reranker":true,"embedder":"fastembed","llm_provider":null,"auth_required":false}"#;
        let health: Health = serde_json::from_str(json).unwrap();
        assert_eq!(health.embedder.as_deref(), Some("fastembed"));
    }

    #[test]
    fn health_tolerates_missing_embedder_field() {
        // An older server without the field must still parse (probe stays green).
        let json =
            r#"{"status":"ok","reranker":true,"llm_provider":"gemini","auth_required":false}"#;
        let health: Health = serde_json::from_str(json).unwrap();
        assert!(health.embedder.is_none());
    }

    #[test]
    fn chunk_result_parses_the_ordinal_when_present() {
        let json = r#"{"path":"a.md","title":"t","date":null,"content":"c","hash":"h","similarity_score":0.9,"ordinal":3}"#;
        let c: ChunkResult = serde_json::from_str(json).unwrap();
        assert_eq!(c.ordinal, 3);
    }

    #[test]
    fn chunk_result_defaults_ordinal_to_zero_when_absent() {
        // An older server omits `ordinal`; parsing must still succeed and leave
        // 0 (the "absent" sentinel the TUI turns into a position fallback).
        let json = r#"{"path":"a.md","title":"t","date":null,"content":"c","hash":"h","similarity_score":0.9}"#;
        let c: ChunkResult = serde_json::from_str(json).unwrap();
        assert_eq!(c.ordinal, 0);
    }
}
