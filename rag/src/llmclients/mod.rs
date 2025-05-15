use crate::document::KimunChunk;

pub mod gemini;
pub mod mistral;

pub trait LLMClient {
    async fn ask<S: AsRef<str>>(
        &self,
        question: S,
        context: Vec<(f64, KimunChunk)>,
    ) -> anyhow::Result<String>;
}
