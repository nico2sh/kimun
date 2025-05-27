use anyhow::bail;
use log::debug;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::document::KimunChunk;

use super::LLMClient;

pub enum GeminiModel {
    Gemini20Flash,
    Gemini20FlashLite,
    Gemini25FlashPreview0417,
    Gemini25ProPreview0325,
    Gemini25ProExp0325,
}

impl std::fmt::Display for GeminiModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            GeminiModel::Gemini20Flash => "gemini-2.0-flash",
            GeminiModel::Gemini20FlashLite => "gemini-2.0-flash-lite",
            GeminiModel::Gemini25FlashPreview0417 => "gemini-2.5-flash-preview-04-17",
            GeminiModel::Gemini25ProPreview0325 => "gemini-2.5-pro-preview-03-25",
            GeminiModel::Gemini25ProExp0325 => "gemini-2.5-pro-exp-03-25",
        };
        write!(f, "{}", s)
    }
}

pub struct GeminiClient {
    api_key: String,
    model: GeminiModel,
}

impl GeminiClient {
    pub fn new(model: GeminiModel) -> Self {
        // Get API key from environment variable
        let api_key =
            std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY environment variable not set");

        Self { api_key, model }
    }

    fn get_prompt(&self, question: String, context: Vec<(f64, KimunChunk)>) -> String {
        let mut context_string = String::new();
        for (distance, chunk) in context {
            context_string.push_str(&format!(
                "--- Document: {} (Relevance: {:.4}) ---\n",
                chunk.metadata.source_path, distance
            ));
            let mut title = chunk.metadata.title.clone();
            if let Some(date) = chunk.metadata.get_date_string() {
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
            context_string.push_str(&chunk.content);
            context_string.push_str("\n\n");
        }

        let prompt = format!(
            r#"
Context information is below.
---------------------
{context_string}
---------------------
Given the context information and not prior knowledge, answer the query.
Query: {question}
Answer:
"#
        );

        prompt
    }
}

impl LLMClient for GeminiClient {
    async fn ask<S: AsRef<str>>(
        &self,
        question: S,
        context: Vec<(f64, crate::document::KimunChunk)>,
    ) -> anyhow::Result<String> {
        // Create a new reqwest client
        let client = Client::new();

        // Prepare the request payload
        let request_payload = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![GeminiPart {
                    text: self.get_prompt(question.as_ref().to_string(), context),
                }],
            }],
        };

        // Make the API call
        let response = client
            .post(format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                self.model, self.api_key
            ))
            .header("Content-Type", "application/json")
            .json(&request_payload)
            .send()
            .await?;

        // Check if request was successful
        if response.status().is_success() {
            // Parse the response
            let gemini_response: GeminiResponse = response.json().await?;

            let response = gemini_response
                .candidates
                .into_iter()
                .flat_map(|c| c.content.parts)
                .map(|p| p.text)
                .collect::<Vec<String>>()
                .join("\n");

            debug!(
                "Prompt Tokens Used: {}",
                gemini_response.usage_metadata.prompt_token_count
            );
            debug!(
                "Completion Tokens Used: {}",
                gemini_response.usage_metadata.candidates_token_count
            );
            debug!(
                "Total Tokens: {}",
                gemini_response.usage_metadata.total_token_count
            );

            Ok(response)
        } else {
            let status = response.status();
            let body = (response.text().await?).to_string();
            bail!("Error: {}\nResponse body: {}", status, body)
        }
    }
}

#[derive(Serialize, Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidates>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: GeminiUsageMetadata,
    #[serde(rename = "modelVersion")]
    model_version: String,
}

#[derive(Serialize, Deserialize)]
struct GeminiCandidates {
    content: GeminiCandidatesContent,
    #[serde(rename = "finishReason")]
    finish_reason: String,
    index: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct GeminiCandidatesContent {
    parts: Vec<GeminiPart>,
    role: String,
}

#[derive(Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: u32,
    #[serde(rename = "totalTokenCount")]
    total_token_count: u32,
    #[serde(rename = "promptTokensDetails")]
    prompt_tokens_details: Vec<GeminiTokenDetails>,
    #[serde(rename = "candidatesTokensDetails")]
    candidates_tokens_details: Option<Vec<GeminiTokenDetails>>,
    #[serde(rename = "thoughtsTokenCount")]
    thoughts_token_count: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct GeminiTokenDetails {
    modality: String,
    #[serde(rename = "tokenCount")]
    token_count: u32,
}
