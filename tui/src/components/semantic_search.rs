//! Semantic search surface (P4): a server-backed [`RowSource`] that queries the
//! RAG server for similar chunks and lists the matching notes. Lives behind a
//! drawer view; only usable when a server is configured and reachable.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_server_client::{ChunkResult, RagClient};

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, FileOp, InputEvent};
use crate::components::file_list::FileListEntry;
use crate::components::note_browser::format_journal_date;
use crate::components::query_list_panel::{ListPanelSpec, QueryListPanel};
use crate::components::search_list::{Emit, RowSource};
use crate::settings::SharedSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// Builds a [`RagClient`] for the current vault from config, or `None` when no
/// server URL is configured. Shared by every RAG query surface.
pub async fn rag_client(settings: &SharedSettings, vault: &NoteVault) -> Option<RagClient> {
    let (url, token) = {
        let settings = settings.read().ok()?;
        let global = &settings.workspace_config.as_ref()?.global;
        (
            global.kimun_server_url.clone()?,
            global.kimun_server_token.clone(),
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
                .and_then(|wc| wc.global.kimun_server_url.clone())
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
        // Debounce: the load engine aborts this task on the next keystroke, so a
        // short leading delay coalesces rapid typing into a single server request
        // (each query is one HTTP POST + a vault-id read).
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
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

/// Spec for the semantic drawer view: a query input over server-ranked note
/// results; Enter/click opens the note, right-click opens the file-ops menu.
pub struct SemanticSpec;

impl ListPanelSpec for SemanticSpec {
    type Row = FileListEntry;
    const TITLE: &'static str = "Semantic";
    const HAS_FILTER: bool = true;
    // Server-backed query: draw a bordered search box separated from the results
    // and a "Searching…" indicator while the request is in flight.
    const BORDERED_INPUT: bool = true;
    // The server already ranks/filters by the query; don't re-filter its
    // semantic results locally by the literal query text (that discards
    // conceptually-relevant notes whose titles lack the words).
    const LOCAL_FILTER: bool = false;

    fn submit(row: &FileListEntry, tx: &AppTx) {
        if let FileListEntry::Note { path, .. } = row {
            tx.send(AppEvent::open(path.clone())).ok();
        }
    }

    fn context_event(row: &FileListEntry) -> Option<AppEvent> {
        match row {
            FileListEntry::Note { path, .. } => {
                Some(AppEvent::FileOp(FileOp::ShowMenu(path.clone())))
            }
            _ => None,
        }
    }

    fn hints() -> Vec<(String, String)> {
        vec![("Enter".into(), "Open".into())]
    }
}

/// The SEMANTIC drawer view: type a query, see the notes the RAG server ranks
/// most similar. Only meaningful when a server is configured + reachable.
pub struct SemanticPanel {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    body: QueryListPanel<SemanticSpec>,
    source_installed: bool,
}

impl SemanticPanel {
    pub fn new(vault: Arc<NoteVault>, settings: SharedSettings, icons: Icons) -> Self {
        Self {
            vault,
            settings,
            body: QueryListPanel::new(icons),
            source_installed: false,
        }
    }

    /// Installs the server-backed source the first time the view is opened.
    pub fn ensure_source(&mut self, tx: &AppTx) {
        if !self.source_installed {
            self.body.set_source(
                SemanticSource::new(self.vault.clone(), self.settings.clone()),
                tx,
            );
            self.source_installed = true;
        }
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.body.hint_shortcuts()
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.body.handle_input(event, tx)
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.body.render(f, rect, theme, focused);
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
