//! Core of the Kimün note-taking app: all file operations, indexing, and
//! note manipulation, with no presentation concerns.
//!
//! The public surface is the [`NoteVault`] facade. A vault is a directory of
//! Markdown notes on disk; `NoteVault` owns the operations over it (create,
//! read, write, rename, search, browse) and is cheap to clone.
//!
//! A few conventions shape this crate:
//!
//! - **`VaultPath` for vault-internal paths.** Everything addressed *inside*
//!   the vault uses [`VaultPath`], a case-insensitive,
//!   `/`-separated, OS-agnostic path. OS path types are reserved for
//!   configuration values (the workspace root) and for converting back to a
//!   real filesystem location when a caller needs one.
//! - **The index is a cache.** Search, backlinks, suggestions, and listings
//!   are served from a SQLite index that mirrors the notes on disk. It can be
//!   rebuilt at any time from the files, which remain the source of truth.
//! - **`nfs` and `system` own the filesystem.** Every direct
//!   `std::fs`/`tokio::fs` call lives in one of them: [`nfs`] for vault-scoped
//!   work (notes, addressed by [`VaultPath`]) and [`system`]
//!   for host-scoped work (the app's own directories, cross-volume moves).
//!   The rest of the crate goes through them.
//!
//! # Example
//!
//! Create a vault in a temporary directory, write a note, and read it back:
//!
//! ```no_run
//! use kimun_core::{NoteVault, SystemPath, VaultConfig};
//! use kimun_core::nfs::VaultPath;
//!
//! let dir = tempfile::tempdir().unwrap();
//! let root = SystemPath::try_absolute(dir.path()).unwrap();
//! let rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let vault = NoteVault::new(VaultConfig::new(root)).await.unwrap();
//!     let path = VaultPath::new("/hello.md");
//!     vault.create_note(&path, "# Hello\n").await.unwrap();
//!     let text = vault.get_note_text(&path).await.unwrap();
//!     assert_eq!(text, "# Hello\n");
//! });
//! ```

pub(crate) mod dates;
/// Error types returned across the crate's public API.
pub mod error;
pub(crate) mod index;
/// Vault-scoped filesystem layer: notes, attachments and backups inside one
/// workspace, addressed by the [`VaultPath`] vault-internal path type. Paired
/// with [`system`], which owns host-scoped paths and operations.
pub mod nfs;
/// Note model: parsing Markdown into details, chunks, links, and tags.
pub mod note;
pub(crate) mod note_locks;
pub(crate) mod note_rename;
pub(crate) mod sync;
/// Host-scoped paths and file operations: the machine kimün runs on, its
/// directories, and the file operations carrying OS-specific knowledge.
pub mod system;
mod vault_attachments;
mod vault_browse;
mod vault_config;
mod vault_dates;
mod vault_entries;
mod vault_guard;
mod vault_journal;
mod vault_links;
mod vault_note_edits;
mod vault_pinned_notes;
mod vault_properties;
mod vault_results;
mod vault_saved_searches;
mod vault_search;
mod vault_wiki;
mod wiki;
pub use index::file::IndexFile;
pub use index::search_terms::{
    expand_bare_note_prefixes, query_has_unterminated_quote, query_token_spans, quote_query_term,
    strip_order_directive, with_order_directive, OrderBy, OrderField, PropertyFilter, PropertyOp,
    PropertyTest, QueryTokenClass, QueryTokenSpan, SearchTerms, MAX_QUERY_LEN,
};
pub use index::{
    IndexDiff, IndexObserver, NoteChange, NoteSuggestion, PropertySortValue, TagSuggestion,
};
pub use nfs::pinned_notes::{PinOutcome, PinToggle, PINNED_NOTES_CAP};
pub use nfs::saved_searches::{saved_search_name_matches, SavedSearch};
pub use nfs::vault_id::VaultId;
pub use nfs::wiki_declaration::WikiDeclaration;
pub use nfs::EntryKind;
pub use system::{image_extension, Host, ImageCache, SystemPath, IMAGE_CACHE_CAP};
pub use vault_browse::{NotesValidation, VaultBrowseOptions, VaultBrowseOptionsBuilder};
pub use vault_config::{
    VaultConfig, DEFAULT_ASSETS_PATH, DEFAULT_INBOX_PATH, DEFAULT_JOURNAL_PATH,
};
pub use vault_dates::{NoteDate, JOURNAL_DATE_SOURCE};
pub use vault_results::{
    AttachmentContent, AttachmentDetails, DirectoryDetails, LinkResolution, ReplacePreview,
    ResultType, SearchResult,
};
pub use vault_search::{DateFilter, DateSource};
pub use wiki::{
    Catalogue, CatalogueEntry, Severity, SkippedSource, StampReport, WikiFinding, WikiInitEntry,
    WikiInitOptions, WikiInitReport, WikiReport, WikiRule,
};

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use error::{FSError, VaultError};
use index::NoteIndex;
use log::debug;
use nfs::{NoteEntryData, VaultPath};
use note::{ContentChunk, NoteContentData, NoteDetails};
use note_locks::NoteLocks;
use sync::VaultSync;
use system::path_to_string;

use crate::nfs::DirectoryEntryData;

/// How many incremented names ([`VaultPath::get_name_on_conflict`]) a
/// conflict-avoiding create tries before giving up. High enough that no real
/// vault reaches it, low enough to bound the work when something pathological
/// (a name every increment collides with) would otherwise spin forever. Shared
/// by [`NoteVault::free_note_path`] and
/// [`NoteVault::create_note_avoiding_conflicts`] so the name they pick agrees.
pub const MAX_CONFLICT_ATTEMPTS: usize = 100;

/// Timing summary of an indexing pass.
pub struct IndexReport {
    /// When the pass started.
    pub start: SystemTime,
    /// How long the pass took (zero until the pass finishes).
    pub duration: Duration,
}

impl IndexReport {
    fn new() -> Self {
        let start = SystemTime::now();
        Self {
            start,
            duration: Duration::default(),
        }
    }

    fn finish(&mut self) {
        let time = SystemTime::now();
        let duration = time.duration_since(self.start).unwrap_or_default();
        self.duration = duration;
    }
}

/// Facade over a vault: a directory of Markdown notes plus its searchable
/// index. Cheap to clone — clones share the index pool and per-note locks.
#[derive(Debug, Clone)]
pub struct NoteVault {
    /// A [`SystemPath`], so everything below this line knows the vault root is
    /// absolute and normalized rather than taking the caller's word for it —
    /// the same guarantee [`VaultConfig`] demands at construction, carried
    /// inward instead of dropped at the door.
    ///
    /// Behind an `Arc` because `Arc::clone` is a refcount bump for the fan-out
    /// tasks (backlink rewrites, indexing) that each need the root.
    workspace_path: Arc<SystemPath>,
    journal_path: VaultPath,
    inbox_path: VaultPath,
    /// The vault's searchable note index. Crate-visible so the index module's
    /// own tests can exercise vault-level flows against index internals.
    pub(crate) index: NoteIndex,
    /// Whether destructive writes back up the previous content first. Mirrors
    /// [`VaultConfig::backup`]; see its docs.
    backup: bool,
    /// Whether automated edits refuse to change existing entries in the
    /// declared sources folder. Mirrors [`VaultConfig::protect_sources`].
    protect_sources: bool,
    /// Per-note in-process write locks; see [`NoteLocks`]. Shared across
    /// clones.
    note_locks: NoteLocks,
    /// The vault id, read from disk once and then served from memory — every
    /// RAG query surface asks for it. Shared across clones; the id is stable
    /// for the life of the vault, so caching cannot go stale.
    vault_id: Arc<tokio::sync::OnceCell<nfs::vault_id::VaultId>>,
}

// SqlitePool doesn't implement PartialEq; two vaults are equivalent when they
// point at the same workspace.
impl PartialEq for NoteVault {
    fn eq(&self, other: &Self) -> bool {
        self.workspace_path == other.workspace_path
    }
}

impl NoteVault {
    /// Creates a new instance of the Note Vault.
    /// Make sure you call `NoteVault::validate_and_init(&self)` to initialize the DB index if
    /// needed.
    pub async fn new(config: VaultConfig) -> Result<Self, VaultError> {
        debug!("Creating new vault Instance");
        let backup = config.backup;
        let protect_sources = config.protect_sources;
        let workspace_path = config.workspace_path;
        if !workspace_path.exists() {
            return Err(VaultError::VaultPathNotFound {
                path: path_to_string(&workspace_path),
            });
        }
        if !workspace_path.is_dir() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: path_to_string(&workspace_path),
                message: "Path provided is not a directory".to_string(),
            }));
        };

        let index_file = config
            .index
            .unwrap_or_else(|| IndexFile::legacy_in_workspace(&workspace_path));
        let index = NoteIndex::open(&index_file).await?;
        let note_vault = Self {
            workspace_path: Arc::new(workspace_path),
            journal_path: VaultPath::new(DEFAULT_JOURNAL_PATH),
            inbox_path: VaultPath::new(DEFAULT_INBOX_PATH),
            index,
            backup,
            protect_sources,
            note_locks: NoteLocks::default(),
            vault_id: Arc::new(tokio::sync::OnceCell::new()),
        };
        Ok(note_vault)
    }

    /// OS path to the workspace root (filesystem root of this vault).
    pub fn workspace_path(&self) -> &SystemPath {
        &self.workspace_path
    }

    /// Registers an [`IndexObserver`] that is notified of every note change the
    /// index records (see [`NoteChange`]). Zero-or-one per vault; registering
    /// again replaces the previous observer. The RAG client is its first
    /// consumer.
    pub fn set_index_observer(&self, observer: Arc<dyn IndexObserver>) {
        self.index.set_observer(observer);
    }

    /// Removes the registered [`IndexObserver`], so no consumer keeps receiving
    /// change events (e.g. when the RAG client for this vault is torn down).
    pub fn clear_index_observer(&self) {
        self.index.clear_observer();
    }

    /// Removes the registered [`IndexObserver`] only if it is exactly `observer`
    /// (by identity). A consumer whose observer has since been replaced by a
    /// newer one leaves that newer observer in place. Use this on teardown so a
    /// superseded handle doesn't deregister its successor.
    pub fn clear_index_observer_if(&self, observer: &Arc<dyn IndexObserver>) {
        self.index.clear_observer_if(observer);
    }

    /// This vault's stable [`VaultId`], read from `.kimun/vault-id` (or
    /// generated and persisted there) on first call and cached in memory after
    /// that. Survives renames and moves, and keys the vault's collection on
    /// the RAG server.
    pub async fn vault_id(&self) -> Result<VaultId, VaultError> {
        let id = self
            .vault_id
            .get_or_try_init(|| nfs::vault_id::read_or_create_vault_id(self.workspace_path()))
            .await?;
        Ok(*id)
    }

    /// `false` when opening the vault self-healed the index schema (missing,
    /// outdated, or invalid), meaning the index is valid but
    /// empty until a sync pass ([`validate_and_init`](Self::validate_and_init))
    /// fills it. Fast paths use this to refuse to operate against an empty
    /// index without paying for a sync.
    pub fn index_ready(&self) -> bool {
        self.index.ready()
    }

    /// Walks the entire vault checking for case-insensitive name collisions.
    /// Runs on a blocking thread because it does synchronous filesystem I/O.
    async fn fail_on_case_conflicts(&self) -> Result<(), VaultError> {
        let workspace = self.workspace_path.clone();
        let conflicts = tokio::task::spawn_blocking(move || nfs::check_case_conflicts(&workspace))
            .await
            .map_err(|e| VaultError::TaskJoin(format!("case-conflict scan: {}", e)))?;
        if !conflicts.is_empty() {
            return Err(VaultError::CaseConflict { conflicts });
        }
        Ok(())
    }
    /// Brings the index in step with the vault on disk. Opening the vault
    /// already self-healed the index schema, so all that remains
    /// is a sync pass: a quick existence scan when the index was already
    /// current, or a full scan when it was just healed (and is thus empty).
    /// This can be slow on large vaults.
    pub async fn validate_and_init(&self) -> Result<IndexReport, VaultError> {
        self.fail_on_case_conflicts().await?;
        if self.index.ready() {
            // We only check if there are new notes
            self.index_notes(NotesValidation::None).await
        } else {
            debug!("Index was healed on open — running a full sync");
            self.index_notes(NotesValidation::Full).await
        }
    }

    /// Deletes all the cached data from the index by recreating its schema,
    /// then rebuilds it with a full sync pass.
    pub async fn recreate_index(&self) -> Result<IndexReport, VaultError> {
        self.fail_on_case_conflicts().await?;
        let index_report = IndexReport::new();
        debug!("Recreating index from Vault request");
        self.index.recreate().await?;
        debug!("Tables created, creating index");
        self.int_index_notes(index_report, NotesValidation::Full)
            .await
    }

    /// Traverses the whole vault directory and verifies the notes to
    /// update the cached data in the DB. The validation is defined by
    /// the validation mode:
    ///
    /// NotesValidation::Full Checks the content of the note by comparing a hash based on the text
    /// conatined in the file.
    /// NotesValidation::Fast Checks the size of the file to identify if the note has changed and
    /// then update the DB entry.
    /// NotesValidation::None Checks if the note exists or not.
    pub async fn index_notes(
        &self,
        validation_mode: NotesValidation,
    ) -> Result<IndexReport, VaultError> {
        let index_report = IndexReport::new();
        self.int_index_notes(index_report, validation_mode).await
    }

    async fn int_index_notes(
        &self,
        mut index_report: IndexReport,
        validation_mode: NotesValidation,
    ) -> Result<IndexReport, VaultError> {
        VaultSync::new(&self.index, self.workspace_path())
            .run(&VaultPath::root(), true, validation_mode, None)
            .await?;
        // A whole-vault sync just completed: the index mirrors the disk, so
        // the readiness probe reports true even when this instance healed or
        // recreated the schema earlier.
        self.index.mark_synced();
        index_report.finish();
        debug!("TIME: {}", index_report.duration.as_secs());
        Ok(index_report)
    }

    /// Returns true if the path resolves to anything (note, directory, attachment)
    /// on disk. Cheaper than loading the full entry when only existence matters.
    pub async fn exists(&self, path: &VaultPath) -> bool {
        nfs::path_exists(self.workspace_path(), path)
            .await
            .unwrap_or(false)
    }

    /// Loads the note at `path` if it exists; otherwise creates it with `default_text`
    /// (or empty if `None`) and returns that text.
    /// Returns the note's text and `true` when the note had to be created (it
    /// did not exist yet), so callers can react to a fresh note — e.g. refresh
    /// a directory listing.
    pub async fn load_or_create_note(
        &self,
        path: &VaultPath,
        default_text: Option<String>,
    ) -> Result<(String, bool), VaultError> {
        match nfs::load_note(self.workspace_path(), path).await {
            Ok(text) => Ok((text, false)),
            Err(e) if e.is_not_found() => {
                let text = default_text.unwrap_or_default();
                self.create_note(path, &text).await?;
                Ok((text, true))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Loads the raw text of the note at `path`.
    ///
    /// When the file doesn't exist you get a [`VaultError::FSError`] wrapping
    /// `FSError::NotePathNotFound`; you can branch on that to lazily create a
    /// note, or use [`Self::load_or_create_note`] instead.
    pub async fn get_note_text(&self, path: &VaultPath) -> Result<String, VaultError> {
        let text = nfs::load_note(self.workspace_path(), path).await?;
        Ok(text)
    }

    /// Loads a note as [`NoteDetails`], carrying its path, raw text, and parsed
    /// metadata.
    ///
    /// Missing-file behaviour matches [`Self::get_note_text`].
    pub async fn load_note(&self, path: &VaultPath) -> Result<NoteDetails, VaultError> {
        let text = self.get_note_text(path).await?;
        Ok(NoteDetails::new(path, text))
    }

    /// Returns the indexed content chunks for the note at `path`, keyed by the
    /// note path they belong to.
    pub async fn get_note_chunks(
        &self,
        path: &VaultPath,
    ) -> Result<HashMap<VaultPath, Vec<ContentChunk>>, VaultError> {
        let a = self.index.get_notes_sections(path, false).await?;
        Ok(a)
    }

    /// Creates a new note at `path` with `text`, failing with
    /// [`VaultError::NoteExists`] if a note is already there. The create is
    /// exclusive (atomic `O_EXCL`), so it never clobbers an existing file.
    /// The new note is indexed before returning.
    pub async fn create_note<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        let entry_data = nfs::create_note_exclusive(self.workspace_path(), path, &text)
            .await
            .map_err(|e| match e {
                FSError::AlreadyExists { path } => VaultError::NoteExists { path },
                other => VaultError::FSError(other),
            })?;
        let note_details = NoteDetails::new(path, text);
        let content_data = self.index.save_note(&entry_data, &note_details).await?;
        Ok((entry_data, content_data))
    }

    /// The on-disk facts about one note — its size and modification time —
    /// without reading it.
    ///
    /// Exists so callers that need a note's mtime (the CLI's JSON output) ask
    /// the vault instead of stat-ing the path themselves: the vault is what
    /// knows where a [`VaultPath`] really lives, including the
    /// case-insensitive resolution behind it.
    pub async fn note_entry(&self, path: &VaultPath) -> Result<NoteEntryData, VaultError> {
        Ok(nfs::entry_data_at(self.workspace_path(), path).await?)
    }

    /// Ends this vault, releasing the index file's handles now instead of
    /// leaving it to whenever the last clone of the connection pool drops.
    ///
    /// Call this before renaming, moving or deleting a workspace's index —
    /// Windows refuses to touch a file that any handle still holds open, and
    /// dropping a pool only *schedules* the close. A vault opened purely to
    /// create or migrate a database should close it before the caller goes on
    /// to move that file around.
    ///
    /// Takes `self` because this is not a per-handle release: clones share one
    /// connection pool, so closing ends the index for *every* clone, which
    /// then fails with a closed-pool error on its next query. Consuming the
    /// receiver keeps that an explicit end-of-life for a vault the caller owns
    /// rather than something a stray clone can do to everyone else — clone
    /// first and close the clone and the same damage is back, so a vault
    /// shared with live readers (the TUI's `Arc<NoteVault>`) must not be
    /// closed at all.
    pub async fn close(self) {
        self.index.close().await;
    }

    /// The first note path at or after `path` that no note occupies: `path`
    /// itself when it is free, otherwise `path` with its name incremented (see
    /// [`VaultPath::get_name_on_conflict`]) until one is.
    ///
    /// Advisory, not a reservation: another writer can take the returned path
    /// before the caller gets to it. It answers "what name would this note
    /// get?" for display — the write-side guarantee is
    /// [`Self::create_note_avoiding_conflicts`], which resolves the same way
    /// but atomically. When every one of [`MAX_CONFLICT_ATTEMPTS`] candidates
    /// is taken, the last one is returned and the caller's create reports the
    /// real error.
    pub async fn free_note_path(&self, path: &VaultPath) -> VaultPath {
        let mut candidate = path.to_owned();
        for _ in 0..MAX_CONFLICT_ATTEMPTS {
            if !self.exists(&candidate).await {
                break;
            }
            candidate = candidate.get_name_on_conflict();
        }
        candidate
    }

    /// Creates a note at `path` with `text`, falling back to
    /// [`VaultPath::get_name_on_conflict`] (and further increments) for up to
    /// [`MAX_CONFLICT_ATTEMPTS`] if the name is already taken. Unlike
    /// [`Self::create_note`], this never fails on collision — it never loads
    /// or overwrites an existing note either, so content that must not be
    /// silently dropped (e.g. a saved Ask answer) always lands in a fresh
    /// note. Returns the path actually used, which may differ from `path`;
    /// callers showing a target path beforehand should pick it with
    /// [`Self::free_note_path`] so the two agree.
    pub async fn create_note_avoiding_conflicts<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<VaultPath, VaultError> {
        let mut candidate = path.to_owned();
        for _ in 0..MAX_CONFLICT_ATTEMPTS {
            match self.create_note(&candidate, &text).await {
                Ok(_) => return Ok(candidate),
                Err(VaultError::NoteExists { .. }) => {
                    candidate = candidate.get_name_on_conflict();
                }
                Err(e) => return Err(e),
            }
        }
        Err(VaultError::FSError(FSError::InvalidPath {
            path: candidate.to_string(),
            message: format!("Could not find a free name after {MAX_CONFLICT_ATTEMPTS} attempts"),
        }))
    }

    /// Creates a directory at `path`, failing with
    /// [`VaultError::DirectoryExists`] if one is already there.
    pub async fn create_directory(
        &self,
        path: &VaultPath,
    ) -> Result<DirectoryEntryData, VaultError> {
        nfs::create_directory(self.workspace_path(), path)
            .await
            .map_err(|e| match e {
                FSError::AlreadyExists { path } => VaultError::DirectoryExists { path },
                other => VaultError::FSError(other),
            })
    }

    /// Backs up the current content of `path` when this vault was opened with
    /// backups enabled (CLI/MCP), and is a no-op otherwise (TUI). Called before
    /// any destructive write so the previous content stays recoverable.
    async fn backup_if_enabled(&self, path: &VaultPath) -> Result<(), VaultError> {
        if self.backup {
            nfs::backup_note(self.workspace_path(), path).await?;
        }
        Ok(())
    }

    /// Acquires the per-note write lock for `path`; see [`NoteLocks::lock_note`].
    async fn lock_note(&self, path: &VaultPath) -> tokio::sync::OwnedMutexGuard<()> {
        self.note_locks.lock_note(path).await
    }

    /// Appends `text` to the note at `path`, creating it (with `default`
    /// content) when absent. The read and the write run under the per-note lock
    /// so concurrent appends to the same note can't lose an update. An
    /// existing source is refused when source protection is on.
    pub async fn append_to_note(
        &self,
        path: &VaultPath,
        text: &str,
        default: Option<String>,
    ) -> Result<(), VaultError> {
        let _guard = self.lock_note(path).await;
        // Checked before the create below: once the file exists, the guard
        // could no longer tell a new source from an old one.
        self.ensure_existing_not_protected(path).await?;
        let (existing, _created) = self.load_or_create_note(path, default).await?;
        let combined = if existing.is_empty() {
            text.to_string()
        } else {
            format!("{existing}\n{text}")
        };
        self.save_note_unchecked(path, combined).await?;
        Ok(())
    }

    /// Writes `text` to the note at `path`, overwriting any existing content
    /// (backing it up first when backups are enabled), and re-indexes the note.
    /// Serialized per note via the per-note write lock.
    pub async fn save_note<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        let _guard = self.lock_note(path).await;
        self.save_note_unlocked(path, text).await
    }

    /// Like [`save_note`] but assumes the caller already holds the per-note lock
    /// (see [`lock_note`]). Used by read-modify-write operations such as
    /// [`replace_in_note`] that hold the lock across both the read and the write.
    /// Refuses an existing source when source protection is on.
    async fn save_note_unlocked<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        self.ensure_existing_not_protected(path).await?;
        self.save_note_unchecked(path, text).await
    }

    /// The write behind [`save_note_unlocked`] with no source check: back up,
    /// write, and index. The caller holds the per-note lock and has made any
    /// protection check that applies.
    pub(crate) async fn save_note_unchecked<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        self.backup_if_enabled(path).await?;
        // Index what was written, not what was handed in: `save_note` restores the
        // file's own line endings, so `text` (always LF from the editor) and the
        // bytes on disk differ for a CRLF note. The scan hashes the file, so
        // hashing `text` here would disagree with it on every save.
        let (entry_data, written) = nfs::save_note(self.workspace_path(), path, &text).await?;
        let note_details = NoteDetails::new(path, written);
        let content_data = self.index.save_note(&entry_data, &note_details).await?;
        Ok((entry_data, content_data))
    }
}

#[cfg(test)]
mod tests;
