// tui/src/cli/commands/mcp/prompts.rs
//
// MCP prompt templates — provide vault-enriched context to LLM clients.

use rmcp::{
    ErrorData as McpError,
    handler::server::{router::prompt::PromptRouter, wrapper::Parameters},
    model::{PromptMessage, PromptMessageRole},
    schemars,
    prompt, prompt_router,
};
use serde::Deserialize;

use super::KimunHandler;

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DailyReviewParams {
    /// Date in YYYY-MM-DD format; defaults to today
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindConnectionsParams {
    /// Vault-relative path to the note, e.g. "projects/my-note"
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ResearchNoteParams {
    /// Vault-relative path to the note
    pub path: String,
    /// Maximum number of related notes to include (default 5)
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrainstormParams {
    /// Topic to brainstorm ideas about
    pub topic: String,
}

// ---------------------------------------------------------------------------
// Prompt implementations
// ---------------------------------------------------------------------------

#[prompt_router(vis = "pub")]
impl KimunHandler {
    #[prompt(description = "Load today's journal entry and ask the LLM to review the day: summarise accomplishments, identify action items, and note recurring themes.")]
    async fn daily_review(
        &self,
        Parameters(p): Parameters<DailyReviewParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[prompt(description = "Load a note and its backlink list, then ask the LLM to identify non-obvious conceptual connections to the rest of the vault.")]
    async fn find_connections(
        &self,
        Parameters(p): Parameters<FindConnectionsParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[prompt(description = "Search the vault using a note's section headings as queries, then ask the LLM to synthesise what is captured and identify gaps.")]
    async fn research_note(
        &self,
        Parameters(p): Parameters<ResearchNoteParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[prompt(description = "Search the vault for a topic and ask the LLM to generate new ideas that build on existing notes, with a suggested note to append them to.")]
    async fn brainstorm(
        &self,
        Parameters(p): Parameters<BrainstormParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }
}
