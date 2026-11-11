//! SEM: the notes the Kimün server ranks most similar to a query. One row per
//! note, in the server's order: a [`FileListEntry::Note`] (the TUI's rows),
//! or a [`SemanticHit`] that keeps the matched chunk (the GUI's preview).

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use crate::ask::AskSource;
use crate::logic::files::{FileListEntry, format_journal_date};
use crate::logic::list::{Emit, ListRow, RowSource, YankTarget};
use crate::rag::rag_client;
use crate::server_client::{ChunkResult, RagClient};
use crate::settings::SharedSettings;

/// How long a load waits before asking the server, so rapid typing coalesces
/// into one request (the list engine aborts the load on the next keystroke).
const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

/// One SEM result: a note's row (FILES' [`FileListEntry::Note`]) and the
/// chunk the server matched in it, so a preview can mark that section.
#[derive(Debug, Clone)]
pub struct SemanticHit {
    pub entry: FileListEntry,
    /// The matched section's innermost heading: what locates it in the note.
    pub heading: String,
    /// The matched chunk's text.
    pub chunk: String,
    /// The chunk's 1-based ordinal: the server's, else its position.
    pub ordinal: usize,
    /// The server's similarity, `0.0..=1.0`.
    pub score: f64,
}

impl ListRow for SemanticHit {
    fn match_text(&self) -> Option<&str> {
        self.entry.match_text()
    }

    fn yank_target(&self) -> Option<YankTarget> {
        self.entry.yank_target()
    }
}

/// One hit per unique note among the server's chunk results, in result
/// order (best first), deduplicated by canonical path: a note keeps its
/// first (best) chunk. A hit's title is that chunk's title (the matched
/// section's breadcrumb), `<no title>` when blank; it opens its note. An
/// absent ordinal (`0`) becomes the chunk's 1-based position in `chunks`.
pub fn chunks_to_hits(chunks: Vec<ChunkResult>, vault: &NoteVault) -> Vec<SemanticHit> {
    let mut seen: HashSet<VaultPath> = HashSet::new();
    let mut out = Vec::new();
    for (position, chunk) in chunks.into_iter().enumerate() {
        let path = VaultPath::new(&chunk.path);
        // Dedup by canonical identity; keep the first (highest-ranked) chunk.
        if !seen.insert(path.flatten().absolute()) {
            continue;
        }
        let filename = path.get_parent_path().1;
        // The chunk title is the matched section's breadcrumb — informative for
        // "which part matched".
        let title = crate::ask::readable_heading(&chunk.title);
        let title = if title.trim().is_empty() {
            "<no title>".to_string()
        } else {
            title
        };
        let journal_date = vault.journal_date(&path).map(format_journal_date);
        // The Source reader's reading of a chunk: the heading without its
        // journal date, the ordinal normalized.
        let source = AskSource::from_chunk(position, chunk);
        out.push(SemanticHit {
            entry: FileListEntry::Note {
                path,
                title,
                filename,
                journal_date,
                is_open: false,
            },
            heading: source.match_heading().to_string(),
            chunk: source.text,
            ordinal: source.ordinal,
            score: source.score,
        });
    }
    out
}

/// The note rows of [`chunks_to_hits`] alone (the TUI's SEM rows).
pub fn chunks_to_entries(chunks: Vec<ChunkResult>, vault: &NoteVault) -> Vec<FileListEntry> {
    chunks_to_hits(chunks, vault)
        .into_iter()
        .map(|hit| hit.entry)
        .collect()
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
/// server is unreachable). It lists [`FileListEntry`] rows (the TUI) or
/// [`SemanticHit`] rows (the GUI), from the same search.
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

    /// The hits for `query`: none for a blank query (the server is not
    /// asked) or a failed search.
    async fn hits(&self, query: &str) -> Vec<SemanticHit> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        // Debounce: the load engine aborts this task on the next keystroke, so a
        // short leading delay coalesces rapid typing into a single server request
        // (each query is one HTTP POST + a vault-id read).
        tokio::time::sleep(DEBOUNCE).await;
        match self.client().await {
            Some(client) => match client.search(query, None).await {
                Ok(chunks) => chunks_to_hits(chunks, &self.vault),
                // Offline / server error → no rows (the surface shows empty; the
                // status indicator tells the user the server is unreachable).
                Err(e) => {
                    log::debug!("semantic search failed: {e}");
                    Vec::new()
                }
            },
            None => Vec::new(),
        }
    }
}

#[async_trait]
impl RowSource<FileListEntry> for SemanticSource {
    async fn load(&self, query: &str, emit: Emit<FileListEntry>) {
        let entries = self.hits(query).await.into_iter().map(|hit| hit.entry);
        emit.replace(entries.collect());
    }
}

#[async_trait]
impl RowSource<SemanticHit> for SemanticSource {
    async fn load(&self, query: &str, emit: Emit<SemanticHit>) {
        emit.replace(self.hits(query).await);
    }
}
