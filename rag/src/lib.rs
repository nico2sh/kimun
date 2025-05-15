use std::path::Path;

use document::ChunkLoader;

use dbembeddings::vecsqlite::VecSQLite;
use dbembeddings::{Embeddings, veclance::VecLance};
use kimun_core::NoteVault;
use llmclients::{LLMClient, gemini::GeminiClient};

mod dbembeddings;
mod document;
mod llmclients;

pub struct KimunRag<E, C>
where
    E: Embeddings,
    C: LLMClient,
{
    embeddings: Box<E>,
    llm_client: Box<C>,
}

impl KimunRag<VecSQLite, GeminiClient> {
    pub fn sqlite<P: AsRef<Path>>(path: P) -> Self {
        Self::new(
            VecSQLite::new(path),
            GeminiClient::new(llmclients::gemini::GeminiModel::Gemini25ProExp0325),
        )
    }
}

impl KimunRag<VecLance, GeminiClient> {
    pub fn lance<P: AsRef<Path>>(path: P) -> Self {
        Self::new(
            VecLance::new(path),
            GeminiClient::new(llmclients::gemini::GeminiModel::Gemini25ProPreview0325),
        )
    }
}

impl<E, C> KimunRag<E, C>
where
    E: Embeddings,
    C: LLMClient,
{
    pub fn new(embeddings: E, llm_client: C) -> Self {
        Self {
            embeddings: Box::new(embeddings),
            llm_client: Box::new(llm_client),
        }
    }

    pub fn init(&mut self) -> anyhow::Result<()> {
        self.embeddings.init()
    }

    pub async fn store_embeddings(&self, vault: NoteVault) -> anyhow::Result<()> {
        let chunk_loader = ChunkLoader::new(vault);
        let chunks = chunk_loader.load_notes()?;

        self.embeddings.store_embeddings(&chunks).await?;
        Ok(())
    }

    pub async fn query(&self, query: String) -> anyhow::Result<String> {
        let context = self.embeddings.query_embedding(&query).await?;

        let answer = self.llm_client.ask(query, context).await?;
        Ok(answer)
    }
}
