use anyhow::bail;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::document::KimunChunk;

use super::LLMClient;

pub struct MistralClient {
    api_key: String,
}

impl MistralClient {
    pub fn new() -> Self {
        // Get API key from environment variable
        let api_key =
            std::env::var("MISTRAL_API_KEY").expect("MISTRAL_API_KEY environment variable not set");
        Self { api_key }
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

impl LLMClient for MistralClient {
    async fn ask<S: AsRef<str>>(
        &self,
        question: S,
        context: Vec<(f64, crate::document::KimunChunk)>,
    ) -> anyhow::Result<String> {
        // Create a new reqwest client
        let client = Client::new();

        // Prepare the request payload
        let request_payload = MistralRequest {
            model: "mistral-large-latest".to_string(), // Replace with the model you want to use

            messages: vec![Message {
                role: "user".to_string(),
                content: self.get_prompt(question.as_ref().to_string(), context),
            }],
        };

        // Make the API call
        let response = client
            .post("https://api.mistral.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_payload)
            .send()
            .await?;

        // Check if request was successful
        if response.status().is_success() {
            // Parse the response
            let mistral_response: MistralResponse = response.json().await?;

            let response = mistral_response
                .choices
                .iter()
                .map(|c| c.message.content.to_owned())
                .collect::<Vec<String>>()
                .join("\n");
            Ok(response)
        } else {
            let status = response.status();
            let body = (response.text().await?).to_string();
            bail!("Error: {}\nResponse body: {}", status, body);
        }
    }
}

#[derive(Serialize)]
struct MistralRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize, Debug)]
struct MistralResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Deserialize, Debug)]
struct Choice {
    index: u32,
    message: Message,
    finish_reason: String,
}

#[derive(Deserialize, Debug)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}
