// tui/src/cli/commands/mcp/prompts.rs
//
// MCP prompt templates — provide vault-enriched context to LLM clients.

use rmcp::{
    ErrorData as McpError,
    handler::server::wrapper::Parameters,
    model::{PromptMessage, PromptMessageRole},
    prompt, prompt_router, schemars,
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
pub struct WeeklyReviewParams {
    /// Any date within the target week in YYYY-MM-DD format; defaults to today
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TriageInboxParams {
    /// Maximum number of inbox notes to include (default 20)
    pub max_notes: Option<u32>,
    /// Maximum number of related notes to include per inbox note (default 3)
    pub max_context: Option<u32>,
}

// ---------------------------------------------------------------------------
// Prompt implementations
// ---------------------------------------------------------------------------

#[prompt_router(vis = "pub")]
impl KimunHandler {
    #[prompt(
        description = "Load today's journal entry and ask the LLM to review the day: summarise accomplishments, identify action items, and note recurring themes."
    )]
    async fn daily_review(
        &self,
        Parameters(p): Parameters<DailyReviewParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        use kimun_core::error::{FSError, VaultError};

        let date_str = match p.date.as_deref() {
            None => chrono::Utc::now().format("%Y-%m-%d").to_string(),
            Some(d) => {
                if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
                    return Err(McpError::invalid_params(
                        format!("Invalid date '{}' — expected YYYY-MM-DD.", d),
                        None,
                    ));
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
            3. Note any open questions or concerns that need follow-up"
        );

        Ok(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            message,
        )])
    }

    #[prompt(
        description = "Load a full week of journal entries and ask the LLM to synthesise themes, accomplishments, and carry-overs."
    )]
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
                    return Err(McpError::invalid_params(
                        format!("Invalid date '{}' — expected YYYY-MM-DD.", d),
                        None,
                    ));
                }
            },
        };

        // Compute Monday and Sunday of the week
        let days_from_monday = anchor.weekday().num_days_from_monday();
        let monday = anchor - Duration::days(days_from_monday as i64);
        let sunday = monday + Duration::days(6);

        // Day names for formatting
        let day_names = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];

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
                Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                    "(no entry)".to_string()
                }
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

        Ok(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            message,
        )])
    }

    #[prompt(
        description = "Review inbox notes and suggest how to organize them: move to journal, promote to a proper note with related context, or keep in inbox for later."
    )]
    async fn triage_inbox(
        &self,
        Parameters(p): Parameters<TriageInboxParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let max_notes = p.max_notes.unwrap_or(20) as usize;
        let max_context = p.max_context.unwrap_or(3) as usize;

        let all_inbox = self
            .vault
            .get_notes(self.vault.inbox_path(), false)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let inbox_notes: Vec<_> = all_inbox.into_iter().take(max_notes).collect();

        if inbox_notes.is_empty() {
            return Ok(vec![PromptMessage::new_text(
                PromptMessageRole::User,
                "The inbox is empty — no notes to triage.".to_string(),
            )]);
        }

        let mut sections = Vec::new();

        for (entry, _content_data) in &inbox_notes {
            let content = match self.vault.get_note_text(&entry.path).await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let search_terms: String = content
                .split_whitespace()
                .take(15)
                .collect::<Vec<_>>()
                .join(" ");

            let mut related_section = String::new();
            if !search_terms.is_empty()
                && let Ok(results) = self.vault.search_notes(&search_terms).await
            {
                let related: Vec<_> = results
                    .iter()
                    .filter(|(e, _)| e.path != entry.path)
                    .take(max_context)
                    .collect();
                if !related.is_empty() {
                    related_section.push_str("\nRelated notes:\n");
                    for (rel_entry, rel_content) in &related {
                        let preview: String = rel_content.title.chars().take(200).collect();
                        related_section
                            .push_str(&format!("- {} — \"{}\"\n", rel_entry.path, preview));
                    }
                }
            }

            let filename = entry.path.get_clean_name();
            sections.push(format!(
                "---\n## {path} (filename: {filename})\n\n{content}\n{related}\n",
                path = entry.path,
                content = content,
                related = related_section,
            ));
        }

        let message = format!(
            "Here are the notes in the inbox ({count} total):\n\n\
            {sections}\
            ---\n\n\
            For each inbox note, suggest what to do:\n\
            1. **Journal** — append the content to the journal entry for the date in the filename \
            (use `append_note` on the journal path `/journal/YYYY-MM-DD`, then delete the inbox note with `move_note` or inform the user)\n\
            2. **Promote** — create a proper note with a descriptive name in an appropriate vault directory \
            (use `create_note` with the enriched content, linking to related notes if helpful, then delete the inbox note)\n\
            3. **Keep** — leave it in the inbox if it needs more thought\n\n\
            Process one note at a time. Use the available tools to execute your suggestions.",
            count = inbox_notes.len(),
            sections = sections.join(""),
        );

        Ok(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            message,
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use kimun_core::{NoteVault, VaultConfig};
    use tempfile::TempDir;

    async fn make_handler() -> (KimunHandler, TempDir) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(crate::test_support::sys(dir.path())))
            .await
            .unwrap();
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
    async fn test_daily_review_invalid_date_returns_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .daily_review(Parameters(DailyReviewParams {
                date: Some("not-a-date".to_string()),
            }))
            .await;
        assert!(result.is_err(), "expected Err for invalid date");
        let err = result.unwrap_err();
        assert!(
            err.message.contains("Invalid date"),
            "expected error message to mention invalid date: {:?}",
            err
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
        assert!(
            text.contains("unique_weekly_mon_xyz"),
            "monday entry: {}",
            text
        );
        assert!(
            text.contains("unique_weekly_wed_xyz"),
            "wednesday entry: {}",
            text
        );
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
    async fn test_weekly_review_invalid_date_returns_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .weekly_review(Parameters(WeeklyReviewParams {
                date: Some("not-a-date".to_string()),
            }))
            .await;
        assert!(result.is_err(), "expected Err for invalid date");
        let err = result.unwrap_err();
        assert!(
            err.message.contains("Invalid date"),
            "expected error message to mention invalid date: {:?}",
            err
        );
    }
}
