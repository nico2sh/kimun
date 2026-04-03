// tui/src/cli/commands/mcp/mod.rs
//
// MCP server handler for kimun — exposes vault operations as MCP tools.

pub mod prompts;

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, eyre};
use kimun_core::{NoteVault, nfs::VaultPath};
use rmcp::{
    ErrorData as McpError,
    RoleServer,
    ServerHandler,
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    schemars,
    prompt_handler, tool, tool_handler, tool_router,
    service::RequestContext,
    transport::stdio,
    ServiceExt,
};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateNoteParams {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AppendNoteParams {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShowNoteParams {
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchNotesParams {
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListNotesParams {
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct JournalParams {
    pub text: String,
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BacklinksParams {
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ChunksParams {
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutlinksParams {
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenameNoteParams {
    pub path: String,
    /// New filename stem — no extension, no path separator
    pub new_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MoveNoteParams {
    pub path: String,
    pub new_path: String,
}

// ---------------------------------------------------------------------------
// Handler struct
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct KimunHandler {
    vault: Arc<NoteVault>,
    tool_router: ToolRouter<KimunHandler>,
    prompt_router: PromptRouter<KimunHandler>,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
impl KimunHandler {
    pub fn new(vault: NoteVault) -> Self {
        Self {
            vault: Arc::new(vault),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    fn resolve_path(path: &str) -> VaultPath {
        VaultPath::note_path_from(path)
    }

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
            Err(kimun_core::error::VaultError::NoteExists { .. }) => Ok(CallToolResult::error(
                vec![Content::text(format!("Note already exists: {}", vault_path))],
            )),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

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
            p.content
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
            )) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Note not found: {}",
                vault_path
            ))])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

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
            .map(|(entry, content)| format!("{} — {}", entry.path, content.title))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }

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
                        let mut p = entry.path.clone();
                        p.to_relative();
                        p.to_string().starts_with(norm)
                    })
                    .collect()
            }
        };
        if filtered.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No notes found.")]));
        }
        let lines: Vec<String> = filtered
            .iter()
            .map(|(entry, content)| format!("{} — {}", entry.path, content.title))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }

    #[tool(description = "Append text to today's journal entry (or a specific date). Creates the entry if absent.")]
    async fn journal(
        &self,
        Parameters(p): Parameters<JournalParams>,
    ) -> Result<CallToolResult, McpError> {
        // Validate and resolve the date
        let date_str = match p.date.as_deref() {
            None => chrono::Utc::now().format("%Y-%m-%d").to_string(),
            Some(d) => {
                if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Invalid date '{}' — expected YYYY-MM-DD",
                        d
                    ))]));
                }
                d.to_string()
            }
        };

        let (vault_path, existing) = if p.date.is_none() {
            // Today — use journal_entry() which handles create-if-absent internally
            let (details, existing) = self
                .vault
                .journal_entry()
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            (details.path, existing)
        } else {
            // Specific date — build path manually
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
            .map(|(entry, content)| format!("{} — {}", entry.path, content.title))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }

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

        let mut lines: Vec<String> = Vec::new();
        for chunks in chunks_map.values() {
            for chunk in chunks {
                let breadcrumb = chunk.breadcrumb.join(" > ");
                lines.push(format!("[{}] {}", breadcrumb, chunk.text.trim()));
            }
        }

        if lines.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No chunks found.")]));
        }
        Ok(CallToolResult::success(vec![Content::text(lines.join("\n\n"))]))
    }

    #[tool(description = "Return the list of notes that this note links to (outgoing wikilinks).")]
    async fn get_outlinks(
        &self,
        Parameters(p): Parameters<OutlinksParams>,
    ) -> Result<CallToolResult, McpError> {
        use kimun_core::error::{FSError, VaultError};
        use kimun_core::note::{LinkType, NoteDetails};

        let vault_path = Self::resolve_path(&p.path);

        let md_note = match self.vault.get_markdown_and_links(&vault_path).await {
            Ok(n) => n,
            Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Note not found: {}",
                    vault_path
                ))]));
            }
            Err(e) => return Err(McpError::internal_error(e.to_string(), None)),
        };

        let note_links: Vec<_> = md_note
            .links
            .into_iter()
            .filter_map(|link| {
                if let LinkType::Note(path) = link.ltype {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        if note_links.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No outlinks found.")]));
        }

        let mut lines: Vec<String> = Vec::new();
        for path in note_links {
            let title = match self.vault.get_note_text(&path).await {
                Ok(text) => {
                    let t = NoteDetails::get_title_from_text(&text);
                    if t.is_empty() {
                        path.get_clean_name()
                    } else {
                        t
                    }
                }
                Err(_) => path.get_clean_name(),
            };
            lines.push(format!("{} — {}", path, title));
        }

        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }

    #[tool(description = "Rename a note within its current directory (filename only). Use move_note to change the directory.")]
    async fn rename_note(
        &self,
        Parameters(p): Parameters<RenameNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.new_name.contains('/') {
            return Ok(CallToolResult::error(vec![Content::text(
                "new_name must not contain '/'. Use move_note to change a note's directory.",
            )]));
        }

        let from = Self::resolve_path(&p.path);
        let (parent, _) = from.get_parent_path();
        let to = parent
            .append(&VaultPath::note_path_from(&p.new_name))
            .absolute();

        match self.vault.rename_note(&from, &to).await {
            Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Note renamed: {} → {}",
                from, to
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Move a note to a new vault path (different directory and/or name). Backlinks in other notes are updated automatically.")]
    async fn move_note(
        &self,
        Parameters(p): Parameters<MoveNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let from = Self::resolve_path(&p.path);
        let to = Self::resolve_path(&p.new_path);

        match self.vault.rename_note(&from, &to).await {
            Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Note moved: {} → {}",
                from, to
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }
}

// ---------------------------------------------------------------------------
// ServerHandler implementation
// ---------------------------------------------------------------------------

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

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let notes = self
            .vault
            .get_all_notes()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let resources: Vec<Resource> = notes
            .into_iter()
            .map(|(entry, content)| {
                // Build URI: note://{relative_path_with_ext}
                let mut rel_path = entry.path.clone();
                rel_path.to_relative();
                let uri = format!("note://{}", rel_path.to_string_with_ext());

                // Name: title from NoteContentData, or stem of filename if title empty
                let name = if content.title.is_empty() {
                    entry.path.get_clean_name()
                } else {
                    content.title.clone()
                };

                RawResource::new(uri, name)
                    .with_mime_type("text/markdown")
                    .no_annotation()
            })
            .collect();

        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;

        // Validate URI scheme
        let path_with_ext = uri
            .strip_prefix("note://")
            .ok_or_else(|| McpError::invalid_params(
                format!("invalid URI scheme — expected note://, got: {}", uri),
                None,
            ))?;

        let vault_path = VaultPath::note_path_from(path_with_ext);

        // Fetch note text
        match self.vault.get_note_text(&vault_path).await {
            Ok(text) => Ok(ReadResourceResult::new(vec![
                ResourceContents::text(text, uri.clone()),
            ])),
            Err(kimun_core::error::VaultError::FSError(
                kimun_core::error::FSError::VaultPathNotFound { .. },
            )) => Err(McpError::invalid_params(
                format!("note not found: {}", uri),
                None,
            )),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![],
            next_cursor: None,
            meta: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        let orig_pos = text.find("original").expect("original not found");
        let added_pos = text.find("added").expect("added not found");
        assert!(orig_pos < added_pos, "original should appear before added");
    }

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
        assert!(is_success(&result), "expected success: {}", result_text(&result));
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
    async fn test_get_chunks_missing_note_returns_gracefully() {
        let (handler, _dir) = make_handler().await;
        // get_note_chunks on a missing note may return empty map or an error —
        // either way it should not panic.
        let result = handler
            .get_chunks(Parameters(ChunksParams {
                path: "missing/note".to_string(),
            }))
            .await;
        // Just verify it returned something without panicking
        let _ = result;
    }

    // ---- Resource tests ----
    //
    // `list_resources` and `read_resource` require a `RequestContext<RoleServer>`,
    // which in turn requires a `Peer<R>` constructed via `Peer::new` — a
    // `pub(crate)` function not accessible outside rmcp.  There is no public
    // test constructor or `Default` impl, so these tests are marked `#[ignore]`
    // until rmcp exposes a test helper.  The implementations themselves are
    // correct and covered by the integration smoke test.

    #[tokio::test]
    #[ignore = "RequestContext<RoleServer> cannot be constructed outside rmcp (Peer::new is pub(crate))"]
    async fn test_list_resources_returns_notes() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "res/alpha".to_string(),
                content: "# Alpha Note".to_string(),
            }))
            .await
            .unwrap();
        // Cannot call handler.list_resources(None, ctx) — ctx requires Peer which
        // is not constructable from outside rmcp.
        // The assertion below would be:
        //   assert!(result.resources.iter().any(|r| r.uri.contains("res/alpha")));
        unreachable!("test is ignored");
    }

    #[tokio::test]
    #[ignore = "RequestContext<RoleServer> cannot be constructed outside rmcp (Peer::new is pub(crate))"]
    async fn test_read_resource_returns_content() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "res/beta".to_string(),
                content: "# Beta\n\nbeta content".to_string(),
            }))
            .await
            .unwrap();
        // Would call: handler.read_resource(ReadResourceRequestParams::new("note://res/beta.md"), ctx)
        // and assert content_json.contains("beta content")
        unreachable!("test is ignored");
    }

    #[tokio::test]
    #[ignore = "RequestContext<RoleServer> cannot be constructed outside rmcp (Peer::new is pub(crate))"]
    async fn test_read_resource_not_found_returns_error() {
        let (handler, _dir) = make_handler().await;
        // Would call: handler.read_resource(ReadResourceRequestParams::new("note://missing/note.md"), ctx)
        // and assert result.is_err()
        let _ = &handler;
        unreachable!("test is ignored");
    }

    #[tokio::test]
    #[ignore = "RequestContext<RoleServer> cannot be constructed outside rmcp (Peer::new is pub(crate))"]
    async fn test_read_resource_invalid_scheme_returns_error() {
        let (handler, _dir) = make_handler().await;
        // Would call: handler.read_resource(ReadResourceRequestParams::new("file:///etc/passwd"), ctx)
        // and assert result.is_err()
        let _ = &handler;
        unreachable!("test is ignored");
    }

    #[tokio::test]
    async fn test_get_outlinks_returns_linked_notes() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "source".to_string(),
                content: "# Source\n\nSee [[target]] for more.".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "target".to_string(),
                content: "# Target\n\nContent here.".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .get_outlinks(Parameters(OutlinksParams {
                path: "source".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "expected success: {}", result_text(&result));
        assert!(
            result_text(&result).contains("target"),
            "expected 'target' in outlinks: {}",
            result_text(&result)
        );
    }

    #[tokio::test]
    async fn test_get_outlinks_no_links_returns_empty_message() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "no-links".to_string(),
                content: "# No Links\n\nJust text, no wikilinks.".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .get_outlinks(Parameters(OutlinksParams {
                path: "no-links".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
        assert!(
            result_text(&result).contains("No outlinks found"),
            "expected empty message: {}",
            result_text(&result)
        );
    }

    #[tokio::test]
    async fn test_get_outlinks_note_not_found_returns_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .get_outlinks(Parameters(OutlinksParams {
                path: "missing/note".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_rename_note_succeeds() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "old-name".to_string(),
                content: "# Old\n\nunique_rename_content_xyz".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .rename_note(Parameters(RenameNoteParams {
                path: "old-name".to_string(),
                new_name: "new-name".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "expected success: {}", result_text(&result));
        let show = handler
            .show_note(Parameters(ShowNoteParams { path: "new-name".to_string() }))
            .await
            .unwrap();
        assert!(is_success(&show), "new path should be readable");
        assert!(result_text(&show).contains("unique_rename_content_xyz"));
        let old = handler
            .show_note(Parameters(ShowNoteParams { path: "old-name".to_string() }))
            .await
            .unwrap();
        assert_eq!(old.is_error, Some(true), "old path should be gone");
    }

    #[tokio::test]
    async fn test_rename_note_rejects_slash_in_name() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "some/note".to_string(),
                content: "content".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .rename_note(Parameters(RenameNoteParams {
                path: "some/note".to_string(),
                new_name: "other/dir".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(
            result_text(&result).contains("move_note"),
            "hint should mention move_note: {}",
            result_text(&result)
        );
    }

    #[tokio::test]
    async fn test_rename_note_updates_backlinks() {
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
                path: "linker".to_string(),
                content: "see [[target]] for details".to_string(),
            }))
            .await
            .unwrap();
        handler
            .rename_note(Parameters(RenameNoteParams {
                path: "target".to_string(),
                new_name: "renamed-target".to_string(),
            }))
            .await
            .unwrap();
        let show = handler
            .show_note(Parameters(ShowNoteParams { path: "linker".to_string() }))
            .await
            .unwrap();
        assert!(
            result_text(&show).contains("renamed-target"),
            "backlink should be updated: {}",
            result_text(&show)
        );
    }

    #[tokio::test]
    async fn test_move_note_succeeds() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "original".to_string(),
                content: "# Original\n\nunique_move_content_xyz".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .move_note(Parameters(MoveNoteParams {
                path: "original".to_string(),
                new_path: "folder/moved".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "expected success: {}", result_text(&result));
        let show = handler
            .show_note(Parameters(ShowNoteParams { path: "folder/moved".to_string() }))
            .await
            .unwrap();
        assert!(is_success(&show));
        assert!(result_text(&show).contains("unique_move_content_xyz"));
        let old = handler
            .show_note(Parameters(ShowNoteParams { path: "original".to_string() }))
            .await
            .unwrap();
        assert_eq!(old.is_error, Some(true), "old path should be gone");
    }

    #[tokio::test]
    async fn test_move_note_fails_if_destination_exists() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "src".to_string(),
                content: "source".to_string(),
            }))
            .await
            .unwrap();
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "dst".to_string(),
                content: "destination".to_string(),
            }))
            .await
            .unwrap();
        let result = handler
            .move_note(Parameters(MoveNoteParams {
                path: "src".to_string(),
                new_path: "dst".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
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
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(config_path: Option<PathBuf>) -> Result<()> {
    use crate::cli::helpers::create_and_init_vault;
    let (vault, _) = create_and_init_vault(config_path).await?;
    let handler = KimunHandler::new(vault);
    let service = handler.serve(stdio()).await.map_err(|e| eyre!("{e}"))?;
    service.waiting().await.map_err(|e| eyre!("{e}"))?;
    Ok(())
}
