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
        history: &[(String, String)],
        context: &[(usize, (f64, FlattenedChunk))],
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
        history: &[(String, String)],
        context: &[(usize, (f64, FlattenedChunk))],
    ) -> anyhow::Result<String> {
        let messages = chat_messages(history, build_prompt(question, history, context));

        let request = match &self.wire {
            Wire::OpenAiCompat => self
                .http
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&ChatRequest {
                    model: self.model.clone(),
                    max_tokens: None,
                    messages,
                }),
            Wire::Anthropic => self
                .http
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&ChatRequest {
                    model: self.model.clone(),
                    max_tokens: Some(4096),
                    messages,
                }),
            Wire::Gemini => {
                let contents = gemini_contents(messages);
                self.http
                    .post(format!(
                        "{}/v1beta/models/{}:generateContent?key={}",
                        self.base_url, self.model, self.api_key
                    ))
                    .json(&GeminiRequest { contents })
            }
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

/// History pairs + the final RAG prompt as one chat transcript. Shared by the
/// OpenAI-compat and Anthropic wires; Gemini maps the same list to `contents`
/// with the "model" role name.
fn chat_messages(history: &[(String, String)], prompt: String) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(history.len() * 2 + 1);
    for (q, a) in history {
        msgs.push(ChatMessage {
            role: "user".into(),
            content: q.clone(),
        });
        msgs.push(ChatMessage {
            role: "assistant".into(),
            content: a.clone(),
        });
    }
    msgs.push(ChatMessage {
        role: "user".into(),
        content: prompt,
    });
    msgs
}

/// Maps a chat transcript (`chat_messages`'s output) to Gemini's `contents`
/// shape: every `"assistant"` role becomes `"model"`, everything else (only
/// ever `"user"` in practice) stays `"user"`, and each message's text becomes
/// its single part.
fn gemini_contents(messages: Vec<ChatMessage>) -> Vec<GeminiContent> {
    messages
        .into_iter()
        .map(|m| GeminiContent {
            role: if m.role == "assistant" {
                "model".into()
            } else {
                "user".into()
            },
            parts: vec![GeminiPart { text: m.content }],
        })
        .collect()
}

/// The one RAG prompt, shared by every provider: chunks are numbered `[i]` in
/// sources order, citations are mandatory for note-derived claims, and the
/// answer may supplement with common knowledge — uncited text IS the signal
/// that a claim is general knowledge, so the two never blur.
///
/// When `history` is non-empty the prompt gains a follow-up-handling line: the
/// preceding turns ride in the chat transcript (see [`chat_messages`]), and a
/// terse reply ("yes", "go on", "the second one") only makes sense against
/// them — so the model is told to read the short reply as accepting/continuing
/// the conversation and to answer the IMPLIED request, drawing on the context
/// chunks where they fit and the conversation itself otherwise. The line is
/// omitted for a first turn (empty history), where there is nothing to resolve
/// a terse reply against.
/// Core's chunk-breadcrumb separator (ASCII Unit Separator, U+001F): the
/// heading path is flattened into the chunk title joined with this control
/// char. Mirrored here (the server does not depend on `kimun_core`) so the
/// prompt can present it as a readable path rather than a raw control char.
const BREADCRUMB_SEP: char = '\u{1f}';

fn build_prompt(
    question: &str,
    history: &[(String, String)],
    context: &[(usize, (f64, FlattenedChunk))],
) -> String {
    let mut context_string = String::new();
    // The `[n]` frame is the ordinal ASSIGNED to the pair upstream, never this
    // loop's position — so the number the model cites is exactly the ordinal the
    // wire source carries, even if the context were reordered before us.
    for (ordinal, (_, chunk)) in context.iter() {
        let mut title = chunk.title.clone();
        let mut date_line = String::new();
        if let Some(date) = chunk.get_date_string() {
            date_line = format!("Date: {date}\n");
            title = title
                .trim()
                .strip_prefix(&date)
                .map(|t| t.trim().to_string())
                .unwrap_or(title);
        }
        // The title is core's chunk breadcrumb: nested headings joined with the
        // control-char separator (U+001F). Render it readably so no control
        // char reaches the model, and so a `Chapter › Section` path is legible.
        let trimmed_title = title.trim().replace(BREADCRUMB_SEP, " \u{203a} ");
        // Omit the ` — "…"` title clause entirely for a blank title, rather
        // than emitting an empty `— ""` that adds noise and no signal.
        let header = if trimmed_title.is_empty() {
            format!("[{}] {}", ordinal, chunk.doc_path)
        } else {
            format!("[{}] {} — \"{trimmed_title}\"", ordinal, chunk.doc_path)
        };
        context_string.push_str(&format!("{header}\n{date_line}{}\n\n", chunk.text));
    }

    // Only present mid-conversation: a terse reply is meaningless on a first
    // turn, so the guidance would just be noise there.
    let followup_line = if history.is_empty() {
        String::new()
    } else {
        "If the user's message is a short reply to the conversation above (e.g. 'yes', 'go on', 'the second one'), interpret it against that conversation and answer the request it implies, using the context below where it fits and the conversation itself otherwise.\n".to_string()
    };

    format!(
        r#"You are an intelligent assistant with access to a personal knowledge base.
Answer the user's question using the numbered context below first; base the answer primarily on it when it is relevant.
{followup_line}Every claim drawn from the context MUST carry an inline citation in the form [n], where n is the number of the supporting context entry. A sentence may carry several citations.
You may supplement with accurate, widely accepted common knowledge when the context falls short — never cite [n] for such claims; leaving them uncited is how they are marked as general knowledge.
Preserve any [[wikilinks]] and #tags that appear in the context verbatim when you quote or reference them.
If neither the context nor common knowledge suffices, respond with: 'I don't have enough information to answer.'

Context:
---------------------
{context_string}---------------------

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
    role: String,
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

    /// 1-based numbering, exactly as [`RagPipeline::answer`] assigns it.
    fn numbered(chunks: Vec<(f64, FlattenedChunk)>) -> Vec<(usize, (f64, FlattenedChunk))> {
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, c)| (i + 1, c))
            .collect()
    }

    #[test]
    fn history_folds_into_alternating_messages_before_the_prompt() {
        let history = vec![
            ("q1".to_string(), "a1".to_string()),
            ("q2".to_string(), "a2".to_string()),
        ];
        let msgs = chat_messages(&history, "PROMPT".to_string());
        let shape: Vec<(&str, &str)> = msgs
            .iter()
            .map(|m| (m.role.as_str(), m.content.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("user", "q1"),
                ("assistant", "a1"),
                ("user", "q2"),
                ("assistant", "a2"),
                ("user", "PROMPT"),
            ]
        );
    }

    #[test]
    fn gemini_contents_maps_assistant_to_model_and_keeps_the_final_prompt() {
        let history = vec![
            ("q1".to_string(), "a1".to_string()),
            ("q2".to_string(), "a2".to_string()),
        ];
        let msgs = chat_messages(&history, "PROMPT".to_string());
        let contents = gemini_contents(msgs);
        let roles: Vec<&str> = contents.iter().map(|c| c.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "model", "user", "model", "user"]);
        assert_eq!(contents.last().unwrap().parts[0].text, "PROMPT");
    }

    #[test]
    fn empty_history_is_a_single_prompt_message() {
        let msgs = chat_messages(&[], "PROMPT".to_string());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn prompt_numbers_chunks_and_mandates_citations() {
        let context = numbered(vec![
            chunk("/intro.md", "intro", "alpha text"),
            chunk("/setup.md", "setup", "beta text"),
        ]);
        let p = build_prompt("how do I start?", &[], &context);
        // numbered frames, prompt order = sources order
        let i1 = p.find("[1]").expect("first chunk numbered");
        let i2 = p.find("[2]").expect("second chunk numbered");
        assert!(i1 < i2);
        assert!(p.contains("alpha text") && p.contains("beta text"));
        // citation contract in the instructions
        assert!(p.contains("cite"), "prompt must instruct citing");
        assert!(p.contains("[n]"), "prompt must name the [n] form");
        assert!(p.contains("how do I start?"));
    }

    #[test]
    fn prompt_frames_use_the_assigned_ordinal_not_loop_position() {
        // The pairing contract: the `[n]` frame is the ordinal on the pair, not
        // where the chunk sits in the slice. A deliberately shuffled numbering
        // (3, 1, 2) must surface verbatim in the prompt — proving the builder
        // never re-enumerates.
        let context = vec![
            (3usize, chunk("/c.md", "c", "gamma text")),
            (1usize, chunk("/a.md", "a", "alpha text")),
            (2usize, chunk("/b.md", "b", "beta text")),
        ];
        let p = build_prompt("q?", &[], &context);
        assert!(p.contains("[3] /c.md"), "gamma keeps ordinal 3: {p}");
        assert!(p.contains("[1] /a.md"), "alpha keeps ordinal 1: {p}");
        assert!(p.contains("[2] /b.md"), "beta keeps ordinal 2: {p}");
        // Prompt order follows the slice, but each frame carries its own ordinal.
        assert!(p.find("gamma text").unwrap() < p.find("alpha text").unwrap());
    }

    #[test]
    fn nested_breadcrumb_title_renders_readably_without_control_chars() {
        // A nested-section chunk's title is the breadcrumb joined with U+001F;
        // the prompt must show `Chapter › Section` and never the raw control char.
        let title = format!("Chapter{BREADCRUMB_SEP}Section");
        let context = numbered(vec![chunk("/book.md", &title, "body")]);
        let p = build_prompt("q?", &[], &context);
        assert!(
            p.contains("— \"Chapter \u{203a} Section\""),
            "readable breadcrumb in the title clause: {p:?}"
        );
        assert!(!p.contains('\u{1f}'), "no control char leaks into the prompt");
    }

    #[test]
    fn empty_title_chunk_omits_the_title_clause() {
        let context = numbered(vec![chunk("some/path", "   ", "chunk body")]);
        let p = build_prompt("q?", &[], &context);
        assert!(
            p.contains("[1] some/path\n"),
            "blank-title chunk keeps just `[i] path`: {p}"
        );
        assert!(
            !p.contains("— \"\""),
            "must not emit an empty title clause: {p}"
        );
    }

    #[test]
    fn followup_instruction_appears_only_with_conversation_history() {
        let context = numbered(vec![chunk("/a.md", "A", "body")]);

        // First turn (empty history): no follow-up guidance — a terse reply is
        // meaningless with nothing to resolve it against.
        let first = build_prompt("yes", &[], &context);
        assert!(
            !first.contains("short reply"),
            "empty history must not add the follow-up line: {first}"
        );

        // Mid-conversation: the guidance to interpret a terse reply against the
        // conversation is present.
        let history = vec![(
            "Should I explain the setup steps?".to_string(),
            "There are three steps; want the details?".to_string(),
        )];
        let mid = build_prompt("yes", &history, &context);
        assert!(
            mid.contains("short reply"),
            "history present must add the follow-up line: {mid}"
        );
        // The raw current question is still the prompt's Question, untouched.
        assert!(mid.contains("Question: yes"));
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
            .ask("q?", &[], &numbered(vec![chunk("/a.md", "A", "body")]))
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

        let answer = client(Wire::Anthropic, base)
            .ask("q?", &[], &[])
            .await
            .unwrap();
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

        let answer = client(Wire::Gemini, base)
            .ask("q?", &[], &[])
            .await
            .unwrap();
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
            .ask("q?", &[], &[])
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
