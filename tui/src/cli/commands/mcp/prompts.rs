// tui/src/cli/commands/mcp/prompts.rs
//
// MCP prompt templates — provide vault-enriched context to LLM clients.

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::Parameters,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WeeklyReviewParams {
    /// Any date within the target week in YYYY-MM-DD format; defaults to today
    pub date: Option<String>,
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
        use kimun_core::error::{FSError, VaultError};
        use kimun_core::nfs::VaultPath;
        use std::collections::HashSet;

        let vault_path = VaultPath::note_path_from(&p.path);
        let max = p.max_results.unwrap_or(5) as usize;

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

        // Extract unique leaf headings from the note's chunks
        let chunks_map = self
            .vault
            .get_note_chunks(&vault_path)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut topics: Vec<String> = Vec::new();
        for chunks in chunks_map.values() {
            for chunk in chunks {
                if let Some(leaf) = chunk.breadcrumb.last() {
                    let topic = leaf.trim().to_string();
                    if !topic.is_empty() && !topics.contains(&topic) {
                        topics.push(topic);
                    }
                }
            }
        }

        // Search each topic; deduplicate results; cap at max
        let source_path_str = vault_path.to_string();
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(source_path_str.clone());

        let mut related_sections: Vec<String> = Vec::new();

        'outer: for topic in &topics {
            let results = self
                .vault
                .search_notes(topic)
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            for (entry, _) in results {
                let path_str = entry.path.to_string();
                if seen.contains(&path_str) {
                    continue;
                }
                seen.insert(path_str.clone());
                let text = self
                    .vault
                    .get_note_text(&entry.path)
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                related_sections.push(format!("=== {} ===\n{}", entry.path, text));
                if related_sections.len() >= max {
                    break 'outer;
                }
            }
        }

        let topics_list = if topics.is_empty() {
            "(no sections found)".to_string()
        } else {
            topics.join(", ")
        };

        let related_block = if related_sections.is_empty() {
            "No related notes found in the vault.".to_string()
        } else {
            related_sections.join("\n\n")
        };

        let message = format!(
            "Here is the note at \"{path}\":\n\n---\n{note_text}\n---\n\n\
            Related notes found by searching section topics ({topics_list}):\n\n\
            {related_block}\n\n\
            Synthesize what the vault knows about this topic. \
            What key ideas are captured? What gaps exist? What questions remain unanswered?",
            path = vault_path,
        );

        Ok(vec![PromptMessage::new_text(PromptMessageRole::User, message)])
    }

    #[prompt(description = "Search the vault for a topic and ask the LLM to generate new ideas that build on existing notes, with a suggested note to append them to.")]
    async fn brainstorm(
        &self,
        Parameters(p): Parameters<BrainstormParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let results = self
            .vault
            .search_notes(&p.topic)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let top: Vec<_> = results.into_iter().take(5).collect();
        let suggested_path = top.first().map(|(entry, _)| entry.path.to_string());

        let mut vault_sections: Vec<String> = Vec::new();
        for (entry, _) in &top {
            let text = self
                .vault
                .get_note_text(&entry.path)
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            vault_sections.push(format!("=== {} ===\n{}", entry.path, text));
        }

        let vault_block = if vault_sections.is_empty() {
            String::new()
        } else {
            format!(
                "Here is relevant content from my vault:\n\n{}\n\n",
                vault_sections.join("\n\n")
            )
        };

        let suggestion_line = match &suggested_path {
            Some(path) => format!("3. Suggested note to append new ideas to: {}\n", path),
            None => String::new(),
        };

        let message = format!(
            "I want to brainstorm ideas about: \"{topic}\"\n\n\
            {vault_block}\
            Based on my existing notes:\n\
            1. Generate 5–10 new ideas related to \"{topic}\" that build on what's already captured\n\
            2. Avoid repeating existing content\n\
            {suggestion_line}",
            topic = p.topic,
        );

        Ok(vec![PromptMessage::new_text(PromptMessageRole::User, message)])
    }

    #[prompt(description = "Load a full week of journal entries and ask the LLM to synthesise themes, accomplishments, and carry-overs.")]
    async fn weekly_review(
        &self,
        Parameters(p): Parameters<WeeklyReviewParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        use chrono::{Datelike, Duration, NaiveDate, Utc};
        use kimun_core::error::{FSError, VaultError};
        use kimun_core::nfs::VaultPath;

        // Parse or default to today
        let anchor: NaiveDate = match p.date.as_deref() {
            None => Utc::now().date_naive(),
            Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                Ok(date) => date,
                Err(_) => {
                    return Ok(vec![PromptMessage::new_text(
                        PromptMessageRole::User,
                        format!("Invalid date '{}' — expected YYYY-MM-DD.", d),
                    )]);
                }
            },
        };

        // Compute Monday and Sunday of the week
        let days_from_monday = anchor.weekday().num_days_from_monday();
        let monday = anchor - Duration::days(days_from_monday as i64);
        let sunday = monday + Duration::days(6);

        // Day names for formatting
        let day_names = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

        let mut days_text = String::new();
        for i in 0..7 {
            let day = monday + Duration::days(i);
            let date_str = day.format("%Y-%m-%d").to_string();
            let journal_path = self
                .vault
                .journal_path()
                .append(&VaultPath::note_path_from(&date_str))
                .absolute();

            let content = match self.vault.get_note_text(&journal_path).await {
                Ok(text) => text,
                Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => "(no entry)".to_string(),
                Err(e) => return Err(McpError::internal_error(e.to_string(), None)),
            };

            days_text.push_str(&format!(
                "{} {}:\n---\n{}\n---\n\n",
                day_names[i as usize], date_str, content
            ));
        }

        let message = format!(
            "Week of {} – {}\n\n{}\
            Please review this week:\n\
            1. What were the main themes and accomplishments?\n\
            2. What carried over unfinished from day to day?\n\
            3. What patterns are worth paying attention to?\n\
            4. What should be prioritised next week?",
            monday.format("%Y-%m-%d"),
            sunday.format("%Y-%m-%d"),
            days_text
        );

        Ok(vec![PromptMessage::new_text(PromptMessageRole::User, message)])
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

    #[tokio::test]
    async fn test_research_note_includes_source_note() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "research/topic".to_string(),
                content: "# Topic\n\n## Background\n\nunique_research_source_xyz\n\n## Open Questions\n\nwhat next?".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .research_note(Parameters(ResearchNoteParams {
                path: "research/topic".to_string(),
                max_results: Some(3),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("unique_research_source_xyz"),
            "expected source note content: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_research_note_includes_related_notes() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "research/main".to_string(),
                content: "# Main\n\n## Rust Programming\n\nabout rust".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "research/related".to_string(),
                content: "# Related\n\nRust Programming is great".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .research_note(Parameters(ResearchNoteParams {
                path: "research/main".to_string(),
                max_results: Some(5),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        assert!(
            text.contains("research/related"),
            "expected related note in prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_research_note_not_found() {
        let (handler, _dir) = make_handler().await;
        let msgs = handler
            .research_note(Parameters(ResearchNoteParams {
                path: "missing/note".to_string(),
                max_results: None,
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(text.contains("not found"), "expected not-found message: {}", text);
    }

    #[tokio::test]
    async fn test_brainstorm_includes_vault_content() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "ideas/rust".to_string(),
                content: "# Rust Ideas\n\nunique_brainstorm_rust_content_xyz".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .brainstorm(Parameters(BrainstormParams {
                topic: "unique_brainstorm_rust_content_xyz".to_string(),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("unique_brainstorm_rust_content_xyz"),
            "expected vault content in prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_brainstorm_suggests_note_to_append() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "ideas/brainstorm_target".to_string(),
                content: "# Brainstorm Target\n\nunique_suggest_xyz_content".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .brainstorm(Parameters(BrainstormParams {
                topic: "unique_suggest_xyz_content".to_string(),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        assert!(
            text.contains("ideas/brainstorm_target"),
            "expected suggested note path: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_brainstorm_no_vault_content_still_returns_prompt() {
        let (handler, _dir) = make_handler().await;
        let msgs = handler
            .brainstorm(Parameters(BrainstormParams {
                topic: "completely_nonexistent_topic_zzz_999".to_string(),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("completely_nonexistent_topic_zzz_999"),
            "expected topic in prompt: {}",
            text
        );
        // No suggestion line when no results
        assert!(
            !text.contains("Suggested note"),
            "should not suggest a note when no results: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_weekly_review_includes_entries_and_marks_missing() {
        let (handler, _dir) = make_handler().await;
        // Create entries for Monday and Wednesday of a known week (2026-03-02 is a Monday)
        handler
            .journal(Parameters(JournalParams {
                text: "monday content unique_weekly_mon_xyz".to_string(),
                date: Some("2026-03-02".to_string()),
            }))
            .await
            .unwrap();
        handler
            .journal(Parameters(JournalParams {
                text: "wednesday content unique_weekly_wed_xyz".to_string(),
                date: Some("2026-03-04".to_string()),
            }))
            .await
            .unwrap();
        let msgs = handler
            .weekly_review(Parameters(WeeklyReviewParams {
                date: Some("2026-03-02".to_string()),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(text.contains("unique_weekly_mon_xyz"), "monday entry: {}", text);
        assert!(text.contains("unique_weekly_wed_xyz"), "wednesday entry: {}", text);
        // Days without entries should show (no entry)
        assert!(text.contains("(no entry)"), "missing days: {}", text);
    }

    #[tokio::test]
    async fn test_weekly_review_date_in_middle_of_week_uses_correct_range() {
        let (handler, _dir) = make_handler().await;
        // 2026-03-04 is a Wednesday — should resolve to Mon 2026-03-02 – Sun 2026-03-08
        let msgs = handler
            .weekly_review(Parameters(WeeklyReviewParams {
                date: Some("2026-03-04".to_string()),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        assert!(
            text.contains("2026-03-02") && text.contains("2026-03-08"),
            "expected Mon 2026-03-02 – Sun 2026-03-08 in: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_weekly_review_invalid_date_returns_graceful_message() {
        let (handler, _dir) = make_handler().await;
        let msgs = handler
            .weekly_review(Parameters(WeeklyReviewParams {
                date: Some("not-a-date".to_string()),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("Invalid date"),
            "expected graceful error message: {}",
            text
        );
    }
}
