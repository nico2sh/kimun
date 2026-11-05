//! SEM: the notes the Kimün server ranks most similar to a query. One row per
//! note (a [`FileListEntry::Note`]), in the server's order.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use crate::logic::files::{FileListEntry, format_journal_date};
use crate::logic::list::{Emit, RowSource};
use crate::rag::rag_client;
use crate::server_client::{ChunkResult, RagClient};
use crate::settings::SharedSettings;

/// How long a load waits before asking the server, so rapid typing coalesces
/// into one request (the list engine aborts the load on the next keystroke).
const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

/// One note row per unique note among the server's chunk results, in result
/// order (best first), deduplicated by canonical path. A row's title is the
/// best chunk's title (the matched section's breadcrumb), `<no title>` when
/// blank; it opens its note.
pub fn chunks_to_entries(chunks: Vec<ChunkResult>, vault: &NoteVault) -> Vec<FileListEntry> {
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

/// Where the source gets its client for each load.
enum Client {
    /// Built from config per load (`crate::rag::rag_client`), so a config
    /// change is picked up; none configured → no rows.
    Settings(SharedSettings),
    /// A fixed client (a test's stub, or a host that already holds one).
    Fixed(Arc<RagClient>),
}

/// Server-backed semantic search source. An empty query yields no rows; a
/// server error yields no rows too (the status indicator tells the user the
/// server is unreachable).
pub struct SemanticSource {
    vault: Arc<NoteVault>,
    client: Client,
}

impl SemanticSource {
    /// The app's source: the client comes from config on every load.
    pub fn new(vault: Arc<NoteVault>, settings: SharedSettings) -> Self {
        Self {
            vault,
            client: Client::Settings(settings),
        }
    }

    /// A source over a fixed client — e.g. `RagClient::with_transport(stub)`.
    pub fn with_client(vault: Arc<NoteVault>, client: Arc<RagClient>) -> Self {
        Self {
            vault,
            client: Client::Fixed(client),
        }
    }

    async fn client(&self) -> Option<Arc<RagClient>> {
        match &self.client {
            Client::Settings(settings) => rag_client(settings, &self.vault).await.map(Arc::new),
            Client::Fixed(client) => Some(client.clone()),
        }
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
        tokio::time::sleep(DEBOUNCE).await;
        let entries = match self.client().await {
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
