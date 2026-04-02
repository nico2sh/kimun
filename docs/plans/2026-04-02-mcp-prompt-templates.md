# MCP Prompt Templates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four MCP prompt templates (`daily_review`, `find_connections`, `research_note`, `brainstorm`) to the kimun MCP server by refactoring `mcp.rs` into a directory and adding a `prompts.rs` module.

**Architecture:** `tui/src/cli/commands/mcp.rs` becomes `mcp/mod.rs` (existing tools + ServerHandler) and `mcp/prompts.rs` (new `#[prompt_router]` impl block). `KimunHandler` gains a `prompt_router` field; the `ServerHandler` impl gets `#[prompt_handler]` stacked alongside `#[tool_handler]`. Each prompt fetches vault data at request time and returns it embedded in `Vec<PromptMessage>`.

**Tech Stack:** rmcp 1.3 (`#[prompt_router]`, `#[prompt]`, `#[prompt_handler]`, `PromptRouter<S>`, `PromptMessage`, `PromptMessageRole`), tokio, kimun_core `NoteVault`, chrono.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `tui/src/cli/commands/mcp.rs` | **Delete** | Replaced by directory |
| `tui/src/cli/commands/mcp/mod.rs` | **Create** | Existing content + prompt infrastructure wiring |
| `tui/src/cli/commands/mcp/prompts.rs` | **Create** | `#[prompt_router]` impl + 4 prompts + param structs + tests |
| `tui/tests/mcp_smoke.rs` | **Modify** | Add `prompts/list` assertion |

---

### Task 1: Refactor `mcp.rs` → `mcp/mod.rs`, scaffold prompt infrastructure

This task converts the file into a directory, adds the `prompt_router` field to `KimunHandler`, and wires `#[prompt_handler]` into the `ServerHandler` impl. A minimal `prompts.rs` stub is created so `Self::prompt_router()` exists and the project compiles.

**Files:**
- Create: `tui/src/cli/commands/mcp/mod.rs`
- Create: `tui/src/cli/commands/mcp/prompts.rs`
- Delete: `tui/src/cli/commands/mcp.rs`

- [ ] **Step 1: Create the `mcp/` directory and copy existing content**

```bash
mkdir -p tui/src/cli/commands/mcp
cp tui/src/cli/commands/mcp.rs tui/src/cli/commands/mcp/mod.rs
```

- [ ] **Step 2: Update `tui/src/cli/commands/mcp/mod.rs`**

Make the following changes to `mod.rs` (each change is shown in context):

**2a. Add `pub mod prompts;` at the top (after the existing comments, before `use`):**

```rust
// tui/src/cli/commands/mcp/mod.rs
//
// MCP server handler for kimun — exposes vault operations as MCP tools.

pub mod prompts;
```

**2b. Add `PromptRouter` to the imports block. Change:**

```rust
use rmcp::{
    ErrorData as McpError,
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
    ServiceExt,
};
```

to:

```rust
use rmcp::{
    ErrorData as McpError,
    ServerHandler,
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    schemars,
    prompt_handler, tool, tool_handler, tool_router,
    transport::stdio,
    ServiceExt,
};
```

**2c. Update `KimunHandler` struct to add the `prompt_router` field:**

```rust
#[derive(Clone)]
pub struct KimunHandler {
    vault: Arc<NoteVault>,
    tool_router: ToolRouter<KimunHandler>,
    prompt_router: PromptRouter<KimunHandler>,
}
```

**2d. Update `KimunHandler::new()` inside the `#[tool_router]` impl block:**

```rust
pub fn new(vault: NoteVault) -> Self {
    Self {
        vault: Arc::new(vault),
        tool_router: Self::tool_router(),
        prompt_router: Self::prompt_router(),
    }
}
```

**2e. Stack `#[prompt_handler]` on the `ServerHandler` impl and add `enable_prompts()` to capabilities:**

```rust
#[tool_handler]
#[prompt_handler]
impl ServerHandler for KimunHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_instructions("Kimun notes MCP server — read and write vault notes via tools.")
    }
    // ... rest of impl unchanged
```

> **Note on macro order:** `#[tool_handler]` (outer) runs first, then `#[prompt_handler]` (inner). Each macro adds its own methods to the ServerHandler impl without touching the other's methods. If compilation fails with a conflict, swap the order to `#[prompt_handler]` outer, `#[tool_handler]` inner.

- [ ] **Step 3: Create minimal `tui/src/cli/commands/mcp/prompts.rs`**

This stub is just enough for `Self::prompt_router()` to exist so `mod.rs` compiles:

```rust
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

#[prompt_router]
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
```

- [ ] **Step 4: Delete the old `mcp.rs`**

```bash
rm tui/src/cli/commands/mcp.rs
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo check --package kimun-notes
```

Expected: compiles cleanly. If you see `Self::prompt_router` not found, check that `pub mod prompts;` is at the top of `mod.rs` and that `prompts.rs` has `#[prompt_router] impl KimunHandler { ... }`.

- [ ] **Step 6: Run existing tests**

```bash
cargo test --package kimun-notes --lib 2>&1 | tail -10
```

Expected: same pass count as before (96 passing, 4 ignored).

- [ ] **Step 7: Commit**

```bash
git add tui/src/cli/commands/mcp/ && git rm tui/src/cli/commands/mcp.rs
git commit -m "refactor(mcp): split mcp.rs into mcp/ directory, scaffold prompt infrastructure"
```

---

### Task 2: Implement `daily_review` prompt + tests

**Files:**
- Modify: `tui/src/cli/commands/mcp/prompts.rs`

- [ ] **Step 1: Add failing tests**

Add a `#[cfg(test)]` module at the bottom of `prompts.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{
        CreateNoteParams, JournalParams,
        Parameters as ToolParameters,
    };
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
            .journal(ToolParameters(JournalParams {
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
            .journal(ToolParameters(JournalParams {
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
}
```

> **Import note:** `Parameters` in `prompts.rs` comes from rmcp (for prompt params). To call the journal *tool* method in tests, use `super::super::Parameters as ToolParameters` — but actually both are the same `Parameters` type. Use `use super::super::JournalParams;` and call `handler.journal(Parameters(JournalParams { ... }))` directly since `Parameters` is the same type.
>
> Simpler alternative: use `use super::super::*;` to get everything from `mod.rs` into the test scope. Then `Parameters` and `JournalParams` are directly available.

Update the test imports to:
```rust
use super::*;
use super::super::*;
use tempfile::TempDir;
use kimun_core::NoteVault;
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "daily_review"
```

Expected: 3 `daily_review` tests fail (not yet implemented error).

- [ ] **Step 3: Implement `daily_review`**

Replace the stub body:

```rust
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
```

Add `use chrono;` at the top of `prompts.rs` if not already present (chrono is in `tui/Cargo.toml`).

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "daily_review"
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/cli/commands/mcp/prompts.rs
git commit -m "feat(mcp): implement daily_review prompt"
```

---

### Task 3: Implement `find_connections` prompt + test

**Files:**
- Modify: `tui/src/cli/commands/mcp/prompts.rs`

- [ ] **Step 1: Add failing tests**

Append to the `tests` module:

```rust
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
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "find_connections"
```

- [ ] **Step 3: Implement `find_connections`**

Replace the stub body:

```rust
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
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "find_connections"
```

Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/cli/commands/mcp/prompts.rs
git commit -m "feat(mcp): implement find_connections prompt"
```

---

### Task 4: Implement `research_note` prompt + tests

**Files:**
- Modify: `tui/src/cli/commands/mcp/prompts.rs`

- [ ] **Step 1: Add failing tests**

Append to the `tests` module:

```rust
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
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "research_note"
```

- [ ] **Step 3: Implement `research_note`**

Replace the stub body:

```rust
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
    for (_section_path, chunks) in &chunks_map {
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
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "research_note"
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/cli/commands/mcp/prompts.rs
git commit -m "feat(mcp): implement research_note prompt"
```

---

### Task 5: Implement `brainstorm` prompt + tests

**Files:**
- Modify: `tui/src/cli/commands/mcp/prompts.rs`

- [ ] **Step 1: Add failing tests**

Append to the `tests` module:

```rust
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
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "brainstorm"
```

- [ ] **Step 3: Implement `brainstorm`**

Replace the stub body:

```rust
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
        match self.vault.get_note_text(&entry.path).await {
            Ok(text) => vault_sections.push(format!("=== {} ===\n{}", entry.path, text)),
            Err(_) => {} // skip notes that can't be read
        }
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
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test --package kimun-notes --lib -q 2>&1 | grep "brainstorm"
```

Expected: 3 tests pass.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test --package kimun-notes --lib 2>&1 | tail -5
```

Expected: all tests pass (96+ passing, 4 ignored).

- [ ] **Step 6: Commit**

```bash
git add tui/src/cli/commands/mcp/prompts.rs
git commit -m "feat(mcp): implement brainstorm prompt"
```

---

### Task 6: Update smoke test to assert `prompts/list`

**Files:**
- Modify: `tui/tests/mcp_smoke.rs`

- [ ] **Step 1: Read the existing smoke test**

Read `tui/tests/mcp_smoke.rs` to understand the current structure before editing.

- [ ] **Step 2: Add a `prompts/list` test**

Add a new constant and a second test function at the bottom of `mcp_smoke.rs`:

```rust
const PROMPTS_LIST_MSG: &str = r#"{"jsonrpc":"2.0","id":3,"method":"prompts/list","params":{}}"#;

#[test]
fn mcp_smoke_prompts_list() {
    // Build the binary first
    let build_status = Command::new("cargo")
        .args(["build", "--package", "kimun-notes"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("failed to run cargo build");
    assert!(build_status.success(), "cargo build failed");

    let config_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let config_path = write_config(config_dir.path(), workspace_dir.path());

    let mut child = Command::new(kimun_bin())
        .args(["--config", config_path.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn kimun mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    writeln!(stdin, "{}", INITIALIZE_MSG).unwrap();
    writeln!(stdin, "{}", INITIALIZED_NOTIF).unwrap();
    writeln!(stdin, "{}", PROMPTS_LIST_MSG).unwrap();
    drop(stdin);

    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut combined = String::new();
    for line in reader.lines() {
        if std::time::Instant::now() > deadline {
            panic!("timed out waiting for prompts/list response");
        }
        match line {
            Ok(l) => {
                combined.push_str(&l);
                combined.push('\n');
                if combined.contains(r#""id":3"#) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.wait();

    let expected_prompts = [
        "daily_review",
        "find_connections",
        "research_note",
        "brainstorm",
    ];
    for prompt in &expected_prompts {
        assert!(
            combined.contains(prompt),
            "prompt '{}' not found in prompts/list response:\n{}",
            prompt,
            combined
        );
    }
}
```

- [ ] **Step 3: Run the smoke tests**

```bash
cargo test --package kimun-notes --test mcp_smoke 2>&1 | tail -10
```

Expected: both `mcp_smoke_tools_list` and `mcp_smoke_prompts_list` pass.

- [ ] **Step 4: Commit**

```bash
git add tui/tests/mcp_smoke.rs
git commit -m "test(mcp): add prompts/list assertion to smoke test"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task |
|---|---|
| `mcp.rs` → `mcp/mod.rs` + `mcp/prompts.rs` | Task 1 |
| `KimunHandler` gains `prompt_router` field | Task 1 |
| `#[prompt_handler]` on ServerHandler + `enable_prompts()` | Task 1 |
| `daily_review`: reads journal by date, graceful if absent | Task 2 |
| `find_connections`: full note text + backlink paths | Task 3 |
| `research_note`: section headings as search queries, full text of results | Task 4 |
| `brainstorm`: search top 5, suggest first result as append target | Task 5 |
| Graceful messages (not McpError) for not-found cases | Tasks 2–5 |
| Unit tests, one-per-prompt (multiple cases each) | Tasks 2–5 |
| Smoke test asserts 4 prompt names in `prompts/list` | Task 6 |

**Placeholder scan:** No TBD, no "similar to Task N" references. All code is complete.

**Type consistency:**
- `VaultPath::note_path_from(&str)` — used in Tasks 1, 3, 4 consistently.
- `PromptMessage::new_text(PromptMessageRole::User, text)` — used in Tasks 2–5 consistently.
- `PromptMessageContent::Text { text }` — used in `first_text` helper in Task 2, carried to Tasks 3–5.
- `Parameters<XxxParams>` from `rmcp::handler::server::wrapper` — same type as tools, imported in `prompts.rs`.
- `McpError::internal_error(msg, None)` — used consistently for vault I/O failures across all prompts.
- `journal_entry()` is **not used** in `daily_review` (we use `get_note_text` to avoid side effects) — consistent with spec ("read-only prompt").
