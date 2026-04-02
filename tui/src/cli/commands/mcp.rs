// tui/src/cli/commands/mcp.rs
//
// MCP server handler for kimun — exposes vault operations as MCP tools.

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, eyre};
use kimun_core::{NoteVault, nfs::VaultPath};
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

// ---------------------------------------------------------------------------
// Handler struct
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct KimunHandler {
    vault: Arc<NoteVault>,
    tool_router: ToolRouter<KimunHandler>,
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
        let _ = p;
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "List all notes in the vault, optionally filtered by path prefix.")]
    async fn list_notes(
        &self,
        Parameters(p): Parameters<ListNotesParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = p;
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Append text to today's journal entry (or a specific date). Creates the entry if absent.")]
    async fn journal(
        &self,
        Parameters(p): Parameters<JournalParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = p;
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Return the list of notes that link to the given note (backlinks).")]
    async fn get_backlinks(
        &self,
        Parameters(p): Parameters<BacklinksParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = p;
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Return the content chunks (sections) of a note as JSON.")]
    async fn get_chunks(
        &self,
        Parameters(p): Parameters<ChunksParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = p;
        Err(McpError::internal_error("not yet implemented", None))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler implementation
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for KimunHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Kimun notes MCP server — read and write vault notes via tools.")
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        _request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        Err(McpError::internal_error("not yet implemented", None))
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
