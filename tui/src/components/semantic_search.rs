//! Semantic search surface (P4): a server-backed [`RowSource`] that queries the
//! RAG server for similar chunks and lists the matching notes. Lives behind a
//! drawer view; only usable when a server is configured and reachable.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_rag_client::{ChunkResult, RagClient};

use crate::components::file_list::FileListEntry;
use crate::components::note_browser::format_journal_date;
use crate::components::search_list::{Emit, RowSource};
use crate::settings::SharedSettings;

/// Builds a [`RagClient`] for the current vault from config, or `None` when no
/// server URL is configured. Shared by every RAG query surface.
pub async fn rag_client(settings: &SharedSettings, vault: &NoteVault) -> Option<RagClient> {
    let (url, token) = {
        let settings = settings.read().ok()?;
        let global = &settings.workspace_config.as_ref()?.global;
        (
            global.rag_server_url.clone()?,
            global.rag_server_token.clone(),
        )
    };
    let vault_id = vault.vault_id().await.ok()?;
    Some(RagClient::new(url, token, vault_id.to_string()))
}

/// Whether a RAG server is configured (drives showing the semantic surface at
/// all). Reachability is a separate, runtime concern.
pub fn rag_configured(settings: &SharedSettings) -> bool {
    settings
        .read()
        .ok()
        .and_then(|s| {
            s.workspace_config
                .as_ref()
                .and_then(|wc| wc.global.rag_server_url.clone())
        })
        .is_some()
}

/// One note row per unique note among the server's chunk results, in result
/// order (best first), deduplicated by canonical path.
fn chunks_to_entries(chunks: Vec<ChunkResult>, vault: &NoteVault) -> Vec<FileListEntry> {
    let mut seen: HashSet<VaultPath> = HashSet::new();
    let mut out = Vec::new();
    for chunk in chunks {
        let path = VaultPath::new(&chunk.path);
        // Dedup by canonical identity; keep the first (highest-ranked) chunk.
        if !seen.insert(path.flatten().absolute()) {
            continue;
        }
        let filename = path.get_parent_path().1;
        // The chunk title is the matched section's breadcrumb — informative for
        // "which part matched".
        let title = if chunk.title.trim().is_empty() {
            "<no title>".to_string()
        } else {
            chunk.title
        };
        let journal_date = vault.journal_date(&path).map(format_journal_date);
        out.push(FileListEntry::Note {
            path,
            title,
            filename,
            journal_date,
            is_open: false,
        });
    }
    out
}

/// Server-backed semantic search source.
pub struct SemanticSource {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
}

impl SemanticSource {
    pub fn new(vault: Arc<NoteVault>, settings: SharedSettings) -> Self {
        Self { vault, settings }
    }
}

#[async_trait]
impl RowSource<FileListEntry> for SemanticSource {
    async fn load(&self, query: &str, emit: Emit<FileListEntry>) {
        if query.trim().is_empty() {
            emit.replace(Vec::new());
            return;
        }
        let entries = match rag_client(&self.settings, &self.vault).await {
            Some(client) => match client.search(query, None).await {
                Ok(chunks) => chunks_to_entries(chunks, &self.vault),
                // Offline / server error → no rows (the surface shows empty; the
                // status indicator tells the user the server is unreachable).
                Err(e) => {
                    log::debug!("semantic search failed: {e}");
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        emit.replace(entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::VaultConfig;
    use tempfile::TempDir;

    fn chunk(path: &str, title: &str, score: f64) -> ChunkResult {
        ChunkResult {
            path: path.to_string(),
            title: title.to_string(),
            date: None,
            content: String::new(),
            hash: String::new(),
            similarity_score: score,
        }
    }

    #[tokio::test]
    async fn dedups_by_note_keeping_first_and_preserves_order() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();

        // Two chunks of "/a.md" (best first) then one of "/b.md".
        let chunks = vec![
            chunk("/a.md", "A > Intro", 0.9),
            chunk("/a.md", "A > Details", 0.7),
            chunk("/b.md", "B", 0.5),
        ];
        let entries = chunks_to_entries(chunks, &vault);
        assert_eq!(entries.len(), 2);
        match (&entries[0], &entries[1]) {
            (
                FileListEntry::Note {
                    path: pa,
                    title: ta,
                    ..
                },
                FileListEntry::Note { path: pb, .. },
            ) => {
                assert_eq!(pa.to_string(), "/a.md");
                assert_eq!(ta, "A > Intro"); // the higher-ranked chunk won
                assert_eq!(pb.to_string(), "/b.md");
            }
            _ => panic!("expected note entries"),
        }
    }
}
