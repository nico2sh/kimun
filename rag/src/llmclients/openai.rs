use anyhow::bail;
use log::debug;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::document::KimunChunk;

use super::LLMClient;

pub struct OpenAIClient {
    api_key: String,
    model: String,
}

impl OpenAIClient {
    pub fn new(model: impl Into<String>) -> Self {
        // Get API key from environment variable
        let api_key =
            std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY environment variable not set");

        Self {
            api_key,
            model: model.into(),
        }
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
impl LLMClient for OpenAIClient {
    async fn ask(
        &self,
        question: &str,
        context: Vec<(f64, crate::document::KimunChunk)>,
    ) -> anyhow::Result<String> {
        // Create a new reqwest client
        let client = Client::new();

        // Prepare the request payload
        let request_payload = OpenAIRequest {
            model: self.model.clone(),
            messages: vec![OpenAIMessage {
                role: "user".to_string(),
                content: self.get_prompt(question.to_string(), context),
            }],
        };

        // Make the API call
        let response = client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_payload)
            .send()
            .await?;

        // Check if request was successful
        if response.status().is_success() {
            // Parse the response
            let openai_response: OpenAIResponse = response.json().await?;

            let response = openai_response
                .choices
                .into_iter()
                .map(|c| c.message.content)
                .collect::<Vec<String>>()
                .join("\n");

            debug!(
                "Prompt Tokens Used: {}",
                openai_response.usage.prompt_tokens
            );
            debug!(
                "Completion Tokens Used: {}",
                openai_response.usage.completion_tokens
            );
            debug!("Total Tokens: {}", openai_response.usage.total_tokens);

            Ok(response)
        } else {
            let status = response.status();
            let body = response.text().await?;
            bail!("OpenAI API error: {}\n{}", status, body)
        }
    }
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
}

#[derive(Serialize, Deserialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}
