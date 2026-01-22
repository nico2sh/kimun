use anyhow::bail;
use log::debug;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::document::KimunChunk;

use super::LLMClient;

pub struct ClaudeClient {
    api_key: String,
    model: String,
}

impl ClaudeClient {
    pub fn new(model: String) -> Self {
        // Get API key from environment variable
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .expect("ANTHROPIC_API_KEY environment variable not set");

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
{context_string}---------------------
Given the context information and not prior knowledge, answer the query.
Query: {question}
Answer:"#
        );

        prompt
    }
}

#[async_trait::async_trait]
impl LLMClient for ClaudeClient {
    async fn ask(
        &self,
        question: &str,
        context: Vec<(f64, crate::document::KimunChunk)>,
    ) -> anyhow::Result<String> {
        // Create a new reqwest client
        let client = Client::new();

        // Prepare the request payload
        let request_payload = ClaudeRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            messages: vec![ClaudeMessage {
                role: "user".to_string(),
                content: self.get_prompt(question.to_string(), context),
            }],
        };

        // Make the API call
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request_payload)
            .send()
            .await?;

        // Check if request was successful
        if response.status().is_success() {
            // Parse the response
            let claude_response: ClaudeResponse = response.json().await?;

            let response = claude_response
                .content
                .into_iter()
                .filter_map(|c| {
                    if c.content_type == "text" {
                        Some(c.text)
                    } else {
                        None
                    }
                })
                .collect::<Vec<String>>()
                .join("\n");

            debug!("Input Tokens Used: {}", claude_response.usage.input_tokens);
            debug!(
                "Output Tokens Used: {}",
                claude_response.usage.output_tokens
            );

            Ok(response)
        } else {
            let status = response.status();
            let body = response.text().await?;
            bail!("Claude API error: {}\n{}", status, body)
        }
    }
}

#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ClaudeMessage>,
}

#[derive(Serialize)]
struct ClaudeMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
    usage: ClaudeUsage,
}

#[derive(Deserialize)]
struct ClaudeContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: u32,
    output_tokens: u32,
}
