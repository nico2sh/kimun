//! Embedder backed by any OpenAI-compatible `/embeddings` endpoint (OpenAI,
//! LM Studio, vLLM, most gateways, and Ollama's `/v1` shim). The output width
//! is probed once at construction.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::document::FlattenedChunk;

use super::Embedder;

pub struct OpenAiEmbedder {
    client: reqwest::Client,
    /// Base URL including the API version, e.g. `https://api.openai.com/v1`.
    url: String,
    model: String,
    api_key: Option<String>,
    dimension: usize,
    doc_prefix: String,
    query_prefix: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedDatum>,
}

#[derive(Deserialize)]
struct EmbedDatum {
    embedding: Vec<f32>,
}

impl OpenAiEmbedder {
    /// Builds the embedder and probes its output dimension. Fails if the server
    /// is unreachable or returns no vector.
    pub async fn new(
        url: String,
        model: String,
        api_key: Option<String>,
        doc_prefix: String,
        query_prefix: String,
    ) -> anyhow::Result<Self> {
        let mut embedder = Self {
            client: reqwest::Client::new(),
            url,
            model,
            api_key,
            dimension: 0,
            doc_prefix,
            query_prefix,
        };
        let probe = embedder
            .embed_batch(vec!["dimension probe".to_string()])
            .await?;
        embedder.dimension = probe
            .first()
            .map(|v| v.len())
            .filter(|&d| d > 0)
            .ok_or_else(|| anyhow::anyhow!("OpenAI embedder returned no vector on probe"))?;
        Ok(embedder)
    }

    async fn embed_batch(&self, input: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let endpoint = format!("{}/embeddings", self.url.trim_end_matches('/'));
        let mut request = self.client.post(endpoint).json(&EmbedRequest {
            model: &self.model,
            input,
        });
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResponse>()
            .await?;
        Ok(response.data.into_iter().map(|d| d.embedding).collect())
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
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
            anyhow::bail!("OpenAI embedder returned no vector for query");
        }
        Ok(vectors.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_to_openai_shape() {
        let body = serde_json::to_value(EmbedRequest {
            model: "text-embedding-3-small",
            input: vec!["hello".to_string()],
        })
        .unwrap();
        assert_eq!(body["model"], "text-embedding-3-small");
        assert_eq!(body["input"][0], "hello");
    }

    #[test]
    fn response_parses_openai_shape() {
        let json =
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.5,0.6]}]}"#;
        let parsed: EmbedResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.data.len(), 1);
        assert_eq!(parsed.data[0].embedding, vec![0.5, 0.6]);
    }
}
