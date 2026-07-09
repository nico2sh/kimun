//! Embedder backed by an [Ollama](https://ollama.com) server's `/api/embed`
//! endpoint. The output width is probed once at construction.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::document::FlattenedChunk;

use super::Embedder;

pub struct OllamaEmbedder {
    client: reqwest::Client,
    /// Base URL, e.g. `http://localhost:11434`.
    url: String,
    model: String,
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
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbedder {
    /// Builds the embedder and probes its output dimension by embedding a short
    /// test string. Fails if the server is unreachable or returns no vector.
    pub async fn new(
        url: String,
        model: String,
        doc_prefix: String,
        query_prefix: String,
    ) -> anyhow::Result<Self> {
        let mut embedder = Self {
            client: reqwest::Client::new(),
            url,
            model,
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
            .ok_or_else(|| anyhow::anyhow!("Ollama embedder returned no vector on probe"))?;
        Ok(embedder)
    }

    async fn embed_batch(&self, input: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        let endpoint = format!("{}/api/embed", self.url.trim_end_matches('/'));
        let response = self
            .client
            .post(endpoint)
            .json(&EmbedRequest {
                model: &self.model,
                input,
            })
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResponse>()
            .await?;
        Ok(response.embeddings)
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
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
            anyhow::bail!("Ollama embedder returned no vector for query");
        }
        Ok(vectors.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_to_ollama_shape() {
        let body = serde_json::to_value(EmbedRequest {
            model: "nomic-embed-text",
            input: vec!["a".to_string(), "b".to_string()],
        })
        .unwrap();
        assert_eq!(body["model"], "nomic-embed-text");
        assert_eq!(body["input"][0], "a");
        assert_eq!(body["input"][1], "b");
    }

    #[test]
    fn response_parses_ollama_shape() {
        let json = r#"{"model":"x","embeddings":[[0.1,0.2,0.3]]}"#;
        let parsed: EmbedResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.embeddings.len(), 1);
        assert_eq!(parsed.embeddings[0], vec![0.1, 0.2, 0.3]);
    }
}
