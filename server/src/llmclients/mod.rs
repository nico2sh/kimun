//! The one LLM client. Every provider speaks "prompt in, answer text out";
//! what varies is only the wire: endpoint path, auth header style, and the
//! request/response JSON shape. `ChatClient` owns the shared behavior — prompt
//! assembly, the HTTP cycle, error shaping — and a [`Wire`] value carries the
//! per-provider variation. Adding a provider means describing its wire, not
//! cloning a client.
//!
//! The API key arrives as a constructor argument (resolved from config or the
//! provider's env var at composition time) — the client never reads the
//! environment and never panics on a missing key.

use anyhow::bail;
use async_trait::async_trait;
use log::debug;
use serde::{Deserialize, Serialize};

use crate::config::LlmConfig;
use crate::document::FlattenedChunk;

#[async_trait]
pub trait LLMClient: Send + Sync {
    async fn ask(
        &self,
        question: &str,
        context: &[(f64, FlattenedChunk)],
    ) -> anyhow::Result<String>;
}

const OPENAI_URL: &str = "https://api.openai.com/v1";
const MISTRAL_URL: &str = "https://api.mistral.ai/v1";
const ANTHROPIC_URL: &str = "https://api.anthropic.com";
const GEMINI_URL: &str = "https://generativelanguage.googleapis.com";

/// The provider's wire shape — the only thing that differs between providers.
enum Wire {
    /// OpenAI chat-completions dialect: `{base}/chat/completions`, bearer auth.
    /// Speaks for OpenAI, Mistral, and any OpenAI-compatible endpoint.
    OpenAiCompat,
    /// Anthropic messages API: `{base}/v1/messages`, `x-api-key` header.
    Anthropic,
    /// Gemini generateContent: model and key live in the URL itself.
    Gemini,
}

pub struct ChatClient {
    http: reqwest::Client,
    wire: Wire,
    /// Provider id for error messages and logs.
    provider: &'static str,
    /// No trailing slash.
    base_url: String,
    model: String,
    api_key: String,
}

impl ChatClient {
    /// Maps a provider config to its wire shape and endpoint. The OpenAI
    /// provider's `url` may point at any OpenAI-compatible server (Ollama,
    /// llama.cpp, OpenRouter); the others have fixed endpoints.
    pub fn from_config(config: &LlmConfig, api_key: String) -> Self {
        let (wire, base_url) = match config {
            LlmConfig::OpenAI { url, .. } => (
                Wire::OpenAiCompat,
                url.clone().unwrap_or_else(|| OPENAI_URL.to_string()),
            ),
            LlmConfig::Mistral { .. } => (Wire::OpenAiCompat, MISTRAL_URL.to_string()),
            LlmConfig::Claude { .. } => (Wire::Anthropic, ANTHROPIC_URL.to_string()),
            LlmConfig::Gemini { .. } => (Wire::Gemini, GEMINI_URL.to_string()),
        };
        Self {
            http: reqwest::Client::new(),
            wire,
            provider: config.provider(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: config.model().to_string(),
            api_key,
        }
    }
}

#[async_trait]
impl LLMClient for ChatClient {
    async fn ask(
        &self,
        question: &str,
        context: &[(f64, FlattenedChunk)],
    ) -> anyhow::Result<String> {
        let prompt = build_prompt(question, context);

        let request = match &self.wire {
            Wire::OpenAiCompat => self
                .http
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&ChatRequest {
                    model: self.model.clone(),
                    max_tokens: None,
                    messages: vec![ChatMessage {
                        role: "user".to_string(),
                        content: prompt,
                    }],
                }),
            Wire::Anthropic => self
                .http
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&ChatRequest {
                    model: self.model.clone(),
                    max_tokens: Some(4096),
                    messages: vec![ChatMessage {
                        role: "user".to_string(),
                        content: prompt,
                    }],
                }),
            Wire::Gemini => self
                .http
                .post(format!(
                    "{}/v1beta/models/{}:generateContent?key={}",
                    self.base_url, self.model, self.api_key
                ))
                .json(&GeminiRequest {
                    contents: vec![GeminiContent {
                        parts: vec![GeminiPart { text: prompt }],
                    }],
                }),
        };

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            bail!("{} API error: {}\n{}", self.provider, status, body);
        }

        let answer = match &self.wire {
            Wire::OpenAiCompat => {
                let parsed: ChatResponse = response.json().await?;
                if let Some(usage) = &parsed.usage {
                    debug!(
                        "{} tokens: {} prompt, {} completion",
                        self.provider, usage.prompt_tokens, usage.completion_tokens
                    );
                }
                parsed
                    .choices
                    .into_iter()
                    .map(|c| c.message.content)
                    .collect::<Vec<String>>()
                    .join("\n")
            }
            Wire::Anthropic => {
                let parsed: AnthropicResponse = response.json().await?;
                if let Some(usage) = &parsed.usage {
                    debug!(
                        "{} tokens: {} input, {} output",
                        self.provider, usage.input_tokens, usage.output_tokens
                    );
                }
                parsed
                    .content
                    .into_iter()
                    .filter(|c| c.kind == "text")
                    .map(|c| c.text)
                    .collect::<Vec<String>>()
                    .join("\n")
            }
            Wire::Gemini => {
                let parsed: GeminiResponse = response.json().await?;
                if let Some(usage) = &parsed.usage_metadata {
                    debug!(
                        "{} tokens: {} prompt, {} candidates",
                        self.provider, usage.prompt_token_count, usage.candidates_token_count
                    );
                }
                parsed
                    .candidates
                    .into_iter()
                    .flat_map(|c| c.content.parts)
                    .map(|p| p.text)
                    .collect::<Vec<String>>()
                    .join("\n")
            }
        };

        Ok(answer)
    }
}

/// The one RAG prompt, shared by every provider: answer notes-first, supplement
/// with common knowledge when the notes fall short, and always distinguish the
/// two. Each context chunk is framed with its note path, relevance, and (when
/// the section title starts with one) its date.
fn build_prompt(question: &str, context: &[(f64, FlattenedChunk)]) -> String {
    let mut context_string = String::new();
    for (relevance, chunk) in context {
        context_string.push_str(&format!(
            "--- Document: {} (Relevance: {:.4}) ---\n",
            chunk.doc_path, relevance
        ));
        let mut title = chunk.title.clone();
        if let Some(date) = chunk.get_date_string() {
            context_string.push_str(&format!("Date: {date}\n"));
            title = title
                .trim()
                .strip_prefix(&date)
                .map(|t| t.to_string())
                .unwrap_or(title);
        }
        if !title.is_empty() {
            context_string.push_str(&format!("# {title}\n"));
        }
        context_string.push_str(&chunk.text);
        context_string.push_str("\n\n");
    }

    format!(
        r#"You are an intelligent assistant with access to a personal knowledge base.
Answer the user's question using the retrieved context first. If the retrieved notes contain relevant information, base your answer primarily on them.
If the context is incomplete, missing, or can be enriched with widely accepted knowledge about the topic related with the question, supplement the answer with accurate common knowledge.
Always distinguish between information from the notes and general knowledge.
If no useful information is available in either, respond with: 'I don't have enough information to answer.'

Retrieved context:
---------------------
{context_string}.
---------------------

Question: {question}"#
    )
}

// ── Wire types ──────────────────────────────────────────────────────────────
// Minimal and tolerant: only the fields the server reads, usage optional, so a
// provider adding response fields never breaks parsing.

/// Request body for both the OpenAI-compat and Anthropic wires (Anthropic
/// additionally requires `max_tokens`; omitted for OpenAI-compat).
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
}

#[derive(Serialize, Deserialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata", default)]
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiCandidateContent,
}

#[derive(Deserialize)]
struct GeminiCandidateContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Deserialize)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount", default)]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount", default)]
    candidates_token_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::HeaderMap, routing::post};
    use std::sync::{Arc, Mutex};

    fn chunk(path: &str, title: &str, text: &str) -> (f64, FlattenedChunk) {
        (
            0.9,
            FlattenedChunk {
                doc_path: path.to_string(),
                doc_hash: "h".to_string(),
                title: title.to_string(),
                text: text.to_string(),
                date: None,
            },
        )
    }

    #[test]
    fn prompt_frames_each_chunk_and_carries_the_question() {
        let ctx = vec![
            chunk("/a.md", "Alpha", "alpha body"),
            chunk("/b.md", "", "beta body"),
        ];
        let prompt = build_prompt("what is alpha?", &ctx);
        assert!(prompt.contains("--- Document: /a.md (Relevance: 0.9000) ---"));
        assert!(prompt.contains("# Alpha\nalpha body"));
        assert!(prompt.contains("beta body"));
        assert!(
            !prompt.contains("# \n"),
            "empty title must not emit a heading"
        );
        assert!(prompt.contains("Question: what is alpha?"));
        assert!(prompt.contains("personal knowledge base"));
    }

    /// A captured request: headers plus the raw JSON body the client sent.
    type Captured = Arc<Mutex<Option<(HeaderMap, serde_json::Value)>>>;

    /// Serves `response` on `route`, capturing what the client sent.
    async fn mock_provider(route: &str, response: serde_json::Value, captured: Captured) -> String {
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

    fn client(wire: Wire, base_url: String) -> ChatClient {
        ChatClient {
            http: reqwest::Client::new(),
            wire,
            provider: "test",
            base_url,
            model: "test-model".to_string(),
            api_key: "sk-test".to_string(),
        }
    }

    #[tokio::test]
    async fn openai_compat_wire_sends_bearer_and_parses_choices() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_provider(
            "/chat/completions",
            serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "the answer"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            }),
            captured.clone(),
        )
        .await;

        let answer = client(Wire::OpenAiCompat, base)
            .ask("q?", &[chunk("/a.md", "A", "body")])
            .await
            .unwrap();
        assert_eq!(answer, "the answer");

        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["authorization"], "Bearer sk-test");
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("q?")
        );
        assert!(
            body.get("max_tokens").is_none(),
            "OpenAI-compat omits max_tokens"
        );
    }

    #[tokio::test]
    async fn anthropic_wire_sends_key_header_and_joins_text_blocks_only() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_provider(
            "/v1/messages",
            serde_json::json!({
                "content": [
                    {"type": "text", "text": "part one"},
                    {"type": "tool_use", "text": "ignored"},
                    {"type": "text", "text": "part two"}
                ],
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }),
            captured.clone(),
        )
        .await;

        let answer = client(Wire::Anthropic, base).ask("q?", &[]).await.unwrap();
        assert_eq!(answer, "part one\npart two");

        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["x-api-key"], "sk-test");
        assert_eq!(headers["anthropic-version"], "2023-06-01");
        assert_eq!(body["max_tokens"], 4096);
    }

    #[tokio::test]
    async fn gemini_wire_puts_key_in_url_and_parses_candidate_parts() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_provider(
            "/v1beta/models/{model_action}",
            serde_json::json!({
                "candidates": [
                    {"content": {"parts": [{"text": "gemini answer"}], "role": "model"}}
                ],
                "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5}
            }),
            captured.clone(),
        )
        .await;

        let answer = client(Wire::Gemini, base).ask("q?", &[]).await.unwrap();
        assert_eq!(answer, "gemini answer");

        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert!(
            headers.get("authorization").is_none(),
            "gemini auth is the url key"
        );
        assert!(
            body["contents"][0]["parts"][0]["text"]
                .as_str()
                .unwrap()
                .contains("q?")
        );
    }

    #[tokio::test]
    async fn provider_error_carries_status_and_body() {
        let app = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    "{\"error\": \"bad key\"}",
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let err = client(Wire::OpenAiCompat, format!("http://{addr}"))
            .ask("q?", &[])
            .await
            .expect_err("401 must surface as an error");
        let msg = err.to_string();
        assert!(msg.contains("401"), "missing status in: {msg}");
        assert!(msg.contains("bad key"), "missing body in: {msg}");
    }

    #[test]
    fn from_config_maps_providers_to_wires_and_default_urls() {
        let cases: Vec<(LlmConfig, &str)> = vec![
            (
                LlmConfig::OpenAI {
                    model: "m".into(),
                    api_key: None,
                    url: None,
                },
                OPENAI_URL,
            ),
            (
                LlmConfig::OpenAI {
                    model: "m".into(),
                    api_key: None,
                    url: Some("http://localhost:11434/v1/".into()),
                },
                "http://localhost:11434/v1", // trailing slash trimmed
            ),
            (
                LlmConfig::Mistral {
                    model: "m".into(),
                    api_key: None,
                },
                MISTRAL_URL,
            ),
            (
                LlmConfig::Claude {
                    model: "m".into(),
                    api_key: None,
                },
                ANTHROPIC_URL,
            ),
            (
                LlmConfig::Gemini {
                    model: "m".into(),
                    api_key: None,
                },
                GEMINI_URL,
            ),
        ];
        for (cfg, expected_base) in cases {
            let client = ChatClient::from_config(&cfg, "k".into());
            assert_eq!(
                client.base_url,
                expected_base,
                "provider {}",
                cfg.provider()
            );
            assert_eq!(client.model, "m");
        }
    }
}
