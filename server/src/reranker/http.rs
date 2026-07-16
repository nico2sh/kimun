//! The one HTTP reranker. Cohere v2, Jina, Voyage, and self-hosted vLLM or
//! Infinity all take the same `POST {base}/rerank` request
//! (`{model, query, documents}`) and answer with scored indices; the only
//! variation is the top-level key (`results` vs Voyage's `data`), absorbed by
//! a serde alias. Same pattern as the embedder's `HttpEmbedder`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::document::FlattenedChunk;

use super::{Reranker, rerank_document};

pub struct HttpReranker {
    client: reqwest::Client,
    /// Base URL including the API version where the provider has one (e.g.
    /// `https://api.cohere.com/v2`); `/rerank` is appended.
    url: String,
    /// Optional: single-model self-hosted servers don't need one.
    model: Option<String>,
    /// Bearer token for endpoints that need one.
    api_key: Option<String>,
}

#[derive(Serialize)]
struct RerankRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    query: &'a str,
    documents: Vec<String>,
    // No top_n/top_k: providers rank the full list by default and the caller
    // truncates locally — Cohere/Jina say `top_n`, Voyage says `top_k`, and
    // omitting both keeps one request shape for all of them.
}

#[derive(Deserialize)]
struct RerankResponse {
    /// Cohere/Jina/vLLM say `results`; Voyage says `data`.
    #[serde(alias = "data")]
    results: Vec<RerankResult>,
}

#[derive(Deserialize)]
struct RerankResult {
    /// Position in the submitted `documents` list.
    index: usize,
    /// Cohere/Jina/Voyage say `relevance_score`; some servers say `score`.
    #[serde(alias = "score")]
    relevance_score: f64,
}

impl HttpReranker {
    /// Builds the reranker and probes the endpoint with a one-document request
    /// so a bad URL, key, or model fails at startup (where the server degrades
    /// gracefully) instead of on the first user query.
    pub async fn new(
        url: String,
        model: Option<String>,
        api_key: Option<String>,
    ) -> anyhow::Result<Self> {
        let reranker = Self {
            client: reqwest::Client::new(),
            url,
            model,
            api_key,
        };
        reranker
            .rerank_texts("probe", vec!["connectivity probe".to_string()])
            .await?;
        Ok(reranker)
    }

    async fn rerank_texts(
        &self,
        query: &str,
        documents: Vec<String>,
    ) -> anyhow::Result<Vec<RerankResult>> {
        let endpoint = format!("{}/rerank", self.url.trim_end_matches('/'));
        let mut request = self.client.post(endpoint).json(&RerankRequest {
            model: self.model.as_deref(),
            query,
            documents,
        });
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await?.error_for_status()?;
        let parsed: RerankResponse = response.json().await?;
        Ok(parsed.results)
    }
}

#[async_trait]
impl Reranker for HttpReranker {
    async fn rerank(
        &self,
        query: &str,
        results: &[(f64, FlattenedChunk)],
        top_k: usize,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        if results.is_empty() {
            return Ok(Vec::new());
        }
        let documents = results
            .iter()
            .map(|(_, chunk)| rerank_document(chunk))
            .collect();
        let scored = self.rerank_texts(query, documents).await?;
        super::select_top_chunks(
            scored
                .into_iter()
                .map(|r| (r.index, r.relevance_score))
                .collect(),
            results,
            top_k,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::HeaderMap, routing::post};
    use std::sync::{Arc, Mutex};

    /// Captured request: headers plus the JSON body the reranker sent.
    type Captured = Arc<Mutex<Option<(HeaderMap, serde_json::Value)>>>;

    async fn mock_endpoint(response: serde_json::Value, captured: Captured) -> String {
        let app = Router::new().route(
            "/rerank",
            post(move |headers: HeaderMap, body: String| async move {
                let json = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                *captured.lock().unwrap() = Some((headers, json));
                Json(response)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn chunk(title: &str, text: &str) -> FlattenedChunk {
        FlattenedChunk {
            doc_path: "/a.md".into(),
            doc_hash: "h".into(),
            title: title.into(),
            text: text.into(),
            date: None,
        }
    }

    /// Cohere/Jina shape: `results` + `relevance_score`, out of order — the
    /// reranker must map indices back to chunks and sort by score.
    #[tokio::test]
    async fn cohere_jina_wire_sends_bearer_and_reorders_by_score() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_endpoint(
            serde_json::json!({"results": [
                {"index": 0, "relevance_score": 0.1},
                {"index": 2, "relevance_score": 0.9},
                {"index": 1, "relevance_score": 0.5}
            ]}),
            captured.clone(),
        )
        .await;

        let r = HttpReranker::new(base, Some("rerank-v3.5".into()), Some("co-key".into()))
            .await
            .unwrap();
        let out = r
            .rerank(
                "q",
                &[
                    (0.9, chunk("A", "a")),
                    (0.8, chunk("B", "b")),
                    (0.7, chunk("C", "c")),
                ],
                2,
            )
            .await
            .unwrap();

        assert_eq!(out.len(), 2, "truncated to top_k");
        assert_eq!(out[0].1.title, "C", "highest relevance first");
        assert_eq!(out[0].0, 0.9);
        assert_eq!(out[1].1.title, "B");

        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["authorization"], "Bearer co-key");
        assert_eq!(body["model"], "rerank-v3.5");
        assert_eq!(body["query"], "q");
        assert_eq!(body["documents"][0], "A\na", "same title+body shape the embedder indexes");
        assert!(
            body.get("top_n").is_none() && body.get("top_k").is_none(),
            "no server-side cut — providers disagree on the parameter name"
        );
    }

    /// Voyage shape: `data` instead of `results` — absorbed by the alias.
    #[tokio::test]
    async fn voyage_wire_parses_data_key_without_model_or_auth() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_endpoint(
            serde_json::json!({"data": [
                {"index": 1, "relevance_score": 0.8},
                {"index": 0, "relevance_score": 0.2}
            ]}),
            captured.clone(),
        )
        .await;

        let r = HttpReranker::new(base, None, None).await.unwrap();
        let out = r
            .rerank("q", &[(0.9, chunk("A", "a")), (0.8, chunk("B", "b"))], 10)
            .await
            .unwrap();

        assert_eq!(out[0].1.title, "B");
        assert_eq!(out[1].1.title, "A");
        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert!(headers.get("authorization").is_none(), "no key → no bearer");
        assert!(body.get("model").is_none(), "model omitted when unset");
    }

    /// A dead endpoint fails at construction (the probe), not on first query —
    /// that's where the server downgrades to plain vector ranking.
    #[tokio::test]
    async fn unreachable_endpoint_fails_the_probe() {
        let err = HttpReranker::new("http://127.0.0.1:1".into(), None, None).await;
        assert!(err.is_err());
    }

    /// An index beyond the submitted documents is a provider bug — surface it
    /// rather than panicking or silently dropping results.
    #[tokio::test]
    async fn out_of_range_index_is_an_error() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_endpoint(
            serde_json::json!({"results": [{"index": 7, "relevance_score": 0.9}]}),
            captured.clone(),
        )
        .await;

        let r = HttpReranker::new(base, None, None).await.unwrap();
        let err = r
            .rerank("q", &[(0.9, chunk("A", "a"))], 5)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("index 7"));
    }

    /// A duplicate index would silently duplicate one chunk and drop another —
    /// same provider-bug class as out-of-range, same treatment. (The probe is
    /// unaffected: it bypasses validation via rerank_texts.)
    #[tokio::test]
    async fn duplicate_index_is_an_error() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_endpoint(
            serde_json::json!({"results": [
                {"index": 0, "relevance_score": 0.9},
                {"index": 0, "relevance_score": 0.8}
            ]}),
            captured.clone(),
        )
        .await;

        let r = HttpReranker::new(base, None, None).await.unwrap();
        let err = r
            .rerank("q", &[(0.9, chunk("A", "a")), (0.8, chunk("B", "b"))], 5)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("more than once"));
    }

    /// Fewer scores than documents means chunks silently vanished server-side
    /// (top_n is deliberately never sent, so a full list is the contract).
    #[tokio::test]
    async fn short_response_is_an_error() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_endpoint(
            serde_json::json!({"results": [{"index": 0, "relevance_score": 0.9}]}),
            captured.clone(),
        )
        .await;

        let r = HttpReranker::new(base, None, None).await.unwrap();
        let err = r
            .rerank("q", &[(0.9, chunk("A", "a")), (0.8, chunk("B", "b"))], 5)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("1 scores for a 2-document request"));
    }
}
