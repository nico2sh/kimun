use crate::document::KimunChunk;
use async_trait::async_trait;

pub mod claude;
pub mod gemini;
pub mod mistral;
pub mod openai;

#[async_trait]
pub trait LLMClient: Send + Sync {
    async fn ask(&self, question: &str, context: &Vec<(f64, KimunChunk)>)
    -> anyhow::Result<String>;
}
