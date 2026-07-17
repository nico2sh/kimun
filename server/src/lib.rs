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
use crate::reranker::{Reranker, sigmoid, validate_scored};
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

/// Preview of where the **context cut** slices an answer's LLM context on a
/// query's ranked pool — the web UI's test-query box renders it so the cut
/// is observable against real vault data, with or without a reranker
/// (adr/0029).
pub struct CutPreview {
    /// Chunks retrieved into the candidate pool.
    pub pool_chunks: usize,
    /// The chunks the configured cut would keep as LLM context; displayed
    /// search rows are marked by membership.
    pub context: std::collections::HashSet<FlattenedChunk>,
    /// Pool scores of the last kept and first dropped chunk — where the cut
    /// actually landed. Often invisible in the displayed rows: those are
    /// note-deduped best chunks, while the pool interleaves every section of
    /// every note. `None` when nothing was cut.
    pub boundary: Option<(f64, f64)>,
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
/// The score-range cut's default cutoff (config `score_range_cutoff`): a
/// chunk survives when its min-max-normalized score —
/// `(s - s_min) / (s_max - s_min)` over the query's ranked pool — is at
/// least this, on both reranker paths (adr/0029). Normalized (not an
/// absolute cutoff) because absolute score bands are model-specific; within
/// one query all pool scores share a scale, so the pool's own spread defines
/// what "relevant" looks like.
const SCORE_RANGE_DEFAULT_CUTOFF: f64 = 0.4;
/// [`score_range_cut`]'s robust range endpoints: the normalization range is
/// measured between these percentiles of the pool's scores instead of the
/// absolute min/max, so one stray chunk at either extreme cannot stretch the
/// range and move the cutoff. Small pools collapse to plain min-max.
const RANGE_LOW_PERCENTILE: f64 = 0.05;
const RANGE_HIGH_PERCENTILE: f64 = 0.95;
/// [`ContextCut::LargestDrop`]'s search window: the biggest score drop is
/// looked for at note positions 3..=30 only (distinct notes, best score
/// each). Below 3 a spiky top would starve the context; past 30 the tail's
/// noise produces phantom elbows. A window, not a keep-clamp — when no drop
/// exists inside it, nothing is cut.
const DROP_WINDOW_MIN: usize = 3;
const DROP_WINDOW_MAX: usize = 30;

/// The query pipeline and its indexing half: the one door to everything the
/// server does with a vault's content. Owns the embedder, the vector store, the
/// optional LLM and reranker, and every policy — chunk splitting, embed
/// batching, dedup, ranking, the context cut, the semantic-only gate. The
/// store behind it is pure storage (see [`VectorStore`]).
pub struct KimunRag {
    store: Arc<dyn VectorStore + Send + Sync>,
    embedder: Arc<dyn Embedder>,
    /// `None` on a semantic-only server — search works, question-answering does
    /// not (adr/0022).
    llm_client: Option<Arc<dyn LLMClient + Send + Sync>>,
    reranker: Option<Arc<dyn Reranker>>,
    /// Which **context cut** sizes both query surfaces, on both reranker
    /// paths (adr/0029).
    context_cut: ContextCut,
    /// The score-range cut's normalized cutoff (config `score_range_cutoff`,
    /// default [`SCORE_RANGE_DEFAULT_CUTOFF`]).
    score_range_cutoff: f64,
    /// The largest-drop cut's note-position search window (config
    /// `drop_window_min`/`drop_window_max`, defaults [`DROP_WINDOW_MIN`] and
    /// [`DROP_WINDOW_MAX`]).
    drop_window: (usize, usize),
    /// One-shot marker for the "reranker scores outside 0..1" warning, so a
    /// misconfigured backend logs once per run instead of per query.
    score_scale_warned: std::sync::atomic::AtomicBool,
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
            score_range_cutoff: SCORE_RANGE_DEFAULT_CUTOFF,
            drop_window: (DROP_WINDOW_MIN, DROP_WINDOW_MAX),
            score_scale_warned: std::sync::atomic::AtomicBool::new(false),
            expected_fingerprint: None,
            fingerprint_checked: tokio::sync::OnceCell::new(),
        }
    }

    /// Select the **context cut** that sizes search rows and answer contexts
    /// on both reranker paths (default: [`ContextCut::ScoreRange`],
    /// adr/0029).
    pub fn with_context_cut(mut self, context_cut: ContextCut) -> Self {
        self.context_cut = context_cut;
        self
    }

    /// Tune the score-range cut's normalized cutoff (default
    /// [`SCORE_RANGE_DEFAULT_CUTOFF`]). Values outside `0.0..=1.0` are
    /// clamped — including a TOML `inf`/`-inf`, which land on 1.0/0.0 (the
    /// strictest/loosest cut, matching what such configs always did). Only
    /// NaN is ignored: `clamp` would propagate it and every score comparison
    /// would fail, silently emptying all results.
    pub fn with_score_range_cutoff(mut self, cutoff: f64) -> Self {
        if cutoff.is_nan() {
            log::warn!(
                "Ignoring NaN score_range_cutoff; keeping {}",
                self.score_range_cutoff
            );
        } else {
            self.score_range_cutoff = cutoff.clamp(0.0, 1.0);
        }
        self
    }

    /// Tune the largest-drop cut's note-position search window (defaults
    /// [`DROP_WINDOW_MIN`]`..=`[`DROP_WINDOW_MAX`]); sanitized to `min ≥ 1`
    /// and `max ≥ min`.
    pub fn with_drop_window(mut self, min: usize, max: usize) -> Self {
        let min = min.max(1);
        self.drop_window = (min, max.max(min));
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

    /// Semantic search: one row per note. The configured **context cut**
    /// decides how many (adr/0029): under `fixed`, the classic `top_k` notes;
    /// under the adaptive cuts, every note whose best chunk survives the cut
    /// (and `top_k` is ignored).
    ///
    /// Ranks the FULL pool before cutting: semantic search lists notes, but a
    /// single section-heavy note can otherwise fill every chunk slot and
    /// collapse (client-side, one row per note) to a single result.
    /// (`answer` keeps chunk-level context — this note-dedup is search-only.)
    pub async fn search(
        &self,
        collection: &CollectionKey,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<ScoredChunk>, RagError> {
        Ok(self.search_impl(collection, query, top_k, false).await?.0)
    }

    /// [`Self::search`] plus a [`CutPreview`] of where the **context cut**
    /// slices an answer's LLM context on the same ranked pool — the web UI's
    /// test-query box renders it. Applies with or without a reranker
    /// (adr/0029).
    pub async fn search_with_cut_preview(
        &self,
        collection: &CollectionKey,
        query: &str,
        top_k: usize,
    ) -> Result<(Vec<ScoredChunk>, Option<CutPreview>), RagError> {
        self.search_impl(collection, query, top_k, true).await
    }

    async fn search_impl(
        &self,
        collection: &CollectionKey,
        query: &str,
        top_k: usize,
        want_preview: bool,
    ) -> Result<(Vec<ScoredChunk>, Option<CutPreview>), RagError> {
        let raw = self.retrieve(collection, query).await?;
        let ranked = self.rank(query, raw).await;
        let pool_chunks = ranked.len();
        let cut = self.cut_len(&ranked, top_k);
        let preview = want_preview.then(|| CutPreview {
            pool_chunks,
            // Every cut keeps a best-first prefix, so the boundary is the
            // prefix edge.
            boundary: (cut > 0 && cut < pool_chunks).then(|| (ranked[cut - 1].0, ranked[cut].0)),
            context: ranked[..cut].iter().map(|(_, c)| c.clone()).collect(),
        });
        let rows = match self.context_cut {
            // fixed: top_k counts NOTES on the search surface — the classic
            // walk of the whole ranked pool until top_k distinct notes.
            // Floored to 1 like cut_len: a file-sourced zero must not
            // silently empty every search.
            ContextCut::Fixed => dedupe_by_note(ranked, top_k.max(1)),
            // adaptive: every note whose best chunk survives the cut.
            _ => {
                let mut kept = ranked;
                kept.truncate(cut);
                dedupe_by_note(kept, usize::MAX)
            }
        };
        Ok((rows, preview))
    }

    /// Question-answering: retrieves the best CHUNKS as context (the LLM wants
    /// sections, not one-per-note) and asks the configured LLM. The context
    /// cut sizes the context on the ranked pool — reranked scores when a
    /// reranker is active, vector scores otherwise (adr/0029); `top_k` only
    /// applies under the `fixed` cut. Fails with [`RagError::SemanticOnly`]
    /// when no LLM is configured; the gate runs before the vector search so no
    /// work is thrown away. `history` (prior question/answer pairs) is
    /// forwarded verbatim to the LLM call only — it never influences
    /// retrieval or ranking, which see only `question`.
    pub async fn answer(
        &self,
        collection: &CollectionKey,
        question: &str,
        history: &[(String, String)],
        top_k: usize,
    ) -> Result<Answer, RagError> {
        let llm = self.llm_client.clone().ok_or(RagError::SemanticOnly)?;
        let raw = self.retrieve(collection, question).await?; // retrieval sees ONLY the question
        let mut context = self.rank(question, raw).await;
        let cut = self.cut_len(&context, top_k);
        context.truncate(cut);
        let text = llm.ask(question, history, &context).await?;
        Ok(Answer {
            text,
            sources: context,
        })
    }

    /// Ranks the retrieved pool: the full pool through the reranker when one
    /// is active, the vector order otherwise. Any reranker misbehavior —
    /// a failed call (endpoint 503, network blip) or a malformed return
    /// (wrong count, duplicate or out-of-range index) — must not fail the
    /// request: the vector-ranked pool is already in hand, so degrade to it,
    /// same policy as a failed init at startup.
    ///
    /// The `(index, score)` pairs are validated HERE, at the one consumption
    /// point ([`validate_scored`] — count, uniqueness, range, sort), so no
    /// impl can bypass the invariants the materialization relies on. The
    /// trait's calibrated-score contract is enforced here too: a batch with
    /// scores outside `0.0..=1.0` is sigmoid-normalized (order-preserving —
    /// a plain clamp would flatten logit-scale batches into an uncuttable
    /// 1.0 plateau) and non-finite scores drop to 0.0 (NaN would poison
    /// every cut comparison), with a once-per-run warning. The reorder then
    /// moves the chunks this function already owns — no clones on the hot
    /// path.
    async fn rank(&self, query: &str, raw: Vec<ScoredChunk>) -> Vec<ScoredChunk> {
        let Some(reranker) = &self.reranker else {
            // `deduplicate_chunks` already sorted best-first.
            return raw;
        };
        let order = match reranker.rerank(query, &raw).await.and_then(|scored| {
            let (scored, violated) = sanitize_scores(scored);
            if violated
                && !self
                    .score_scale_warned
                    .swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                log::warn!(
                    "Reranker returned scores outside 0..1 — normalized (sigmoid; non-finite → 0). \
                     The context cuts assume calibrated relevance scores; check the rerank \
                     endpoint's score scale"
                );
            }
            // Re-validates (and re-sorts, now on sanitized scores) even
            // though in-tree impls already did: the trait is public and the
            // materialization below must never panic on a stranger's impl.
            validate_scored(scored, raw.len())
        }) {
            Ok(order) => order,
            Err(e) => {
                log::warn!(
                    "Reranker failed mid-query ({e:#}); falling back to plain vector ranking"
                );
                return raw;
            }
        };
        // Indices were validated above — every take() hits a full slot.
        let mut slots: Vec<Option<FlattenedChunk>> =
            raw.into_iter().map(|(_, chunk)| Some(chunk)).collect();
        order
            .into_iter()
            .map(|(index, score)| (score, slots[index].take().expect("validated unique index")))
            .collect()
    }

    /// How many chunks of the best-first `ranked` pool the configured
    /// **context cut** keeps. Every cut is a prefix cut, so a length is the
    /// whole answer — callers truncate or slice. `top_k` is floored to 1:
    /// the web form rejects 0, but a hand-edited config file bypasses that
    /// gate, and under `fixed` a zero would silently empty every result.
    fn cut_len(&self, ranked: &[ScoredChunk], top_k: usize) -> usize {
        match self.context_cut {
            ContextCut::Fixed => top_k.max(1).min(ranked.len()),
            ContextCut::ScoreRange => score_range_cut_len(ranked, self.score_range_cutoff),
            ContextCut::LargestDrop => largest_drop_cut_len(ranked, self.drop_window),
        }
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

/// The pipeline half of the reranker score contract: scores must be
/// calibrated `0..=1`. A batch violating that is sigmoid-normalized —
/// monotonic, so the ranking survives, unlike a clamp that would flatten a
/// logit-scale batch into an uncuttable plateau at 1.0 — and non-finite
/// scores drop to 0.0 (NaN survives both sigmoid and clamp, and one NaN in
/// the pool poisons every cut comparison into keeping nothing). The bool
/// reports whether a violation was seen, for the once-per-run warning.
fn sanitize_scores(scored: Vec<(usize, f64)>) -> (Vec<(usize, f64)>, bool) {
    let violated = scored
        .iter()
        .any(|(_, score)| !score.is_finite() || !(0.0..=1.0).contains(score));
    if !violated {
        return (scored, false);
    }
    let sane = scored
        .into_iter()
        .map(|(index, score)| {
            (
                index,
                if score.is_finite() {
                    sigmoid(score)
                } else {
                    0.0
                },
            )
        })
        .collect();
    (sane, true)
}

/// The [`ContextCut::ScoreRange`] cut. No count cut at all — scores are
/// min-max normalized within the (already [`CANDIDATE_POOL`]-capped) pool and
/// a chunk survives when its normalized score reaches `cutoff` (config
/// `score_range_cutoff`, default [`SCORE_RANGE_DEFAULT_CUTOFF`]): the pool's own
/// spread decides how many chunks
/// are relevant, not a fixed `top_k`. The range is measured between the
/// [`RANGE_LOW_PERCENTILE`]/[`RANGE_HIGH_PERCENTILE`] scores rather than the
/// absolute extremes (winsorized min-max), so a stray chunk at either end
/// cannot stretch the range and drag the cutoff with it; on small pools the
/// percentile indices collapse to the actual extremes. Normalization is
/// shift/scale invariant, so negative scores need no special casing; a flat
/// range carries no signal to cut on and the pool is kept whole. `sorted`
/// must be best-first, which [`deduplicate_chunks`] guarantees.
fn score_range_cut_len(sorted: &[ScoredChunk], cutoff: f64) -> usize {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    // Best-first order: the high percentile sits near the front, the low one
    // near the back. floor/ceil bias both endpoints toward the bulk.
    let high = sorted[((1.0 - RANGE_HIGH_PERCENTILE) * (n - 1) as f64).floor() as usize].0;
    let low = sorted[((1.0 - RANGE_LOW_PERCENTILE) * (n - 1) as f64).ceil() as usize].0;
    let range = high - low;
    if range <= 0.0 {
        return n;
    }
    let cutoff = low + cutoff * range;
    sorted
        .iter()
        .take_while(|(score, _)| *score >= cutoff)
        .count()
}

/// The [`ContextCut::LargestDrop`] cut: find the biggest RELATIVE drop
/// between consecutive scores — `(s[i] − s[i+1]) / s[i]` — and cut there; the
/// drop's position alone decides the kept count. Relative (not absolute)
/// because absolute gaps scale with the embedder's score band, so the same
/// shape of pool would cut differently per model; a ratio to the preceding
/// score is scale-invariant. The window is the *search range* for the drop
/// (note positions [`DROP_WINDOW_MIN`]`..=`[`DROP_WINDOW_MAX`]), not a clamp:
/// it stops degenerate elbows (a spiky top yielding a 1-chunk context,
/// phantom gaps deep in the noise tail), and a window with no drop at all
/// means no evidence of a relevance boundary — the pool stays whole, same
/// philosophy as [`score_range_cut`]'s flat rule. Ties cut at the earliest
/// drop (precision over recall). The note that closes the winning gap is
/// kept too — a deliberate recall bump at the boundary. A non-positive base
/// score makes the ratio meaningless (zero division, flipped sign) and the
/// scores are sorted, so the search stops at the first one. `sorted` must be
/// best-first, which [`deduplicate_chunks`] guarantees.
///
/// The gap is searched over each distinct NOTE's best score, not the raw
/// chunk pool: a multi-section note interleaves its extra sections between
/// other notes' scores, filling every chunk-level gap with a staircase of
/// tiny steps that structurally masks the real elbow. The kept context is
/// then every pool chunk at or above the gap-closing note's best score, so
/// kept notes' extra sections ride along.
fn largest_drop_cut_len(sorted: &[ScoredChunk], (window_min, window_max): (usize, usize)) -> usize {
    use std::collections::HashSet;

    // First occurrence per note in the best-first pool = that note's best.
    let mut seen: HashSet<&str> = HashSet::new();
    let note_best: Vec<f64> = sorted
        .iter()
        .filter(|(_, chunk)| seen.insert(chunk.doc_path.as_str()))
        .map(|(score, _)| *score)
        .collect();

    // (relative drop, notes above the gap)
    let mut best: Option<(f64, usize)> = None;
    for keep in window_min.max(1)..=window_max.min(note_best.len().saturating_sub(1)) {
        let base = note_best[keep - 1];
        if base <= 0.0 {
            break;
        }
        let drop = (base - note_best[keep]) / base;
        if drop > 0.0 && best.is_none_or(|(largest, _)| drop > largest) {
            best = Some((drop, keep));
        }
    }
    match best {
        Some((_, keep)) => {
            // `keep` notes above the gap + the gap-closing one below it: keep
            // every chunk scoring at least that note's best (a prefix — the
            // pool is sorted).
            let boundary = note_best[keep];
            sorted
                .iter()
                .take_while(|(score, _)| *score >= boundary)
                .count()
        }
        None => sorted.len(),
    }
}

/// Deduplicates embedding results by FlattenedChunk (keeping the highest score
/// per unique chunk) and returns them sorted best-first. Scores are similarities
/// (higher = better) for both backends, so when no reranker reorders the pool
/// this ordering is what the context cuts ([`score_range_cut_len`],
/// [`largest_drop_cut_len`], `search`'s note-dedup) rely on.
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
    // The HashMap destroyed the query's ordering; restore best-first — the
    // prefix cuts assume it when no reranker reorders the pool.
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
        async fn ask(
            &self,
            _: &str,
            _: &[(String, String)],
            _: &[ScoredChunk],
        ) -> anyhow::Result<String> {
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
        // (Fixed cut — the strategy where top_k governs the search surface.)
        let rag = rag(section_heavy_store(), false).with_context_cut(ContextCut::Fixed);
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
        // Fixed cut: this test is about chunk dedup, not the adaptive cut.
        let out = rag(store, false)
            .with_context_cut(ContextCut::Fixed)
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
        // scores are min-max normalized within the pool and everything at or
        // above 0.4 survives. Here min 0.70 / max 0.99 → raw cutoff 0.816, so
        // 0.99/0.98/0.97 stay while 0.80 (normalized 0.34) and 0.70 (0.0)
        // drop, even though the request asked for top_k = 2.
        let rag = rag(section_heavy_store(), true);
        let answer = rag.answer(&key("vault-1"), "q", &[], 2).await.unwrap();
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
        // below the 0.4 line, out, regardless of the requested top_k.
        let store = FakeVectorStore {
            results: vec![
                (0.90, chunk("/a.md", "a1")),
                (0.40, chunk("/b.md", "b1")),
                (0.30, chunk("/c.md", "c1")),
            ],
            ..Default::default()
        };
        let answer = rag(store, true)
            .answer(&key("vault-1"), "q", &[], 2)
            .await
            .unwrap();
        assert_eq!(answer.sources.len(), 1);
        assert_eq!(answer.sources[0].1.doc_path, "/a.md");
    }

    #[test]
    fn score_range_cut_is_shift_invariant_and_keeps_flat_pools_whole() {
        // Min-max normalization only sees relative position, so negative
        // scores need no special casing: -0.1 normalizes to 1.0, -0.5 to 0.0.
        let pool: Vec<ScoredChunk> = vec![(-0.1, chunk("/a.md", "a")), (-0.5, chunk("/b.md", "b"))];
        assert_eq!(score_range_cut_len(&pool, SCORE_RANGE_DEFAULT_CUTOFF), 1);

        // Flat scores carry no signal to cut on — everything stays.
        let flat: Vec<ScoredChunk> = vec![
            (0.7, chunk("/a.md", "a")),
            (0.7, chunk("/b.md", "b")),
            (0.7, chunk("/c.md", "c")),
        ];
        assert_eq!(score_range_cut_len(&flat, SCORE_RANGE_DEFAULT_CUTOFF), 3);

        assert_eq!(score_range_cut_len(&[], SCORE_RANGE_DEFAULT_CUTOFF), 0);
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
    fn score_range_cut_is_not_stretched_by_extreme_outliers() {
        // One garbage chunk at 0.05 would stretch a plain min-max range
        // (cutoff 0.05 + 0.4×0.80 = 0.37 → nearly everything survives). The
        // percentile endpoints ignore it: over 21 chunks, high = idx 1
        // (0.84), low = idx 19 (0.60) → cutoff 0.60 + 0.4×0.24 = 0.696 →
        // only the real head (0.85/0.84/0.75/0.70) survives.
        let mut scores = vec![0.85, 0.84, 0.75, 0.70];
        scores.extend([0.65; 15]);
        scores.extend([0.60, 0.05]);
        assert_eq!(
            score_range_cut_len(&pool(&scores), SCORE_RANGE_DEFAULT_CUTOFF),
            4
        );

        // The cutoff is tunable (config score_range_cutoff): at 0.5 the raw
        // line moves to 0.60 + 0.5×0.24 = 0.72 and the 0.70 chunk drops.
        assert_eq!(score_range_cut_len(&pool(&scores), 0.5), 3);
    }

    /// The default largest-drop window in tuple form, for direct cut tests.
    const DW: (usize, usize) = (DROP_WINDOW_MIN, DROP_WINDOW_MAX);

    #[test]
    fn largest_drop_cut_cuts_at_the_biggest_gap_in_the_window() {
        // Biggest gap sits between positions 5 and 6 → keep the 5 above it
        // plus the element that closes the gap → 6.
        let p = pool(&[0.90, 0.89, 0.88, 0.87, 0.86, 0.50, 0.49, 0.48]);
        assert_eq!(largest_drop_cut_len(&p, DW), 6);

        // The window is a config knob now: narrowing it to 3..=4 hides the
        // big gap and the largest in-window drop (0.87→0.86) wins instead —
        // keep through its gap-closer.
        assert_eq!(largest_drop_cut_len(&p, (3, 4)), 5);
    }

    #[test]
    fn largest_drop_cut_only_searches_the_window() {
        // The window is where the drop is SEARCHED, not a keep-clamp. A cliff
        // between positions 2 and 3 (cut would keep 2 < window min) is not a
        // candidate; the window itself is flat → no elbow → whole pool stays.
        let p = pool(&[0.90, 0.90, 0.20, 0.20, 0.20, 0.20, 0.20]);
        assert_eq!(largest_drop_cut_len(&p, DW), 7);

        // Same beyond the far edge: flat through position 30, cliff at 31.
        let mut scores = vec![0.9; 31];
        scores.extend([0.1; 4]);
        assert_eq!(largest_drop_cut_len(&pool(&scores), DW), 35);
    }

    #[test]
    fn largest_drop_cut_sees_through_same_note_staircases() {
        // /a.md's extra sections (.74/.69/.65) fill the note-level gap with
        // small chunk steps: a chunk-level search would pick .62→.55
        // (rel 0.113) as the biggest window gap and keep everything. The gap
        // must be searched over each note's BEST score — .90/.85/.80/.62/.55
        // — where .80→.62 (rel 0.225) wins; the gap-closing note /d.md joins
        // (boundary .62) and every chunk at or above it rides along.
        let p = vec![
            (0.90, chunk("/a.md", "s1")),
            (0.85, chunk("/b.md", "s1")),
            (0.80, chunk("/c.md", "s1")),
            (0.74, chunk("/a.md", "s2")),
            (0.69, chunk("/a.md", "s3")),
            (0.65, chunk("/a.md", "s4")),
            (0.62, chunk("/d.md", "s1")),
            (0.55, chunk("/e.md", "s1")),
        ];
        let kept = largest_drop_cut_len(&p, DW);
        assert_eq!(kept, 7, "everything ≥ the gap-closing note's .62");
        assert!(p[..kept].iter().all(|(s, _)| *s >= 0.62));
    }

    #[test]
    fn largest_drop_cut_ties_cut_earliest_and_small_pools_stay_whole() {
        // Two equal RELATIVE drops of 0.5 — (0.80−0.40)/0.80 at gap 3 and
        // (0.30−0.15)/0.30 at gap 5 → the earliest wins, plus the gap-closing
        // element → 4.
        let p = pool(&[0.90, 0.89, 0.80, 0.40, 0.30, 0.15, 0.10]);
        assert_eq!(largest_drop_cut_len(&p, DW), 4);

        // At or below the window start there is nothing to search.
        assert_eq!(largest_drop_cut_len(&pool(&[0.9, 0.2, 0.1]), DW), 3);
        assert_eq!(largest_drop_cut_len(&[], DW), 0);

        // Relative gaps need a positive base: non-positive scores can't form
        // an elbow, so an all-negative pool stays whole.
        let negatives = pool(&[-0.1, -0.2, -0.3, -0.4, -0.5]);
        assert_eq!(largest_drop_cut_len(&negatives, DW), 5);
    }

    /// A reranker whose every call fails — the mid-query degradation path.
    struct FailingReranker;

    #[async_trait::async_trait]
    impl Reranker for FailingReranker {
        async fn rerank(
            &self,
            _query: &str,
            _results: &[ScoredChunk],
        ) -> anyhow::Result<Vec<(usize, f64)>> {
            anyhow::bail!("rerank endpoint returned 503")
        }
    }

    #[tokio::test]
    async fn search_and_answer_degrade_to_vector_ranking_when_the_reranker_fails() {
        // search: falls back to the vector-ranked pool, and the cut still
        // applies — score-range keeps the three /a.md chunks of the
        // 0.99/0.98/0.97/0.80/0.70 store → one surviving note.
        let searcher = rag(section_heavy_store(), true).with_reranker(Arc::new(FailingReranker));
        let out = searcher.search(&key("v"), "q", 2).await.unwrap();
        assert_eq!(out.len(), 1, "request succeeds on the raw pool");
        assert_eq!(out[0].1.doc_path, "/a.md");

        // answer: same fallback, same cut → 3 chunks of context.
        let answerer = rag(section_heavy_store(), true).with_reranker(Arc::new(FailingReranker));
        let answer = answerer.answer(&key("v"), "q", &[], 2).await.unwrap();
        assert_eq!(answer.sources.len(), 3);
    }

    /// Scores the first chunk high and the rest low, preserving order — the
    /// cut must read THESE scores, not the vector ones.
    struct SpikyReranker;

    #[async_trait::async_trait]
    impl Reranker for SpikyReranker {
        async fn rerank(
            &self,
            _query: &str,
            results: &[ScoredChunk],
        ) -> anyhow::Result<Vec<(usize, f64)>> {
            Ok((0..results.len())
                .map(|i| (i, if i == 0 { 0.9 } else { 0.1 }))
                .collect())
        }
    }

    /// Emits raw logit-scale scores (5.0, 4.0, …) — a half-compatible
    /// backend violating the trait's 0..1 contract.
    struct LogitReranker;

    #[async_trait::async_trait]
    impl Reranker for LogitReranker {
        async fn rerank(
            &self,
            _query: &str,
            results: &[ScoredChunk],
        ) -> anyhow::Result<Vec<(usize, f64)>> {
            Ok((0..results.len()).map(|i| (i, 5.0 - i as f64)).collect())
        }
    }

    #[tokio::test]
    async fn non_finite_cutoff_is_ignored_not_poisonous() {
        // TOML accepts `nan`; a plain clamp would propagate it and every
        // score comparison would fail → all queries silently empty. The
        // builder ignores NaN and keeps the default.
        let ragged = rag(section_heavy_store(), false).with_score_range_cutoff(f64::NAN);
        let out = ragged.search(&key("v"), "q", 10).await.unwrap();
        assert_eq!(out.len(), 1, "default cutoff still applies");

        // ±inf clamp to 1.0/0.0 — the strictest/loosest cut, preserving what
        // a pre-existing `score_range_cutoff = inf` config always meant
        // rather than silently reverting it to the default.
        let strict = rag(section_heavy_store(), false).with_score_range_cutoff(f64::INFINITY);
        let out = strict.search(&key("v"), "q", 10).await.unwrap();
        assert_eq!(out.len(), 1, "inf → cutoff 1.0 keeps only the range top");

        let loose = rag(section_heavy_store(), false).with_score_range_cutoff(f64::NEG_INFINITY);
        let out = loose.search(&key("v"), "q", 10).await.unwrap();
        assert_eq!(out.len(), 3, "-inf → cutoff 0.0 keeps the whole pool");
    }

    #[tokio::test]
    async fn out_of_contract_reranker_scores_are_sigmoid_normalized() {
        // Scores 5.0/4.0/3.0 violate the 0..1 contract. The pipeline squashes
        // the whole batch through a sigmoid — order-preserving, so the cuts
        // still see the contrast (a clamp would flatten everything into an
        // uncuttable 1.0 plateau and keep the whole pool): σ(5)≈.993,
        // σ(4)≈.982, σ(3)≈.953 → score-range cutoff ≈ .969 keeps 2.
        let store = FakeVectorStore {
            results: pool(&[0.7, 0.7, 0.7]),
            ..Default::default()
        };
        let ragged = rag(store, true).with_reranker(Arc::new(LogitReranker));
        let answer = ragged.answer(&key("v"), "q", &[], 2).await.unwrap();
        assert_eq!(answer.sources.len(), 2, "normalized scores keep the cut");
        assert!(
            answer
                .sources
                .iter()
                .all(|(s, _)| (0.0..=1.0).contains(s) && *s < 1.0)
        );
    }

    /// First chunk gets a NaN score — the value that survives both sigmoid
    /// and clamp, and poisons every cut comparison if it reaches the pool.
    struct NanReranker;

    #[async_trait::async_trait]
    impl Reranker for NanReranker {
        async fn rerank(
            &self,
            _query: &str,
            results: &[ScoredChunk],
        ) -> anyhow::Result<Vec<(usize, f64)>> {
            Ok((0..results.len())
                .map(|i| {
                    (
                        i,
                        if i == 0 {
                            f64::NAN
                        } else {
                            0.9 - i as f64 * 0.1
                        },
                    )
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn nan_reranker_scores_do_not_empty_results() {
        // A NaN score used to slip through the clamp, sort to the pool head,
        // and turn every cut comparison false — all queries silently empty.
        // Sanitizing maps it to 0.0 (bottom); the rest of the batch ranks.
        let store = FakeVectorStore {
            results: pool(&[0.7, 0.7, 0.7]),
            ..Default::default()
        };
        let ragged = rag(store, true).with_reranker(Arc::new(NanReranker));
        let answer = ragged.answer(&key("v"), "q", &[], 3).await.unwrap();
        assert!(!answer.sources.is_empty(), "NaN must not empty the results");
        assert!(answer.sources.iter().all(|(s, _)| s.is_finite()));
    }

    /// A broken impl that skipped validation: one duplicate index per pair.
    struct DuplicateIndexReranker;

    #[async_trait::async_trait]
    impl Reranker for DuplicateIndexReranker {
        async fn rerank(
            &self,
            _query: &str,
            results: &[ScoredChunk],
        ) -> anyhow::Result<Vec<(usize, f64)>> {
            Ok((0..results.len()).map(|_| (0, 0.5)).collect())
        }
    }

    /// An impl still following the OLD trait contract: returns only its
    /// top-1 pair instead of one per input.
    struct ShortReranker;

    #[async_trait::async_trait]
    impl Reranker for ShortReranker {
        async fn rerank(
            &self,
            _query: &str,
            _results: &[ScoredChunk],
        ) -> anyhow::Result<Vec<(usize, f64)>> {
            Ok(vec![(0, 0.9)])
        }
    }

    #[tokio::test]
    async fn malformed_reranker_output_degrades_instead_of_panicking() {
        // rank() re-validates at the consumption point: a duplicate index or
        // a short return (both bypassing validate_scored) must fall back to
        // the vector-ranked pool — never panic, never silently shrink it.
        // On section_heavy_store's vector order, score-range keeps the three
        // /a.md chunks → one search row, same as the failing-reranker test.
        let dup = rag(section_heavy_store(), true).with_reranker(Arc::new(DuplicateIndexReranker));
        let out = dup.search(&key("v"), "q", 2).await.unwrap();
        assert_eq!(out.len(), 1, "duplicate index → vector-order fallback");
        assert_eq!(out[0].1.doc_path, "/a.md");

        let short = rag(section_heavy_store(), true).with_reranker(Arc::new(ShortReranker));
        let out = short.search(&key("v"), "q", 2).await.unwrap();
        assert_eq!(out.len(), 1, "short return → vector-order fallback");
    }

    #[tokio::test]
    async fn zero_top_k_from_a_config_file_is_floored() {
        // apply_form rejects top_k = 0, but a hand-edited TOML bypasses that
        // gate — the pipeline floors to 1 instead of silently emptying every
        // fixed-cut result.
        let fixed = rag(section_heavy_store(), true).with_context_cut(ContextCut::Fixed);
        assert_eq!(fixed.search(&key("v"), "q", 0).await.unwrap().len(), 1);
        assert_eq!(
            fixed
                .answer(&key("v"), "q", &[], 0)
                .await
                .unwrap()
                .sources
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn the_cut_runs_on_reranked_scores() {
        // Vector scores are flat (no cut signal at all); the reranker's spiky
        // scores are what the score-range cut must see → 1-chunk context,
        // top_k ignored (adr/0029).
        let store = || FakeVectorStore {
            results: pool(&[0.7, 0.7, 0.7, 0.7, 0.7]),
            ..Default::default()
        };
        let adaptive = rag(store(), true).with_reranker(Arc::new(SpikyReranker));
        let answer = adaptive.answer(&key("v"), "q", &[], 4).await.unwrap();
        assert_eq!(answer.sources.len(), 1, "cut on reranked scores");
        assert_eq!(answer.sources[0].0, 0.9);

        // fixed keeps exactly top_k reranked chunks — the classic behavior.
        let fixed = rag(store(), true)
            .with_reranker(Arc::new(SpikyReranker))
            .with_context_cut(ContextCut::Fixed);
        let answer = fixed.answer(&key("v"), "q", &[], 4).await.unwrap();
        assert_eq!(answer.sources.len(), 4);

        // search with a reranker: only the surviving note shows.
        let searcher = rag(store(), false).with_reranker(Arc::new(SpikyReranker));
        let rows = searcher.search(&key("v"), "q", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn search_cut_preview_marks_the_answer_context() {
        // No reranker: the preview says how many pooled chunks an answer
        // would keep, and which displayed rows are part of that context.
        let searcher = rag(section_heavy_store(), false);
        let (hits, preview) = searcher
            .search_with_cut_preview(&key("v"), "q", 10)
            .await
            .unwrap();
        let preview = preview.expect("no reranker → preview present");
        assert_eq!(preview.pool_chunks, 5);
        assert_eq!(
            preview.context.len(),
            3,
            "score-range keeps 0.99/0.98/0.97 of the 0.99..0.70 pool"
        );
        assert_eq!(
            preview.boundary,
            Some((0.97, 0.80)),
            "boundary = last kept / first dropped pool score"
        );
        // The search surface now shows exactly the surviving notes: /a.md's
        // best chunk is in the context, /b.md (0.80) and /c.md (0.70) fell
        // below the cut — one row.
        assert_eq!(hits.len(), 1);
        assert!(preview.context.contains(&hits[0].1));

        // With a reranker attached the cut (and its preview) still applies —
        // here the reranker fails, so the cut runs on the vector order.
        let reranked = rag(section_heavy_store(), false).with_reranker(Arc::new(FailingReranker));
        let (_, preview) = reranked
            .search_with_cut_preview(&key("v"), "q", 10)
            .await
            .unwrap();
        assert!(preview.is_some(), "preview on the reranked path too");
    }

    #[tokio::test]
    async fn answer_uses_the_configured_context_cut() {
        // Scores where the two algorithms disagree: score-range keeps 2
        // (min 0.84 / max 1.0 → cutoff 0.904, so 1.00 and 0.93 clear it),
        // largest-drop keeps 5 (relative gaps: 0.02/0.88 at gap 3 vs the
        // slightly larger 0.02/0.86 at gap 4, plus the gap-closing element;
        // the bigger drops at positions 1 and 2 are outside the search
        // window).
        let scores = [1.00, 0.93, 0.88, 0.86, 0.84];
        let store = || FakeVectorStore {
            results: pool(&scores),
            ..Default::default()
        };
        let default_cut = rag(store(), true);
        assert_eq!(
            default_cut
                .answer(&key("v"), "q", &[], 2)
                .await
                .unwrap()
                .sources
                .len(),
            2,
            "default is score-range"
        );

        let elbow = rag(store(), true).with_context_cut(ContextCut::LargestDrop);
        assert_eq!(
            elbow
                .answer(&key("v"), "q", &[], 2)
                .await
                .unwrap()
                .sources
                .len(),
            5
        );
    }

    #[tokio::test]
    async fn answer_on_semantic_only_server_is_a_typed_rejection() {
        let rag = rag(section_heavy_store(), false);
        assert!(!rag.can_answer());
        match rag.answer(&key("vault-1"), "q", &[], 5).await {
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
