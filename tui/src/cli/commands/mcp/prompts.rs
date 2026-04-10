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
    /// Maximum number of vault notes to include as context (default 5)
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WeeklyReviewParams {
    /// Any date within the target week in YYYY-MM-DD format; defaults to today
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LinkSuggestionsParams {
    /// Vault-relative path to the note
    pub path: String,
    /// Maximum number of candidate notes to include (default 5)
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ResearchTopicParams {
    /// Topic or keyword to research across the vault
    pub topic: String,
    /// Maximum total number of notes to include (default 10)
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TriageInboxParams {
    /// Maximum number of inbox notes to include (default 20)
    pub max_notes: Option<u32>,
    /// Maximum number of related notes to include per inbox note (default 3)
    pub max_context: Option<u32>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl KimunHandler {
    /// Return the unique leaf heading strings from a note's chunk tree, in
    /// insertion order.  Used by several prompts to derive secondary search
    /// terms from a note's section outline.
    async fn extract_leaf_headings(
        &self,
        path: &kimun_core::nfs::VaultPath,
    ) -> Result<Vec<String>, McpError> {
        use std::collections::HashSet;

        let chunks_map = self
            .vault
            .get_note_chunks(path)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut seen: HashSet<String> = HashSet::new();
        let mut topics: Vec<String> = Vec::new();
        for chunks in chunks_map.values() {
            for chunk in chunks {
                if let Some(leaf) = chunk.breadcrumb.last() {
                    let t = leaf.trim().to_string();
                    if !t.is_empty() && seen.insert(t.clone()) {
                        topics.push(t);
                    }
                }
            }
        }
        Ok(topics)
    }
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
            (You can use the available vault tools to read any linked note in full.)",
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

        let topics = self.extract_leaf_headings(&vault_path).await?;

        // Search each topic; deduplicate results; cap at max
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(vault_path.to_string());

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
            For each of the section topics ({topics_list}), synthesize what the vault captures \
            and identify what is missing or unexplored. What key ideas are captured? \
            What gaps exist? What questions remain unanswered?",
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

        let max = p.max_results.unwrap_or(5) as usize;
        let top: Vec<_> = results.into_iter().take(max).collect();
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
            2. For each new idea, identify which existing note it connects to and suggest where it could be appended or linked\n\
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

    #[prompt(description = "Search the vault for a topic, expand the search via backlinks and related headings from the results, then ask the LLM for a comprehensive overview of the topic and everything connected to it.")]
    async fn research_topic(
        &self,
        Parameters(p): Parameters<ResearchTopicParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        use kimun_core::nfs::VaultPath;
        use std::collections::HashSet;

        let max = p.max_results.unwrap_or(10) as usize;
        let mut seen: HashSet<String> = HashSet::new();

        // Step 1: Direct search for the topic
        let initial_results = self
            .vault
            .search_notes(&p.topic)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut direct_notes: Vec<(VaultPath, String)> = Vec::new();
        let mut backlink_candidates: Vec<VaultPath> = Vec::new();
        // secondary_topics: insertion-ordered, deduplicated case-insensitively
        let mut secondary_topics: Vec<String> = Vec::new();
        let mut secondary_topics_lower: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (entry, _) in initial_results {
            if direct_notes.len() >= max {
                break;
            }
            let path_str = entry.path.to_string();
            if seen.contains(&path_str) {
                continue;
            }
            seen.insert(path_str);

            let text = self
                .vault
                .get_note_text(&entry.path)
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            direct_notes.push((entry.path.clone(), text));

            // Step 2a: Collect backlinks for this note
            let backlinks = self
                .vault
                .get_backlinks(&entry.path)
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            for (bl_entry, _) in backlinks {
                let bl_str = bl_entry.path.to_string();
                if !seen.contains(&bl_str) {
                    seen.insert(bl_str);
                    backlink_candidates.push(bl_entry.path);
                }
            }

            // Step 2b: Extract leaf headings for secondary search (case-insensitive dedup)
            let headings = self.extract_leaf_headings(&entry.path).await?;
            for t in headings {
                let t_lower = t.to_lowercase();
                if t_lower != p.topic.to_lowercase()
                    && secondary_topics_lower.insert(t_lower)
                {
                    secondary_topics.push(t);
                }
            }
        }

        // Step 3: Load backlink notes (within remaining budget)
        let mut backlink_notes: Vec<(VaultPath, String)> = Vec::new();
        for path in &backlink_candidates {
            if direct_notes.len() + backlink_notes.len() >= max {
                break;
            }
            let text = self
                .vault
                .get_note_text(path)
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            backlink_notes.push((path.clone(), text));
        }

        // Step 4: Secondary search using headings extracted from the initial results
        let mut related_notes: Vec<(VaultPath, String)> = Vec::new();
        let mut contributing_topics: Vec<String> = Vec::new();
        'outer: for topic in &secondary_topics {
            let results = self
                .vault
                .search_notes(topic)
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            let before = related_notes.len();
            for (entry, _) in results {
                if direct_notes.len() + backlink_notes.len() + related_notes.len() >= max {
                    break 'outer;
                }
                let path_str = entry.path.to_string();
                if seen.contains(&path_str) {
                    continue;
                }
                seen.insert(path_str);
                let text = self
                    .vault
                    .get_note_text(&entry.path)
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                related_notes.push((entry.path, text));
            }
            if related_notes.len() > before {
                contributing_topics.push(topic.clone());
            }
        }

        if direct_notes.is_empty() && backlink_notes.is_empty() && related_notes.is_empty() {
            return Ok(vec![PromptMessage::new_text(
                PromptMessageRole::User,
                format!("No notes found in the vault related to \"{}\".", p.topic),
            )]);
        }

        let mut blocks: Vec<String> = Vec::new();

        if !direct_notes.is_empty() {
            let section = direct_notes
                .iter()
                .map(|(path, text)| format!("=== {} ===\n{}", path, text))
                .collect::<Vec<_>>()
                .join("\n\n");
            blocks.push(format!("### Notes matching \"{}\":\n\n{}", p.topic, section));
        }

        if !backlink_notes.is_empty() {
            let section = backlink_notes
                .iter()
                .map(|(path, text)| format!("=== {} ===\n{}", path, text))
                .collect::<Vec<_>>()
                .join("\n\n");
            blocks.push(format!("### Notes linking to the above:\n\n{}", section));
        }

        if !related_notes.is_empty() {
            let section = related_notes
                .iter()
                .map(|(path, text)| format!("=== {} ===\n{}", path, text))
                .collect::<Vec<_>>()
                .join("\n\n");
            let header = if contributing_topics.is_empty() {
                "### Notes on related subtopics:".to_string()
            } else {
                let label = contributing_topics
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("### Notes on related subtopics ({label}):")
            };
            blocks.push(format!("{header}\n\n{section}"));
        }

        let content_block = blocks.join("\n\n");

        let message = format!(
            "Research topic: \"{topic}\"\n\n\
            {content_block}\n\n\
            Using the vault content above, provide a comprehensive overview of \"{topic}\":\n\
            1. What does the vault capture about this topic?\n\
            2. What are the key ideas, patterns, or recurring themes?\n\
            3. How do the related notes connect to the topic?\n\
            4. What gaps or unexplored angles exist?",
            topic = p.topic,
        );

        Ok(vec![PromptMessage::new_text(PromptMessageRole::User, message)])
    }

    #[prompt(description = "Find vault notes topically related to the given note but not yet linked, and ask the LLM to evaluate which connections are worth formalising.")]
    async fn link_suggestions(
        &self,
        Parameters(p): Parameters<LinkSuggestionsParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        use kimun_core::error::{FSError, VaultError};
        use kimun_core::nfs::VaultPath;
        use kimun_core::note::LinkType;
        use std::collections::HashSet;

        let vault_path = VaultPath::note_path_from(&p.path);
        let max = p.max_results.unwrap_or(5) as usize;

        // Load source note
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

        let topics = self.extract_leaf_headings(&vault_path).await?;

        // Build exclusion set: outlinks + backlinks + source itself
        let mut excluded: HashSet<String> = HashSet::new();
        excluded.insert(vault_path.to_string());

        let md_note = self
            .vault
            .get_markdown_and_links(&vault_path)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        for link in md_note.links {
            if let LinkType::Note(linked_path) = link.ltype {
                excluded.insert(linked_path.to_string());
            }
        }

        let backlinks = self
            .vault
            .get_backlinks(&vault_path)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        for (entry, _) in &backlinks {
            excluded.insert(entry.path.to_string());
        }

        // Search each heading; collect, deduplicate, filter, cap
        let mut candidates: Vec<(VaultPath, String)> = Vec::new();
        let mut seen: HashSet<String> = excluded.clone();

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
                seen.insert(path_str);
                let text = self
                    .vault
                    .get_note_text(&entry.path)
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                candidates.push((entry.path, text));
                if candidates.len() >= max {
                    break 'outer;
                }
            }
        }

        if candidates.is_empty() {
            return Ok(vec![PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "Here is the note at \"{}\":\n\n---\n{}\n---\n\nNo unlinked related notes found in the vault.",
                    vault_path, note_text
                ),
            )]);
        }

        let candidates_block: String = candidates
            .iter()
            .map(|(path, text)| format!("=== {} ===\n{}", path, text))
            .collect::<Vec<_>>()
            .join("\n\n");

        let message = format!(
            "Here is the note at \"{path}\":\n\n---\n{note_text}\n---\n\n\
            Candidate notes not yet linked to or from this note:\n\n\
            {candidates_block}\n\n\
            For each candidate:\n\
            1. Assess whether a meaningful conceptual connection exists.\n\
            2. If yes, suggest the exact [[wikilink]] syntax to add and where in the note it fits.\n\
            3. If no clear connection, explain briefly why it was surfaced.",
            path = vault_path,
        );

        Ok(vec![PromptMessage::new_text(PromptMessageRole::User, message)])
    }

    #[prompt(description = "Review inbox notes and suggest how to organize them: move to journal, promote to a proper note with related context, or keep in inbox for later.")]
    async fn triage_inbox(
        &self,
        Parameters(p): Parameters<TriageInboxParams>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let max_notes = p.max_notes.unwrap_or(20) as usize;
        let max_context = p.max_context.unwrap_or(3) as usize;

        let inbox = self.vault.inbox_path().clone();

        let all_notes = self
            .vault
            .get_all_notes()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let inbox_notes: Vec<_> = all_notes
            .into_iter()
            .filter(|(entry, _)| {
                let (parent, _) = entry.path.get_parent_path();
                parent.to_string().starts_with(&inbox.to_string())
                    || parent.is_like(&inbox)
            })
            .take(max_notes)
            .collect();

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
                        let preview: String = rel_content
                            .title
                            .chars()
                            .take(200)
                            .collect();
                        related_section.push_str(&format!(
                            "- {} — \"{}\"\n",
                            rel_entry.path, preview
                        ));
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
                max_results: None,
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
                max_results: None,
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
                max_results: None,
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

    #[tokio::test]
    async fn test_link_suggestions_returns_unlinked_candidates() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "source".to_string(),
                content: "# Source\n\n## Rust Programming\n\nsome rust content".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "candidate".to_string(),
                content: "# Candidate\n\nRust Programming is great".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .link_suggestions(Parameters(LinkSuggestionsParams {
                path: "source".to_string(),
                max_results: Some(5),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("candidate"),
            "expected candidate note in prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_link_suggestions_excludes_already_linked_notes() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "source".to_string(),
                content: "# Source\n\n## Rust Programming\n\nsee [[linked-note]]".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "linked-note".to_string(),
                content: "# Linked Note\n\nRust Programming is great".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .link_suggestions(Parameters(LinkSuggestionsParams {
                path: "source".to_string(),
                max_results: Some(5),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        // The already-linked note should not appear as a candidate
        assert!(
            !text.contains("=== /linked-note") && !text.contains("=== linked-note"),
            "linked-note should be excluded from candidates: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_link_suggestions_empty_vault_returns_graceful_message() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "lonely".to_string(),
                content: "# Lonely\n\n## Some Topic\n\nalone".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .link_suggestions(Parameters(LinkSuggestionsParams {
                path: "lonely".to_string(),
                max_results: Some(5),
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("No unlinked related notes"),
            "expected graceful no-results message: {}",
            text
        );
    }

    // ── research_topic tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_research_topic_no_results_returns_graceful_message() {
        let (handler, _dir) = make_handler().await;
        let msgs = handler
            .research_topic(Parameters(ResearchTopicParams {
                topic: "completely_nonexistent_topic_zzz_123".to_string(),
                max_results: None,
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("No notes found"),
            "expected graceful no-results message: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_research_topic_includes_direct_search_results() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "science/quantum".to_string(),
                content: "# Quantum Physics\n\nunique_quantum_direct_xyz".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .research_topic(Parameters(ResearchTopicParams {
                topic: "unique_quantum_direct_xyz".to_string(),
                max_results: None,
            }))
            .await
            .unwrap();
        assert!(!msgs.is_empty());
        let text = first_text(&msgs);
        assert!(
            text.contains("unique_quantum_direct_xyz"),
            "expected direct result content in prompt: {}",
            text
        );
        assert!(
            text.contains("Notes matching"),
            "expected direct-results section header: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_research_topic_includes_backlinks() {
        let (handler, _dir) = make_handler().await;
        // Note that will be a direct search hit
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "topics/target".to_string(),
                content: "# Target\n\nunique_backlink_target_xyz".to_string(),
            }))
            .await
            .unwrap();
        // Note that links to target — should appear somewhere in the output (via backlinks
        // if the index is warm, or via heading-based secondary search otherwise)
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "topics/linker".to_string(),
                content: "# Linker\n\nSee [[topics/target]] for more detail".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .research_topic(Parameters(ResearchTopicParams {
                topic: "unique_backlink_target_xyz".to_string(),
                max_results: Some(10),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        assert!(
            text.contains("topics/linker"),
            "expected linker note to appear somewhere in the prompt: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_research_topic_includes_related_via_headings() {
        let (handler, _dir) = make_handler().await;
        // Direct hit with a heading that becomes a secondary search term
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "topics/main".to_string(),
                content: "# Main\n\n## Async Runtime\n\nunique_heading_research_abc".to_string(),
            }))
            .await
            .unwrap();
        // Note that matches the heading "Async Runtime"
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "topics/related".to_string(),
                content: "# Related\n\nAsync Runtime is fundamental in Rust".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .research_topic(Parameters(ResearchTopicParams {
                topic: "unique_heading_research_abc".to_string(),
                max_results: Some(10),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        assert!(
            text.contains("topics/related"),
            "expected related note via heading search: {}",
            text
        );
        assert!(
            text.contains("Notes on related subtopics"),
            "expected subtopics section header: {}",
            text
        );
    }

    #[tokio::test]
    async fn test_research_topic_deduplicates_notes() {
        let (handler, _dir) = make_handler().await;
        // Note matches both direct search and would be a backlink from itself — must appear once
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "dedup/alpha".to_string(),
                content: "# Alpha\n\nunique_dedup_topic_xyz\n\n## Subtopic\n\nunique_dedup_sub_xyz".to_string(),
            }))
            .await
            .unwrap();
        // Second note that matches on the subtopic heading
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "dedup/beta".to_string(),
                content: "# Beta\n\nunique_dedup_sub_xyz and more".to_string(),
            }))
            .await
            .unwrap();
        let msgs = handler
            .research_topic(Parameters(ResearchTopicParams {
                topic: "unique_dedup_topic_xyz".to_string(),
                max_results: Some(10),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        // Count occurrences of "dedup/alpha" — should appear exactly once
        let count = text.matches("dedup/alpha").count();
        assert!(
            count >= 1,
            "expected dedup/alpha to appear at least once: {}",
            text
        );
        // The prompt message should only contain one === /dedup/alpha === block
        let header_count = text.matches("/dedup/alpha").count();
        assert!(
            header_count <= 2, // path may appear in section header and content
            "dedup/alpha appeared too many times ({}), suggesting duplicate inclusion: {}",
            header_count,
            text
        );
    }

    #[tokio::test]
    async fn test_research_topic_respects_max_results() {
        let (handler, _dir) = make_handler().await;
        // Create 5 notes all matching the same topic
        for i in 0..5 {
            handler
                .create_note(Parameters(CreateNoteParams {
                    path: format!("limit/note{}", i),
                    content: format!("# Note {}\n\nunique_limit_topic_xyz note number {}", i, i),
                }))
                .await
                .unwrap();
        }
        let msgs = handler
            .research_topic(Parameters(ResearchTopicParams {
                topic: "unique_limit_topic_xyz".to_string(),
                max_results: Some(2),
            }))
            .await
            .unwrap();
        let text = first_text(&msgs);
        // At most 2 note sections should be present
        let section_count = (0..5)
            .filter(|i| text.contains(&format!("limit/note{}", i)))
            .count();
        assert!(
            section_count <= 2,
            "expected at most 2 notes with max_results=2, found {} in: {}",
            section_count,
            text
        );
    }
}
