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
        let _ = (p, Self::resolve_path);
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Append text to an existing note. Creates the note if it does not exist.")]
    async fn append_note(
        &self,
        Parameters(p): Parameters<AppendNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = p;
        Err(McpError::internal_error("not yet implemented", None))
    }

    #[tool(description = "Return the full markdown content of a note.")]
    async fn show_note(
        &self,
        Parameters(p): Parameters<ShowNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = p;
        Err(McpError::internal_error("not yet implemented", None))
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
