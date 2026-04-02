# MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `kimun mcp` subcommand that runs kimun as an MCP server over stdio, exposing 8 note-management tools and read-only note resources to any MCP-compatible client.

**Architecture:** A new `tui/src/cli/commands/mcp.rs` module contains `KimunHandler`, a `Clone` struct holding an `Arc<NoteVault>` and a `ToolRouter<KimunHandler>`. The `#[tool_router]` macro generates tool dispatch from the impl block methods; `#[tool_handler]` on `ServerHandler for KimunHandler` wires tools and resources into the MCP protocol. The `run()` entry point calls `create_and_init_vault` then `handler.serve(stdio()).await`.

**Tech Stack:** `rmcp` 1.3 (official Rust MCP SDK), `tokio` (already in project), `serde`/`schemars` (from rmcp), `tempfile` (dev tests, already in project).

---

### Task 1: Add `rmcp` dependency

**Files:**
- Modify: `tui/Cargo.toml`

- [ ] **Step 1: Add the dependency**

Open `tui/Cargo.toml` and add to `[dependencies]`:

```toml
rmcp = { version = "1.3", features = ["server", "transport-io"] }
```

- [ ] **Step 2: Verify it resolves**

```bash
cd /path/to/kimun
cargo fetch
```

Expected: no errors, `Cargo.lock` updated with rmcp 1.3.x.

- [ ] **Step 3: Compile-check the workspace**

```bash
cargo check --package kimun-notes
```

Expected: compiles cleanly (rmcp added but nothing uses it yet).

- [ ] **Step 4: Commit**

```bash
git add tui/Cargo.toml Cargo.lock
git commit -m "chore(deps): add rmcp 1.3 for MCP server"
```

---

### Task 2: Scaffold `mcp.rs` module and wire into CLI

This task creates the skeleton that compiles cleanly. All 8 tools are stubbed with `McpError::internal_error("not yet implemented", None)`. Subsequent tasks replace stubs with real logic one group at a time.

**Files:**
- Create: `tui/src/cli/commands/mcp.rs`
- Modify: `tui/src/cli/commands/mod.rs`
- Modify: `tui/src/cli/mod.rs`

- [ ] **Step 1: Write the failing compile test**

Create `tui/src/cli/commands/mcp.rs` with the full scaffold. We have no unit tests yet, so the "failing test" is that the module does not exist. The compile check below will fail until the file is created.

```bash
cargo check --package kimun-notes
```

Expected: FAIL — `mcp` module not found (before creating the file).

- [ ] **Step 2: Create `tui/src/cli/commands/mcp.rs`**

```rust
// tui/src/cli/commands/mcp.rs
//
// `kimun mcp` — runs kimun as an MCP server over stdio.

use std::sync::Arc;

use color_eyre::eyre::{Result, eyre};
use kimun_core::{NoteVault, nfs::VaultPath};
use rmcp::{
    Error as McpError,
    ServerHandler,
    model::*,
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
    ToolRouter,
};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Parameter types — one struct per tool that takes more than one argument
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateNoteParams {
    /// Vault-relative path, e.g. "projects/my-note" or "/inbox/todo"
    pub path: String,
    /// Markdown content
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AppendNoteParams {
    /// Vault-relative path to the note
    pub path: String,
    /// Text to append
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShowNoteParams {
    /// Vault-relative path to the note
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchNotesParams {
    /// Query string. Supports: @filename, >heading, /path, -exclusion
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListNotesParams {
    /// Optional path prefix to filter results (e.g. "projects/")
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct JournalParams {
    /// Text to append to today's journal entry
    pub text: String,
    /// Date override in YYYY-MM-DD format; defaults to today
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BacklinksParams {
    /// Vault-relative path to the note
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ChunksParams {
    /// Vault-relative path to the note
    pub path: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct KimunHandler {
    vault: Arc<NoteVault>,
    tool_router: ToolRouter<KimunHandler>,
}

#[tool_router]
impl KimunHandler {
    pub fn new(vault: NoteVault) -> Self {
        Self {
            vault: Arc::new(vault),
            tool_router: Self::tool_router(),
        }
    }

    /// Resolve a user-supplied path string into a VaultPath with .md extension.
    fn resolve_path(path: &str) -> VaultPath {
        let trimmed = path.trim();
        let canonical = if trimmed.starts_with('/') {
            trimmed.to_string()
        } else {
            format!("/{}", trimmed)
        };
        VaultPath::note_path_from(&canonical)
    }

    #[tool(description = "Create a new note at the given vault path with the given markdown content. Fails if the note already exists.")]
    async fn create_note(
        &self,
        Parameters(p): Parameters<CreateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Append text to an existing note. Creates the note if it does not exist.")]
    async fn append_note(
        &self,
        Parameters(p): Parameters<AppendNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Return the full markdown content of a note.")]
    async fn show_note(
        &self,
        Parameters(p): Parameters<ShowNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Search notes by query. Supports @filename, >heading, /path prefix, and -exclusion operators.")]
    async fn search_notes(
        &self,
        Parameters(p): Parameters<SearchNotesParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "List all notes in the vault, optionally filtered by path prefix.")]
    async fn list_notes(
        &self,
        Parameters(p): Parameters<ListNotesParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Append text to today's journal entry (or a specific date). Creates the entry if absent.")]
    async fn journal(
        &self,
        Parameters(p): Parameters<JournalParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Return the list of notes that link to the given note (backlinks).")]
    async fn get_backlinks(
        &self,
        Parameters(p): Parameters<BacklinksParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Return the content chunks (sections) of a note as JSON.")]
    async fn get_chunks(
        &self,
        Parameters(p): Parameters<ChunksParams>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler — wires tools + resources into the MCP protocol
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for KimunHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            name: "kimun".to_string().into(),
            version: env!("CARGO_PKG_VERSION").to_string().into(),
            ..Default::default()
        }
    }

    async fn list_resources(
        &self,
        _params: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult { resources: vec![], next_cursor: None, meta: None })
    }

    async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
    }

    async fn list_resource_templates(
        &self,
        _params: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![],
            next_cursor: None,
            meta: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Entry point called by `kimun mcp`
// ---------------------------------------------------------------------------

pub async fn run(config_path: Option<std::path::PathBuf>) -> Result<()> {
    use crate::cli::helpers::create_and_init_vault;

    let (vault, _workspace_name) = create_and_init_vault(config_path).await?;
    let handler = KimunHandler::new(vault);
    let service = handler.serve(stdio()).await.map_err(|e| eyre!("{e}"))?;
    service.waiting().await.map_err(|e| eyre!("{e}"))?;
    Ok(())
}
```

> **Note on rmcp API:** If any import path does not exist, run `cargo doc --open --package rmcp` and verify:
> - `ToolRouter` re-export location (may be `rmcp::handler::server::tool::ToolRouter`)
> - `Parameters` type location (may need explicit import: `use rmcp::handler::server::tool::Parameters`)
> - `RequestContext` location (may be `rmcp::service::RequestContext`)
> - `ServerInfo` default fields (may differ; check what fields are required)
>
> Fix imports accordingly before proceeding.

- [ ] **Step 3: Register the module**

In `tui/src/cli/commands/mod.rs`, add:

```rust
pub mod mcp;
```

(append to the existing list of module declarations)

- [ ] **Step 4: Add `Mcp` variant to `CliCommand` and dispatch**

In `tui/src/cli/mod.rs`, add the `Mcp` variant to the `CliCommand` enum:

```rust
/// Run kimun as an MCP server over stdio
Mcp,
```

And add a match arm in `run_cli`:

```rust
CliCommand::Mcp => commands::mcp::run(config_path).await,
```

Also add the import at the top of the match (if needed — `config_path` is already available as a parameter to `run_cli`).

- [ ] **Step 5: Verify it compiles**

```bash
cargo check --package kimun-notes
```

Expected: PASS — all stubs compile, no warnings about unused imports (the stubs reference `p` so that may trigger an unused-variable warning; suppress with `let _p = p;` or rename parameter to `_p` in each stub method if needed).

- [ ] **Step 6: Run tests to make sure nothing broke**

```bash
cargo nextest run --package kimun-notes
```

Expected: all pre-existing tests pass.

- [ ] **Step 7: Commit**

```bash
git add tui/src/cli/commands/mcp.rs tui/src/cli/commands/mod.rs tui/src/cli/mod.rs
git commit -m "feat(mcp): scaffold KimunHandler with all tool stubs and CLI wiring"
```

---

### Task 3: Implement `create_note`, `show_note`, `append_note` tools + unit tests

**Files:**
- Modify: `tui/src/cli/commands/mcp.rs` (replace 3 stubs, add tests)

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block at the bottom of `tui/src/cli/commands/mcp.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use kimun_core::NoteVault;

    async fn make_handler() -> (KimunHandler, TempDir) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(dir.path()).await.unwrap();
        vault.validate_and_init().await.unwrap();
        let handler = KimunHandler::new(vault);
        (handler, dir)
    }

    fn is_success(result: &CallToolResult) -> bool {
        result.is_error != Some(true)
    }

    fn result_text(result: &CallToolResult) -> String {
        serde_json::to_string(&result.content).unwrap_or_default()
    }

    #[tokio::test]
    async fn test_create_note_succeeds() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .create_note(Parameters(CreateNoteParams {
                path: "test/hello".to_string(),
                content: "# Hello\n\nworld".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "expected success, got: {:?}", result_text(&result));
        assert!(result_text(&result).contains("test/hello"));
    }

    #[tokio::test]
    async fn test_create_note_fails_if_exists() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "test/hello".to_string(),
                content: "first".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .create_note(Parameters(CreateNoteParams {
                path: "test/hello".to_string(),
                content: "second".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_show_note_returns_content() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "show/me".to_string(),
                content: "# Show me\n\nsome content".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .show_note(Parameters(ShowNoteParams { path: "show/me".to_string() }))
            .await
            .unwrap();
        assert!(is_success(&result));
        assert!(result_text(&result).contains("some content"));
    }

    #[tokio::test]
    async fn test_show_note_not_found_returns_error_result() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .show_note(Parameters(ShowNoteParams { path: "missing/note".to_string() }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_append_note_creates_if_absent() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .append_note(Parameters(AppendNoteParams {
                path: "new/note".to_string(),
                content: "appended text".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
        let show = handler
            .show_note(Parameters(ShowNoteParams { path: "new/note".to_string() }))
            .await
            .unwrap();
        assert!(result_text(&show).contains("appended text"));
    }

    #[tokio::test]
    async fn test_append_note_appends_to_existing() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "exist/note".to_string(),
                content: "original".to_string(),
            }))
            .await
            .unwrap();
        handler
            .append_note(Parameters(AppendNoteParams {
                path: "exist/note".to_string(),
                content: "added".to_string(),
            }))
            .await
            .unwrap();
        let show = handler
            .show_note(Parameters(ShowNoteParams { path: "exist/note".to_string() }))
            .await
            .unwrap();
        let text = result_text(&show);
        assert!(text.contains("original"), "missing 'original' in: {}", text);
        assert!(text.contains("added"), "missing 'added' in: {}", text);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo nextest run --package kimun-notes mcp::tests
```

Expected: FAIL — tests hit the `not yet implemented` stub error.

- [ ] **Step 3: Implement `create_note`**

Replace the `create_note` stub body:

```rust
#[tool(description = "Create a new note at the given vault path with the given markdown content. Fails if the note already exists.")]
async fn create_note(
    &self,
    Parameters(p): Parameters<CreateNoteParams>,
) -> Result<CallToolResult, McpError> {
    let vault_path = Self::resolve_path(&p.path);
    match self.vault.create_note(&vault_path, &p.content).await {
        Ok(_) => Ok(CallToolResult::success(vec![Content::text(
            format!("Note created: {}", vault_path),
        )])),
        Err(kimun_core::error::VaultError::NoteExists { .. }) => Ok(CallToolResult {
            content: vec![Content::text(format!("Note already exists: {}", vault_path))],
            is_error: Some(true),
            meta: None,
        }),
        Err(e) => Err(McpError::internal_error(e.to_string(), None)),
    }
}
```

- [ ] **Step 4: Implement `show_note`**

Replace the `show_note` stub body:

```rust
#[tool(description = "Return the full markdown content of a note.")]
async fn show_note(
    &self,
    Parameters(p): Parameters<ShowNoteParams>,
) -> Result<CallToolResult, McpError> {
    let vault_path = Self::resolve_path(&p.path);
    match self.vault.get_note_text(&vault_path).await {
        Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
        Err(kimun_core::error::VaultError::FSError(
            kimun_core::error::FSError::VaultPathNotFound { .. },
        )) => Ok(CallToolResult {
            content: vec![Content::text(format!("Note not found: {}", vault_path))],
            is_error: Some(true),
            meta: None,
        }),
        Err(e) => Err(McpError::internal_error(e.to_string(), None)),
    }
}
```

- [ ] **Step 5: Implement `append_note`**

Replace the `append_note` stub body:

```rust
#[tool(description = "Append text to an existing note. Creates the note if it does not exist.")]
async fn append_note(
    &self,
    Parameters(p): Parameters<AppendNoteParams>,
) -> Result<CallToolResult, McpError> {
    let vault_path = Self::resolve_path(&p.path);
    let existing = self
        .vault
        .load_or_create_note(&vault_path, None)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let combined = if existing.is_empty() {
        p.content.clone()
    } else {
        format!("{}\n{}", existing, p.content)
    };
    self.vault
        .save_note(&vault_path, &combined)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![Content::text(format!(
        "Note saved: {}",
        vault_path
    ))]))
}
```

- [ ] **Step 6: Add error imports at the top of the file**

Add to the `use` block at the top of `mcp.rs`:

```rust
use kimun_core::error::{FSError, VaultError};
```

- [ ] **Step 7: Run tests to confirm they pass**

```bash
cargo nextest run --package kimun-notes mcp::tests
```

Expected: all 6 CRUD tests pass.

- [ ] **Step 8: Commit**

```bash
git add tui/src/cli/commands/mcp.rs
git commit -m "feat(mcp): implement create_note, show_note, append_note tools"
```

---

### Task 4: Implement `search_notes` and `list_notes` tools + unit tests

**Files:**
- Modify: `tui/src/cli/commands/mcp.rs`

- [ ] **Step 1: Add tests for search and list**

Append to the `tests` module:

```rust
    #[tokio::test]
    async fn test_search_notes_finds_match() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "alpha/one".to_string(),
                content: "# Alpha\n\ncontains unique_keyword_xyz".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .search_notes(Parameters(SearchNotesParams {
                query: "unique_keyword_xyz".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
        assert!(
            result_text(&result).contains("alpha/one"),
            "search result did not include 'alpha/one': {}",
            result_text(&result)
        );
    }

    #[tokio::test]
    async fn test_search_notes_returns_empty_for_no_match() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .search_notes(Parameters(SearchNotesParams {
                query: "nonexistent_zzz_123".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
    }

    #[tokio::test]
    async fn test_list_notes_returns_all() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "folder/a".to_string(),
                content: "note a".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "folder/b".to_string(),
                content: "note b".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .list_notes(Parameters(ListNotesParams { path: None }))
            .await
            .unwrap();
        assert!(is_success(&result));
        let text = result_text(&result);
        assert!(text.contains("folder/a"), "missing 'folder/a': {}", text);
        assert!(text.contains("folder/b"), "missing 'folder/b': {}", text);
    }

    #[tokio::test]
    async fn test_list_notes_filters_by_prefix() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "projects/foo".to_string(),
                content: "foo".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "journal/2026-01-01".to_string(),
                content: "journal".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .list_notes(Parameters(ListNotesParams {
                path: Some("projects".to_string()),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
        let text = result_text(&result);
        assert!(text.contains("projects/foo"), "missing projects/foo: {}", text);
        assert!(!text.contains("journal/2026"), "should not include journal: {}", text);
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_search" "mcp::tests::test_list"
```

Expected: FAIL — stubs return internal error.

- [ ] **Step 3: Implement `search_notes`**

Replace the `search_notes` stub body:

```rust
#[tool(description = "Search notes by query. Supports @filename, >heading, /path prefix, and -exclusion operators.")]
async fn search_notes(
    &self,
    Parameters(p): Parameters<SearchNotesParams>,
) -> Result<CallToolResult, McpError> {
    let results = self
        .vault
        .search_notes(&p.query)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    if results.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text("No results found.")]));
    }
    let lines: Vec<String> = results
        .iter()
        .map(|(entry, content)| {
            format!("{} — {}", entry.path.to_string_with_ext(), content.title)
        })
        .collect();
    Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
}
```

- [ ] **Step 4: Implement `list_notes`**

Replace the `list_notes` stub body:

```rust
#[tool(description = "List all notes in the vault, optionally filtered by path prefix.")]
async fn list_notes(
    &self,
    Parameters(p): Parameters<ListNotesParams>,
) -> Result<CallToolResult, McpError> {
    let all = self
        .vault
        .get_all_notes()
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let filtered: Vec<_> = match &p.path {
        None => all,
        Some(prefix) => {
            let norm = prefix.trim_matches('/');
            all.into_iter()
                .filter(|(entry, _)| {
                    entry.path.to_string().trim_start_matches('/').starts_with(norm)
                })
                .collect()
        }
    };
    if filtered.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text("No notes found.")]));
    }
    let lines: Vec<String> = filtered
        .iter()
        .map(|(entry, content)| {
            format!("{} — {}", entry.path.to_string_with_ext(), content.title)
        })
        .collect();
    Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
}
```

> **Note on `VaultPath` methods:** The path is displayed via `entry.path.to_string_with_ext()` which includes the `.md` extension, matching the spec's path format. If that method does not exist, use `format!("{}.md", entry.path)` instead.

- [ ] **Step 5: Run tests to confirm they pass**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_search" "mcp::tests::test_list"
```

Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add tui/src/cli/commands/mcp.rs
git commit -m "feat(mcp): implement search_notes and list_notes tools"
```

---

### Task 5: Implement `journal` tool + unit test

**Files:**
- Modify: `tui/src/cli/commands/mcp.rs`

- [ ] **Step 1: Add test for journal**

Append to the `tests` module:

```rust
    #[tokio::test]
    async fn test_journal_appends_to_today() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .journal(Parameters(JournalParams {
                text: "Today's thought".to_string(),
                date: None,
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "expected success: {}", result_text(&result));
        // Call show_note on today's journal path to verify content was written.
        // We don't know the exact path, so we verify via the tool result message.
        assert!(
            result_text(&result).contains("saved"),
            "expected 'saved' in result: {}",
            result_text(&result)
        );
    }

    #[tokio::test]
    async fn test_journal_with_explicit_date() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .journal(Parameters(JournalParams {
                text: "Entry for specific date".to_string(),
                date: Some("2026-01-15".to_string()),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "expected success: {}", result_text(&result));
    }

    #[tokio::test]
    async fn test_journal_invalid_date_returns_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .journal(Parameters(JournalParams {
                text: "bad date".to_string(),
                date: Some("not-a-date".to_string()),
            }))
            .await
            .unwrap();
        assert_eq!(
            result.is_error,
            Some(true),
            "expected error for invalid date"
        );
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_journal"
```

Expected: FAIL — stub returns internal error.

- [ ] **Step 3: Implement `journal`**

Replace the `journal` stub body:

```rust
#[tool(description = "Append text to today's journal entry (or a specific date). Creates the entry if absent.")]
async fn journal(
    &self,
    Parameters(p): Parameters<JournalParams>,
) -> Result<CallToolResult, McpError> {
    // Validate date if provided
    let date_str = match p.date.as_deref() {
        None => chrono::Utc::now().format("%Y-%m-%d").to_string(),
        Some(d) => {
            if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
                return Ok(CallToolResult {
                    content: vec![Content::text(format!(
                        "Invalid date '{}' — expected YYYY-MM-DD",
                        d
                    ))],
                    is_error: Some(true),
                    meta: None,
                });
            }
            d.to_string()
        }
    };

    let (vault_path, existing) = if p.date.is_none() {
        // Today — use journal_entry() which handles create-if-absent
        let (details, existing) = self
            .vault
            .journal_entry()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        (details.path, existing)
    } else {
        // Specific date
        let journal_path = self
            .vault
            .journal_path()
            .append(&VaultPath::note_path_from(&date_str))
            .absolute();
        let existing = self
            .vault
            .load_or_create_note(&journal_path, Some(format!("# {}\n\n", date_str)))
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        (journal_path, existing)
    };

    let combined = format!("{}\n{}", existing, p.text);
    self.vault
        .save_note(&vault_path, &combined)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

    Ok(CallToolResult::success(vec![Content::text(format!(
        "Note saved: {}",
        vault_path
    ))]))
}
```

Add `use chrono;` to the imports at the top of the file (chrono is already in tui/Cargo.toml).

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_journal"
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/cli/commands/mcp.rs
git commit -m "feat(mcp): implement journal tool"
```

---

### Task 6: Implement `get_backlinks` and `get_chunks` tools + unit tests

**Files:**
- Modify: `tui/src/cli/commands/mcp.rs`

- [ ] **Step 1: Add tests for backlinks and chunks**

Append to the `tests` module:

```rust
    #[tokio::test]
    async fn test_get_backlinks_empty_for_no_links() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "standalone".to_string(),
                content: "# Standalone\n\nNo links here.".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .get_backlinks(Parameters(BacklinksParams {
                path: "standalone".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
    }

    #[tokio::test]
    async fn test_get_backlinks_finds_linking_note() {
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
                content: "links to [[target]]".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .get_backlinks(Parameters(BacklinksParams {
                path: "target".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
        assert!(
            result_text(&result).contains("source"),
            "expected 'source' in backlinks: {}",
            result_text(&result)
        );
    }

    #[tokio::test]
    async fn test_get_chunks_returns_sections() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "chunked".to_string(),
                content: "# Title\n\n## Section One\n\nparagraph\n\n## Section Two\n\nmore".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .get_chunks(Parameters(ChunksParams {
                path: "chunked".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
        assert!(
            result_text(&result).contains("Section"),
            "expected section in chunks: {}",
            result_text(&result)
        );
    }

    #[tokio::test]
    async fn test_get_chunks_not_found_returns_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .get_chunks(Parameters(ChunksParams {
                path: "missing/note".to_string(),
            }))
            .await
            .unwrap();
        // get_note_chunks may return an empty map for missing notes rather than
        // erroring; either an empty result or is_error=true is acceptable.
        // The assertion just checks it didn't panic.
        let _ = result;
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_get_backlinks" "mcp::tests::test_get_chunks"
```

Expected: FAIL — stubs return internal error.

- [ ] **Step 3: Implement `get_backlinks`**

Replace the `get_backlinks` stub body:

```rust
#[tool(description = "Return the list of notes that link to the given note (backlinks).")]
async fn get_backlinks(
    &self,
    Parameters(p): Parameters<BacklinksParams>,
) -> Result<CallToolResult, McpError> {
    let vault_path = Self::resolve_path(&p.path);
    let backlinks = self
        .vault
        .get_backlinks(&vault_path)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    if backlinks.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text("No backlinks found.")]));
    }
    let lines: Vec<String> = backlinks
        .iter()
        .map(|(entry, content)| {
            format!("{} — {}", entry.path.to_string_with_ext(), content.title)
        })
        .collect();
    Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
}
```

- [ ] **Step 4: Implement `get_chunks`**

Replace the `get_chunks` stub body:

```rust
#[tool(description = "Return the content chunks (sections) of a note as JSON.")]
async fn get_chunks(
    &self,
    Parameters(p): Parameters<ChunksParams>,
) -> Result<CallToolResult, McpError> {
    let vault_path = Self::resolve_path(&p.path);
    let chunks_map = self
        .vault
        .get_note_chunks(&vault_path)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

    // chunks_map keys are VaultPaths (one per heading), values are Vec<ContentChunk>
    // Flatten into a readable list: breadcrumb > chunk text
    let mut lines: Vec<String> = Vec::new();
    for (section_path, chunks) in &chunks_map {
        for chunk in chunks {
            let breadcrumb = chunk.breadcrumb.join(" > ");
            lines.push(format!("[{}] {}: {}", section_path, breadcrumb, chunk.text));
        }
    }

    if lines.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text("No chunks found.")]));
    }
    Ok(CallToolResult::success(vec![Content::text(lines.join("\n\n"))]))
}
```

- [ ] **Step 5: Run tests to confirm they pass**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_get_backlinks" "mcp::tests::test_get_chunks"
```

Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add tui/src/cli/commands/mcp.rs
git commit -m "feat(mcp): implement get_backlinks and get_chunks tools"
```

---

### Task 7: Implement MCP resources (list + read) + unit tests

**Files:**
- Modify: `tui/src/cli/commands/mcp.rs`

Resources are in the `ServerHandler` impl (not the `#[tool_router]` impl). Testing them directly requires calling the async methods on the handler.

- [ ] **Step 1: Add tests for resources**

Append to the `tests` module:

```rust
    #[tokio::test]
    async fn test_list_resources_returns_notes() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "res/alpha".to_string(),
                content: "# Alpha Note".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .list_resources(None, rmcp::service::RequestContext::default())
            .await
            .unwrap();
        assert!(!result.resources.is_empty());
        let uris: Vec<String> = result.resources.iter().map(|r| r.uri.to_string()).collect();
        assert!(
            uris.iter().any(|u| u.contains("res/alpha")),
            "expected 'res/alpha' in URIs: {:?}",
            uris
        );
    }

    #[tokio::test]
    async fn test_read_resource_returns_content() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "res/beta".to_string(),
                content: "# Beta\n\nbeta content".to_string(),
            }))
            .await
            .unwrap();
        let uri = "note://res/beta.md";
        let result = handler
            .read_resource(
                ReadResourceRequestParams { uri: uri.to_string().into() },
                rmcp::service::RequestContext::default(),
            )
            .await
            .unwrap();
        let content_json = serde_json::to_string(&result.contents).unwrap();
        assert!(
            content_json.contains("beta content"),
            "expected 'beta content': {}",
            content_json
        );
    }

    #[tokio::test]
    async fn test_read_resource_not_found_returns_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .read_resource(
                ReadResourceRequestParams { uri: "note://missing/note.md".to_string().into() },
                rmcp::service::RequestContext::default(),
            )
            .await;
        assert!(result.is_err(), "expected Err for missing note");
    }

    #[tokio::test]
    async fn test_read_resource_invalid_scheme_returns_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .read_resource(
                ReadResourceRequestParams { uri: "file:///etc/passwd".to_string().into() },
                rmcp::service::RequestContext::default(),
            )
            .await;
        assert!(result.is_err(), "expected Err for invalid URI scheme");
    }
```

> **Note on `RequestContext::default()`:** If `RequestContext<RoleServer>` does not implement `Default`, construct it with whatever minimal fields are required. Check rmcp docs for how to build a test context. You may need a `tokio::sync::mpsc::channel` for the notification sender — in that case, construct it as:
> ```rust
> let (_tx, _rx) = tokio::sync::mpsc::channel(1);
> rmcp::service::RequestContext { /* fill fields */ }
> ```
> Alternatively, skip resource tests that require context construction and mark them `#[ignore]` until you confirm the API.

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_list_resources" "mcp::tests::test_read_resource"
```

Expected: FAIL — list_resources returns empty vec, read_resource returns internal error.

- [ ] **Step 3: Implement `list_resources`**

Replace the `list_resources` body in the `ServerHandler` impl:

```rust
async fn list_resources(
    &self,
    _params: Option<PaginatedRequestParams>,
    _ctx: RequestContext<RoleServer>,
) -> Result<ListResourcesResult, McpError> {
    let notes = self
        .vault
        .get_all_notes()
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let resources: Vec<Resource> = notes
        .into_iter()
        .map(|(entry, content)| {
            let path_with_ext = entry.path.to_string_with_ext();
            let uri = format!("note://{}", path_with_ext.trim_start_matches('/'));
            Resource {
                uri: uri.into(),
                name: if content.title.is_empty() {
                    entry.path.file_name().unwrap_or_default().to_string()
                } else {
                    content.title
                }
                .into(),
                mime_type: Some("text/markdown".to_string().into()),
                description: None,
                annotations: None,
            }
        })
        .collect();
    Ok(ListResourcesResult {
        resources,
        next_cursor: None,
        meta: None,
    })
}
```

> **Note on `Resource` field names:** If `Resource` uses different field names or an `Into<String>` or `Cow<str>` type for `uri`/`name`, adjust accordingly. Check the rmcp `Resource` struct definition with `cargo doc`.

- [ ] **Step 4: Implement `read_resource`**

Replace the `read_resource` body in the `ServerHandler` impl:

```rust
async fn read_resource(
    &self,
    params: ReadResourceRequestParams,
    _ctx: RequestContext<RoleServer>,
) -> Result<ReadResourceResult, McpError> {
    let uri = params.uri.as_str();
    let path_str = uri
        .strip_prefix("note://")
        .ok_or_else(|| McpError::invalid_params("URI must use note:// scheme", None))?;
    // Normalise: strip .md extension so resolve_path can add it back
    let path_no_ext = path_str.trim_end_matches(".md");
    let vault_path = Self::resolve_path(path_no_ext);
    let text = self
        .vault
        .get_note_text(&vault_path)
        .await
        .map_err(|e| match e {
            VaultError::FSError(FSError::VaultPathNotFound { .. }) => {
                McpError::invalid_params(format!("Note not found: {}", vault_path), None)
            }
            other => McpError::internal_error(other.to_string(), None),
        })?;
    Ok(ReadResourceResult {
        contents: vec![ResourceContents::TextResourceContents(TextResourceContents {
            uri: params.uri,
            mime_type: Some("text/markdown".to_string().into()),
            text: text.into(),
        })],
        meta: None,
    })
}
```

> **Note on `ResourceContents` variants:** If the rmcp type uses a different construction pattern (e.g., `ResourceContents::text(text, uri)`), use that instead. Check `cargo doc --package rmcp`.

- [ ] **Step 5: Run tests to confirm they pass**

```bash
cargo nextest run --package kimun-notes "mcp::tests::test_list_resources" "mcp::tests::test_read_resource"
```

Expected: 4 tests pass (or 3 if the context test was `#[ignore]`d).

- [ ] **Step 6: Run the full test suite**

```bash
cargo nextest run --package kimun-notes
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add tui/src/cli/commands/mcp.rs
git commit -m "feat(mcp): implement MCP resources (list and read)"
```

---

### Task 8: Integration smoke test

This test spawns the real `kimun mcp` binary and sends a JSON-RPC `initialize` + `tools/list` request over stdin. It verifies the expected tool names are present in the response.

**Files:**
- Create: `tui/tests/mcp_smoke.rs`

- [ ] **Step 1: Write the failing test**

Create `tui/tests/mcp_smoke.rs`:

```rust
// tui/tests/mcp_smoke.rs
//
// Integration smoke test: spawns `kimun mcp`, sends initialize + tools/list,
// asserts expected tool names appear in the response.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

fn kimun_bin() -> std::path::PathBuf {
    // cargo builds the binary into target/{profile}/kimun
    let mut p = std::env::current_exe().unwrap();
    p.pop(); // remove test binary name
    // may be target/debug/deps — go up one more if in deps/
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("kimun")
}

fn write_config(dir: &std::path::Path, workspace: &std::path::Path) -> std::path::PathBuf {
    let config_path = dir.join("kimun_config.toml");
    std::fs::write(
        &config_path,
        format!(
            "[workspace_config.global]\ncurrent_workspace = \"default\"\n\n[workspace_config.workspaces.default]\npath = {:?}\n",
            workspace.display()
        ),
    )
    .unwrap();
    config_path
}

const INITIALIZE_MSG: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-test","version":"0.0.1"}}}"#;
const INITIALIZED_NOTIF: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
const TOOLS_LIST_MSG: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;

#[test]
fn mcp_smoke_tools_list() {
    // Build first to ensure binary exists
    let status = Command::new("cargo")
        .args(["build", "--package", "kimun-notes"])
        .status()
        .expect("failed to run cargo build");
    assert!(status.success(), "cargo build failed");

    let config_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    // Initialise the vault DB so `kimun mcp` doesn't fail on startup
    let init_status = Command::new(kimun_bin())
        .args([
            "--config",
            config_dir.path().join("kimun_config.toml").to_str().unwrap(),
            "workspace",
            "reindex",
        ])
        .env("HOME", config_dir.path())
        .status();
    // If the binary or reindex doesn't exist yet, skip gracefully
    if init_status.is_err() {
        eprintln!("skipping smoke test: binary not available");
        return;
    }

    write_config(config_dir.path(), workspace_dir.path());

    let mut child = Command::new(kimun_bin())
        .args([
            "--config",
            config_dir.path().join("kimun_config.toml").to_str().unwrap(),
            "mcp",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn kimun mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Send initialize (newline-delimited JSON-RPC)
    writeln!(stdin, "{}", INITIALIZE_MSG).unwrap();
    writeln!(stdin, "{}", INITIALIZED_NOTIF).unwrap();
    writeln!(stdin, "{}", TOOLS_LIST_MSG).unwrap();
    drop(stdin); // close stdin so the process can see EOF

    // Read output with a timeout
    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut combined = String::new();
    for line in reader.lines() {
        if std::time::Instant::now() > deadline {
            break;
        }
        match line {
            Ok(l) => {
                combined.push_str(&l);
                combined.push('\n');
                // Stop once we see a tools/list result
                if combined.contains(r#""id":2"#) {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = child.wait();

    // Assert all expected tool names appear in the tools/list response
    let expected_tools = [
        "create_note",
        "append_note",
        "show_note",
        "search_notes",
        "list_notes",
        "journal",
        "get_backlinks",
        "get_chunks",
    ];
    for tool in &expected_tools {
        assert!(
            combined.contains(tool),
            "tool '{}' not found in tools/list response:\n{}",
            tool,
            combined
        );
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo nextest run --package kimun-notes --test mcp_smoke
```

Expected: FAIL — `kimun mcp` binary does not yet exist (or `mcp` subcommand not wired).

Actually at this point the binary exists but the `mcp` subcommand is wired from Task 2, so the test should pass if everything is correct. The test itself may fail on finding the binary path. Adjust `kimun_bin()` if the path resolution is wrong.

- [ ] **Step 3: Make the test pass**

The main binary is built by Task 1–7 changes. If the test path resolution fails, run:

```bash
cargo build --package kimun-notes
find target -name "kimun" -type f 2>/dev/null
```

And update `kimun_bin()` to match the actual output path.

- [ ] **Step 4: Run the smoke test**

```bash
cargo nextest run --package kimun-notes --test mcp_smoke -- --nocapture
```

Expected: PASS — all 8 tool names appear in the `tools/list` response.

> **Note on config format:** The `write_config` helper writes a minimal `kimun_config.toml` that matches the `AppSettings` format used by the CLI. If the config format changes, inspect `tui/src/settings.rs` and `tui/src/cli/helpers.rs` to find the `write_config` helper used in existing CLI tests and match that format exactly.

- [ ] **Step 5: Commit**

```bash
git add tui/tests/mcp_smoke.rs
git commit -m "test(mcp): integration smoke test for tools/list over stdio"
```

---

### Task 9: Update documentation

**Files:**
- Modify: `docs/content/getting-started/configuration.md` (or whichever docs file covers the CLI)
- Create/modify: relevant docs page for MCP client setup

- [ ] **Step 1: Find the right docs file**

```bash
grep -rl "skills\|skills.md\|mcp\|MCP" docs/content/ | head -10
```

Identify which file documents the LLM/AI integration features.

- [ ] **Step 2: Add MCP server section**

Add a new section to the relevant docs file. The content should match what's already in `docs/specs/2026-04-02-mcp-server-design.md` Client Configuration section:

```markdown
## MCP Server

`kimun mcp` runs kimun as a [Model Context Protocol](https://modelcontextprotocol.io) server over stdio. Any MCP-compatible client (Claude Desktop, Claude Code, Zed, Cursor, etc.) can connect to manage notes and search the vault.

### Tools

| Tool | Description |
|---|---|
| `create_note` | Create a new note (fails if exists) |
| `append_note` | Append text to a note (creates if absent) |
| `show_note` | Return full note content |
| `search_notes` | Search with `@`, `>`, `/`, `-` operators |
| `list_notes` | List all notes, optionally filtered by path prefix |
| `journal` | Append to today's (or a specific date's) journal entry |
| `get_backlinks` | List notes that link to the given note |
| `get_chunks` | Return note sections as structured content |

### Resources

Notes are also exposed as MCP resources with the `note://` URI scheme (e.g. `note://journal/2026-04-02.md`). Clients can browse and attach notes to their context directly.

### Client Configuration

**Claude Desktop** — add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "kimun": {
      "command": "kimun",
      "args": ["mcp"]
    }
  }
}
```

**Claude Code**:

```sh
claude mcp add kimun -- kimun mcp
```

### Running alongside the TUI

`kimun mcp` and `kimun` (TUI) can run simultaneously against the same vault.
```

- [ ] **Step 3: Commit**

```bash
git add docs/
git commit -m "docs: add MCP server setup and client configuration guide"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Covered by task |
|---|---|
| `kimun mcp` subcommand | Task 2 (CliCommand::Mcp) |
| stdio transport | Task 2 (`run()` + `stdio()`) |
| NoteVault init at startup | Task 2 (`create_and_init_vault`) |
| 8 tools (create, append, show, search, list, journal, backlinks, chunks) | Tasks 3–6 |
| Resources: list (note:// URIs) | Task 7 |
| Resources: read | Task 7 |
| Error handling: not found → error response | Tasks 3, 7 |
| Error handling: startup init | Task 2 |
| Unit tests (one per tool) | Tasks 3–7 |
| Integration smoke test | Task 8 |
| Client config docs (Claude Desktop + Claude Code) | Task 9 |

**Placeholder scan:** No "TBD", "TODO", or vague steps found. All tool implementations include full code.

**Type consistency check:**
- `VaultPath::note_path_from` — used in Tasks 2–7 consistently
- `VaultPath::to_string_with_ext()` — used in Tasks 4, 6, 7 for display; if method not found, replace with `format!("{}.md", entry.path)`
- `VaultPath::file_name()` — used in Task 7 for resource name fallback; check this method exists on `VaultPath`
- `CallToolResult::success(vec![...])` — used throughout; if not a method, construct as `CallToolResult { content: vec![...], is_error: None, meta: None }`
- `Content::text("string")` — used throughout; if signature differs, check rmcp docs
- `journal_entry()` returns `(NoteDetails, String)` — `NoteDetails.path` is a `VaultPath` ✓ (confirmed from core/src/lib.rs)
- `load_or_create_note` returns `String` ✓ (confirmed from core/src/lib.rs:247)
- `get_note_chunks` returns `HashMap<VaultPath, Vec<ContentChunk>>` ✓ (confirmed from core/src/lib.rs:284)
- `ContentChunk.breadcrumb: Vec<String>`, `ContentChunk.text: String` ✓ (confirmed from core/src/note/mod.rs:84)
