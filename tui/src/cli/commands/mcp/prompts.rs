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
        use kimun_core::error::{FSError, VaultError};

        let date_str = match p.date.as_deref() {
            None => chrono::Utc::now().format("%Y-%m-%d").to_string(),
            Some(d) => {
                if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
                    return Ok(vec![PromptMessage::new_text(
                        PromptMessageRole::User,
                        format!("Invalid date '{}' — expected YYYY-MM-DD.", d),
                    )]);
                }
                d.to_string()
            }
        };

        let journal_path = self
            .vault
            .journal_path()
            .append(&kimun_core::nfs::VaultPath::note_path_from(&date_str))
            .absolute();

        let journal_text = match self.vault.get_note_text(&journal_path).await {
            Ok(t) => t,
            Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                return Ok(vec![PromptMessage::new_text(
                    PromptMessageRole::User,
                    format!("No journal entry found for {}.", date_str),
                )]);
            }
            Err(e) => return Err(McpError::internal_error(e.to_string(), None)),
        };

        let message = format!(
            "Here is my journal entry for {date_str}:\n\n---\n{journal_text}\n---\n\n\
            Please review this journal entry:\n\
            1. Summarize what was accomplished\n\
            2. Identify any action items or follow-ups\n\
            3. Note any recurring themes worth tracking"
        );

        Ok(vec![PromptMessage::new_text(PromptMessageRole::User, message)])
    }

    #[prompt(description = "Load a note and its backlink list, then ask the LLM to identify non-obvious conceptual connections to the rest of the vault.")]
    async fn find_connections(
        &self,
        Parameters(p): Parameters<FindConnectionsParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        use kimun_core::error::{FSError, VaultError};
        use kimun_core::nfs::VaultPath;

        let vault_path = VaultPath::note_path_from(&p.path);

        let note_text = match self.vault.get_note_text(&vault_path).await {
            Ok(t) => t,
            Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                return Ok(vec![PromptMessage::new_text(
                    PromptMessageRole::User,
                    format!("Note not found: {}", vault_path),
                )]);
            }
            Err(e) => return Err(McpError::internal_error(e.to_string(), None)),
        };

        let backlinks = self
            .vault
            .get_backlinks(&vault_path)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let backlinks_section = if backlinks.is_empty() {
            String::new()
        } else {
            let paths: Vec<String> = backlinks
                .iter()
                .map(|(entry, _)| format!("- {}", entry.path))
                .collect();
            format!(
                "\nNotes that link to this note:\n{}\n",
                paths.join("\n")
            )
        };

        let message = format!(
            "Here is the note at \"{path}\":\n\n---\n{note_text}\n---\n{backlinks_section}\n\
            Identify non-obvious conceptual connections between this note and the rest of the vault. \
            What themes link them? What ideas are worth exploring further?\n\
            (You can call the show_note tool to read any linked note in full.)",
            path = vault_path,
        );

        Ok(vec![PromptMessage::new_text(PromptMessageRole::User, message)])
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::*;
    use tempfile::TempDir;
    use kimun_core::NoteVault;

    async fn make_handler() -> (KimunHandler, TempDir) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(dir.path()).await.unwrap();
        vault.validate_and_init().await.unwrap();
        let handler = KimunHandler::new(vault);
        (handler, dir)
    }

    /// Extract the text from the first PromptMessage's content.
    fn first_text(msgs: &[PromptMessage]) -> String {
        match msgs.first().map(|m| &m.content) {
            Some(PromptMessageContent::Text { text }) => text.clone(),
            _ => String::new(),
        }
    }

    #[tokio::test]
    async fn test_daily_review_no_entry_returns_graceful_message() {
        let (handler, _dir) = make_handler().await;
        let msgs = handler
            .daily_review(Parameters(DailyReviewParams { date: None }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("No journal entry"),
            "expected graceful message, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_daily_review_with_entry_includes_content() {
        let (handler, _dir) = make_handler().await;
        // Create today's entry via the journal tool
        handler
            .journal(Parameters(JournalParams {
                text: "worked on unique_daily_review_content_xyz".to_string(),
                date: None,
            }))
            .await
            .unwrap();
        let msgs = handler
            .daily_review(Parameters(DailyReviewParams { date: None }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("unique_daily_review_content_xyz"),
            "expected journal content in prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_daily_review_specific_date() {
        let (handler, _dir) = make_handler().await;
        handler
            .journal(Parameters(JournalParams {
                text: "specific date entry content".to_string(),
                date: Some("2026-01-15".to_string()),
            }))
            .await
            .unwrap();
        let msgs = handler
            .daily_review(Parameters(DailyReviewParams {
                date: Some("2026-01-15".to_string()),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("specific date entry content"),
            "expected entry in prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_find_connections_includes_note_content() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "my/note".to_string(),
                content: "# My Note\n\nunique_connections_content_abc".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .find_connections(Parameters(FindConnectionsParams {
                path: "my/note".to_string(),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("unique_connections_content_abc"),
            "expected note content in prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_find_connections_lists_backlinks() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "target".to_string(),
                content: "# Target".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "source".to_string(),
                content: "see [[target]] for details".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .find_connections(Parameters(FindConnectionsParams {
                path: "target".to_string(),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        assert!(
            text.contains("source"),
            "expected backlink 'source' in prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_find_connections_no_backlinks_omits_section() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "lone/note".to_string(),
                content: "# Lone\n\nno links to here".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .find_connections(Parameters(FindConnectionsParams {
                path: "lone/note".to_string(),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        // Note content should be present; backlinks section should be absent
        assert!(text.contains("Lone"), "expected note content: {}", text);
        assert!(
            !text.contains("Notes that link"),
            "should not have backlinks section: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_find_connections_note_not_found() {
        let (handler, _dir) = make_handler().await;
        let msgs = handler
            .find_connections(Parameters(FindConnectionsParams {
                path: "missing/note".to_string(),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(text.contains("not found"), "expected not-found message: {}", text);
    }
}
