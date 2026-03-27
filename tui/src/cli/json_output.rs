use chrono::Utc;
use kimun_core::nfs::NoteEntryData;
use kimun_core::note::NoteContentData;
use kimun_core::nfs::VaultPath;
use kimun_core::NoteVault;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::cli::metadata_extractor::{extract_tags, extract_links, extract_headers};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonHeader {
    pub level: u32,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonOutputMetadata {
    pub workspace: String,
    pub workspace_path: String,
    pub total_results: usize,
    pub query: Option<String>,
    pub is_listing: bool,
    pub generated_at: String,
}

/// Nested note-level metadata extracted from content
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonNoteMetadata {
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub headers: Vec<JsonHeader>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonNoteEntry {
    pub path: String,
    pub title: String,
    pub content: String,
    pub size: u64,
    pub modified: u64,
    pub created: u64,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_date: Option<String>,
    pub metadata: JsonNoteMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonOutput {
    pub metadata: JsonOutputMetadata,
    pub notes: Vec<JsonNoteEntry>,
}

/// Format note entries with their content as JSON output (async, supports vault operations)
pub async fn format_notes_with_content_as_json(
    vault: &NoteVault,
    entries: &[(NoteEntryData, NoteContentData)],
    content_map: &[(VaultPath, String)],
    workspace_name: &str,
    workspace_path: &str,
    query: Option<&str>,
    is_listing: bool,
    _include_backlinks: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    // Build a map for quick content lookup
    let content_lookup: HashMap<String, String> = content_map
        .iter()
        .map(|(path, content)| (path.to_string(), content.clone()))
        .collect();

    let output_metadata = JsonOutputMetadata {
        workspace: workspace_name.to_string(),
        workspace_path: workspace_path.to_string(),
        total_results: entries.len(),
        query: query.map(|q| q.to_string()),
        is_listing,
        generated_at: Utc::now().to_rfc3339(),
    };

    let notes = entries
        .iter()
        .map(|(entry_data, content_data)| {
            let path_str = entry_data.path.to_string();
            let path_with_ext = if path_str.ends_with(".md") {
                path_str.clone()
            } else {
                format!("{}.md", path_str)
            };

            // Get content from the map, or empty string if not found
            let content = content_lookup.get(&path_str).cloned().unwrap_or_default();

            let tags = extract_tags(&content);
            let links = extract_links(&content);
            let headers = extract_headers(&content);

            // Detect journal date using vault
            let journal_date = vault
                .journal_date(&entry_data.path)
                .map(|d| d.format("%Y-%m-%d").to_string());

            // Use modified as created (fallback until created timestamp is tracked separately)
            let created = entry_data.modified_secs;

            // TODO: implement backlinks when include_backlinks is true
            // For now, backlinks are not included regardless of the flag

            JsonNoteEntry {
                path: path_with_ext,
                title: content_data.title.clone(),
                content,
                size: entry_data.size,
                modified: entry_data.modified_secs,
                created,
                hash: format!("{:x}", content_data.hash),
                journal_date,
                metadata: JsonNoteMetadata {
                    tags,
                    links,
                    headers,
                },
            }
        })
        .collect();

    let output = JsonOutput { metadata: output_metadata, notes };
    Ok(serde_json::to_string(&output)?)
}

/// Format note entries as JSON output, fetching note content from vault
pub async fn format_notes_as_json(
    vault: &NoteVault,
    entries: &[(NoteEntryData, NoteContentData)],
    workspace_name: &str,
    query: Option<&str>,
    is_listing: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let workspace_path = vault.workspace_path.to_string_lossy().to_string();

    // Fetch actual content for each note to enable metadata extraction
    let mut content_map = Vec::new();
    for (entry_data, _) in entries {
        if let Ok(content) = vault.get_note_text(&entry_data.path).await {
            content_map.push((entry_data.path.clone(), content));
        }
    }

    format_notes_with_content_as_json(
        vault,
        entries,
        &content_map,
        workspace_name,
        &workspace_path,
        query,
        is_listing,
        false,
    )
    .await
}
