use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;

use dbembeddings::embedder::Embedder;
use dbembeddings::{CollectionInfo, EmbeddedChunk, VectorStore};
use llmclients::LLMClient;
use log::debug;

use crate::config::ContextCut;
use crate::document::{FlattenedChunk, KimunDoc};

// Re-export commonly used types and functions
use crate::reranker::Reranker;
pub use document::{KimunSection, split_chunks_for_rag};

pub mod dbembeddings;
pub mod document;
pub mod llmclients;
pub mod reranker;

// Public modules for server
pub mod auth;
pub mod config;
pub mod handlers;
pub mod logbuffer;
pub mod server_state;
pub mod webui;

/// A retrieved chunk with its relevance score (higher = better).
pub type ScoredChunk = (f64, FlattenedChunk);

/// A validated collection key — the **Vault ID** as the server accepts it
/// (adr/0020). Constructible only through [`parse`](Self::parse), so holding
/// one *is* the proof of validation: every [`KimunRag`] operation demands a
/// `&CollectionKey`, and a request path that skipped validation does not
/// compile rather than serving an unchecked collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionKey(String);

impl CollectionKey {
    /// Validates a raw vault id. Rejects blank ids (which would cross-mix every
    /// blank-id vault), any id with characters outside `[A-Za-z0-9._-]`, and
    /// ids starting with `__` — that prefix is reserved for server metadata
    /// collections (the embedder fingerprint, adr/0025) and is excluded from
    /// listings and the fingerprint wipe. So a key stays a safe, non-colliding
    /// collection-name segment (Kimün always sends a UUID; adr/0020).
    pub fn parse(vault_id: &str) -> Result<Self, RagError> {
        let ok = !vault_id.is_empty()
            && !vault_id.starts_with("__")
            && vault_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if ok {
            Ok(Self(vault_id.to_string()))
        } else {
            Err(RagError::Validation(
                "vault_id must be non-empty, must not start with '__' (reserved), and contain only [A-Za-z0-9._-]"
                    .to_string(),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CollectionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The answer to a question, with the chunks the LLM saw as context.
pub struct Answer {
    pub text: String,
    pub sources: Vec<ScoredChunk>,
}

/// The server's domain error: every way a request can fail, each variant
/// carrying its meaning rather than a status code. The HTTP mapping lives in
/// one place (`IntoResponse` in the handlers module): `Validation` → 400,
/// `NotFound` → 404, `SemanticOnly`/`Unconfigured` → 503 (adr/0022, adr/0024),
/// `Backend` → 500.
#[derive(Debug)]
pub enum RagError {
    /// The request itself is malformed (bad vault id, unparseable job id).
    Validation(String),
    /// The named thing doesn't exist (e.g. an expired or unknown job).
    NotFound(String),
    /// No LLM configured — this server answers semantic searches only
    /// (adr/0022).
    SemanticOnly,
    /// No embedder configured — the server is *unconfigured*: no vector store,
    /// no indexing, no search, no answering (adr/0024). Distinct from
    /// [`SemanticOnly`](Self::SemanticOnly) (embedder present, LLM absent) so
    /// the two states never blur.
    Unconfigured,
    Backend(anyhow::Error),
}

impl Display for RagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RagError::Validation(msg) => write!(f, "{msg}"),
            RagError::NotFound(msg) => write!(f, "{msg}"),
            RagError::SemanticOnly => {
                write!(f, "no LLM configured; this server is semantic-only")
            }
            RagError::Unconfigured => {
                write!(
                    f,
                    "no embedder configured; this server is unconfigured — open the web UI to configure one"
                )
            }
            RagError::Backend(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RagError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RagError::Backend(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for RagError {
    fn from(e: anyhow::Error) -> Self {
        RagError::Backend(e)
    }
}

// ── Pipeline policy ─────────────────────────────────────────────────────────
// The one home of the retrieval/indexing numbers. Stores never see these.

/// Preferred sub-chunk size in chars (~200 tokens) when splitting a section to
/// the embedding window.
const CHUNK_TARGET: usize = 800;
/// Hard cap per sub-chunk in chars before a split is forced.
const CHUNK_MAX: usize = 1536;
/// Chunks embedded (and stored) per batch during indexing.
const EMBED_BATCH: usize = 100;
/// Candidate pool pulled from the vector store per query, before dedup and
/// reranking cut it down.
const CANDIDATE_POOL: usize = 80;
/// No-reranker `answer` context: a chunk makes the LLM context when its
/// min-max-normalized score — `(s - s_min) / (s_max - s_min)` over the query's
/// pool — is at least this. Normalized (not an absolute cutoff) because
/// absolute cosine bands are embedder-specific; within one query all pool
/// scores share a scale, so the pool's own spread defines what "relevant"
/// looks like.
const NO_RERANK_NORM_CUTOFF: f64 = 0.5;
/// [`ContextCut::LargestDrop`]'s search window: the biggest score drop is
/// looked for at cut positions 3..=30 only. Below 3 a spiky top would starve
/// the context; past 30 the tail's noise produces phantom elbows. A window,
/// not a keep-clamp — when no drop exists inside it, nothing is cut.
const DROP_WINDOW_MIN: usize = 3;
const DROP_WINDOW_MAX: usize = 30;

/// The query pipeline and its indexing half: the one door to everything the
/// server does with a vault's content. Owns the embedder, the vector store, the
/// optional LLM and reranker, and every policy — chunk splitting, embed
/// batching, dedup, rerank-or-take, the top_k cut, the semantic-only gate. The
/// store behind it is pure storage (see [`VectorStore`]).
pub struct KimunRag {
    store: Arc<dyn VectorStore + Send + Sync>,
    embedder: Arc<dyn Embedder>,
    /// `None` on a semantic-only server — search works, question-answering does
    /// not (adr/0022).
    llm_client: Option<Arc<dyn LLMClient + Send + Sync>>,
    reranker: Option<Arc<dyn Reranker>>,
    /// Which **context cut** sizes the no-reranker `answer` context (adr/0027).
    context_cut: ContextCut,
    /// The embedder fingerprint this pipeline must find recorded with the
    /// store's data (adr/0025). `None` skips the gate (tests, callers that
    /// enforce it themselves).
    expected_fingerprint: Option<String>,
    /// One-shot success marker for the fingerprint gate. A failed check is NOT
    /// cached — every data operation retries until the store is reachable, so
    /// a store that was down at boot never lets data ops run unverified.
    fingerprint_checked: tokio::sync::OnceCell<()>,
}

impl KimunRag {
    /// Create a new KimunRag instance from its parts. Pass `None` for the LLM
    /// on a semantic-only server.
    pub fn new(
        store: Arc<dyn VectorStore + Send + Sync>,
        embedder: Arc<dyn Embedder>,
        llm_client: Option<Arc<dyn LLMClient + Send + Sync>>,
    ) -> Self {
        Self {
            store,
            embedder,
            llm_client,
            reranker: None,
            context_cut: ContextCut::default(),
            expected_fingerprint: None,
            fingerprint_checked: tokio::sync::OnceCell::new(),
        }
    }

    /// Select the **context cut** for no-reranker answers (default:
    /// [`ContextCut::ScoreRange`]).
    pub fn with_context_cut(mut self, context_cut: ContextCut) -> Self {
        self.context_cut = context_cut;
        self
    }

    /// Attach a reranker (built by [`reranker::from_config`]). The caller owns
    /// the failure policy: reranker initialization is non-fatal at the server
    /// level — on error nothing is attached and results fall back to plain
    /// vector ranking. `/health` reports the actual state via
    /// [`Self::has_reranker`].
    pub fn with_reranker(mut self, reranker: Arc<dyn Reranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    /// Whether a reranker is actually active. Config may ask for one that then
    /// failed to initialize, so capability probes must ask the rag, not the
    /// config.
    pub fn has_reranker(&self) -> bool {
        self.reranker.is_some()
    }

    /// Arms the **embedder fingerprint** gate (adr/0025): before the first data
    /// operation touches the store, the recorded fingerprint is compared to
    /// `fingerprint` and a mismatch wipes all collections. Deliberately lazy —
    /// a store that is unreachable at boot (e.g. Qdrant still starting) must
    /// not prevent the server from binding; the check runs on first use
    /// instead, and keeps retrying until it succeeds.
    pub fn with_fingerprint(mut self, fingerprint: String) -> Self {
        self.expected_fingerprint = Some(fingerprint);
        self
    }

    /// Runs the fingerprint gate now (idempotent; success is remembered,
    /// failure retried on the next call). Startup calls this once as a
    /// best-effort eager check; every data operation calls it as the actual
    /// guarantee.
    pub async fn check_fingerprint(&self) -> Result<(), RagError> {
        let Some(expected) = &self.expected_fingerprint else {
            return Ok(());
        };
        self.fingerprint_checked
            .get_or_try_init(|| async {
                let wiped = enforce_embedder_fingerprint(self.store.as_ref(), expected).await?;
                if wiped {
                    log::warn!(
                        "Embedder changed (fingerprint now `{expected}`): wiped ALL stored \
                         collections. Every vault re-indexes on its next reconciliation."
                    );
                }
                Ok::<(), RagError>(())
            })
            .await?;
        Ok(())
    }

    /// Whether this server can answer questions — the capability gate for
    /// `answer` (false on a semantic-only server, adr/0022).
    pub fn can_answer(&self) -> bool {
        self.llm_client.is_some()
    }

    /// Semantic search: one result per note, `top_k` counts NOTES.
    ///
    /// Ranks the FULL pool before cutting: semantic search lists notes, but a
    /// single section-heavy note can otherwise fill every chunk slot and
    /// collapse (client-side, one row per note) to a single result.
    /// Rerank/sort everything, then keep each note's best chunk and take the
    /// top_k notes. (`answer` keeps chunk-level context — this note-dedup is
    /// search-only.)
    pub async fn search(
        &self,
        collection: &CollectionKey,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<ScoredChunk>, RagError> {
        let raw = self.retrieve(collection, query).await?;
        let pool_size = raw.len();
        let ranked = match &self.reranker {
            Some(reranker) => reranker.rerank(query, raw, pool_size).await?,
            // `deduplicate_chunks` already sorted best-first.
            None => raw,
        };
        Ok(dedupe_by_note(ranked, top_k))
    }

    /// Question-answering: retrieves the best CHUNKS as context (the LLM wants
    /// sections, not one-per-note) and asks the configured LLM. With a
    /// reranker, exactly `top_k` chunks; without one, `top_k` is ignored and
    /// the pool's normalized-score cutoff decides the context size
    /// ([`adaptive_context_cut`]). Fails with [`RagError::SemanticOnly`] when
    /// no LLM is configured; the gate runs before the vector search so no work
    /// is thrown away.
    pub async fn answer(
        &self,
        collection: &CollectionKey,
        question: &str,
        top_k: usize,
    ) -> Result<Answer, RagError> {
        let llm = self.llm_client.clone().ok_or(RagError::SemanticOnly)?;
        let raw = self.retrieve(collection, question).await?;
        let context = match &self.reranker {
            Some(reranker) => reranker.rerank(question, raw, top_k).await?,
            None => match self.context_cut {
                ContextCut::ScoreRange => score_range_cut(raw),
                ContextCut::LargestDrop => largest_drop_cut(raw),
            },
        };
        let text = llm.ask(question, &context).await?;
        Ok(Answer {
            text,
            sources: context,
        })
    }

    /// Embed the query and pull the deduplicated candidate pool from the store
    /// — the shared front half of `search` and `answer`.
    async fn retrieve(
        &self,
        collection: &CollectionKey,
        query: &str,
    ) -> Result<Vec<ScoredChunk>, RagError> {
        self.check_fingerprint().await?;
        let vector = self.embedder.prompt_embedding(query).await?;
        let raw = self
            .store
            .query(collection.as_str(), vector, CANDIDATE_POOL)
            .await?;
        Ok(deduplicate_chunks(raw))
    }

    /// Index a push of documents into a vault's collection: diff each doc's
    /// content hash against the store's records, drop the stale chunks of
    /// changed docs, then split → embed → store what's new or changed. Docs
    /// whose hash is unchanged are skipped without touching the store.
    pub async fn index(
        &self,
        collection: &CollectionKey,
        docs: &[KimunDoc],
    ) -> Result<IndexStats, RagError> {
        self.check_fingerprint().await?;
        let indexed_notes = self.store.indexed_notes(collection.as_str()).await?;
        debug!(
            "Indexing {} docs against {} already indexed",
            docs.len(),
            indexed_notes.len()
        );

        let mut stats = IndexStats {
            indexed: 0,
            skipped: 0,
            updated: 0,
            removed: 0,
            errors: 0,
        };
        let mut stale_paths: Vec<String> = Vec::new();
        let mut to_index: Vec<&KimunDoc> = Vec::new();

        for doc in docs {
            match indexed_notes.get(&doc.path) {
                Some(indexed) if indexed.content_hash == doc.hash => stats.skipped += 1,
                Some(_) => {
                    // Changed: its old chunks must go before the new ones land.
                    stats.updated += 1;
                    stale_paths.push(doc.path.clone());
                    to_index.push(doc);
                }
                None => {
                    stats.indexed += 1;
                    to_index.push(doc);
                }
            }
        }

        // A changed note is re-split wholesale, but a small edit leaves most
        // sub-chunks textually identical — pull the old rows before deleting
        // them so unchanged sub-chunks reuse their stored vector instead of a
        // fresh embedder round-trip. Keyed by title + text: exactly the inputs
        // the embedder sees. `remove` on lookup moves the vector out (a
        // duplicate identical chunk in one note just re-embeds — rare and
        // harmless).
        let cache_key = |title: &str, text: &str| format!("{title}\u{0}{text}");
        let mut vector_cache: HashMap<String, Vec<f32>> = HashMap::new();
        if !stale_paths.is_empty() {
            for row in self
                .store
                .chunks_with_vectors(collection.as_str(), &stale_paths)
                .await?
            {
                let EmbeddedChunk { chunk, vector } = row;
                vector_cache.insert(cache_key(&chunk.title, &chunk.text), vector);
            }
            self.store.delete(collection.as_str(), &stale_paths).await?;
        }

        // Sub-split sections to the embedding window, then embed each
        // sub-chunk 1:1 — so a row's stored text is exactly the text that
        // produced its vector.
        let owned: Vec<KimunDoc> = to_index.into_iter().cloned().collect();
        let chunks = FlattenedChunk::from_chunks_split(&owned, CHUNK_TARGET, CHUNK_MAX);
        debug!("{} docs split to {} chunks", owned.len(), chunks.len());

        let mut rows: Vec<EmbeddedChunk> = Vec::with_capacity(chunks.len());
        let mut to_embed: Vec<FlattenedChunk> = Vec::new();
        for chunk in chunks {
            match vector_cache.remove(&cache_key(&chunk.title, &chunk.text)) {
                Some(vector) => rows.push(EmbeddedChunk { vector, chunk }),
                None => to_embed.push(chunk),
            }
        }
        if !rows.is_empty() {
            debug!("Reusing {} stored vectors for unchanged chunks", rows.len());
        }

        // Embed EVERYTHING before storing ANYTHING. The old rows are already
        // deleted, so a note must end this pass either fully present at its
        // new hash or wholly absent: absent notes drop out of /hashes and the
        // client's reconcile re-pushes them (self-healing), while a partial
        // store at the new hash would read as complete (hash == hash) and the
        // missing chunks would never be repaired.
        for batch in to_embed.chunks(EMBED_BATCH) {
            let embeddings = self.embedder.generate_embeddings(batch).await?;
            if embeddings.len() != batch.len() {
                return Err(RagError::Backend(anyhow::anyhow!(
                    "embedder returned {} vectors for {} chunks",
                    embeddings.len(),
                    batch.len()
                )));
            }
            rows.extend(
                batch
                    .iter()
                    .zip(embeddings)
                    .map(|(chunk, vector)| EmbeddedChunk {
                        chunk: chunk.clone(),
                        vector,
                    }),
            );
        }

        // Store per note, and each store() call is atomic in both backends
        // (one SQLite tx / one Qdrant upsert) — so a mid-loop failure leaves
        // every note either complete or absent, never partial.
        let mut by_path: HashMap<String, Vec<EmbeddedChunk>> = HashMap::new();
        for row in rows {
            by_path
                .entry(row.chunk.doc_path.clone())
                .or_default()
                .push(row);
        }
        for note_rows in by_path.into_values() {
            self.store.store(collection.as_str(), &note_rows).await?;
        }

        Ok(stats)
    }

    /// Remove notes (all their chunks) from a vault's collection.
    pub async fn delete_notes(
        &self,
        collection: &CollectionKey,
        paths: &[String],
    ) -> Result<(), RagError> {
        self.check_fingerprint().await?;
        self.store.delete(collection.as_str(), paths).await?;
        Ok(())
    }

    /// Reconcile support: the `{note path → content hash}` set the server holds
    /// for a vault (adr/0019).
    pub async fn note_hashes(
        &self,
        collection: &CollectionKey,
    ) -> Result<HashMap<String, String>, RagError> {
        self.check_fingerprint().await?;
        let notes = self.store.indexed_notes(collection.as_str()).await?;
        Ok(notes
            .into_iter()
            .map(|(path, note)| (path, note.content_hash))
            .collect())
    }

    /// Every collection with its indexed-note count (admin UI).
    pub async fn collections(&self) -> Result<Vec<CollectionInfo>, RagError> {
        self.check_fingerprint().await?;
        Ok(self.store.list_collections().await?)
    }

    /// Just the collection names (vault ids).
    pub async fn collection_names(&self) -> Result<Vec<String>, RagError> {
        self.check_fingerprint().await?;
        Ok(self.store.collection_names().await?)
    }
}

/// Collapse ranked chunks to one row per note — the best (first-seen, so
/// highest-ranked) chunk of each `doc_path` — and keep at most `top_k` notes.
/// Input must already be ranked best-first.
fn dedupe_by_note(ranked: Vec<ScoredChunk>, top_k: usize) -> Vec<ScoredChunk> {
    use std::collections::HashSet;
    if top_k == 0 {
        return Vec::new();
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (score, chunk) in ranked {
        if seen.insert(chunk.doc_path.clone()) {
            out.push((score, chunk));
            if out.len() == top_k {
                break;
            }
        }
    }
    out
}

/// The [`ContextCut::ScoreRange`] cut. No count cut at all — scores are
/// min-max normalized within the (already [`CANDIDATE_POOL`]-capped) pool and
/// a chunk survives when its normalized score reaches
/// [`NO_RERANK_NORM_CUTOFF`]: the pool's own spread decides how many chunks
/// are relevant, not a fixed `top_k`. Normalization is shift/scale invariant,
/// so negative scores need no special casing; a flat pool (zero range) carries
/// no signal to cut on and is kept whole. `sorted` must be best-first, which
/// [`deduplicate_chunks`] guarantees.
fn score_range_cut(mut sorted: Vec<ScoredChunk>) -> Vec<ScoredChunk> {
    let (Some(&(max, _)), Some(&(min, _))) = (sorted.first(), sorted.last()) else {
        return sorted;
    };
    let range = max - min;
    if range <= 0.0 {
        return sorted;
    }
    let cutoff = min + NO_RERANK_NORM_CUTOFF * range;
    let keep = sorted
        .iter()
        .take_while(|(score, _)| *score >= cutoff)
        .count();
    sorted.truncate(keep);
    sorted
}

/// The [`ContextCut::LargestDrop`] cut: find the biggest drop between
/// consecutive scores and cut there — the drop's position alone decides the
/// kept count. The window is the *search range* for the drop (cut positions
/// [`DROP_WINDOW`]), not a clamp: it stops degenerate elbows (a spiky top
/// yielding a 1-chunk context, phantom gaps deep in the noise tail), and a
/// window with no drop at all means no evidence of a relevance boundary — the
/// pool stays whole, same philosophy as [`score_range_cut`]'s flat rule. Ties
/// cut at the earliest drop (precision over recall). `sorted` must be
/// best-first, which [`deduplicate_chunks`] guarantees.
fn largest_drop_cut(mut sorted: Vec<ScoredChunk>) -> Vec<ScoredChunk> {
    let mut best: Option<(f64, usize)> = None; // (drop, kept count)
    for keep in DROP_WINDOW_MIN..=DROP_WINDOW_MAX.min(sorted.len().saturating_sub(1)) {
        let drop = sorted[keep - 1].0 - sorted[keep].0;
        if drop > 0.0 && best.is_none_or(|(largest, _)| drop > largest) {
            best = Some((drop, keep));
        }
    }
    if let Some((_, keep)) = best {
        sorted.truncate(keep);
    }
    sorted
}

/// Deduplicates embedding results by FlattenedChunk (keeping the highest score
/// per unique chunk) and returns them sorted best-first. Scores are similarities
/// (higher = better) for both backends, so this ordering is what the
/// no-reranker cuts ([`adaptive_context_cut`], `search`'s note-dedup) rely on.
fn deduplicate_chunks(results: Vec<ScoredChunk>) -> Vec<ScoredChunk> {
    let original_count = results.len();
    let mut dedup_map: HashMap<FlattenedChunk, f64> = HashMap::new();

    for (score, chunk) in results {
        // Keep the chunk with the highest score
        dedup_map
            .entry(chunk)
            .and_modify(|existing_score| {
                if score > *existing_score {
                    *existing_score = score;
                }
            })
            .or_insert(score);
    }

    let mut deduplicated: Vec<ScoredChunk> = dedup_map
        .into_iter()
        .map(|(chunk, score)| (score, chunk))
        .collect();
    // The HashMap destroyed the query's ordering; restore best-first so a
    // no-reranker `take(top_k)` keeps the actual top matches.
    deduplicated.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    debug!(
        "After deduplication: {} unique results (from {} total)",
        deduplicated.len(),
        original_count
    );

    deduplicated
}

/// Startup guard for the **embedder fingerprint** (adr/0025): compares the
/// configured embedder's fingerprint against the one recorded with the store's
/// data. On mismatch the stored vectors are unusable by definition — drop every
/// collection and record the new fingerprint; the now-empty server makes every
/// client's next reconciliation re-push everything. A fresh store just records
/// it. Returns `true` when data was wiped (the caller logs loudly).
pub async fn enforce_embedder_fingerprint(
    store: &(dyn VectorStore + Send + Sync),
    expected: &str,
) -> anyhow::Result<bool> {
    match store.read_fingerprint().await? {
        Some(existing) if existing == expected => Ok(false),
        Some(_) => {
            store.drop_all_collections().await?;
            store.write_fingerprint(expected).await?;
            Ok(true)
        }
        None => {
            store.write_fingerprint(expected).await?;
            Ok(false)
        }
    }
}

/// Statistics from indexing operation
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub indexed: usize,
    pub skipped: usize,
    pub updated: usize,
    pub removed: usize,
    pub errors: usize,
}

impl Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Index Stats: ")?;
        writeln!(f, "  > Indexed: {}", self.indexed)?;
        writeln!(f, "  > Skipped: {}", self.skipped)?;
        writeln!(f, "  > Updated: {}", self.updated)?;
        writeln!(f, "  > Removed: {}", self.removed)?;
        writeln!(f, "  > Errors: {}", self.errors)
    }
}

/// Fake store, embedder, and LLM shared by the pipeline tests here and the
/// webui router tests — so both exercise their surface without a real backend.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::dbembeddings::IndexedNote;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Canned query results plus a record of every store/delete call, so index
    /// tests can assert what reached the store.
    pub(crate) struct FakeVectorStore {
        pub results: Vec<ScoredChunk>,
        pub notes: HashMap<String, IndexedNote>,
        /// Pre-existing rows served by `chunks_with_vectors` (the embedding
        /// cache the pipeline may reuse from).
        pub chunk_rows: Vec<EmbeddedChunk>,
        pub stored: Mutex<Vec<(String, Vec<String>)>>,
        /// Every row handed to `store`, verbatim.
        pub stored_rows: Mutex<Vec<EmbeddedChunk>>,
        pub deleted: Mutex<Vec<(String, Vec<String>)>>,
        pub fingerprint: Mutex<Option<String>>,
        pub dropped_all: Mutex<bool>,
        /// When set, `read_fingerprint` errors — simulates a store that is
        /// unreachable (Qdrant down) for the lazy-gate tests.
        pub fingerprint_unavailable: Mutex<bool>,
    }

    impl Default for FakeVectorStore {
        fn default() -> Self {
            Self {
                results: vec![(
                    0.9,
                    FlattenedChunk {
                        doc_path: "/notes/a.md".into(),
                        doc_hash: "h".into(),
                        title: "A".into(),
                        text: "hello world".into(),
                        date: None,
                    },
                )],
                notes: HashMap::new(),
                chunk_rows: Vec::new(),
                stored: Mutex::new(Vec::new()),
                stored_rows: Mutex::new(Vec::new()),
                deleted: Mutex::new(Vec::new()),
                fingerprint: Mutex::new(None),
                dropped_all: Mutex::new(false),
                fingerprint_unavailable: Mutex::new(false),
            }
        }
    }

    #[async_trait]
    impl VectorStore for FakeVectorStore {
        async fn store(&self, collection: &str, rows: &[EmbeddedChunk]) -> anyhow::Result<()> {
            self.stored.lock().unwrap().push((
                collection.to_string(),
                rows.iter().map(|r| r.chunk.doc_path.clone()).collect(),
            ));
            self.stored_rows
                .lock()
                .unwrap()
                .extend(rows.iter().cloned());
            Ok(())
        }
        async fn chunks_with_vectors(
            &self,
            _: &str,
            paths: &[String],
        ) -> anyhow::Result<Vec<EmbeddedChunk>> {
            Ok(self
                .chunk_rows
                .iter()
                .filter(|r| paths.contains(&r.chunk.doc_path))
                .cloned()
                .collect())
        }
        async fn delete(&self, collection: &str, paths: &[String]) -> anyhow::Result<()> {
            self.deleted
                .lock()
                .unwrap()
                .push((collection.to_string(), paths.to_vec()));
            Ok(())
        }
        async fn query(&self, _: &str, _: Vec<f32>, _: usize) -> anyhow::Result<Vec<ScoredChunk>> {
            Ok(self.results.clone())
        }
        async fn indexed_notes(&self, _: &str) -> anyhow::Result<HashMap<String, IndexedNote>> {
            Ok(self.notes.clone())
        }
        async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>> {
            Ok(vec![CollectionInfo {
                name: "vault-1".into(),
                note_count: 3,
            }])
        }
        async fn collection_names(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec!["vault-1".into()])
        }
        async fn read_fingerprint(&self) -> anyhow::Result<Option<String>> {
            if *self.fingerprint_unavailable.lock().unwrap() {
                anyhow::bail!("store unreachable");
            }
            Ok(self.fingerprint.lock().unwrap().clone())
        }
        async fn write_fingerprint(&self, fingerprint: &str) -> anyhow::Result<()> {
            *self.fingerprint.lock().unwrap() = Some(fingerprint.to_string());
            Ok(())
        }
        async fn drop_all_collections(&self) -> anyhow::Result<()> {
            *self.dropped_all.lock().unwrap() = true;
            Ok(())
        }
    }

    /// Deterministic embedder: non-zero vector per text, no model download.
    pub(crate) struct FakeEmbedder;

    #[async_trait]
    impl Embedder for FakeEmbedder {
        async fn generate_embeddings(
            &self,
            content: &[FlattenedChunk],
        ) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(content.iter().map(|c| embed(&c.text)).collect())
        }
        async fn prompt_embedding(&self, content: &str) -> anyhow::Result<Vec<f32>> {
            Ok(embed(content))
        }
        fn dimension(&self) -> usize {
            8
        }
    }

    fn embed(text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; 8];
        v[0] = 1.0;
        for (i, b) in text.bytes().enumerate() {
            v[1 + (i % 7)] += b as f32;
        }
        v
    }

    pub(crate) struct FakeLlm;

    #[async_trait]
    impl LLMClient for FakeLlm {
        async fn ask(&self, _: &str, _: &[ScoredChunk]) -> anyhow::Result<String> {
            Ok("answer".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{FakeEmbedder, FakeLlm, FakeVectorStore};
    use super::*;
    use crate::dbembeddings::IndexedNote;
    use crate::document::KimunSection;

    fn chunk(path: &str, section: &str) -> FlattenedChunk {
        FlattenedChunk {
            doc_path: path.to_string(),
            doc_hash: "h".to_string(),
            title: section.to_string(),
            text: format!("{path}#{section}"),
            date: None,
        }
    }

    fn key(s: &str) -> CollectionKey {
        CollectionKey::parse(s).unwrap()
    }

    fn rag(store: FakeVectorStore, llm: bool) -> KimunRag {
        KimunRag::new(
            Arc::new(store),
            Arc::new(FakeEmbedder),
            if llm { Some(Arc::new(FakeLlm)) } else { None },
        )
    }

    #[test]
    fn reranker_is_absent_until_successfully_enabled() {
        let r = rag(FakeVectorStore::default(), false);
        assert!(
            !r.has_reranker(),
            "a fresh KimunRag reports no reranker — /health must not claim one"
        );
    }

    /// A store returning note A three times at the top, then B, then C —
    /// the shape that exposes the note-vs-chunk top_k difference.
    fn section_heavy_store() -> FakeVectorStore {
        FakeVectorStore {
            results: vec![
                (0.99, chunk("/a.md", "intro")),
                (0.98, chunk("/a.md", "body")),
                (0.97, chunk("/a.md", "end")),
                (0.80, chunk("/b.md", "b1")),
                (0.70, chunk("/c.md", "c1")),
            ],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn search_returns_one_result_per_note_and_top_k_counts_notes() {
        // Chunk-level top_k=2 would return two /a.md chunks → one note after the
        // client's one-row-per-note collapse; search must surface A and B.
        let rag = rag(section_heavy_store(), false);
        let out = rag.search(&key("vault-1"), "q", 2).await.unwrap();
        assert_eq!(out.len(), 2, "top_k counts NOTES, not chunks");
        assert_eq!(out[0].1.doc_path, "/a.md");
        assert_eq!(out[0].1.title, "intro", "keeps the note's best chunk");
        assert_eq!(out[1].1.doc_path, "/b.md");
    }

    #[tokio::test]
    async fn search_dedupes_identical_chunks_keeping_best_score() {
        let dup = chunk("/a.md", "s");
        let store = FakeVectorStore {
            results: vec![
                (0.5, dup.clone()),
                (0.9, dup.clone()),
                (0.7, chunk("/b.md", "s")),
            ],
            ..Default::default()
        };
        let out = rag(store, false)
            .search(&key("vault-1"), "q", 10)
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, 0.9, "duplicate keeps its highest score");
        assert_eq!(out[0].1.doc_path, "/a.md");
    }

    #[tokio::test]
    async fn answer_keeps_chunk_level_context_cut_by_normalized_score() {
        // The LLM context is CHUNKS — /a.md may fill several slots (no
        // note-dedup) — and without a reranker the cut ignores top_k entirely:
        // scores are min-max normalized within the pool and the top half
        // survives. Here min 0.70 / max 0.99 → raw cutoff 0.845, so
        // 0.99/0.98/0.97 stay while 0.80 (normalized 0.34) and 0.70 (0.0)
        // drop, even though the request asked for top_k = 2.
        let rag = rag(section_heavy_store(), true);
        let answer = rag.answer(&key("vault-1"), "q", 2).await.unwrap();
        assert_eq!(answer.text, "answer");
        assert_eq!(answer.sources.len(), 3);
        assert_eq!(answer.sources[0].1.doc_path, "/a.md");
        assert_eq!(
            answer.sources[1].1.doc_path, "/a.md",
            "chunk-level: no note-dedup"
        );
    }

    #[tokio::test]
    async fn answer_without_reranker_has_no_count_floor() {
        // A spiky top score trims the context to the genuinely relevant
        // chunks: 0.4 and 0.3 normalize to 0.17 and 0.0 against a 0.9 top —
        // below the 0.5 line, out, regardless of the requested top_k.
        let store = FakeVectorStore {
            results: vec![
                (0.90, chunk("/a.md", "a1")),
                (0.40, chunk("/b.md", "b1")),
                (0.30, chunk("/c.md", "c1")),
            ],
            ..Default::default()
        };
        let answer = rag(store, true).answer(&key("vault-1"), "q", 2).await.unwrap();
        assert_eq!(answer.sources.len(), 1);
        assert_eq!(answer.sources[0].1.doc_path, "/a.md");
    }

    #[test]
    fn score_range_cut_is_shift_invariant_and_keeps_flat_pools_whole() {
        // Min-max normalization only sees relative position, so negative
        // scores need no special casing: -0.1 normalizes to 1.0, -0.5 to 0.0.
        let pool: Vec<ScoredChunk> = vec![
            (-0.1, chunk("/a.md", "a")),
            (-0.5, chunk("/b.md", "b")),
        ];
        assert_eq!(score_range_cut(pool).len(), 1);

        // Flat scores carry no signal to cut on — everything stays.
        let flat: Vec<ScoredChunk> = vec![
            (0.7, chunk("/a.md", "a")),
            (0.7, chunk("/b.md", "b")),
            (0.7, chunk("/c.md", "c")),
        ];
        assert_eq!(score_range_cut(flat).len(), 3);

        assert!(score_range_cut(Vec::new()).is_empty());
    }

    /// A pool of `n` chunks with the given scores, distinct paths.
    fn pool(scores: &[f64]) -> Vec<ScoredChunk> {
        scores
            .iter()
            .enumerate()
            .map(|(i, &s)| (s, chunk(&format!("/n{i}.md"), "s")))
            .collect()
    }

    #[test]
    fn largest_drop_cut_cuts_at_the_biggest_gap_in_the_window() {
        // Biggest gap sits between positions 5 and 6 → keep 5.
        let p = pool(&[0.90, 0.89, 0.88, 0.87, 0.86, 0.50, 0.49, 0.48]);
        assert_eq!(largest_drop_cut(p).len(), 5);
    }

    #[test]
    fn largest_drop_cut_only_searches_the_window() {
        // The window is where the drop is SEARCHED, not a keep-clamp. A cliff
        // between positions 2 and 3 (cut would keep 2 < window min) is not a
        // candidate; the window itself is flat → no elbow → whole pool stays.
        let p = pool(&[0.90, 0.90, 0.20, 0.20, 0.20, 0.20, 0.20]);
        assert_eq!(largest_drop_cut(p).len(), 7);

        // Same beyond the far edge: flat through position 30, cliff at 31.
        let mut scores = vec![0.9; 31];
        scores.extend([0.1; 4]);
        assert_eq!(largest_drop_cut(pool(&scores)).len(), 35);
    }

    #[test]
    fn largest_drop_cut_ties_cut_earliest_and_small_pools_stay_whole() {
        // Two equal 0.30 drops (keep 3 vs keep 5) → the earliest wins.
        let p = pool(&[0.90, 0.89, 0.88, 0.58, 0.57, 0.27, 0.26]);
        assert_eq!(largest_drop_cut(p).len(), 3);

        // At or below the window start there is nothing to search.
        assert_eq!(largest_drop_cut(pool(&[0.9, 0.2, 0.1])).len(), 3);
        assert!(largest_drop_cut(Vec::new()).is_empty());
    }

    #[tokio::test]
    async fn answer_uses_the_configured_context_cut() {
        // Scores where the two algorithms disagree: score-range keeps 2
        // (min 0.84 / max 1.0 → cutoff 0.92, so 1.00 and 0.93 clear it),
        // largest-drop keeps 3 (window drops tie at 0.02 → earliest; the
        // bigger drops at positions 1 and 2 are outside the search window).
        let scores = [1.00, 0.93, 0.88, 0.86, 0.84];
        let store = || FakeVectorStore {
            results: pool(&scores),
            ..Default::default()
        };
        let default_cut = rag(store(), true);
        assert_eq!(
            default_cut.answer(&key("v"), "q", 2).await.unwrap().sources.len(),
            2,
            "default is score-range"
        );

        let elbow = rag(store(), true).with_context_cut(ContextCut::LargestDrop);
        assert_eq!(elbow.answer(&key("v"), "q", 2).await.unwrap().sources.len(), 3);
    }

    #[tokio::test]
    async fn answer_on_semantic_only_server_is_a_typed_rejection() {
        let rag = rag(section_heavy_store(), false);
        assert!(!rag.can_answer());
        match rag.answer(&key("vault-1"), "q", 5).await {
            Err(RagError::SemanticOnly) => {}
            other => panic!("expected SemanticOnly, got {:?}", other.map(|a| a.text)),
        }
    }

    fn doc(path: &str, hash: &str, text: &str) -> KimunDoc {
        KimunDoc {
            path: path.to_string(),
            hash: hash.to_string(),
            sections: vec![KimunSection {
                title: "T".to_string(),
                text: text.to_string(),
            }],
        }
    }

    fn note(path: &str, hash: &str) -> (String, IndexedNote) {
        (
            path.to_string(),
            IndexedNote {
                path: path.to_string(),
                content_hash: hash.to_string(),
                last_indexed: 0,
            },
        )
    }

    #[tokio::test]
    async fn index_diffs_hashes_new_changed_unchanged() {
        // Store already holds a.md@h1 and b.md@h1. Push: a.md unchanged,
        // b.md changed, c.md new.
        let store = FakeVectorStore {
            notes: [note("a.md", "h1"), note("b.md", "h1")].into(),
            ..Default::default()
        };
        let rag = rag(store, false);
        let stats = rag
            .index(
                &key("v"),
                &[
                    doc("a.md", "h1", "alpha"),
                    doc("b.md", "h2", "beta v2"),
                    doc("c.md", "h1", "gamma"),
                ],
            )
            .await
            .unwrap();

        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.updated, 1);
        assert_eq!(stats.indexed, 1);
    }

    #[tokio::test]
    async fn index_deletes_stale_chunks_of_changed_docs_only() {
        let fake = Arc::new(FakeVectorStore {
            notes: [note("a.md", "h1"), note("b.md", "h1")].into(),
            ..Default::default()
        });
        let rag = KimunRag::new(fake.clone(), Arc::new(FakeEmbedder), None);
        rag.index(
            &key("v"),
            &[doc("a.md", "h1", "alpha"), doc("b.md", "h2", "beta v2")],
        )
        .await
        .unwrap();

        // Only b.md (changed) was deleted, and only b.md's chunks were stored.
        let deleted = fake.deleted.lock().unwrap().clone();
        assert_eq!(deleted, vec![("v".to_string(), vec!["b.md".to_string()])]);
        let stored = fake.stored.lock().unwrap().clone();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].1, vec!["b.md".to_string()]);
    }

    #[tokio::test]
    async fn index_reuses_stored_vectors_for_unchanged_chunks() {
        use std::sync::Mutex;

        /// Records every text it embeds, so the test can assert which chunks
        /// actually hit the embedder.
        struct RecordingEmbedder(Mutex<Vec<String>>);
        #[async_trait::async_trait]
        impl Embedder for RecordingEmbedder {
            async fn generate_embeddings(
                &self,
                content: &[FlattenedChunk],
            ) -> anyhow::Result<Vec<Vec<f32>>> {
                let mut calls = self.0.lock().unwrap();
                calls.extend(content.iter().map(|c| c.text.clone()));
                Ok(vec![vec![1.0; 8]; content.len()])
            }
            async fn prompt_embedding(&self, _: &str) -> anyhow::Result<Vec<f32>> {
                Ok(vec![1.0; 8])
            }
            fn dimension(&self) -> usize {
                8
            }
        }

        let cached = |text: &str, vector: Vec<f32>| EmbeddedChunk {
            chunk: FlattenedChunk {
                doc_path: "a.md".to_string(),
                doc_hash: "h1".to_string(),
                title: "T".to_string(),
                text: text.to_string(),
                date: None,
            },
            vector,
        };
        let store = Arc::new(FakeVectorStore {
            notes: [note("a.md", "h1")].into(),
            chunk_rows: vec![cached("alpha", vec![9.0; 8]), cached("beta", vec![8.0; 8])],
            ..Default::default()
        });
        let embedder = Arc::new(RecordingEmbedder(Mutex::new(Vec::new())));
        let rag = KimunRag::new(store.clone(), embedder.clone(), None);

        // The note changed (h1 → h2): section "alpha" is untouched, "beta"
        // became "gamma".
        let doc = KimunDoc {
            path: "a.md".to_string(),
            hash: "h2".to_string(),
            sections: vec![
                KimunSection {
                    title: "T".to_string(),
                    text: "alpha".to_string(),
                },
                KimunSection {
                    title: "T".to_string(),
                    text: "gamma".to_string(),
                },
            ],
        };
        rag.index(&key("v"), &[doc]).await.unwrap();

        // Only the changed chunk reached the embedder.
        assert_eq!(*embedder.0.lock().unwrap(), vec!["gamma".to_string()]);

        let rows = store.stored_rows.lock().unwrap();
        let alpha = rows.iter().find(|r| r.chunk.text == "alpha").unwrap();
        // Reused vector, but re-stamped with the note's NEW hash.
        assert_eq!(alpha.vector, vec![9.0; 8]);
        assert_eq!(alpha.chunk.doc_hash, "h2");
        let gamma = rows.iter().find(|r| r.chunk.text == "gamma").unwrap();
        assert_eq!(gamma.chunk.doc_hash, "h2");
        assert!(
            !rows.iter().any(|r| r.chunk.text == "beta"),
            "stale chunk gone"
        );
    }

    #[tokio::test]
    async fn index_stores_nothing_when_embedding_fails() {
        /// Always errors — simulates a rate-limited / dead embedder.
        struct FailingEmbedder;
        #[async_trait::async_trait]
        impl Embedder for FailingEmbedder {
            async fn generate_embeddings(
                &self,
                _: &[FlattenedChunk],
            ) -> anyhow::Result<Vec<Vec<f32>>> {
                anyhow::bail!("embedder down")
            }
            async fn prompt_embedding(&self, _: &str) -> anyhow::Result<Vec<f32>> {
                anyhow::bail!("embedder down")
            }
            fn dimension(&self) -> usize {
                8
            }
        }

        // The store holds a.md@h1 with one reusable chunk ("alpha").
        let store = Arc::new(FakeVectorStore {
            notes: [note("a.md", "h1")].into(),
            chunk_rows: vec![EmbeddedChunk {
                chunk: FlattenedChunk {
                    doc_path: "a.md".to_string(),
                    doc_hash: "h1".to_string(),
                    title: "T".to_string(),
                    text: "alpha".to_string(),
                    date: None,
                },
                vector: vec![9.0; 8],
            }],
            ..Default::default()
        });
        let rag = KimunRag::new(store.clone(), Arc::new(FailingEmbedder), None);

        // a.md changes: "alpha" reusable, "gamma" needs the (dead) embedder.
        let doc = KimunDoc {
            path: "a.md".to_string(),
            hash: "h2".to_string(),
            sections: vec![
                KimunSection {
                    title: "T".to_string(),
                    text: "alpha".to_string(),
                },
                KimunSection {
                    title: "T".to_string(),
                    text: "gamma".to_string(),
                },
            ],
        };
        assert!(rag.index(&key("v"), &[doc]).await.is_err());

        // All-or-nothing: NO rows stored — not even the reusable one. A
        // partial store at h2 would make /hashes report the note complete
        // and reconcile would never repair the missing chunk.
        assert!(store.stored_rows.lock().unwrap().is_empty());
        assert!(store.stored.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fingerprint_fresh_store_records_without_wiping() {
        let store = FakeVectorStore::default();
        let wiped = enforce_embedder_fingerprint(&store, "fastembed:default:1024")
            .await
            .unwrap();
        assert!(!wiped);
        assert!(!*store.dropped_all.lock().unwrap());
        assert_eq!(
            store.fingerprint.lock().unwrap().as_deref(),
            Some("fastembed:default:1024")
        );
    }

    #[tokio::test]
    async fn fingerprint_match_is_a_no_op() {
        let store = FakeVectorStore::default();
        *store.fingerprint.lock().unwrap() = Some("fastembed:default:1024".to_string());
        let wiped = enforce_embedder_fingerprint(&store, "fastembed:default:1024")
            .await
            .unwrap();
        assert!(!wiped);
        assert!(!*store.dropped_all.lock().unwrap());
    }

    #[tokio::test]
    async fn fingerprint_mismatch_wipes_and_rerecords() {
        // The embedder changed: stored vectors are garbage for the new model,
        // and reconciliation can't see it (note hashes unchanged) — wipe, so
        // every client's next reconcile re-pushes everything (adr/0025).
        let store = FakeVectorStore::default();
        *store.fingerprint.lock().unwrap() = Some("fastembed:default:1024".to_string());
        let wiped = enforce_embedder_fingerprint(&store, "ollama:nomic-embed-text:768")
            .await
            .unwrap();
        assert!(wiped);
        assert!(*store.dropped_all.lock().unwrap());
        assert_eq!(
            store.fingerprint.lock().unwrap().as_deref(),
            Some("ollama:nomic-embed-text:768")
        );
    }

    #[tokio::test]
    async fn fingerprint_gate_runs_before_first_data_op_and_wipes_on_mismatch() {
        let fake = Arc::new(FakeVectorStore::default());
        *fake.fingerprint.lock().unwrap() = Some("old:fp:1".into());
        let rag = KimunRag::new(fake.clone(), Arc::new(FakeEmbedder), None)
            .with_fingerprint("new:fp:8".into());
        rag.search(&key("vault-1"), "q", 5).await.unwrap();
        assert!(*fake.dropped_all.lock().unwrap());
        assert_eq!(
            fake.fingerprint.lock().unwrap().as_deref(),
            Some("new:fp:8")
        );
    }

    #[tokio::test]
    async fn fingerprint_gate_retries_after_store_outage() {
        // A store that is down at boot (Qdrant still starting) must not let
        // data ops run unverified — but must recover without a restart once
        // the store is reachable (adr/0025).
        let fake = Arc::new(FakeVectorStore::default());
        *fake.fingerprint_unavailable.lock().unwrap() = true;
        let rag = KimunRag::new(fake.clone(), Arc::new(FakeEmbedder), None)
            .with_fingerprint("fp:8".into());
        assert!(rag.check_fingerprint().await.is_err(), "eager check fails");
        assert!(
            rag.search(&key("vault-1"), "q", 5).await.is_err(),
            "data op must not run while the fingerprint is unverifiable"
        );
        *fake.fingerprint_unavailable.lock().unwrap() = false;
        rag.search(&key("vault-1"), "q", 5).await.unwrap();
        assert_eq!(fake.fingerprint.lock().unwrap().as_deref(), Some("fp:8"));
    }

    #[test]
    fn dedupe_by_note_returns_all_distinct_notes_when_under_top_k() {
        let ranked = vec![
            (0.9, chunk("/a.md", "s")),
            (0.8, chunk("/a.md", "s2")),
            (0.7, chunk("/b.md", "s")),
        ];
        let out = dedupe_by_note(ranked, 20);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1.doc_path, "/a.md");
        assert_eq!(out[1].1.doc_path, "/b.md");
    }

    #[test]
    fn dedupe_by_note_top_k_zero_is_empty() {
        let ranked = vec![(0.9, chunk("/a.md", "s"))];
        assert!(dedupe_by_note(ranked, 0).is_empty());
    }
}
