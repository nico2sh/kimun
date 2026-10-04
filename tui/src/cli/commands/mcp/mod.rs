// tui/src/cli/commands/mcp/mod.rs
//
// MCP server handler for kimun — exposes vault operations as MCP tools.

pub mod prompts;

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, eyre};
use kimun_core::note::{FrontmatterFormat, NoteMetadata, PropertyInput, PropertyKind};
use kimun_core::{NoteVault, nfs::VaultPath};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    prompt_handler, schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};

use crate::cli::json_output::JsonProperties;

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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QuickNoteParams {
    /// Text content for the quick note
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OverwriteNoteParams {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReplaceInNoteParams {
    pub path: String,
    /// Text to find
    pub old: String,
    /// Replacement text (may use $1/${name} capture references when regex is true)
    pub new: String,
    /// Replace every occurrence instead of requiring a unique match
    pub replace_all: Option<bool>,
    /// Treat `old` as a regular expression instead of a literal substring
    pub regex: Option<bool>,
    /// Preview the resulting content without writing the note (dry run)
    pub preview: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteNoteParams {
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPropertiesParams {
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetPropertyParams {
    pub path: String,
    /// Property key (case-insensitive; a new key is written as spelled)
    pub key: String,
    /// The value: a string (typed like the key's values in other notes, or by its look for a new key), a number, true/false, or an array of strings for a list
    pub value: serde_json::Value,
    /// Force a type: text, number, bool, date, datetime or list. Needed to store a value that doesn't fit the key's type elsewhere in the vault; only this note changes.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Frontmatter syntax for a note that has no frontmatter yet: "toml" (default) or "yaml". An existing block keeps its format.
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RemovePropertyParams {
    pub path: String,
    /// Property key (case-insensitive)
    pub key: String,
}

/// `get_properties` reply: properties in the note's order, plus its labels.
#[derive(Serialize)]
struct PropertiesReply {
    properties: JsonProperties,
    tags: Vec<String>,
}

/// A `set_property` JSON value as core's [`PropertyInput`]: a string is left
/// to core's typing rules; a number, boolean or array *implies* its type (still
/// checked against the key's type in the vault); `kind` forces one.
fn property_input(value: &serde_json::Value, kind: Option<&str>) -> Result<PropertyInput, String> {
    use serde_json::Value;
    // One JSON scalar as the text core types, with the type it implies.
    fn scalar(v: &Value) -> Option<(String, Option<PropertyKind>)> {
        match v {
            Value::String(s) => Some((s.clone(), None)),
            Value::Number(n) => Some((n.to_string(), Some(PropertyKind::Number))),
            Value::Bool(b) => Some((b.to_string(), Some(PropertyKind::Bool))),
            _ => None,
        }
    }
    let forced = kind.map(str::parse::<PropertyKind>).transpose()?;
    let input = match value {
        Value::Array(items) => items
            .iter()
            .map(|item| scalar(item).map(|(text, _)| text))
            .collect::<Option<Vec<_>>>()
            .map(|values| PropertyInput::new(values).implied(PropertyKind::List))
            .ok_or("list items must be strings, numbers or true/false")?,
        _ => match scalar(value) {
            Some((text, Some(implied))) => PropertyInput::new(vec![text]).implied(implied),
            Some((text, None)) => PropertyInput::new(vec![text]),
            None => {
                return Err("value must be a string, number, true/false or an array".to_string());
            }
        },
    };
    Ok(input.forced(forced))
}

// ---------------------------------------------------------------------------
// Handler struct
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct KimunHandler {
    vault: Arc<NoteVault>,
    // Read at runtime by the `#[tool_handler]` / `#[prompt_handler]` generated
    // impls (see below); clippy can't see through the macro expansion, so it
    // flags these as unread despite being the live tool/prompt dispatch tables.
    #[allow(dead_code)]
    tool_router: ToolRouter<KimunHandler>,
    #[allow(dead_code)]
    prompt_router: PromptRouter<KimunHandler>,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

/// Map a vault error to the result a note-operation handler returns: the core
/// user-facing message as a tool error the model can react to, or an internal
/// protocol error. The message and the recoverable/internal split both come from
/// core (`VaultError::user_message`), so the MCP server and the CLI render
/// identical wording and a new error variant flows automatically.
fn vault_err(e: kimun_core::error::VaultError) -> Result<CallToolResult, McpError> {
    match e.user_message() {
        Some(msg) => Ok(CallToolResult::error(vec![Content::text(msg)])),
        None => Err(McpError::internal_error(e.to_string(), None)),
    }
}

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

    #[tool(
        description = "Create a new note at the given vault path with the given markdown content. Fails if the note already exists."
    )]
    async fn create_note(
        &self,
        Parameters(p): Parameters<CreateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        match self.vault.create_note(&vault_path, &p.content).await {
            Ok(_) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Note created: {}",
                vault_path
            ))])),
            Err(e) => vault_err(e),
        }
    }

    #[tool(description = "Append text to an existing note. Creates the note if it does not exist.")]
    async fn append_note(
        &self,
        Parameters(p): Parameters<AppendNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        self.vault
            .append_to_note(&vault_path, &p.content, None)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Note saved: {}",
            vault_path
        ))]))
    }

    #[tool(
        description = "Replace a note's entire content with new markdown. The previous content is backed up first. Destructive.",
        annotations(destructive_hint = true)
    )]
    async fn overwrite_note(
        &self,
        Parameters(p): Parameters<OverwriteNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        if p.content.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "Refusing to overwrite with empty content (this would wipe the note); pass content, or use delete_note to remove it",
            )]));
        }
        match self.vault.save_note(&vault_path, &p.content).await {
            Ok(_) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Note saved: {}",
                vault_path
            ))])),
            Err(e) => vault_err(e),
        }
    }

    #[tool(
        description = "Replace text in a note. `old` is a literal substring by default; set regex=true to treat it as a regular expression, in which case `new` may reference capture groups ($1, ${name}; $$ for a literal $). The match must be unique unless replace_all is true. Set preview=true to get the resulting content back without writing (dry run). The previous content is backed up first. Destructive.",
        annotations(destructive_hint = true)
    )]
    async fn replace_in_note(
        &self,
        Parameters(p): Parameters<ReplaceInNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        let all = p.replace_all.unwrap_or(false);
        let regex = p.regex.unwrap_or(false);

        if p.preview.unwrap_or(false) {
            return match self
                .vault
                .preview_replace(&vault_path, &p.old, &p.new, all, regex)
                .await
            {
                Ok(pv) => Ok(CallToolResult::success(vec![Content::text(format!(
                    "{} occurrence(s) would be replaced in {} (preview — not written). Resulting content:\n\n{}",
                    pv.count, vault_path, pv.content
                ))])),
                Err(e) => vault_err(e),
            };
        }

        match self
            .vault
            .replace_in_note(&vault_path, &p.old, &p.new, all, regex)
            .await
        {
            Ok(n) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Replaced {} occurrence(s) in {}",
                n, vault_path
            ))])),
            Err(e) => vault_err(e),
        }
    }

    #[tool(
        description = "Delete a note. The content is backed up first. Destructive.",
        annotations(destructive_hint = true)
    )]
    async fn delete_note(
        &self,
        Parameters(p): Parameters<DeleteNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        match self.vault.delete_note(&vault_path).await {
            Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Note deleted: {}",
                vault_path
            ))])),
            Err(e) => vault_err(e),
        }
    }

    #[tool(
        description = "Return a note's frontmatter properties as JSON, in the note's order: {\"properties\": {\"status\": \"done\", \"priority\": 2, \"due\": \"2024-03-01\", \"tags\": [\"a\"]}, \"tags\": [\"a\", \"inline\"]}. `tags` is every label of the note — its inline #hashtags plus its frontmatter tags."
    )]
    async fn get_properties(
        &self,
        Parameters(p): Parameters<GetPropertiesParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        // Both fields from the one text read, so they can't disagree when the
        // index lags behind an outside edit.
        let meta = match self.vault.get_note_text(&vault_path).await {
            Ok(text) => NoteMetadata::of(&text),
            Err(e) => return vault_err(e),
        };
        let reply = PropertiesReply {
            properties: JsonProperties(meta.properties),
            tags: meta.tags,
        };
        let json = serde_json::to_string(&reply)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Set a frontmatter property on a note, keeping the rest of its frontmatter (format, comments, order) as is. `value` is a string, number, true/false, or an array of strings (a list). `tags` is always a list (comma-separated text becomes separate items; [] clears it); `aliases` and `cssclasses` are always lists too, values kept as given. Any other value must fit the type the key has in other notes — a string is read as that type (a single string for a list key becomes a one-item list), and a number, true/false or array must match it too; a value that doesn't fit is refused — pass `type` to store it anyway (only this note changes). For a key no other note has, a number/true/false/array keeps its JSON type and a string is typed by its look: 5 → number, true → true/false, 2024-03-01 → date, 2024-03-01T14:30 → date-time (local, or with its offset as written), else text; text that only looks numeric (02134, 1.10) stays text. A note without frontmatter gets a TOML (+++) block unless format is \"yaml\"."
    )]
    async fn set_property(
        &self,
        Parameters(p): Parameters<SetPropertyParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        let input = property_input(&p.value, p.kind.as_deref()).and_then(|input| {
            let format = p
                .format
                .as_deref()
                .map(str::parse::<FrontmatterFormat>)
                .transpose()?;
            Ok((input, format.unwrap_or_default()))
        });
        let (input, format) = match input {
            Ok(input) => input,
            Err(msg) => return Ok(CallToolResult::error(vec![Content::text(msg)])),
        };
        match self
            .vault
            .set_property_from_input(&vault_path, &p.key, &input, format)
            .await
        {
            Ok(value) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Set {} = {} ({}) in {}",
                p.key,
                value,
                value.kind(),
                vault_path
            ))])),
            Err(e) => vault_err(e),
        }
    }

    #[tool(
        description = "Remove a frontmatter property from a note (with any comment lines directly above it). Not an error when the note doesn't have it.",
        annotations(destructive_hint = true)
    )]
    async fn remove_property(
        &self,
        Parameters(p): Parameters<RemovePropertyParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        match self.vault.remove_property(&vault_path, &p.key).await {
            Ok(true) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Removed {} from {}",
                p.key, vault_path
            ))])),
            Ok(false) => Ok(CallToolResult::success(vec![Content::text(format!(
                "No property '{}' in {}",
                p.key, vault_path
            ))])),
            Err(e) => vault_err(e),
        }
    }

    #[tool(description = "Return the full markdown content of a note.")]
    async fn show_note(
        &self,
        Parameters(p): Parameters<ShowNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let vault_path = Self::resolve_path(&p.path);
        match self.vault.get_note_text(&vault_path).await {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(e) => vault_err(e),
        }
    }

    #[tool(
        description = "Search notes by query. Supports =name (or name:name) to match by note name, @heading (or in:heading), /path prefix, #label (or lb:label) for hashtag-derived labels, <note (or lk:note) for notes that link to the given note (its backlinks), >note (or fwd:note) for the notes the given note links to (its forward links), %key<op>value (or prop:key<op>value) for frontmatter properties with op one of = != < <= > >= (e.g. %status=done, %priority>=2, %due<2024-04-01; numbers compare numerically, dates chronologically, on a list = means contains; quote values or keys with spaces: %status=\"in progress\", %\"due date\"<2024-04-01; * is a wildcard with = and !=: %status=d*) or a bare %key for notes that have the property at all, ^%key (or or:prop:key; ^%\"due date\" for a key with spaces) to sort by a property (-^%key descending; notes without it last), and - prefix for exclusion (e.g. -term, -#label, -lb:label, -=name, -@heading, -/path, -<note, -lk:note, ->note, -fwd:note, -%key, -%key=value). The link filters match by note name (the .md extension is optional, case-insensitive); a bare name matches a linked note in any folder, a path like <dir/note disambiguates, and * wildcards are allowed (<proj*). Labels (#label) come from hashtags in note body text and from the frontmatter `tags` property (a list, or one string) — hashtags written inside frontmatter, fenced code blocks, inline code, HTML, markdown link bodies, and [[wikilinks]] are not indexed. Inline label names are ASCII [A-Za-z0-9_]+; all labels are matched case-insensitively. Long queries are truncated at 8 KB."
    )]
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
            return Ok(CallToolResult::success(vec![Content::text(
                "No results found.",
            )]));
        }
        let lines: Vec<String> = results
            .iter()
            .map(|(entry, content)| format!("{} — {}", entry.path, content.title))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
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
            return Ok(CallToolResult::success(vec![Content::text(
                "No notes found.",
            )]));
        }
        let lines: Vec<String> = filtered
            .iter()
            .map(|(entry, content)| format!("{} — {}", entry.path, content.title))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        description = "Append text to today's journal entry (or a specific date). Creates the entry if absent."
    )]
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

        // Both today and a specific date resolve to journal/<date>; append under
        // the per-note lock so concurrent journal writes can't lose an entry.
        let vault_path = self
            .vault
            .journal_path()
            .append(&VaultPath::note_path_from(&date_str))
            .absolute();
        self.vault
            .append_to_note(&vault_path, &p.text, Some(format!("# {}\n\n", date_str)))
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
            return Ok(CallToolResult::success(vec![Content::text(
                "No backlinks found.",
            )]));
        }
        let lines: Vec<String> = backlinks
            .iter()
            .map(|(entry, content)| format!("{} — {}", entry.path, content.title))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
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
                let breadcrumb = chunk
                    .breadcrumb
                    .replace(kimun_core::note::BREADCRUMB_SEP, " > ");
                lines.push(format!("[{}] {}", breadcrumb, chunk.text.trim()));
            }
        }

        if lines.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No chunks found.",
            )]));
        }
        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n\n"),
        )]))
    }

    #[tool(description = "Return the list of notes that this note links to (outgoing wikilinks).")]
    async fn get_outlinks(
        &self,
        Parameters(p): Parameters<OutlinksParams>,
    ) -> Result<CallToolResult, McpError> {
        use kimun_core::note::{LinkType, NoteDetails};

        let vault_path = Self::resolve_path(&p.path);

        let md_note = match self.vault.get_markdown_and_links(&vault_path).await {
            Ok(n) => n,
            Err(e) => return vault_err(e),
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
            return Ok(CallToolResult::success(vec![Content::text(
                "No outlinks found.",
            )]));
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

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        description = "Rename a note within its current directory (filename only). Use move_note to change the directory."
    )]
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
            Err(e) if e.is_user_error() => Ok(CallToolResult::error(vec![Content::text(format!(
                "Note not found or destination already exists: {} → {}",
                from, to
            ))])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(
        description = "Move a note to a new vault path (different directory and/or name). Backlinks in other notes are updated automatically."
    )]
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
            Err(e) if e.is_user_error() => Ok(CallToolResult::error(vec![Content::text(format!(
                "Note not found or destination already exists: {} → {}",
                from, to
            ))])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(
        description = "Quickly capture a thought into a timestamped note in the inbox directory. Returns the path of the created note."
    )]
    async fn quick_note(
        &self,
        Parameters(p): Parameters<QuickNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.content.trim().is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "Content cannot be empty.",
            )]));
        }
        match self.vault.quick_note(&p.content).await {
            Ok(details) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Note saved: {}",
                details.path
            ))])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
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
        .with_instructions(
            "Kimun notes MCP server — read and write vault notes via tools. \
             Search, listing, backlinks, and labels are served from an index that \
             these tools keep in sync automatically. If vault files are modified \
             outside Kimün (e.g. edited directly with sed, another editor, or a sync \
             tool), the index goes stale and results may be wrong until the workspace \
             is reindexed — run `kimun workspace reindex` and reconnect to this server.",
        )
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
        let path_with_ext = uri.strip_prefix("note://").ok_or_else(|| {
            McpError::invalid_params(
                format!("invalid URI scheme — expected note://, got: {}", uri),
                None,
            )
        })?;

        let vault_path = VaultPath::note_path_from(path_with_ext);

        // Fetch note text
        match self.vault.get_note_text(&vault_path).await {
            Ok(text) => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                text,
                uri.clone(),
            )])),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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

    async fn set_prop(
        handler: &KimunHandler,
        path: &str,
        key: &str,
        value: serde_json::Value,
        kind: Option<&str>,
    ) -> CallToolResult {
        handler
            .set_property(Parameters(SetPropertyParams {
                path: path.to_string(),
                key: key.to_string(),
                value,
                kind: kind.map(str::to_string),
                format: None,
            }))
            .await
            .unwrap()
    }

    async fn properties_json(handler: &KimunHandler, path: &str) -> serde_json::Value {
        let result = handler
            .get_properties(Parameters(GetPropertiesParams {
                path: path.to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "{}", result_text(&result));
        let text = match &result.content[0].raw {
            RawContent::Text(t) => t.text.clone(),
            other => panic!("expected text content, got {other:?}"),
        };
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn json_values_imply_their_type_and_type_forces_it() {
        use serde_json::json;
        let text = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            property_input(&json!("5"), None),
            Ok(PropertyInput::new(text(&["5"])))
        );
        assert_eq!(
            property_input(&json!(5), None),
            Ok(PropertyInput::new(text(&["5"])).implied(PropertyKind::Number))
        );
        assert_eq!(
            property_input(&json!(true), None),
            Ok(PropertyInput::new(text(&["true"])).implied(PropertyKind::Bool))
        );
        assert_eq!(
            property_input(&json!(["a", 2]), None),
            Ok(PropertyInput::new(text(&["a", "2"])).implied(PropertyKind::List))
        );
        assert_eq!(
            property_input(&json!([]), None),
            Ok(PropertyInput::new(vec![]).implied(PropertyKind::List))
        );
        assert_eq!(
            property_input(&json!(2024), Some("text")),
            Ok(PropertyInput::new(text(&["2024"]))
                .implied(PropertyKind::Number)
                .forced(Some(PropertyKind::Text)))
        );
        assert!(property_input(&json!({"a": 1}), None).is_err());
        assert!(property_input(&json!([["nested"]]), None).is_err());
        assert!(property_input(&json!("x"), Some("float")).is_err());
    }

    #[tokio::test]
    async fn test_property_tools_round_trip() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "garden".to_string(),
                content: "Garden #outdoor".to_string(),
            }))
            .await
            .unwrap();
        for (key, value) in [
            ("status", serde_json::json!("active")),
            ("priority", serde_json::json!(2)),
            ("due", serde_json::json!("2026-05-01")),
            ("tags", serde_json::json!(["garden", "spring"])),
        ] {
            let result = set_prop(&handler, "garden", key, value, None).await;
            assert!(is_success(&result), "{}", result_text(&result));
        }
        assert_eq!(
            properties_json(&handler, "garden").await,
            serde_json::json!({
                "properties": {
                    "status": "active",
                    "priority": 2,
                    "due": "2026-05-01",
                    "tags": ["garden", "spring"],
                },
                "tags": ["garden", "outdoor", "spring"],
            })
        );

        let removed = handler
            .remove_property(Parameters(RemovePropertyParams {
                path: "garden".to_string(),
                key: "Status".to_string(),
            }))
            .await
            .unwrap();
        assert!(result_text(&removed).contains("Removed"));
        let again = handler
            .remove_property(Parameters(RemovePropertyParams {
                path: "garden".to_string(),
                key: "status".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&again) && result_text(&again).contains("No property"));
    }

    #[tokio::test]
    async fn test_set_property_refuses_vault_type_mismatch() {
        let (handler, _dir) = make_handler().await;
        for path in ["a", "b", "c"] {
            handler
                .create_note(Parameters(CreateNoteParams {
                    path: path.to_string(),
                    content: "body".to_string(),
                }))
                .await
                .unwrap();
        }
        set_prop(&handler, "a", "priority", serde_json::json!(1), None).await;
        let refused = set_prop(&handler, "b", "priority", serde_json::json!("high"), None).await;
        assert_eq!(refused.is_error, Some(true));
        assert!(result_text(&refused).contains("holds number values"));
        let typed = set_prop(
            &handler,
            "b",
            "priority",
            serde_json::json!("high"),
            Some("text"),
        )
        .await;
        assert!(is_success(&typed), "{}", result_text(&typed));
        let bad_type = set_prop(&handler, "b", "x", serde_json::json!("1"), Some("float")).await;
        assert_eq!(bad_type.is_error, Some(true));
        // `priority` is tied number/text by now; `rating` is clearly a number.
        set_prop(&handler, "a", "rating", serde_json::json!(4), None).await;
        let json_bool = set_prop(&handler, "c", "rating", serde_json::json!(true), None).await;
        assert_eq!(
            json_bool.is_error,
            Some(true),
            "a JSON value's own type is still checked against the vault"
        );
        assert!(
            result_text(&json_bool).contains("holds number values"),
            "refused for its type, not for a missing note: {}",
            result_text(&json_bool)
        );
        let cleared = set_prop(&handler, "b", "tags", serde_json::json!([]), None).await;
        assert!(is_success(&cleared), "{}", result_text(&cleared));
    }

    #[tokio::test]
    async fn test_get_properties_missing_note_is_tool_error() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .get_properties(Parameters(GetPropertiesParams {
                path: "nope".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
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
        assert!(
            is_success(&result),
            "expected success, got: {:?}",
            result_text(&result)
        );
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
    async fn test_overwrite_note_replaces_whole_body() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "n".to_string(),
                content: "old body".to_string(),
            }))
            .await
            .unwrap();

        let result = handler
            .overwrite_note(Parameters(OverwriteNoteParams {
                path: "n".to_string(),
                content: "new body".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "got: {:?}", result_text(&result));

        let shown = handler
            .show_note(Parameters(ShowNoteParams {
                path: "n".to_string(),
            }))
            .await
            .unwrap();
        assert!(result_text(&shown).contains("new body"));
        assert!(!result_text(&shown).contains("old body"));
    }

    #[tokio::test]
    async fn test_replace_in_note_unique_match() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "n".to_string(),
                content: "hello world".to_string(),
            }))
            .await
            .unwrap();

        let result = handler
            .replace_in_note(Parameters(ReplaceInNoteParams {
                path: "n".to_string(),
                old: "world".to_string(),
                new: "there".to_string(),
                replace_all: None,
                regex: None,
                preview: None,
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "got: {:?}", result_text(&result));

        let shown = handler
            .show_note(Parameters(ShowNoteParams {
                path: "n".to_string(),
            }))
            .await
            .unwrap();
        assert!(result_text(&shown).contains("hello there"));
    }

    #[tokio::test]
    async fn test_replace_in_note_non_unique_is_error() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "n".to_string(),
                content: "a a".to_string(),
            }))
            .await
            .unwrap();

        let result = handler
            .replace_in_note(Parameters(ReplaceInNoteParams {
                path: "n".to_string(),
                old: "a".to_string(),
                new: "b".to_string(),
                replace_all: None,
                regex: None,
                preview: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_delete_note_removes_it() {
        let (handler, _dir) = make_handler().await;
        handler
            .create_note(Parameters(CreateNoteParams {
                path: "n".to_string(),
                content: "x".to_string(),
            }))
            .await
            .unwrap();

        let result = handler
            .delete_note(Parameters(DeleteNoteParams {
                path: "n".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result), "got: {:?}", result_text(&result));

        let shown = handler
            .show_note(Parameters(ShowNoteParams {
                path: "n".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(shown.is_error, Some(true));
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
            .show_note(Parameters(ShowNoteParams {
                path: "show/me".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&result));
        assert!(result_text(&result).contains("some content"));
    }

    #[tokio::test]
    async fn test_show_note_not_found_returns_error_result() {
        let (handler, _dir) = make_handler().await;
        let result = handler
            .show_note(Parameters(ShowNoteParams {
                path: "missing/note".to_string(),
            }))
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
            .show_note(Parameters(ShowNoteParams {
                path: "new/note".to_string(),
            }))
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
            .show_note(Parameters(ShowNoteParams {
                path: "exist/note".to_string(),
            }))
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
        assert!(
            is_success(&result),
            "expected success: {}",
            result_text(&result)
        );
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
        assert!(
            is_success(&result),
            "expected success: {}",
            result_text(&result)
        );
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
        assert!(
            is_success(&result),
            "expected success: {}",
            result_text(&result)
        );
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
                content: "# Title\n\n## Section One\n\nparagraph\n\n## Section Two\n\nmore"
                    .to_string(),
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
        assert!(
            is_success(&result),
            "expected success: {}",
            result_text(&result)
        );
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
        assert!(
            is_success(&result),
            "expected success: {}",
            result_text(&result)
        );
        let show = handler
            .show_note(Parameters(ShowNoteParams {
                path: "new-name".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&show), "new path should be readable");
        assert!(result_text(&show).contains("unique_rename_content_xyz"));
        let old = handler
            .show_note(Parameters(ShowNoteParams {
                path: "old-name".to_string(),
            }))
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
            .show_note(Parameters(ShowNoteParams {
                path: "linker".to_string(),
            }))
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
        assert!(
            is_success(&result),
            "expected success: {}",
            result_text(&result)
        );
        let show = handler
            .show_note(Parameters(ShowNoteParams {
                path: "folder/moved".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_success(&show));
        assert!(result_text(&show).contains("unique_move_content_xyz"));
        let old = handler
            .show_note(Parameters(ShowNoteParams {
                path: "original".to_string(),
            }))
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
        assert!(
            text.contains("projects/foo"),
            "missing projects/foo: {}",
            text
        );
        assert!(
            !text.contains("journal/2026"),
            "should not include journal: {}",
            text
        );
    }
}
