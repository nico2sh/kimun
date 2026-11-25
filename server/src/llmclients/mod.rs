//! The one LLM client. Every provider speaks "prompt in, answer text out";
//! what varies is only the wire: endpoint path, auth header style, and the
//! request/response JSON shape. `ChatClient` owns the shared behavior — prompt
//! assembly, the HTTP cycle, error shaping — and a `Wire` value carries the
//! per-provider variation. Adding a provider means describing its wire, not
//! cloning a client.
//!
//! The API key arrives as a constructor argument (resolved from config or the
//! provider's env var at composition time) — the client never reads the
//! environment and never panics on a missing key.

use anyhow::bail;
use async_trait::async_trait;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

use crate::agent::MODEL_ONLY_LABEL;
use crate::config::{LlmConfig, OPENROUTER_URL, ReasoningEffort};
use crate::document::FlattenedChunk;
use crate::tools::ToolSpec;

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait LLMClient: Send + Sync {
    async fn ask(
        &self,
        question: &str,
        history: &[(String, String)],
        context: &[(usize, (f64, FlattenedChunk))],
    ) -> anyhow::Result<String>;

    /// One turn of a tool-calling exchange: the model reads `system`, the
    /// transcript and the tools it may call, and either answers or asks for
    /// tool calls. A client that cannot call tools keeps this default, and the
    /// caller falls back to [`LLMClient::ask`].
    async fn chat(
        &self,
        _system: &str,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> anyhow::Result<ModelReply> {
        Err(ToolsUnsupported.into())
    }
}

/// A tool call the model asked for. `args` is the parsed JSON arguments; when
/// the model sent arguments that are not valid JSON, it is the raw text as a
/// [`Value::String`], for the caller to treat as a malformed call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
    /// Provider data the wire must send back with this call on the next turn,
    /// opaque to the caller. Only Gemini sets it: `{"thoughtSignature": "…"}`.
    pub extra: Option<Value>,
}

impl ModelToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, args: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            args,
            extra: None,
        }
    }
}

/// One message of a tool-calling transcript, independent of the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatMessage {
    User(String),
    Assistant {
        text: Option<String>,
        calls: Vec<ModelToolCall>,
    },
    /// The result of the call whose [`ModelToolCall::id`] is `call_id`.
    ToolResult {
        call_id: String,
        text: String,
    },
}

/// What the model answered in one [`LLMClient::chat`] turn.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelReply {
    /// A final answer, with no tool calls.
    Text(String),
    /// At least one tool call, plus any text the model wrote with them.
    Calls {
        text: Option<String>,
        calls: Vec<ModelToolCall>,
    },
}

/// The endpoint or model does not accept tools. The caller answers in a single
/// pass instead. Downcast it from the `anyhow::Error` that `chat` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the model endpoint does not support tool calling")]
pub struct ToolsUnsupported;

const OPENAI_URL: &str = "https://api.openai.com/v1";
const MISTRAL_URL: &str = "https://api.mistral.ai/v1";
const ANTHROPIC_URL: &str = "https://api.anthropic.com";
const GEMINI_URL: &str = "https://generativelanguage.googleapis.com";

/// Longest time one model request takes before it fails. Slow CPU-only local
/// models take minutes on a long context, so the limit is generous.
pub const LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// The HTTP client for model requests, with a total timeout per request.
fn http_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        // Fails only when no TLS backend starts, where `Client::new` panics
        // too.
        .expect("the HTTP client for the LLM provider")
}

/// The provider's wire shape — the only thing that differs between providers.
enum Wire {
    /// OpenAI chat-completions dialect: `{base}/chat/completions`, bearer auth.
    /// Speaks for OpenAI, Mistral, OpenRouter, and any OpenAI-compatible
    /// endpoint. `reasoning` is OpenRouter's `reasoning.effort` extension;
    /// `None` sends no `reasoning` object, so the model keeps its default.
    OpenAiCompat { reasoning: Option<ReasoningEffort> },
    /// Anthropic messages API: `{base}/v1/messages`, `x-api-key` header.
    Anthropic,
    /// Gemini generateContent: the model lives in the URL, the key in the
    /// `x-goog-api-key` header.
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
                Wire::OpenAiCompat { reasoning: None },
                url.clone().unwrap_or_else(|| OPENAI_URL.to_string()),
            ),
            LlmConfig::Mistral { .. } => (
                Wire::OpenAiCompat { reasoning: None },
                MISTRAL_URL.to_string(),
            ),
            LlmConfig::Claude { .. } => (Wire::Anthropic, ANTHROPIC_URL.to_string()),
            LlmConfig::Gemini { .. } => (Wire::Gemini, GEMINI_URL.to_string()),
            LlmConfig::OpenRouter { reasoning, .. } => (
                Wire::OpenAiCompat {
                    reasoning: *reasoning,
                },
                OPENROUTER_URL.to_string(),
            ),
        };
        Self {
            http: http_client(LLM_REQUEST_TIMEOUT),
            wire,
            provider: config.provider(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: config.model().to_string(),
            api_key,
        }
    }
}

impl ChatClient {
    /// The same client with another request timeout.
    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.http = http_client(timeout);
        self
    }

    /// A transport error without its URL on the Gemini wire. The key no
    /// longer travels in the URL; this keeps any part of a Gemini URL out of
    /// logs and error events all the same.
    fn scrub(&self, error: anyhow::Error) -> anyhow::Error {
        if !matches!(self.wire, Wire::Gemini) {
            return error;
        }
        match error.downcast::<reqwest::Error>() {
            Ok(e) => e.without_url().into(),
            Err(other) => other,
        }
    }

    /// The POST for this wire, with its endpoint and auth, carrying `body`.
    /// Shared by `ask` and `chat`, so both speak to the same URL the same way.
    fn post<T: Serialize + ?Sized>(&self, body: &T) -> reqwest::RequestBuilder {
        match &self.wire {
            Wire::OpenAiCompat { .. } => self
                .http
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(body),
            Wire::Anthropic => self
                .http
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(body),
            Wire::Gemini => self
                .http
                .post(format!(
                    "{}/v1beta/models/{}:generateContent",
                    self.base_url, self.model
                ))
                .header("x-goog-api-key", &self.api_key)
                .json(body),
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
        self.ask_inner(question, history, context)
            .await
            .map_err(|e| self.scrub(e))
    }

    async fn chat(
        &self,
        system: &str,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> anyhow::Result<ModelReply> {
        self.chat_inner(system, messages, tools)
            .await
            .map_err(|e| self.scrub(e))
    }
}

impl ChatClient {
    async fn ask_inner(
        &self,
        question: &str,
        history: &[(String, String)],
        context: &[(usize, (f64, FlattenedChunk))],
    ) -> anyhow::Result<String> {
        let messages = chat_messages(history, build_prompt(question, history, context));

        let request = match &self.wire {
            Wire::OpenAiCompat { reasoning } => self.post(&ChatRequest {
                model: self.model.clone(),
                max_tokens: None,
                messages,
                reasoning: reasoning.map(|effort| ReasoningRequest { effort }),
            }),
            Wire::Anthropic => self.post(&ChatRequest {
                model: self.model.clone(),
                max_tokens: Some(4096),
                messages,
                reasoning: None,
            }),
            Wire::Gemini => self.post(&GeminiRequest {
                contents: gemini_contents(messages),
            }),
        };

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(api_error(self.provider, status, &body));
        }

        let answer = match &self.wire {
            Wire::OpenAiCompat { .. } => {
                let parsed: ChatResponse = response.json().await?;
                parsed.log_usage(self.provider);
                answer_text(parsed, self.provider)?
            }
            Wire::Anthropic => {
                let parsed: AnthropicResponse = response.json().await?;
                parsed.log_usage(self.provider);
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
                parsed.log_usage(self.provider);
                parsed
                    .candidates
                    .into_iter()
                    .flat_map(|c| c.content.parts)
                    .filter_map(|p| p.text)
                    .collect::<Vec<String>>()
                    .join("\n")
            }
        };

        Ok(answer)
    }

    async fn chat_inner(
        &self,
        system: &str,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
    ) -> anyhow::Result<ModelReply> {
        let body = match &self.wire {
            Wire::OpenAiCompat { reasoning } => {
                openai_chat_body(&self.model, *reasoning, system, messages, tools)
            }
            Wire::Anthropic => anthropic_chat_body(&self.model, system, messages, tools),
            Wire::Gemini => gemini_chat_body(system, messages, tools)?,
        };

        let response = self.post(&body).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(chat_api_error(self.provider, status, &body));
        }

        match &self.wire {
            Wire::OpenAiCompat { .. } => {
                let parsed: ChatResponse = response.json().await?;
                parsed.log_usage(self.provider);
                openai_reply(parsed, self.provider)
            }
            Wire::Anthropic => {
                let parsed: AnthropicResponse = response.json().await?;
                parsed.log_usage(self.provider);
                anthropic_reply(parsed, self.provider)
            }
            Wire::Gemini => {
                let parsed: GeminiResponse = response.json().await?;
                parsed.log_usage(self.provider);
                gemini_reply(parsed, self.provider)
            }
        }
    }
}

/// The error for a failed provider call: provider, HTTP status and body.
fn api_error(provider: &str, status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    anyhow::anyhow!("{provider} API error: {status}\n{body}")
}

/// Most characters of a provider error line that go to the log.
const ERROR_LINE_CHARS: usize = 200;

/// The first line of `text`, cut to `max` characters. Logs carry this much of
/// a provider error, never the request content.
pub(crate) fn first_line(text: &str, max: usize) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(max)
        .collect()
}

/// [`api_error`] for a `chat` call, except that a client error whose body
/// talks about tools or functions means the endpoint or model rejects tool
/// calling: that becomes [`ToolsUnsupported`], so the caller falls back.
fn chat_api_error(provider: &str, status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    let client_error = matches!(status.as_u16(), 400 | 404 | 422);
    let lower = body.to_lowercase();
    // `tool` also covers `tools` and `tool_choice`.
    if client_error && (lower.contains("tool") || lower.contains("function")) {
        warn!(
            "{provider} rejected tools ({status}): {}",
            first_line(body, ERROR_LINE_CHARS)
        );
        return ToolsUnsupported.into();
    }
    api_error(provider, status, body)
}

/// The text of a reply: its non-blank parts joined, or `None` when there are
/// none.
fn joined_text(parts: impl IntoIterator<Item = String>) -> Option<String> {
    let parts: Vec<String> = parts
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// The reply for the text and calls of one model turn. No calls and no text is
/// the same "no answer text" error that `ask` raises.
fn model_reply(
    text: Option<String>,
    calls: Vec<ModelToolCall>,
    provider: &str,
) -> anyhow::Result<ModelReply> {
    if !calls.is_empty() {
        return Ok(ModelReply::Calls { text, calls });
    }
    match text {
        Some(text) => Ok(ModelReply::Text(text)),
        None => Err(no_answer_error(provider)),
    }
}

/// OpenAI-compatible `chat/completions` body: the system prompt as the first
/// message, assistant calls as `tool_calls` with JSON-string `arguments`, and
/// results as `role: "tool"` messages.
fn openai_chat_body(
    model: &str,
    reasoning: Option<ReasoningEffort>,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> Value {
    let mut msgs = Vec::with_capacity(messages.len() + 1);
    if !system.is_empty() {
        msgs.push(json!({"role": "system", "content": system}));
    }
    for message in messages {
        msgs.push(match message {
            ChatMessage::User(text) => json!({"role": "user", "content": text}),
            ChatMessage::Assistant { text, calls } => {
                let mut m = json!({"role": "assistant", "content": text});
                if !calls.is_empty() {
                    m["tool_calls"] = calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {"name": c.name, "arguments": openai_arguments(&c.args)},
                            })
                        })
                        .collect();
                }
                m
            }
            ChatMessage::ToolResult { call_id, text } => {
                json!({"role": "tool", "tool_call_id": call_id, "content": text})
            }
        });
    }
    let mut body = json!({"model": model, "messages": msgs});
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {"name": t.name, "description": t.description, "parameters": t.schema},
                })
            })
            .collect();
    }
    if let Some(effort) = reasoning {
        body["reasoning"] = json!(ReasoningRequest { effort });
    }
    body
}

/// A call's arguments as the JSON string OpenAI expects. A malformed call
/// (raw text kept as a string) goes back exactly as the model sent it.
fn openai_arguments(args: &Value) -> String {
    match args {
        Value::String(raw) => raw.clone(),
        other => other.to_string(),
    }
}

/// Anthropic `messages` body: `system` at the top level, assistant calls as
/// `tool_use` blocks after the text, and results as `tool_result` blocks in a
/// `user` message. Consecutive user-side messages merge into one, so every
/// result of a turn sits in the message right after it.
fn anthropic_chat_body(
    model: &str,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> Value {
    let mut msgs: Vec<(&str, Vec<Value>)> = Vec::new();
    for message in messages {
        let (role, blocks) = match message {
            ChatMessage::User(text) => ("user", vec![json!({"type": "text", "text": text})]),
            ChatMessage::Assistant { text, calls } => {
                // Anthropic rejects an empty text block.
                let text_block = text
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| json!({"type": "text", "text": t}));
                let call_blocks = calls.iter().map(
                    |c| json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.args}),
                );
                (
                    "assistant",
                    text_block.into_iter().chain(call_blocks).collect(),
                )
            }
            ChatMessage::ToolResult { call_id, text } => (
                "user",
                vec![json!({"type": "tool_result", "tool_use_id": call_id, "content": text})],
            ),
        };
        match msgs.last_mut() {
            Some((last_role, last_blocks)) if *last_role == role => last_blocks.extend(blocks),
            _ => msgs.push((role, blocks)),
        }
    }
    let msgs: Vec<Value> = msgs
        .into_iter()
        .map(|(role, content)| json!({"role": role, "content": content}))
        .collect();
    let mut body = json!({"model": model, "max_tokens": 4096, "messages": msgs});
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(
                |t| json!({"name": t.name, "description": t.description, "input_schema": t.schema}),
            )
            .collect();
    }
    body
}

/// Gemini `generateContent` body: `systemInstruction`, calls as `functionCall`
/// parts, and results as `functionResponse` parts in a `user` content.
/// Gemini matches a result to its call by function NAME, and its call ids are
/// synthesized per reply (they repeat across turns), so each result takes the
/// name from the most recent assistant turn's calls.
fn gemini_chat_body(
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> anyhow::Result<Value> {
    let mut contents: Vec<(&str, Vec<Value>)> = Vec::new();
    let mut names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for message in messages {
        let (role, parts) = match message {
            ChatMessage::User(text) => ("user", vec![json!({"text": text})]),
            ChatMessage::Assistant { text, calls } => {
                names = calls
                    .iter()
                    .map(|c| (c.id.as_str(), c.name.as_str()))
                    .collect();
                let text_part = text
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| json!({"text": t}));
                let call_parts = calls.iter().map(|c| {
                    let mut part = json!({"functionCall": {"name": c.name, "args": c.args}});
                    // Round-trip the part's thought signature, if it had one.
                    if let Some(sig) = c.extra.as_ref().and_then(|e| e.get(GEMINI_SIGNATURE)) {
                        part[GEMINI_SIGNATURE] = sig.clone();
                    }
                    part
                });
                ("model", text_part.into_iter().chain(call_parts).collect())
            }
            ChatMessage::ToolResult { call_id, text } => {
                let Some(name) = names.get(call_id.as_str()) else {
                    bail!("tool result {call_id} has no matching call in the previous turn");
                };
                (
                    "user",
                    vec![
                        json!({"functionResponse": {"name": name, "response": {"content": text}}}),
                    ],
                )
            }
        };
        match contents.last_mut() {
            Some((last_role, last_parts)) if *last_role == role => last_parts.extend(parts),
            _ => contents.push((role, parts)),
        }
    }
    let contents: Vec<Value> = contents
        .into_iter()
        .map(|(role, parts)| json!({"role": role, "parts": parts}))
        .collect();
    let mut body = json!({"contents": contents});
    if !system.is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": system}]});
    }
    if !tools.is_empty() {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|t| {
                let mut declaration = json!({"name": t.name, "description": t.description});
                // Gemini has rejected `parameters` with no properties, so a
                // tool without arguments sends none.
                let has_properties = t
                    .schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|p| !p.is_empty());
                if has_properties {
                    declaration["parameters"] = gemini_schema(&t.schema);
                }
                declaration
            })
            .collect();
        body["tools"] = json!([{"functionDeclarations": declarations}]);
    }
    Ok(body)
}

/// The key of a Gemini thought signature, both on a reply part and in
/// [`ModelToolCall::extra`].
const GEMINI_SIGNATURE: &str = "thoughtSignature";

/// JSON-schema keywords Gemini's function declarations reject.
const GEMINI_REJECTED_KEYWORDS: [&str; 8] = [
    "additionalProperties",
    "$schema",
    "$id",
    "$ref",
    "$defs",
    "$comment",
    "definitions",
    "examples",
];

/// `schema` without the keywords Gemini rejects, at every nesting level.
/// Only schema positions are walked: the names under `properties` are
/// property names, never keywords, so a property called `additionalProperties`
/// stays.
fn gemini_schema(schema: &Value) -> Value {
    let Value::Object(map) = schema else {
        return schema.clone();
    };
    let mut out = serde_json::Map::new();
    for (key, value) in map {
        if GEMINI_REJECTED_KEYWORDS.contains(&key.as_str()) {
            continue;
        }
        let value = match (key.as_str(), value) {
            ("properties", Value::Object(props)) => Value::Object(
                props
                    .iter()
                    .map(|(name, s)| (name.clone(), gemini_schema(s)))
                    .collect(),
            ),
            ("items", s) => match s {
                Value::Array(list) => Value::Array(list.iter().map(gemini_schema).collect()),
                s => gemini_schema(s),
            },
            ("anyOf" | "oneOf" | "allOf", Value::Array(list)) => {
                Value::Array(list.iter().map(gemini_schema).collect())
            }
            (_, v) => v.clone(),
        };
        out.insert(key.clone(), value);
    }
    Value::Object(out)
}

/// An OpenAI-style reply. `arguments` is a JSON string per the API; one that
/// is not valid JSON stays a raw [`Value::String`] for the caller to count as
/// malformed, and an object (some compatible servers) is taken as is. A blank
/// string or `null` is no arguments: local servers send it for a tool that
/// takes none.
fn openai_reply(response: ChatResponse, provider: &str) -> anyhow::Result<ModelReply> {
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    for choice in response.choices {
        texts.extend(choice.message.content);
        for c in choice.message.tool_calls.unwrap_or_default() {
            let args = match c.function.arguments {
                Value::String(raw) if raw.trim().is_empty() => json!({}),
                Value::String(raw) => match serde_json::from_str(&raw) {
                    Ok(Value::Null) => json!({}),
                    Ok(parsed) => parsed,
                    Err(_) => Value::String(raw),
                },
                Value::Null => json!({}),
                other => other,
            };
            let id =
                c.id.filter(|id| !id.is_empty())
                    .unwrap_or_else(|| format!("call_{}", calls.len()));
            calls.push(ModelToolCall::new(id, c.function.name, args));
        }
    }
    model_reply(joined_text(texts), calls, provider)
}

fn anthropic_reply(response: AnthropicResponse, provider: &str) -> anyhow::Result<ModelReply> {
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    for block in response.content {
        match block.kind.as_str() {
            "text" => texts.push(block.text),
            "tool_use" => calls.push(ModelToolCall::new(block.id, block.name, block.input)),
            _ => {}
        }
    }
    model_reply(joined_text(texts), calls, provider)
}

/// A Gemini reply. Gemini gives calls no ids, so they get `call_0`, `call_1`,
/// … in reply order.
fn gemini_reply(response: GeminiResponse, provider: &str) -> anyhow::Result<ModelReply> {
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    for part in response
        .candidates
        .into_iter()
        .flat_map(|c| c.content.parts)
    {
        texts.extend(part.text);
        if let Some(fc) = part.function_call {
            calls.push(ModelToolCall {
                extra: part
                    .thought_signature
                    .map(|sig| json!({ GEMINI_SIGNATURE: sig })),
                id: format!("call_{}", calls.len()),
                name: fc.name,
                args: if fc.args.is_null() {
                    json!({})
                } else {
                    fc.args
                },
            });
        }
    }
    model_reply(joined_text(texts), calls, provider)
}

/// The answer text of an OpenAI-style response: every choice's content,
/// joined. Null, absent and blank content all count as "no text": a reasoning
/// model that spends its whole budget thinking returns one of them, and an
/// empty answer would show in Ask as a blank reply with no explanation.
fn answer_text(response: ChatResponse, provider: &str) -> anyhow::Result<String> {
    let parts: Vec<String> = response
        .choices
        .into_iter()
        .filter_map(|c| c.message.content)
        .filter(|text| !text.trim().is_empty())
        .collect();
    if parts.is_empty() {
        return Err(no_answer_error(provider));
    }
    Ok(parts.join("\n"))
}

/// The error for a reply with no answer text.
fn no_answer_error(provider: &str) -> anyhow::Error {
    // Only OpenRouter has a reasoning setting to point the user at.
    let hint = if provider == "openrouter" {
        " A reasoning model can use its whole token budget thinking: set a lower `reasoning` effort or pick another model."
    } else {
        ""
    };
    anyhow::anyhow!("{provider} returned no answer text (empty content).{hint}")
}

/// History pairs + the final RAG prompt as one chat transcript. Shared by the
/// OpenAI-compat and Anthropic wires; Gemini maps the same list to `contents`
/// with the "model" role name.
fn chat_messages(history: &[(String, String)], prompt: String) -> Vec<WireMessage> {
    let mut msgs = Vec::with_capacity(history.len() * 2 + 1);
    for (q, a) in history {
        msgs.push(WireMessage {
            role: "user".into(),
            content: q.clone(),
        });
        msgs.push(WireMessage {
            role: "assistant".into(),
            content: a.clone(),
        });
    }
    msgs.push(WireMessage {
        role: "user".into(),
        content: prompt,
    });
    msgs
}

/// Maps a chat transcript (`chat_messages`'s output) to Gemini's `contents`
/// shape: every `"assistant"` role becomes `"model"`, everything else (only
/// ever `"user"` in practice) stays `"user"`, and each message's text becomes
/// its single part.
fn gemini_contents(messages: Vec<WireMessage>) -> Vec<GeminiContent> {
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
/// answer may supplement with common knowledge, each such passage starting
/// with [`MODEL_ONLY_LABEL`] and never cited, so the two never blur.
///
/// When `history` is non-empty the prompt gains two mid-conversation aids: a
/// follow-up-handling line and a one-line recap of the assistant's last message.
/// The preceding turns ride in the chat transcript (see [`chat_messages`]), and
/// a terse reply ("yes", "go on", "the second one") only makes sense against
/// the assistant's PREVIOUS message — usually its closing question or offer. The
/// follow-up line binds the reply to that message explicitly (continue from it;
/// do NOT re-answer the user's earlier question), and the recap quotes the tail
/// of that message immediately before `Question:` so the offer sits physically
/// next to the terse reply instead of several messages up the transcript. Both
/// are omitted for a first turn (empty history), where there is nothing to
/// resolve a terse reply against — the empty-history prompt is byte-identical to
/// before.
/// Core's chunk-breadcrumb separator (ASCII Unit Separator, U+001F): the
/// heading path is flattened into the chunk title joined with this control
/// char. Mirrored here (the server does not depend on `kimun_core`) so the
/// prompt can present it as a readable path rather than a raw control char.
const BREADCRUMB_SEP: char = '\u{1f}';

/// Chars quoted from the END of the assistant's last message in the recap line
/// (see [`build_prompt`]). ~200 chars is enough to carry a closing question or
/// offer without pulling in the whole answer.
const RECAP_TAIL_CHARS: usize = 200;

/// The last `max` chars of `s`, cut on a char boundary (never mid-codepoint).
/// Shorter strings pass through whole. Used for the mid-conversation recap,
/// which quotes the END of the assistant's previous message (where its offer
/// or closing question lives).
fn tail_chars(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let start = s
        .char_indices()
        .nth(total - max)
        .map(|(i, _)| i)
        .unwrap_or(0);
    s[start..].to_string()
}

/// The numbered context block: one entry per chunk, framed `[n] path — "title"`
/// with an optional `Date:` line, then the chunk text. Shared by the
/// single-pass prompt and the answer loop, so the model reads chunks the same
/// way on both paths.
pub(crate) fn render_context(context: &[(usize, (f64, FlattenedChunk))]) -> String {
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
    context_string
}

/// The follow-up instruction, only present mid-conversation: a terse reply is
/// meaningless on a first turn, so the guidance would just be noise there. The
/// line binds a short reply to the assistant's PREVIOUS message (its closing
/// question/offer), not to the user's earlier question — the failure this
/// fixes was the model re-answering that earlier question instead of
/// continuing from its offer.
pub(crate) const FOLLOWUP_INSTRUCTION: &str = "If the user's latest message is a short or affirmative reply (e.g. 'yes', 'go on', 'the second one'), it responds to your PREVIOUS message above — usually its closing question or offer. Continue from THAT message: carry out what it proposed, and do NOT re-answer the user's earlier question. Draw on the context below where it fits and the conversation itself otherwise.\n";

/// Mid-conversation, the tail of the assistant's last message, quoted right
/// before `Question:` so its closing offer sits next to the terse reply
/// instead of several messages up the transcript. Empty history → no recap.
/// Citation markers are already stripped from history answers by the client.
pub(crate) fn recap_line(history: &[(String, String)]) -> String {
    match history.last() {
        Some((_, prev_a)) => {
            let tail = tail_chars(prev_a, RECAP_TAIL_CHARS);
            format!("Your previous message ended: \"{tail}\"\n")
        }
        None => String::new(),
    }
}

fn build_prompt(
    question: &str,
    history: &[(String, String)],
    context: &[(usize, (f64, FlattenedChunk))],
) -> String {
    let context_string = render_context(context);
    let followup_line = if history.is_empty() {
        ""
    } else {
        FOLLOWUP_INSTRUCTION
    };
    let recap = recap_line(history);

    format!(
        r#"You are an intelligent assistant with access to a personal knowledge base.
Answer the user's question using the numbered context below first; base the answer primarily on it when it is relevant.
{followup_line}Every claim drawn from the context MUST carry an inline citation in the form [n], where n is the number of the supporting context entry. A sentence may carry several citations.
You may supplement with accurate, widely accepted common knowledge when the context falls short. Start each such passage with the label "{MODEL_ONLY_LABEL}" and never cite [n] for it.
Preserve any [[wikilinks]] and #tags that appear in the context verbatim when you quote or reference them.
If neither the context nor common knowledge suffices, respond with: 'I don't have enough information to answer.'

Context:
---------------------
{context_string}---------------------

{recap}Question: {question}"#
    )
}

// ── Wire types ──────────────────────────────────────────────────────────────
// Minimal and tolerant: only the fields the server reads, usage optional, so a
// provider adding response fields never breaks parsing.

/// Request body for the OpenAI-compat, OpenRouter and Anthropic wires
/// (Anthropic additionally requires `max_tokens`; omitted for OpenAI-compat).
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    messages: Vec<WireMessage>,
    /// OpenRouter only.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningRequest>,
}

/// OpenRouter's unified `reasoning` request object. The answer text comes back
/// in `message.content`; any reasoning trace in `message.reasoning` is ignored.
#[derive(Serialize)]
struct ReasoningRequest {
    effort: ReasoningEffort,
}

#[derive(Serialize, Deserialize)]
struct WireMessage {
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
    message: ChatResponseMessage,
}

/// A response message. `content` is `null` or absent when a reasoning model
/// spends its whole budget thinking (OpenRouter surfaces this), so it is
/// optional here, unlike the request-side [`WireMessage`].
#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<String>,
    /// Only on a `chat` reply that asks for tool calls.
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    /// Some compatible servers leave it out or send `null`; [`openai_reply`]
    /// then makes one up.
    #[serde(default)]
    id: Option<String>,
    function: OpenAiFunctionCall,
}

#[derive(Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    /// A JSON-encoded string per the OpenAI API. Some compatible servers send
    /// a plain object instead, so any JSON value is accepted here.
    #[serde(default)]
    arguments: Value,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

impl ChatResponse {
    fn log_usage(&self, provider: &str) {
        if let Some(usage) = &self.usage {
            debug!(
                "{provider} tokens: {} prompt, {} completion",
                usage.prompt_tokens, usage.completion_tokens
            );
        }
    }
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
    /// `tool_use` blocks only.
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    input: Value,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

impl AnthropicResponse {
    fn log_usage(&self, provider: &str) {
        if let Some(usage) = &self.usage {
            debug!(
                "{provider} tokens: {} input, {} output",
                usage.input_tokens, usage.output_tokens
            );
        }
    }
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
    parts: Vec<GeminiResponsePart>,
}

/// A reply part: text, or a function call on a `chat` reply.
#[derive(Deserialize)]
struct GeminiResponsePart {
    #[serde(default)]
    text: Option<String>,
    #[serde(rename = "functionCall", default)]
    function_call: Option<GeminiFunctionCall>,
    /// Gemini thinking models sign a `functionCall` part; the signature must go
    /// back on that same part in the next request.
    #[serde(rename = "thoughtSignature", default)]
    thought_signature: Option<String>,
}

#[derive(Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Deserialize)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount", default)]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount", default)]
    candidates_token_count: u32,
}

impl GeminiResponse {
    fn log_usage(&self, provider: &str) {
        if let Some(usage) = &self.usage_metadata {
            debug!(
                "{provider} tokens: {} prompt, {} candidates",
                usage.prompt_token_count, usage.candidates_token_count
            );
        }
    }
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
                meta: Default::default(),
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
        // Passages from the model's own knowledge carry the label.
        assert!(p.contains(MODEL_ONLY_LABEL), "{p}");
        assert!(!p.contains("leaving them uncited"), "{p}");
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
        assert!(
            !p.contains('\u{1f}'),
            "no control char leaks into the prompt"
        );
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
    fn followup_instruction_binds_short_replies_to_the_assistants_last_message() {
        let context = numbered(vec![chunk("/a.md", "A", "body")]);

        // First turn (empty history): no follow-up guidance — a terse reply is
        // meaningless with nothing to resolve it against.
        let first = build_prompt("yes", &[], &context);
        assert!(
            !first.contains("your PREVIOUS message"),
            "empty history must not add the follow-up line: {first}"
        );

        // Mid-conversation: the guidance must bind the terse reply to the
        // ASSISTANT'S PREVIOUS message and forbid re-answering the earlier
        // question — the strengthened instruction the fix installs.
        let history = vec![(
            "Should I explain the setup steps?".to_string(),
            "There are three steps; want the details?".to_string(),
        )];
        let mid = build_prompt("yes", &history, &context);
        assert!(
            mid.contains("your PREVIOUS message"),
            "follow-up line must bind the reply to the assistant's previous message: {mid}"
        );
        assert!(
            mid.contains("do NOT re-answer the user's earlier question"),
            "follow-up line must forbid re-answering the earlier question: {mid}"
        );
        // The raw current question is still the prompt's Question, untouched.
        assert!(mid.contains("Question: yes"));
    }

    #[test]
    fn recap_quotes_the_last_answers_tail_only_with_history() {
        let context = numbered(vec![chunk("/a.md", "A", "body")]);

        // First turn: no recap line at all — byte-identical first-turn prompt.
        let first = build_prompt("yes", &[], &context);
        assert!(
            !first.contains("Your previous message ended:"),
            "empty history must not add a recap line: {first}"
        );

        // Mid-conversation: the recap quotes the assistant's last message,
        // carrying its END (the closing offer) right before the Question.
        let closing = "Would you like tips on planting or caring for rosemary in your raised beds?";
        let history = vec![(
            "tell me more about rosemary seeds".to_string(),
            format!("Rosemary seeds are slow to germinate. {closing}"),
        )];
        let mid = build_prompt("yes", &history, &context);
        assert!(
            mid.contains("Your previous message ended:"),
            "history present must add a recap line: {mid}"
        );
        assert!(
            mid.contains(closing),
            "recap must carry the answer's TAIL (its closing offer): {mid}"
        );
        // The recap sits immediately before the (untouched) Question.
        let recap_at = mid.find("Your previous message ended:").unwrap();
        let question_at = mid.find("Question: yes").unwrap();
        assert!(recap_at < question_at, "recap precedes Question: {mid}");
    }

    #[test]
    fn recap_keeps_only_the_tail_of_a_long_answer_on_a_char_boundary() {
        let context = numbered(vec![chunk("/a.md", "A", "body")]);
        // A long multi-byte opening then a distinctive closing offer: the recap
        // drops the opening and keeps the closing, cut on a char boundary.
        let opening = "é".repeat(RECAP_TAIL_CHARS);
        let closing = "SHALL_I_CONTINUE?";
        let history = vec![("q".to_string(), format!("{opening} {closing}"))];
        let mid = build_prompt("yes", &history, &context);
        assert!(mid.contains(closing), "tail (offer) kept: {mid}");
        // The full opening cannot survive — it is longer than the tail budget.
        let recap_es = mid
            .lines()
            .find(|l| l.starts_with("Your previous message ended:"))
            .unwrap()
            .chars()
            .filter(|c| *c == 'é')
            .count();
        assert!(
            recap_es < RECAP_TAIL_CHARS,
            "opening trimmed to fit the tail budget: {recap_es}"
        );
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

        let answer = client(Wire::OpenAiCompat { reasoning: None }, base)
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
    async fn openrouter_wire_sends_reasoning_effort_and_ignores_reasoning_trace() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_provider(
            "/chat/completions",
            serde_json::json!({
                "choices": [{"message": {
                    "role": "assistant",
                    "content": "the answer",
                    "reasoning": "thinking out loud"
                }}]
            }),
            captured.clone(),
        )
        .await;

        let answer = client(
            Wire::OpenAiCompat {
                reasoning: Some(ReasoningEffort::High),
            },
            base,
        )
        .ask("q?", &[], &[])
        .await
        .unwrap();
        assert_eq!(answer, "the answer");

        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["authorization"], "Bearer sk-test");
        assert_eq!(body["reasoning"], serde_json::json!({"effort": "high"}));
    }

    #[tokio::test]
    async fn null_or_missing_content_is_a_clear_error() {
        for message in [
            serde_json::json!({"role": "assistant", "content": null, "reasoning": "hmm"}),
            serde_json::json!({"role": "assistant"}),
            serde_json::json!({"role": "assistant", "content": "  \n"}),
        ] {
            let captured: Captured = Arc::new(Mutex::new(None));
            let base = mock_provider(
                "/chat/completions",
                serde_json::json!({"choices": [{"message": message}]}),
                captured,
            )
            .await;
            let err = client(Wire::OpenAiCompat { reasoning: None }, base)
                .ask("q?", &[], &[])
                .await
                .expect_err("no content must not be an empty answer");
            assert!(err.to_string().contains("no answer text"), "{err}");
        }
    }

    #[test]
    fn empty_answer_hint_names_reasoning_only_for_openrouter() {
        let empty = || ChatResponse {
            choices: vec![],
            usage: None,
        };
        let or = answer_text(empty(), "openrouter").unwrap_err().to_string();
        assert!(or.contains("`reasoning`"), "{or}");
        let oa = answer_text(empty(), "openai").unwrap_err().to_string();
        assert!(!oa.contains("reasoning"), "{oa}");
    }

    #[tokio::test]
    async fn null_content_in_one_choice_keeps_the_others() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_provider(
            "/chat/completions",
            serde_json::json!({"choices": [
                {"message": {"role": "assistant", "content": null}},
                {"message": {"role": "assistant", "content": "kept"}}
            ]}),
            captured,
        )
        .await;
        let answer = client(Wire::OpenAiCompat { reasoning: None }, base)
            .ask("q?", &[], &[])
            .await
            .unwrap();
        assert_eq!(answer, "kept");
    }

    #[tokio::test]
    async fn openrouter_wire_without_effort_omits_reasoning() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_provider(
            "/chat/completions",
            serde_json::json!({"choices": [{"message": {"role": "assistant", "content": "ok"}}]}),
            captured.clone(),
        )
        .await;
        client(Wire::OpenAiCompat { reasoning: None }, base)
            .ask("q?", &[], &[])
            .await
            .unwrap();
        let (_, body) = captured.lock().unwrap().take().unwrap();
        assert!(body.get("reasoning").is_none());
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
    async fn gemini_wire_sends_key_in_header_and_parses_candidate_parts() {
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
            "gemini auth is the x-goog-api-key header"
        );
        assert_eq!(headers.get("x-goog-api-key").unwrap(), "sk-test");
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

        let err = client(
            Wire::OpenAiCompat { reasoning: None },
            format!("http://{addr}"),
        )
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
            (
                LlmConfig::OpenRouter {
                    model: "m".into(),
                    api_key: None,
                    reasoning: None,
                },
                OPENROUTER_URL,
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

    // ── Tool calling (`chat`) ────────────────────────────────────────────────

    fn spec(name: &'static str, schema: serde_json::Value) -> ToolSpec {
        ToolSpec {
            name,
            description: "a tool",
            schema,
        }
    }

    fn call(id: &str, name: &str, args: serde_json::Value) -> ModelToolCall {
        ModelToolCall::new(id, name, args)
    }

    /// A user question, an assistant turn with text and two calls, both
    /// results, then a follow-up question: every message shape at once.
    fn transcript() -> Vec<ChatMessage> {
        vec![
            ChatMessage::User("what did I plant?".into()),
            ChatMessage::Assistant {
                text: Some("Let me look.".into()),
                calls: vec![
                    call("c1", "search", json!({"query": "plant"})),
                    call("c2", "list_date_keys", json!({})),
                ],
            },
            ChatMessage::ToolResult {
                call_id: "c1".into(),
                text: "result one".into(),
            },
            ChatMessage::ToolResult {
                call_id: "c2".into(),
                text: "result two".into(),
            },
        ]
    }

    fn search_schema() -> serde_json::Value {
        json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]})
    }

    #[test]
    fn openai_chat_body_carries_tools_calls_and_tool_results() {
        let body = openai_chat_body(
            "m",
            Some(ReasoningEffort::Low),
            "SYSTEM",
            &transcript(),
            &[spec("search", search_schema())],
        );
        assert_eq!(body["model"], "m");
        assert_eq!(body["reasoning"], json!({"effort": "low"}));
        assert_eq!(
            body["tools"],
            json!([{"type": "function", "function": {
                "name": "search", "description": "a tool", "parameters": search_schema()
            }}])
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0], json!({"role": "system", "content": "SYSTEM"}));
        assert_eq!(
            msgs[1],
            json!({"role": "user", "content": "what did I plant?"})
        );
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[2]["content"], "Let me look.");
        assert_eq!(
            msgs[2]["tool_calls"][0],
            json!({"id": "c1", "type": "function", "function": {
                "name": "search", "arguments": "{\"query\":\"plant\"}"
            }})
        );
        assert_eq!(msgs[2]["tool_calls"][1]["function"]["arguments"], "{}");
        assert_eq!(
            msgs[3],
            json!({"role": "tool", "tool_call_id": "c1", "content": "result one"})
        );
        assert_eq!(
            msgs[4],
            json!({"role": "tool", "tool_call_id": "c2", "content": "result two"})
        );
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn openai_chat_body_without_reasoning_or_tools_omits_both() {
        let body = openai_chat_body("m", None, "S", &[ChatMessage::User("q".into())], &[]);
        assert!(body.get("reasoning").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn openai_reply_with_calls_parses_arguments_and_keeps_text() {
        let response: ChatResponse = serde_json::from_value(json!({
            "choices": [{"message": {
                "role": "assistant",
                "content": "Searching.",
                "tool_calls": [
                    {"id": "call_a", "type": "function",
                     "function": {"name": "search", "arguments": "{\"query\":\"rosemary\"}"}},
                    {"id": "call_b", "type": "function",
                     "function": {"name": "list_date_keys", "arguments": "{}"}}
                ]
            }}]
        }))
        .unwrap();
        assert_eq!(
            openai_reply(response, "test").unwrap(),
            ModelReply::Calls {
                text: Some("Searching.".into()),
                calls: vec![
                    call("call_a", "search", json!({"query": "rosemary"})),
                    call("call_b", "list_date_keys", json!({})),
                ],
            }
        );
    }

    #[test]
    fn openai_reply_text_only_is_text_and_blank_is_the_no_answer_error() {
        let text: ChatResponse = serde_json::from_value(json!({
            "choices": [{"message": {"role": "assistant", "content": "the answer"}}]
        }))
        .unwrap();
        assert_eq!(
            openai_reply(text, "test").unwrap(),
            ModelReply::Text("the answer".into())
        );
        let blank: ChatResponse = serde_json::from_value(json!({
            "choices": [{"message": {"role": "assistant", "content": null}}]
        }))
        .unwrap();
        let err = openai_reply(blank, "test").unwrap_err().to_string();
        assert!(err.contains("no answer text"), "{err}");
    }

    #[test]
    fn openai_reply_with_malformed_arguments_keeps_the_raw_string() {
        let response: ChatResponse = serde_json::from_value(json!({
            "choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
                {"id": "x", "type": "function",
                 "function": {"name": "search", "arguments": "{\"query\": oops"}}
            ]}}]
        }))
        .unwrap();
        assert_eq!(
            openai_reply(response, "test").unwrap(),
            ModelReply::Calls {
                text: None,
                calls: vec![call("x", "search", json!("{\"query\": oops"))],
            }
        );
    }

    fn reply_args(arguments: Value) -> Value {
        let response: ChatResponse = serde_json::from_value(json!({
            "choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
                {"id": "x", "type": "function",
                 "function": {"name": "list_date_keys", "arguments": arguments}}
            ]}}]
        }))
        .unwrap();
        let ModelReply::Calls { calls, .. } = openai_reply(response, "test").unwrap() else {
            panic!("no calls");
        };
        calls[0].args.clone()
    }

    #[test]
    fn openai_reply_with_blank_or_null_arguments_has_no_arguments() {
        assert_eq!(reply_args(json!("")), json!({}));
        assert_eq!(reply_args(json!("  \n\t ")), json!({}));
        assert_eq!(reply_args(Value::Null), json!({}));
        assert_eq!(reply_args(json!("null")), json!({}));
    }

    #[test]
    fn openai_reply_with_non_blank_invalid_arguments_stays_raw() {
        assert_eq!(reply_args(json!(" oops ")), json!(" oops "));
        assert_eq!(reply_args(json!("{}")), json!({}));
    }

    #[test]
    fn gemini_request_carries_the_key_in_a_header_not_the_url() {
        let mut c = client(Wire::Gemini, "https://example.test".into());
        c.api_key = "SECRET-KEY-123".into();
        let request = c.post(&json!({})).build().unwrap();
        let url = request.url().as_str();
        assert!(!url.contains("SECRET-KEY-123"), "{url}");
        assert!(!url.contains("key="), "{url}");
        assert_eq!(
            url,
            "https://example.test/v1beta/models/test-model:generateContent"
        );
        assert_eq!(
            request.headers().get("x-goog-api-key").unwrap(),
            "SECRET-KEY-123"
        );
    }

    #[tokio::test]
    async fn gemini_transport_error_does_not_leak_the_key() {
        // A port that was free a moment ago: the connection is refused.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let mut c = client(Wire::Gemini, format!("http://{addr}"));
        c.api_key = "SECRET-KEY-123".into();
        let chat = c
            .chat("system", &[ChatMessage::User("q".into())], &[])
            .await
            .expect_err("a closed port fails");
        let ask = c
            .ask("q?", &[], &[])
            .await
            .expect_err("a closed port fails");
        for err in [chat, ask] {
            for text in [format!("{err}"), format!("{err:?}"), format!("{err:#}")] {
                assert!(!text.contains("SECRET-KEY-123"), "{text}");
                assert!(!text.contains("key="), "{text}");
                assert!(!text.contains(&addr.to_string()), "no url: {text}");
            }
        }
    }

    #[tokio::test]
    async fn stalled_provider_fails_at_the_request_timeout() {
        // Accepts the connection and never answers.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let c = client(
            Wire::OpenAiCompat { reasoning: None },
            format!("http://{addr}"),
        )
        .with_timeout(Duration::from_millis(200));
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            c.chat("system", &[ChatMessage::User("q".into())], &[]),
        )
        .await
        .expect("the request timeout fires before the test's own");
        let elapsed = started.elapsed();
        let err = result.expect_err("a stalled call fails");
        assert!(
            err.downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout),
            "{err:?}"
        );
        assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    }

    #[test]
    fn first_line_keeps_one_line_and_cuts_it() {
        assert_eq!(
            first_line("\n  {\"error\": \"no tools\"}\nsecret body", 200),
            "{\"error\": \"no tools\"}"
        );
        assert_eq!(first_line(&"é".repeat(300), 200).chars().count(), 200);
        assert_eq!(first_line("", 200), "");
    }

    #[test]
    fn openai_reply_synthesizes_a_missing_or_null_call_id() {
        let response: ChatResponse = serde_json::from_value(json!({
            "choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
                {"type": "function", "function": {"name": "search", "arguments": "{}"}},
                {"id": null, "type": "function", "function": {"name": "list_date_keys", "arguments": "{}"}},
                {"id": "", "type": "function", "function": {"name": "read_note", "arguments": "{}"}},
                {"id": "kept", "type": "function", "function": {"name": "related_notes", "arguments": "{}"}}
            ]}}]
        }))
        .unwrap();
        let ModelReply::Calls { calls, .. } = openai_reply(response, "test").unwrap() else {
            panic!("calls expected");
        };
        let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["call_0", "call_1", "call_2", "kept"]);
    }

    #[test]
    fn gemini_chat_body_omits_parameters_of_a_tool_without_arguments() {
        let specs = crate::tools::tool_specs();
        let body = gemini_chat_body("S", &[ChatMessage::User("q".into())], &specs).unwrap();
        let decls = body["tools"][0]["functionDeclarations"].as_array().unwrap();
        let list = decls
            .iter()
            .find(|d| d["name"] == "list_date_keys")
            .unwrap();
        assert!(list.get("parameters").is_none(), "{list}");
        let search = decls.iter().find(|d| d["name"] == "search").unwrap();
        assert_eq!(search["parameters"]["type"], "object");
    }

    #[test]
    fn openai_chat_body_sends_a_malformed_call_back_as_its_raw_text() {
        let msgs = vec![ChatMessage::Assistant {
            text: None,
            calls: vec![call("x", "search", json!("{\"query\": oops"))],
        }];
        let body = openai_chat_body("m", None, "S", &msgs, &[]);
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["arguments"],
            "{\"query\": oops"
        );
        assert_eq!(body["messages"][1]["content"], serde_json::Value::Null);
    }

    #[test]
    fn anthropic_chat_body_has_input_schema_tool_use_and_tool_result_blocks() {
        let mut msgs = transcript();
        msgs.push(ChatMessage::User("and then?".into()));
        let body = anthropic_chat_body("m", "SYSTEM", &msgs, &[spec("search", search_schema())]);
        assert_eq!(body["model"], "m");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["system"], "SYSTEM");
        assert_eq!(
            body["tools"],
            json!([{"name": "search", "description": "a tool", "input_schema": search_schema()}])
        );
        let m = body["messages"].as_array().unwrap();
        assert_eq!(
            m.len(),
            3,
            "user, assistant, user(results + follow-up): {m:?}"
        );
        assert_eq!(
            m[0],
            json!({"role": "user", "content": [{"type": "text", "text": "what did I plant?"}]})
        );
        assert_eq!(
            m[1],
            json!({"role": "assistant", "content": [
                {"type": "text", "text": "Let me look."},
                {"type": "tool_use", "id": "c1", "name": "search", "input": {"query": "plant"}},
                {"type": "tool_use", "id": "c2", "name": "list_date_keys", "input": {}}
            ]})
        );
        assert_eq!(
            m[2],
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "c1", "content": "result one"},
                {"type": "tool_result", "tool_use_id": "c2", "content": "result two"},
                {"type": "text", "text": "and then?"}
            ]})
        );
    }

    #[test]
    fn anthropic_reply_parses_tool_use_blocks_and_text() {
        let response: AnthropicResponse = serde_json::from_value(json!({
            "content": [
                {"type": "text", "text": "Let me check."},
                {"type": "tool_use", "id": "toolu_1", "name": "search", "input": {"query": "x"}}
            ],
            "stop_reason": "tool_use"
        }))
        .unwrap();
        assert_eq!(
            anthropic_reply(response, "test").unwrap(),
            ModelReply::Calls {
                text: Some("Let me check.".into()),
                calls: vec![call("toolu_1", "search", json!({"query": "x"}))],
            }
        );
        let text: AnthropicResponse = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]
        }))
        .unwrap();
        assert_eq!(
            anthropic_reply(text, "test").unwrap(),
            ModelReply::Text("a\nb".into())
        );
        let blank: AnthropicResponse =
            serde_json::from_value(json!({"content": [{"type": "text", "text": " "}]})).unwrap();
        let err = anthropic_reply(blank, "test").unwrap_err().to_string();
        assert!(err.contains("no answer text"), "{err}");
    }

    #[test]
    fn gemini_chat_body_has_function_declarations_calls_and_named_responses() {
        let body =
            gemini_chat_body("SYSTEM", &transcript(), &[spec("search", search_schema())]).unwrap();
        assert_eq!(
            body["systemInstruction"],
            json!({"parts": [{"text": "SYSTEM"}]})
        );
        assert_eq!(
            body["tools"],
            json!([{"functionDeclarations": [
                {"name": "search", "description": "a tool", "parameters": search_schema()}
            ]}])
        );
        let c = body["contents"].as_array().unwrap();
        assert_eq!(c.len(), 3, "{c:?}");
        assert_eq!(
            c[0],
            json!({"role": "user", "parts": [{"text": "what did I plant?"}]})
        );
        assert_eq!(
            c[1],
            json!({"role": "model", "parts": [
                {"text": "Let me look."},
                {"functionCall": {"name": "search", "args": {"query": "plant"}}},
                {"functionCall": {"name": "list_date_keys", "args": {}}}
            ]})
        );
        assert_eq!(
            c[2],
            json!({"role": "user", "parts": [
                {"functionResponse": {"name": "search", "response": {"content": "result one"}}},
                {"functionResponse": {"name": "list_date_keys", "response": {"content": "result two"}}}
            ]})
        );
    }

    #[test]
    fn gemini_tool_result_names_follow_the_latest_assistant_turn() {
        // Gemini ids repeat across replies (`call_0` each time), so a result
        // takes its name from the turn just before it, not an earlier one.
        let msgs = vec![
            ChatMessage::User("q".into()),
            ChatMessage::Assistant {
                text: None,
                calls: vec![call("call_0", "search", json!({"query": "a"}))],
            },
            ChatMessage::ToolResult {
                call_id: "call_0".into(),
                text: "r1".into(),
            },
            ChatMessage::Assistant {
                text: None,
                calls: vec![call("call_0", "read_note", json!({"path": "a.md"}))],
            },
            ChatMessage::ToolResult {
                call_id: "call_0".into(),
                text: "r2".into(),
            },
        ];
        let body = gemini_chat_body("S", &msgs, &[]).unwrap();
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"]["name"],
            "search"
        );
        assert_eq!(
            body["contents"][4]["parts"][0]["functionResponse"]["name"],
            "read_note"
        );
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn gemini_schema_strips_rejected_keywords_at_every_level() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                // A property NAMED like a keyword is a property, not a keyword.
                "additionalProperties": {"type": "string"},
                "nested": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {"x": {"type": "string"}}
                },
                "tags": {"type": "array", "items": {"type": "object", "additionalProperties": false}}
            },
            "required": ["nested"]
        });
        assert_eq!(
            gemini_schema(&schema),
            json!({
                "type": "object",
                "properties": {
                    "additionalProperties": {"type": "string"},
                    "nested": {"type": "object", "properties": {"x": {"type": "string"}}},
                    "tags": {"type": "array", "items": {"type": "object"}}
                },
                "required": ["nested"]
            })
        );
        let body = gemini_chat_body("S", &[], &[spec("t", schema)]).unwrap();
        assert!(
            !body["tools"][0]["functionDeclarations"][0]["parameters"]
                .to_string()
                .contains("\"additionalProperties\":false"),
            "the body sends the stripped schema"
        );
    }

    #[test]
    fn gemini_reply_synthesizes_call_ids_in_order() {
        let response: GeminiResponse = serde_json::from_value(json!({
            "candidates": [{"content": {"role": "model", "parts": [
                {"text": "Looking."},
                {"functionCall": {"name": "search", "args": {"query": "a"}}},
                {"functionCall": {"name": "list_date_keys", "args": {}}}
            ]}}]
        }))
        .unwrap();
        assert_eq!(
            gemini_reply(response, "test").unwrap(),
            ModelReply::Calls {
                text: Some("Looking.".into()),
                calls: vec![
                    call("call_0", "search", json!({"query": "a"})),
                    call("call_1", "list_date_keys", json!({})),
                ],
            }
        );
        let text: GeminiResponse = serde_json::from_value(json!({
            "candidates": [{"content": {"role": "model", "parts": [{"text": "done"}]}}]
        }))
        .unwrap();
        assert_eq!(
            gemini_reply(text, "test").unwrap(),
            ModelReply::Text("done".into())
        );
        let blank: GeminiResponse =
            serde_json::from_value(json!({"candidates": [{"content": {"parts": []}}]})).unwrap();
        let err = gemini_reply(blank, "test").unwrap_err().to_string();
        assert!(err.contains("no answer text"), "{err}");
    }

    #[test]
    fn http_400_about_tools_maps_to_tools_unsupported() {
        use reqwest::StatusCode;
        for (status, body) in [
            (
                StatusCode::BAD_REQUEST,
                r#"{"error":"tools is not supported"}"#,
            ),
            (
                StatusCode::NOT_FOUND,
                "No endpoints found that support tool use",
            ),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unknown field TOOL_CHOICE",
            ),
            (StatusCode::BAD_REQUEST, "Function calling is not enabled"),
        ] {
            let err = chat_api_error("test", status, body);
            assert!(
                err.downcast_ref::<ToolsUnsupported>().is_some(),
                "{status} {body} must map: {err}"
            );
        }
        for (status, body) in [
            (StatusCode::BAD_REQUEST, "context length exceeded"),
            (StatusCode::UNAUTHORIZED, "invalid tool key"),
            (StatusCode::INTERNAL_SERVER_ERROR, "tools crashed"),
        ] {
            let err = chat_api_error("test", status, body);
            assert!(
                err.downcast_ref::<ToolsUnsupported>().is_none(),
                "{status} {body} must stay ordinary"
            );
            let msg = err.to_string();
            assert!(msg.contains("test API error"), "{msg}");
            assert!(msg.contains(status.as_str()) && msg.contains(body), "{msg}");
        }
    }

    #[tokio::test]
    async fn chat_over_http_rejecting_tools_is_tools_unsupported() {
        let app = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    r#"{"error":{"message":"This model does not support tools"}}"#,
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let err = client(
            Wire::OpenAiCompat { reasoning: None },
            format!("http://{addr}"),
        )
        .chat("S", &[ChatMessage::User("q".into())], &[])
        .await
        .unwrap_err();
        assert!(err.is::<ToolsUnsupported>(), "{err}");
    }

    #[tokio::test]
    async fn chat_over_http_posts_the_wire_body_and_parses_the_reply() {
        let captured: Captured = Arc::new(Mutex::new(None));
        let base = mock_provider(
            "/v1/messages",
            json!({"content": [
                {"type": "tool_use", "id": "t1", "name": "search", "input": {"query": "q"}}
            ]}),
            captured.clone(),
        )
        .await;
        let reply = client(Wire::Anthropic, base)
            .chat(
                "SYS",
                &[ChatMessage::User("q".into())],
                &[spec("search", search_schema())],
            )
            .await
            .unwrap();
        assert_eq!(
            reply,
            ModelReply::Calls {
                text: None,
                calls: vec![call("t1", "search", json!({"query": "q"}))],
            }
        );
        let (headers, body) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["x-api-key"], "sk-test");
        assert_eq!(body["system"], "SYS");
        assert_eq!(body["tools"][0]["name"], "search");
    }

    #[tokio::test]
    async fn default_chat_is_tools_unsupported() {
        struct AskOnly;
        #[async_trait]
        impl LLMClient for AskOnly {
            async fn ask(
                &self,
                _: &str,
                _: &[(String, String)],
                _: &[(usize, (f64, FlattenedChunk))],
            ) -> anyhow::Result<String> {
                Ok(String::new())
            }
        }
        let err = AskOnly.chat("S", &[], &[]).await.unwrap_err();
        assert!(err.is::<ToolsUnsupported>());
    }

    fn signed(id: &str, name: &str, sig: &str) -> ModelToolCall {
        ModelToolCall {
            extra: Some(json!({"thoughtSignature": sig})),
            ..ModelToolCall::new(id, name, json!({}))
        }
    }

    #[test]
    fn gemini_reply_keeps_each_parts_thought_signature_in_extra() {
        let response: GeminiResponse = serde_json::from_value(json!({
            "candidates": [{"content": {"role": "model", "parts": [
                {"functionCall": {"name": "search", "args": {}}, "thoughtSignature": "SIG_A"},
                {"functionCall": {"name": "list_date_keys", "args": {}}}
            ]}}]
        }))
        .unwrap();
        assert_eq!(
            gemini_reply(response, "test").unwrap(),
            ModelReply::Calls {
                text: None,
                calls: vec![
                    signed("call_0", "search", "SIG_A"),
                    call("call_1", "list_date_keys", json!({})),
                ],
            }
        );
    }

    #[test]
    fn gemini_chat_body_sends_the_signature_only_on_the_part_that_had_one() {
        let msgs = vec![
            ChatMessage::User("q".into()),
            ChatMessage::Assistant {
                text: None,
                calls: vec![
                    signed("call_0", "search", "SIG_A"),
                    call("call_1", "list_date_keys", json!({})),
                ],
            },
        ];
        let body = gemini_chat_body("S", &msgs, &[]).unwrap();
        assert_eq!(
            body["contents"][1]["parts"],
            json!([
                {"functionCall": {"name": "search", "args": {}}, "thoughtSignature": "SIG_A"},
                {"functionCall": {"name": "list_date_keys", "args": {}}}
            ])
        );
    }

    #[test]
    fn openai_and_anthropic_ignore_extra() {
        let msgs = vec![ChatMessage::Assistant {
            text: None,
            calls: vec![signed("c1", "search", "SIG_A")],
        }];
        let openai = openai_chat_body("m", None, "S", &msgs, &[]);
        let anthropic = anthropic_chat_body("m", "S", &msgs, &[]);
        assert!(!openai.to_string().contains("SIG_A"), "{openai}");
        assert!(!anthropic.to_string().contains("SIG_A"), "{anthropic}");
        assert!(!openai.to_string().contains("thoughtSignature"));
        assert!(!anthropic.to_string().contains("thoughtSignature"));
    }
}
