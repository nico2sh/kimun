use chrono::Utc;
use kimun_core::nfs::NoteEntryData;
use kimun_core::note::NoteContentData;
use kimun_core::nfs::VaultPath;
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

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonNoteEntry {
    pub path: String,
    pub title: String,
    pub content: String,
    pub size: u64,
    pub modified: u64,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<JsonHeader>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonOutput {
    pub metadata: JsonOutputMetadata,
    pub notes: Vec<JsonNoteEntry>,
}

/// Format note entries with their content as JSON output
pub fn format_notes_with_content_as_json(
    entries: &[(NoteEntryData, NoteContentData)],
    content_map: &[(VaultPath, String)],
    workspace_name: &str,
    workspace_path: &str,
    query: Option<&str>,
    is_listing: bool,
) -> Result<String, serde_json::Error> {
    // Build a map for quick content lookup
    let content_lookup: HashMap<String, String> = content_map
        .iter()
        .map(|(path, content)| (path.to_string(), content.clone()))
        .collect();

    let metadata = JsonOutputMetadata {
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

            JsonNoteEntry {
                path: path_with_ext,
                title: content_data.title.clone(),
                content,
                size: entry_data.size,
                modified: entry_data.modified_secs,
                hash: format!("{:x}", content_data.hash),
                tags: Some(tags),
                links: Some(links),
                headers: Some(headers),
            }
        })
        .collect();

    let output = JsonOutput { metadata, notes };
    serde_json::to_string(&output)
}

/// Format note entries as JSON output (without content)
pub fn format_notes_as_json(
    entries: &[(NoteEntryData, NoteContentData)],
    workspace_name: &str,
    workspace_path: &str,
    query: Option<&str>,
    is_listing: bool,
) -> Result<String, serde_json::Error> {
    let content_map = vec![];
    format_notes_with_content_as_json(entries, &content_map, workspace_name, workspace_path, query, is_listing)
}
