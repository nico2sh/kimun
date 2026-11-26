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
    /// Date-typed frontmatter properties, one per property.
    pub dates: Vec<WireDate>,
    /// The note's labels (inline `#hashtags` and frontmatter `tags`).
    pub tags: Vec<String>,
    /// As-written targets of the note's links to other notes.
    pub links: Vec<String>,
}

/// A date a note carries: the lowercased property key and its day as
/// `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireDate {
    pub source: String,
    pub day: String,
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

/// Body of `PUT /api/collections/{vault_id}/label`.
#[derive(Debug, Serialize)]
pub struct LabelRequest {
    pub label: String,
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
    /// The client's local date as `YYYY-MM-DD`; sent on `/api/answer` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today: Option<String>,
    /// Search filters; sent on `/api/embeddings` only, and only when set, so
    /// an unfiltered request is byte-identical to one from an older client.
    #[serde(flatten)]
    pub filter: SearchFilter,
}

/// Narrows a search to notes with a date in a range, in a folder, or with all
/// the given tags. Every part is optional; an empty filter changes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SearchFilter {
    /// First day of the range. The server needs it together with `date_to`.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt_day")]
    pub date_from: Option<chrono::NaiveDate>,
    /// Last day of the range, inclusive.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt_day")]
    pub date_to: Option<chrono::NaiveDate>,
    /// Which date property the range applies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_key: Option<String>,
    /// Only notes under this folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Only notes carrying all of these tags.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl SearchFilter {
    /// True when no part is set.
    pub fn is_empty(&self) -> bool {
        self.date_from.is_none()
            && self.date_to.is_none()
            && self.date_key.is_none()
            && self.folder.is_none()
            && self.tags.is_empty()
    }
}

/// Serializes an optional day as `YYYY-MM-DD`.
mod opt_day {
    use serde::Serializer;

    pub fn serialize<S: Serializer>(
        day: &Option<chrono::NaiveDate>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match day {
            Some(d) => s.serialize_str(&d.format("%Y-%m-%d").to_string()),
            None => s.serialize_none(),
        }
    }
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
    /// (no embedder → no indexing, no search). Optional for the same
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
    /// The server's own version. `None` means a server that predates version
    /// reporting — necessarily outdated.
    #[serde(default)]
    pub version: Option<String>,
    /// A newer stable server release, when the server's own update check found
    /// one; `None` otherwise (and on servers too old to report it).
    #[serde(default)]
    pub latest_version: Option<String>,
    /// Whether the server accepts search filters. Absent on older servers.
    #[serde(default)]
    pub search_filters: bool,
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
    /// What the assistant did, step by step. Absent from older servers.
    #[serde(default)]
    pub trail: Vec<String>,
}

/// One event of a streamed answer (`POST /api/answer/stream`). A stream is any
/// number of `Step`s, then exactly one `Answer` or `Error`.
#[derive(Debug)]
pub enum AnswerEvent {
    /// One trail step, as the assistant takes it.
    Step(String),
    /// The finished answer; always the last event of a successful stream.
    Answer(AnswerResult),
    /// The answer failed (the server's message, or a broken stream); last.
    Error(String),
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
            today: None,
            filter: SearchFilter::default(),
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
            today: Some("2026-11-25".into()),
            filter: SearchFilter::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""history":[{"question":"q1","answer":"a1"}]"#));
        assert!(json.contains(r#""today":"2026-11-25""#));
    }

    #[test]
    fn query_request_omits_absent_today() {
        let req = QueryRequest {
            vault_id: "v".into(),
            query: "q".into(),
            context_size: None,
            history: vec![],
            today: None,
            filter: SearchFilter::default(),
        };
        assert!(!serde_json::to_string(&req).unwrap().contains("today"));
    }

    #[test]
    fn answer_result_parses_with_and_without_trail() {
        let with: AnswerResult =
            serde_json::from_str(r#"{"answer":"a","sources":[],"trail":["Searched"]}"#).unwrap();
        assert_eq!(with.trail, vec!["Searched".to_string()]);
        let without: AnswerResult = serde_json::from_str(r#"{"answer":"a","sources":[]}"#).unwrap();
        assert!(without.trail.is_empty());
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
        // An unconfigured server (no embedder) sends embedder: null.
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
