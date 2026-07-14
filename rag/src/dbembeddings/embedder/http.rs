//! The one HTTP embedder. Ollama's `/api/embed` and any OpenAI-compatible
//! `/embeddings` endpoint take the same request (`{model, input}`) and differ
//! only in path, auth, and response shape — an [`EmbedWire`] value carries that
//! variation, everything else (probe, batching, prefixing) is shared. Same
//! pattern as the LLM `ChatClient`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::document::FlattenedChunk;

use super::Embedder;

/// The endpoint's wire shape — the only thing that differs between providers.
enum EmbedWire {
    /// Ollama native: `{base}/api/embed`, no auth, `{"embeddings": [[..]]}`.
    Ollama,
    /// OpenAI-compatible: `{base}/embeddings`, optional bearer,
    /// `{"data": [{"index", "embedding"}]}` (sorted by `index` — not every
    /// compatible server returns input order).
    OpenAiCompat,
}

pub struct HttpEmbedder {
    client: reqwest::Client,
    wire: EmbedWire,
    /// Base URL — for OpenAI-compatible endpoints including the API version
    /// (e.g. `https://api.openai.com/v1`); for Ollama the server root.
    url: String,
    model: String,
    /// Bearer token for OpenAI-compatible endpoints that need one.
    api_key: Option<String>,
    dimension: usize,
    /// Prefix prepended to document text before embedding (model-specific, e.g.
    /// `search_document: ` for nomic). Empty by default.
    doc_prefix: String,
    /// Prefix prepended to query text before embedding.
    query_prefix: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct OllamaResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    data: Vec<OpenAiDatum>,
}

#[derive(Deserialize)]
struct OpenAiDatum {
    /// Position in the input batch; defaults to 0 when omitted (then the sort
    /// is a stable no-op, preserving array order).
    #[serde(default)]
    index: usize,
    embedding: Vec<f32>,
}

impl HttpEmbedder {
    /// An embedder for an Ollama server's `/api/embed`.
    pub async fn ollama(
        url: String,
        model: String,
        doc_prefix: String,
        query_prefix: String,
    ) -> anyhow::Result<Self> {
        Self::probe(Self {
            client: reqwest::Client::new(),
            wire: EmbedWire::Ollama,
            url,
            model,
            api_key: None,
            dimension: 0,
            doc_prefix,
            query_prefix,
        })
        .await
    }

    /// An embedder for any OpenAI-compatible `/embeddings` endpoint (OpenAI,
    /// LM Studio, vLLM, gateways, Ollama's `/v1` shim).
    pub async fn openai(
        url: String,
        model: String,
        api_key: Option<String>,
        doc_prefix: String,
        query_prefix: String,
    ) -> anyhow::Result<Self> {
        Self::probe(Self {
            client: reqwest::Client::new(),
            wire: EmbedWire::OpenAiCompat,
            url,
            model,
            api_key,
            dimension: 0,
            doc_prefix,
            query_prefix,
        })
        .await
    }

    /// Probes the output dimension by embedding a short test string. Fails if
    /// the server is unreachable or returns no vector.
    async fn probe(mut embedder: Self) -> anyhow::Result<Self> {
        let probe = embedder
            .embed_batch(vec!["dimension probe".to_string()])
            .await?;
        embedder.dimension = probe
            .first()
            .map(|v| v.len())
            .filter(|&d| d > 0)
            .ok_or_else(|| anyhow::anyhow!("embedder returned no vector on probe"))?;
        Ok(embedder)
    }

    async fn embed_batch(&self, input: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let base = self.url.trim_end_matches('/');
        let endpoint = match self.wire {
            EmbedWire::Ollama => format!("{base}/api/embed"),
            EmbedWire::OpenAiCompat => format!("{base}/embeddings"),
        };
        let mut request = self.client.post(endpoint).json(&EmbedRequest {
            model: &self.model,
            input,
        });
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await?.error_for_status()?;

        match self.wire {
            EmbedWire::Ollama => {
                let parsed: OllamaResponse = response.json().await?;
                Ok(parsed.embeddings)
            }
            EmbedWire::OpenAiCompat => {
                let parsed: OpenAiResponse = response.json().await?;
                let mut data = parsed.data;
                data.sort_by_key(|d| d.index);
                Ok(data.into_iter().map(|d| d.embedding).collect())
            }
        }
    }
}

#[async_trait]
impl Embedder for HttpEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    async fn generate_embeddings(
        &self,
        content: &[FlattenedChunk],
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let input = content
            .iter()
            .map(|c| format!("{}{}\n{}", self.doc_prefix, c.title, c.text))
            .collect();
        self.embed_batch(input).await
    }

    async fn prompt_embedding(&self, content: &str) -> anyhow::Result<Vec<f32>> {
        let mut vectors = self
            .embed_batch(vec![format!("{}{}", self.query_prefix, content)])
            .await?;
        if vectors.is_empty() {
            anyhow::bail!("embedder returned no vector for query");
        }
        Ok(vectors.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::HeaderMap, routing::post};
    use std::sync::{Arc, Mutex};

    /// Captured request: headers plus the JSON body the embedder sent.
    type Captured = Arc<Mutex<Option<(HeaderMap, serde_json::Value)>>>;

    async fn mock_endpoint(
        route: &str,
        response: serde_json::Value,
        captured: Captured,
    ) -> String {
        let app = Router::new().route(
            route,
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

    #[tokio::test]
    async fn ollama_wire_probes_dimension_and_prefixes_docs() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_endpoint(
            "/api/embed",
            serde_json::json!({"embeddings": [[0.1, 0.2, 0.3]]}),
            captured.clone(),
        )
        .await;

        let e = HttpEmbedder::ollama(
            base,
            "nomic".into(),
            "search_document: ".into(),
            "search_query: ".into(),
        )
        .await
        .unwrap();
        assert_eq!(e.dimension(), 3, "dimension comes from the probe");

        let out = e.generate_embeddings(&[chunk("T", "body")]).await.unwrap();
        assert_eq!(out, vec![vec![0.1, 0.2, 0.3]]);
        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert!(headers.get("authorization").is_none(), "ollama sends no auth");
        assert_eq!(body["model"], "nomic");
        assert_eq!(body["input"][0], "search_document: T\nbody");
    }

    #[tokio::test]
    async fn openai_wire_sends_bearer_and_sorts_by_index() {
        let captured: Captured = Arc::new(Mutex::new(None));
        // Out-of-order data: index 1 first — the embedder must sort.
        let base = mock_endpoint(
            "/embeddings",
            serde_json::json!({"data": [
                {"index": 1, "embedding": [9.0, 9.0]},
                {"index": 0, "embedding": [1.0, 1.0]}
            ]}),
            captured.clone(),
        )
        .await;

        let e = HttpEmbedder::openai(
            base,
            "text-embedding-3-small".into(),
            Some("sk-embed".into()),
            String::new(),
            String::new(),
        )
        .await
        .unwrap();

        let out = e
            .generate_embeddings(&[chunk("A", "a"), chunk("B", "b")])
            .await
            .unwrap();
        assert_eq!(out[0], vec![1.0, 1.0], "sorted back to input order");
        assert_eq!(out[1], vec![9.0, 9.0]);
        let (headers, _) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["authorization"], "Bearer sk-embed");
    }

    #[tokio::test]
    async fn query_embedding_uses_the_query_prefix() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_endpoint(
            "/api/embed",
            serde_json::json!({"embeddings": [[0.5]]}),
            captured.clone(),
        )
        .await;

        let e = HttpEmbedder::ollama(base, "m".into(), "doc: ".into(), "query: ".into())
            .await
            .unwrap();
        let v = e.prompt_embedding("find me").await.unwrap();
        assert_eq!(v, vec![0.5]);
        let (_, body) = captured.lock().unwrap().take().unwrap();
        assert_eq!(body["input"][0], "query: find me");
    }
}
