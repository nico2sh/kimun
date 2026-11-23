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
mod vault_pinned_notes;
mod vault_properties;
mod vault_saved_searches;
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
pub use nfs::EntryKind;
pub use system::{Host, SystemPath};

use std::{
    collections::HashMap,
    fmt::Display,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::{NaiveDate, Utc};
use error::{FSError, VaultError};
use futures_util::{stream, Stream, StreamExt, TryStreamExt};
use index::NoteIndex;
use log::{debug, warn};
use nfs::{NoteEntryData, VaultPath};
use note::{ContentChunk, NoteContentData, NoteDetails};
use note_locks::NoteLocks;
use note_rename::{rename_dest_err, NoteRename};
use sync::VaultSync;
use system::path_to_string;

use crate::nfs::pinned_notes;
use crate::nfs::DirectoryEntryData;

/// Default directory for journal entries, one note per day.
pub const DEFAULT_JOURNAL_PATH: &str = "/journal";
/// Default directory for quick-capture notes (see [`NoteVault::quick_note`]).
pub const DEFAULT_INBOX_PATH: &str = "/inbox";
/// Default directory for attachments (see
/// [`NoteVault::default_attachments_path`]).
pub const DEFAULT_ASSETS_PATH: &str = "/assets";

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

/// Configuration passed to [`NoteVault::new`].
///
/// `workspace_path` is the OS path to the vault's root directory.
/// `db_path` overrides where the SQLite cache is stored. When `None`,
/// the cache lives at `<workspace_path>/kimun.sqlite` (legacy default).
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// The vault's root directory on this machine.
    pub workspace_path: SystemPath,
    /// Override for the index location. When `None`, the index lives at
    /// `<workspace_path>/kimun.sqlite` (legacy default).
    pub index: Option<IndexFile>,
    /// When `true`, destructive automated edits (overwrite, replace, delete, and
    /// the backlink rewrites of rename/move) copy a note's previous content into
    /// a hidden in-vault backup directory before mutating it. The TUI leaves this
    /// off; the CLI and MCP server turn it on.
    pub backup: bool,
}

impl VaultConfig {
    /// Builds a config for the vault rooted at `workspace_path`, with the
    /// legacy default cache location and backups disabled.
    ///
    /// Takes a [`SystemPath`] rather than any `Into<PathBuf>` so a path that
    /// the OS cannot use — relative, or carrying a `.` component that Windows
    /// reads literally inside a verbatim path — cannot reach `create_dir_all`
    /// or the SQLite open. Callers holding a raw path resolve it first, and
    /// the failure lands where the path is built, not where it is opened.
    pub fn new(workspace_path: SystemPath) -> Self {
        Self {
            workspace_path,
            index: None,
            backup: false,
        }
    }

    /// Overrides where the index is stored, instead of the legacy
    /// in-workspace default.
    pub fn with_index(mut self, index: IndexFile) -> Self {
        self.index = Some(index);
        self
    }

    /// Enables or disables backing up note content before destructive edits
    /// (see the [`backup`](Self::backup) field).
    pub fn with_backup(mut self, backup: bool) -> Self {
        self.backup = backup;
        self
    }
}

/// Result of a dry-run replace ([`NoteVault::preview_replace`]): how many matches
/// would be replaced, and the note's content after the replacement. Nothing is
/// written to disk.
#[derive(Debug, Clone)]
pub struct ReplacePreview {
    /// Number of matches that would be replaced.
    pub count: usize,
    /// The note's content after the replacement.
    pub content: String,
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

    /// Directory under which journal entries are created.
    pub fn journal_path(&self) -> &VaultPath {
        &self.journal_path
    }

    /// Directory under which quick-capture notes are created.
    pub fn inbox_path(&self) -> &VaultPath {
        &self.inbox_path
    }

    /// Overrides the inbox directory used by [`Self::quick_note`].
    pub fn set_inbox_path(&mut self, path: VaultPath) {
        self.inbox_path = path;
    }

    /// Creates a timestamped note under the inbox directory. On name collision
    /// (including TOCTOU between the in-memory probe and the FS create), tries
    /// the next suffix up to `-99`. The retry loop calls `create_note`
    /// directly so each iteration's existence check is the atomic
    /// `O_EXCL` open inside `nfs::create_note_exclusive`.
    pub async fn quick_note(&self, text: &str) -> Result<NoteDetails, VaultError> {
        let base_name = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
        let candidate = |name: &str| {
            self.inbox_path
                .append(&VaultPath::note_path_from(name))
                .absolute()
        };

        for attempt in 0..=99 {
            let path = if attempt == 0 {
                candidate(&base_name)
            } else if attempt == 1 {
                continue; // attempts are labelled `name`, `name-2`, … `name-99`
            } else {
                candidate(&format!("{}-{}", base_name, attempt))
            };
            match self.create_note(&path, text).await {
                Ok(_) => return Ok(NoteDetails::new(&path, text)),
                Err(VaultError::NoteExists { .. }) => continue,
                Err(e) => return Err(e),
            }
        }

        let placeholder = candidate(&base_name);
        Err(VaultError::FSError(FSError::InvalidPath {
            path: placeholder.to_string(),
            message: "Could not find a free quick note name".to_string(),
        }))
    }

    /// Loads today's journal entry, creating it with a date heading if it does
    /// not exist yet. Returns the note details, its content, and `true` when the
    /// entry was freshly created.
    pub async fn journal_entry(&self) -> Result<(NoteDetails, String, bool), VaultError> {
        let (title, note_path) = self.get_todays_journal();
        let (content, created) = self
            .load_or_create_note(&note_path, Some(format!("# {}\n\n", title)))
            .await?;
        let details = NoteDetails::new(&note_path, &content);
        Ok((details, content, created))
    }

    fn get_todays_journal(&self) -> (String, VaultPath) {
        let today_string = crate::dates::format_iso_date(Utc::now().date_naive());

        (
            today_string.clone(),
            self.journal_path
                .append(&VaultPath::note_path_from(&today_string))
                .absolute(),
        )
    }

    /// Parses the date out of a journal note path, or `None` when `note_path`
    /// is not a `YYYY-MM-DD` note directly under the journal directory.
    pub fn journal_date(&self, note_path: &VaultPath) -> Option<NaiveDate> {
        if !note_path.is_note() {
            return None;
        }

        let (parent, _) = note_path.get_parent_path();
        if parent.eq(&self.journal_path) {
            let name = note_path.get_clean_name();
            crate::dates::parse_iso_date(&name)
        } else {
            None
        }
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

    /// Searches notes using the vault's query syntax (see [`SearchTerms`]).
    /// Returns each matching note's entry and content data.
    pub async fn search_notes<S: AsRef<str>>(
        &self,
        search_query: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let search_query = search_query.as_ref();
        let a = self.index.search(search_query).await?;
        Ok(a)
    }

    /// Returns every distinct label persisted in the vault, lowercased.
    pub async fn list_labels(&self) -> Result<Vec<String>, VaultError> {
        Ok(self.index.list_labels().await?)
    }

    /// Returns notes whose name (filename without extension) starts with
    /// `prefix`, case-insensitive, capped at `limit`. Used to feed the
    /// wikilink autocomplete popup — note that the inserted wikilink target
    /// is the `name` field, not the `path`.
    pub async fn suggest_notes_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<NoteSuggestion>, VaultError> {
        Ok(self.index.suggest_notes_by_prefix(prefix, limit).await?)
    }

    /// Returns tag labels matching `prefix` (case-insensitive) paired with
    /// usage counts, capped at `limit`. Used to feed the hashtag autocomplete
    /// popup in both the editor and the search box.
    pub async fn suggest_tags_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<TagSuggestion>, VaultError> {
        Ok(self.index.suggest_tags_by_prefix(prefix, limit).await?)
    }

    /// Returns every distinct label in the vault paired with the number of
    /// notes carrying it. Labels are returned sorted alphabetically.
    pub async fn label_counts(&self) -> Result<Vec<(String, usize)>, VaultError> {
        let rows = self.index.label_counts().await?;
        Ok(rows.into_iter().map(|(n, c)| (n, c as usize)).collect())
    }

    /// Returns every note path that carries the given label. The label
    /// argument is lowercased before lookup, matching how labels are stored.
    pub async fn notes_with_label<S: AsRef<str>>(
        &self,
        name: S,
    ) -> Result<Vec<VaultPath>, VaultError> {
        Ok(self.index.notes_with_label(name.as_ref()).await?)
    }

    /// Get notes under the given path. When `recursive` is false, only direct
    /// children are returned.
    pub async fn get_notes(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let notes = self.index.get_notes(path, recursive).await?;
        Ok(notes)
    }

    /// Counts the notes under each immediate child directory of `parent`,
    /// at any depth. Keys are canonical child directory paths; a child with
    /// no notes is absent, and notes directly in `parent` are not counted.
    pub async fn child_note_counts(
        &self,
        parent: &VaultPath,
    ) -> Result<HashMap<VaultPath, usize>, VaultError> {
        let depth = parent.canonical().get_slices().len();
        let mut counts: HashMap<VaultPath, usize> = HashMap::new();
        for (base, n) in self.index.note_counts_by_base(parent).await? {
            let mut child = VaultPath::new(base).canonical();
            let mut len = child.get_slices().len();
            if len <= depth {
                continue; // a note directly in `parent`
            }
            while len > depth + 1 {
                child = child.get_parent_path().0;
                len -= 1;
            }
            *counts.entry(child).or_insert(0) += n as usize;
        }
        Ok(counts)
    }

    /// Returns every note in the vault with its entry and content data.
    pub async fn get_all_notes(&self) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let a = self.index.get_all_notes().await?;
        Ok(a)
    }
    /// Resolves a vault path to its real OS filesystem location under the
    /// workspace root.
    pub fn path_to_pathbuf(&self, path: &VaultPath) -> PathBuf {
        path.to_pathbuf(self.workspace_path())
    }

    /// Walks the vault per `options` and yields every entry it finds — note,
    /// directory or attachment — as a [`SearchResult`], in discovery order,
    /// as soon as the walker has processed it. Nothing happens until the
    /// stream is first polled; from then on the walk runs on its own task to
    /// completion, even if the stream is dropped early (the index sync must
    /// not be left half-applied). A sync error surfaces as the stream's final
    /// `Err` item. A recursive browse from the root doubles as a full index
    /// sync: by the time the stream ends, the index reports ready.
    ///
    /// Use this when rows should appear while a slow (e.g. network) directory
    /// is still being read; [`browse_vault`](Self::browse_vault) collects the
    /// same walk into one `Vec`.
    pub fn browse_vault_stream(
        &self,
        options: VaultBrowseOptions,
    ) -> impl Stream<Item = Result<SearchResult, VaultError>> + Send + 'static {
        let vault = self.clone();
        // `once(..).flatten()` defers the spawn to the first poll, so building
        // the stream needs no runtime and an unpolled stream walks nothing.
        stream::once(async move {
            let (sender, receiver) = futures_channel::mpsc::unbounded();
            // `sender` moves into the task and drops when the walk ends, which
            // is what closes `receiver` and lets `outcome` be polled.
            let walk = tokio::spawn(async move { vault.browse_into(options, sender).await });
            let outcome = stream::once(async move {
                match walk.await {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(Err(e)),
                    Err(e) => Some(Err(VaultError::TaskJoin(format!("vault browse: {}", e)))),
                }
            })
            .filter_map(std::future::ready);
            receiver.map(Ok).chain(outcome)
        })
        .flatten()
    }

    /// Walks the vault per `options` and returns every entry it finds — the
    /// collected form of [`browse_vault_stream`](Self::browse_vault_stream),
    /// same sync side effect included.
    pub async fn browse_vault(
        &self,
        options: VaultBrowseOptions,
    ) -> Result<Vec<SearchResult>, VaultError> {
        self.browse_vault_stream(options).try_collect().await
    }

    /// The browse walk itself: syncs the subtree into the index while sending
    /// each discovered entry to `sender`, then marks a whole-vault walk synced.
    async fn browse_into(
        &self,
        options: VaultBrowseOptions,
        sender: futures_channel::mpsc::UnboundedSender<SearchResult>,
    ) -> Result<(), VaultError> {
        let start = std::time::SystemTime::now();
        debug!("> Start fetching files with Options:\n{}", options);

        // Logged here as well as returned: when the consumer dropped the
        // stream early nobody polls the outcome, and a failed sync must not
        // vanish silently.
        if let Err(e) = VaultSync::new(&self.index, self.workspace_path())
            .run(
                &options.path,
                options.recursive,
                options.validation,
                Some(sender),
            )
            .await
        {
            warn!("browse of {} failed to sync the index: {}", options.path, e);
            return Err(e);
        }

        // A recursive browse from the root is a whole-vault sync: the index
        // now mirrors the disk, so the readiness probe must report true —
        // same as int_index_notes. A partial-subtree browse must NOT mark
        // synced (the rest of a healed index could still be empty).
        if options.recursive && options.path.is_root_or_empty() {
            self.index.mark_synced();
        }

        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("> Files fetched in {} milliseconds", time.as_millis());

        Ok(())
    }

    /// Returns all subdirectories under `path`.
    /// Non-recursive returns only the immediate children; recursive returns the full tree.
    pub fn get_directories(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<DirectoryDetails>, VaultError> {
        Ok(nfs::list_directories(
            self.workspace_path(),
            path,
            recursive,
        )?)
    }

    /// Converts a note's raw Markdown into rendered Markdown and extracts all links.
    ///
    /// - WikiLinks (`[[note]]`) are converted to standard Markdown links.
    /// - Note links are resolved to vault-relative absolute paths.
    /// - Hashtags become Markdown links (`[#tag](#tag)`) and are added to the links list.
    /// - Image paths are resolved to absolute OS paths so renderers can load them directly.
    ///   Relative image paths are resolved against the note's location in the vault.
    ///   External image URLs are kept as-is.
    pub async fn get_markdown_and_links(
        &self,
        path: &VaultPath,
    ) -> Result<note::MarkdownNote, VaultError> {
        let note = self.load_note(path).await?;
        let note_parent = if note.path.is_note() {
            note.path.get_parent_path().0
        } else {
            note.path.clone()
        };
        let (md_text, mut links) = note.get_markdown_and_links();
        // Since this function is intended to return content ready to be rendered
        // We need the full path of the image links, so any markdown processor can find the image,
        // the full path can only be resolved from here as we have the vault path
        let (md_text, image_links) = note::process_image_links(&md_text, |alt_text, raw_path| {
            let resolved = if note::scan::is_remote_url(raw_path) {
                raw_path.to_string()
            } else {
                let image_vault_path = if raw_path.starts_with('/') {
                    VaultPath::new(raw_path)
                } else {
                    note_parent.append(&VaultPath::new(raw_path)).flatten()
                };
                image_vault_path
                    .to_pathbuf(self.workspace_path())
                    .display()
                    .to_string()
            };
            let link = note::NoteLink::image(&resolved, alt_text, raw_path);
            (resolved, link)
        });
        links.extend(image_links);
        Ok(note::MarkdownNote {
            text: md_text,
            links,
        })
    }

    /// Returns all notes that contain a link pointing to `path`: a path link
    /// (markdown, or a wikilink written from the root or the note's folder)
    /// to exactly this note, or a wikilink by name whose written components
    /// end this note's path (`[[dup]]`, `[[work/dup]]` for `/a/work/dup.md`)
    /// — see [`note::NoteTarget::matches`]. An ambiguous name link is a
    /// backlink of every note it can point at.
    pub async fn get_backlinks(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        Ok(self.index.get_backlinks(path).await?)
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
    /// so concurrent appends to the same note can't lose an update.
    pub async fn append_to_note(
        &self,
        path: &VaultPath,
        text: &str,
        default: Option<String>,
    ) -> Result<(), VaultError> {
        let _guard = self.lock_note(path).await;
        let (existing, _created) = self.load_or_create_note(path, default).await?;
        let combined = if existing.is_empty() {
            text.to_string()
        } else {
            format!("{existing}\n{text}")
        };
        self.save_note_unlocked(path, combined).await?;
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
    async fn save_note_unlocked<S: AsRef<str>>(
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

    /// Default attachments directory (e.g. `/assets`) inside the workspace.
    pub fn default_attachments_path(&self) -> VaultPath {
        VaultPath::new(DEFAULT_ASSETS_PATH)
    }

    /// Builds a candidate path for a new attachment under
    /// [`Self::default_attachments_path`], using `prefix` and `ext` plus the current
    /// unix-nanosecond timestamp for uniqueness. Nanoseconds (rather than
    /// millis) make same-instant collisions vanishingly unlikely for
    /// human-driven actions like clipboard paste.
    ///
    /// Does not check for collisions; callers that need stronger uniqueness
    /// guarantees should retry with [`Self::exists`] or use a different strategy.
    pub fn generate_attachment_path(&self, prefix: &str, ext: &str) -> VaultPath {
        let ts = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let filename = format!("{prefix}_{ts}.{ext}");
        self.default_attachments_path()
            .append(&VaultPath::new(filename))
    }

    /// Writes an attachment (raw bytes — e.g. an encoded PNG) to `path` under
    /// the workspace. Creates parent directories as needed. The attachment is
    /// not added to the notes index.
    pub async fn save_attachment(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), VaultError> {
        nfs::save_attachment(self.workspace_path(), path, bytes).await?;
        Ok(())
    }

    /// Where a note link leads: the notes `target` matches
    /// ([`note::NoteTarget::matches`]), looked up in the index.
    ///
    /// - [`note::NoteTarget::Name`] (a wikilink by name): every note named
    ///   like it whose path ends with the written components, in any folder,
    ///   case-insensitively; several are [`LinkResolution::Ambiguous`].
    /// - [`note::NoteTarget::Path`]: the note at exactly that path.
    ///
    /// No match is [`LinkResolution::Missing`] with the path a new note
    /// would get: a name at the vault root plus its written components
    /// (`[[work/new]]` → `/work/new.md`), a path as it is.
    pub async fn resolve_note_target(
        &self,
        target: &note::NoteTarget,
    ) -> Result<LinkResolution, VaultError> {
        let mut hits = match target {
            note::NoteTarget::Name(_) => {
                let mut hits = self.index.search_note_by_name(target.name()).await?;
                hits.retain(|(entry, _)| target.matches(&entry.path));
                hits
            }
            note::NoteTarget::Path(path) => self.index.search_note_by_path(path).await?,
        };
        Ok(match hits.len() {
            0 => LinkResolution::Missing(target.path().canonical()),
            1 => {
                let (entry, content) = hits.remove(0);
                LinkResolution::Note(entry, content)
            }
            _ => LinkResolution::Ambiguous(hits),
        })
    }

    /// The shortest wikilink target naming the note at `path` alone (no
    /// brackets, no note extension): its name when no other note shares it
    /// (`dup`), else the fewest trailing folders that tell it apart
    /// (`work/dup`), else its vault-absolute path (`/work/dup`) when even
    /// its whole path ends another note's.
    pub async fn wikilink_target_for(&self, path: &VaultPath) -> Result<String, VaultError> {
        let namesakes: Vec<VaultPath> = self
            .index
            .search_note_by_name(path.get_name())
            .await?
            .into_iter()
            .map(|(entry, _)| entry.path)
            .collect();
        Ok(note::unique_wikilink(path, 1, &namesakes))
    }

    /// Deletes the note at `path` (backing it up first when backups are
    /// enabled). The index row is removed before the file, so the index never
    /// points at a missing file.
    pub async fn delete_note(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        path.ensure_note()?;
        let _guard = self.lock_note(&path).await;
        self.backup_if_enabled(&path).await?;

        // Delete in the index first so it never points at a missing file.
        self.index.delete_notes(std::slice::from_ref(&path)).await?;

        nfs::delete_note(self.workspace_path(), &path).await?;

        // Best-effort: a pin-file failure never fails a delete that already
        // succeeded on disk and in the index.
        pinned_notes::on_note_deleted(self.workspace_path(), &path).await;

        Ok(())
    }

    /// Computes the result of replacing `pattern` with `replacement` in `text`.
    /// Pure — no I/O, no locking.
    ///
    /// `pattern` is a literal substring by default. When `regex` is `true` it is
    /// a regular expression ([`regex`] crate syntax) and `replacement` may
    /// reference capture groups (`$1`, `${name}`; use `$$` for a literal `$`).
    /// Use inline flags for line/case behaviour, e.g. `(?m)`, `(?s)`, `(?i)`.
    ///
    /// When `all` is `false` the match must be unique: returns
    /// [`VaultError::ReplaceTextNotFound`] when there is no match and
    /// [`VaultError::ReplaceTextNotUnique`] when there is more than one. When
    /// `all` is `true` every match is replaced. An invalid regex yields
    /// [`VaultError::InvalidRegex`]. `path` is only used to build the errors.
    /// Returns `(match count, new content)`.
    fn compute_replacement(
        text: &str,
        pattern: &str,
        replacement: &str,
        all: bool,
        regex: bool,
        path: &VaultPath,
    ) -> Result<(usize, String), VaultError> {
        let count;
        let updated = if regex {
            let re = regex::Regex::new(pattern).map_err(|e| VaultError::InvalidRegex {
                pattern: pattern.to_string(),
                message: e.to_string(),
            })?;
            count = re.find_iter(text).count();
            if count == 0 {
                return Err(VaultError::ReplaceTextNotFound {
                    path: path.flatten(),
                });
            }
            if !all && count > 1 {
                return Err(VaultError::ReplaceTextNotUnique {
                    path: path.flatten(),
                });
            }
            // regex `replacen` treats a limit of 0 as "replace all".
            let limit = if all { 0 } else { 1 };
            re.replacen(text, limit, replacement).into_owned()
        } else {
            count = if pattern.is_empty() {
                0
            } else {
                text.matches(pattern).count()
            };
            if count == 0 {
                return Err(VaultError::ReplaceTextNotFound {
                    path: path.flatten(),
                });
            }
            if !all && count > 1 {
                return Err(VaultError::ReplaceTextNotUnique {
                    path: path.flatten(),
                });
            }
            if all {
                text.replace(pattern, replacement)
            } else {
                text.replacen(pattern, replacement, 1)
            }
        };
        Ok((count, updated))
    }

    /// Replaces occurrences of `pattern` with `replacement` in the note at
    /// `path`, writing the result. See `compute_replacement` for the matching
    /// rules (literal vs `regex`, unique-unless-`all`, capture references).
    /// Returns the number of replacements made.
    pub async fn replace_in_note(
        &self,
        path: &VaultPath,
        pattern: &str,
        replacement: &str,
        all: bool,
        regex: bool,
    ) -> Result<usize, VaultError> {
        // Hold the per-note lock across the read and the write so a concurrent
        // in-process writer can't change the note between them (lost update).
        let _guard = self.lock_note(path).await;
        let text = self.get_note_text(path).await?;
        let (count, updated) =
            Self::compute_replacement(&text, pattern, replacement, all, regex, path)?;
        // Already holding the per-note lock; use the unlocked save so we don't
        // re-acquire it (the tokio Mutex is not reentrant).
        self.save_note_unlocked(path, updated).await?;
        Ok(count)
    }

    /// Flips the checkbox of the task on source line `line` (0-based) of the
    /// note at `path`, writing the result. `seen` is the line as the caller
    /// read it; the note is left alone and [`VaultError::TaskChanged`] is
    /// returned when that line is no longer that task (see [`note::toggle_task`]).
    pub async fn toggle_task(
        &self,
        path: &VaultPath,
        line: usize,
        seen: &str,
    ) -> Result<(), VaultError> {
        // The per-note lock spans the read and the write, as in
        // `replace_in_note`.
        let _guard = self.lock_note(path).await;
        let text = self.get_note_text(path).await?;
        let updated = note::toggle_task(&text, line, seen)
            .ok_or_else(|| VaultError::TaskChanged { path: path.clone() })?;
        self.save_note_unlocked(path, updated).await?;
        Ok(())
    }

    /// Dry-run of [`Self::replace_in_note`]: computes what the note would contain after
    /// the replacement **without writing** (and without taking the write lock —
    /// the result is advisory). Returns the same errors the real replace would
    /// (not found / not unique / invalid regex).
    pub async fn preview_replace(
        &self,
        path: &VaultPath,
        pattern: &str,
        replacement: &str,
        all: bool,
        regex: bool,
    ) -> Result<ReplacePreview, VaultError> {
        let text = self.get_note_text(path).await?;
        let (count, content) =
            Self::compute_replacement(&text, pattern, replacement, all, regex, path)?;
        Ok(ReplacePreview { count, content })
    }

    /// The note's labels: inline `#hashtags` and frontmatter `tags:` together,
    /// lowercased, sorted, distinct. Index-backed: a missing or not-yet-indexed
    /// note returns an empty list, not `NotFound`.
    pub async fn get_tags(&self, path: &VaultPath) -> Result<Vec<String>, VaultError> {
        Ok(self.index.labels_of(path).await?)
    }

    /// Deletes the directory at `path` and its contents, removing the
    /// corresponding index rows first.
    pub async fn delete_directory(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        path.ensure_directory()?;

        self.index
            .delete_directories(std::slice::from_ref(&path))
            .await?;

        nfs::delete_directory(self.workspace_path(), &path).await?;

        // Best-effort: a pin-file failure never fails a delete that already
        // succeeded on disk and in the index.
        pinned_notes::on_directory_deleted(self.workspace_path(), &path).await;

        Ok(())
    }

    /// Renames the note `from` to `to`, rewriting links to it (wikilinks,
    /// Markdown links, and the note's own self-links) in every backlinking note
    /// so they keep pointing at the renamed note. A name link that also leads
    /// to another note of the same name is ambiguous and left alone; a
    /// rewritten one names `to` by the shortest suffix that names it alone
    /// (at least as many folders as written); a move keeping a unique name
    /// rewrites no name link. After a move, the note's own relative links
    /// are rewritten from its new folder. Fails if `to` already exists.
    /// The whole operation — locks, filesystem move, link rewrites, index
    /// commit — is `NoteRename`'s (`note_rename.rs`); see its docs for the stage order and
    /// failure atomicity.
    pub async fn rename_note(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        NoteRename::new(
            &self.index,
            self.workspace_path(),
            self.backup,
            &self.note_locks,
        )
        .rename(from, to)
        .await
    }

    /// Renames the directory `from` to `to`, updating the index paths of all
    /// notes beneath it. Fails if `to` already exists. No link text is
    /// rewritten: path links into the folder, and wikilinks naming it as a
    /// folder part, break (a known gap).
    pub async fn rename_directory(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();

        nfs::rename_directory(self.workspace_path(), &from, &to)
            .await
            .map_err(rename_dest_err)?;

        // Best-effort: a pin-file failure never fails a rename that already
        // succeeded on disk. Fine before the index call too — that step
        // cannot roll back the filesystem move either.
        pinned_notes::on_directory_renamed(self.workspace_path(), &from, &to).await;

        self.index.rename_directory(&from, &to).await?;

        Ok(())
    }

    /// Classifies the entry at `path` with a single filesystem stat: a
    /// directory, a note (`.md`), or an attachment (any other file). The one
    /// door callers use to pick the right rename/delete path for a vault entry.
    /// Errors with [`VaultError::FSError`] / `VaultPathNotFound` when nothing
    /// exists at `path`.
    pub async fn entry_kind(&self, path: &VaultPath) -> Result<EntryKind, VaultError> {
        let path = path.flatten();
        let meta = nfs::metadata_at(self.workspace_path(), &path).await?;
        Ok(nfs::classify(&meta, &path))
    }

    /// Reads an attachment's [`AttachmentDetails`] — size, last-modified, file
    /// extension, and a preview of its content (the decoded text for text files,
    /// capped; nothing for binary files). The attachment is never indexed.
    pub async fn get_attachment_details(
        &self,
        path: &VaultPath,
    ) -> Result<AttachmentDetails, VaultError> {
        let path = path.flatten();
        let read = nfs::read_attachment(self.workspace_path(), &path).await?;
        let extension = attachment_extension(&path);
        let content = match read.content {
            nfs::FileText::Text { text, truncated } => AttachmentContent::Text { text, truncated },
            nfs::FileText::Binary => AttachmentContent::Binary,
        };
        Ok(AttachmentDetails {
            path,
            size: read.size,
            modified_secs: read.modified_secs,
            extension,
            content,
        })
    }

    /// Renames or moves an attachment (any non-note file). Plain filesystem
    /// rename: unlike [`Self::rename_note`], it does **not** rewrite the
    /// embed/link references to the attachment in notes, since attachments are
    /// not part of the note-link graph. Fails if `to` already exists.
    pub async fn rename_attachment(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();
        nfs::rename_attachment(self.workspace_path(), &from, &to)
            .await
            .map_err(rename_dest_err)?;
        Ok(())
    }

    /// Deletes an attachment file. No index or backup involvement — attachments
    /// are not indexed.
    pub async fn delete_attachment(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        nfs::delete_attachment(self.workspace_path(), &path).await?;
        Ok(())
    }

    /// Deletes the entry at `path`, dispatching on its [`EntryKind`]: a note
    /// (index- and backup-aware), a directory (recursive, reindexed), or an
    /// attachment (plain file). The one door the UI uses to delete any entry
    /// without classifying it itself.
    pub async fn delete_entry(&self, path: &VaultPath) -> Result<(), VaultError> {
        match self.entry_kind(path).await? {
            EntryKind::Note => self.delete_note(path).await,
            EntryKind::Directory => self.delete_directory(path).await,
            EntryKind::Attachment => self.delete_attachment(path).await,
        }
    }

    /// Renames or moves the entry at `from` to `to`, dispatching on `from`'s
    /// [`EntryKind`]: a note (rewrites backlinks), a directory (reindexes the
    /// notes beneath it), or an attachment (plain file rename — references to it
    /// are not rewritten). A move is just a cross-directory rename. The one door
    /// the UI uses to rename or move any entry without classifying it itself.
    pub async fn rename_entry(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        match self.entry_kind(from).await? {
            EntryKind::Note => self.rename_note(from, to).await,
            EntryKind::Directory => self.rename_directory(from, to).await,
            EntryKind::Attachment => self.rename_attachment(from, to).await,
        }
    }
}

/// Lowercased file extension of an attachment path, if any (`diagram.png` →
/// `png`). `VaultPath` already lowercases its components.
fn attachment_extension(path: &VaultPath) -> Option<String> {
    let (_, name) = path.get_parent_path();
    std::path::Path::new(&name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
}

/// Where a note link leads ([`NoteVault::resolve_note_target`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    /// Exactly one note matches: its index entry and content data.
    Note(NoteEntryData, NoteContentData),
    /// A name several notes match, in index order.
    Ambiguous(Vec<(NoteEntryData, NoteContentData)>),
    /// No note matches: the canonical path a new note for the link would get.
    Missing(VaultPath),
}

/// A directory entry within the vault.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirectoryDetails {
    /// Path of the directory inside the vault.
    pub path: VaultPath,
}

/// A single entry produced by browsing the vault: a note, directory, or
/// attachment, identified by its path.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// Path of the entry inside the vault.
    pub path: VaultPath,
    /// What kind of entry this is, plus any content data for notes.
    pub rtype: ResultType,
}

impl SearchResult {
    /// Builds a note result carrying its content data.
    pub fn note(path: &VaultPath, content_data: &NoteContentData) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Note(content_data.to_owned()),
        }
    }
    /// Builds a directory result.
    pub fn directory(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Directory,
        }
    }
    /// Builds an attachment result.
    pub fn attachment(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Attachment,
        }
    }
}

/// Kind of a [`SearchResult`].
#[derive(Debug, Clone, PartialEq)]
pub enum ResultType {
    /// A note, with its parsed content data.
    Note(NoteContentData),
    /// A directory.
    Directory,
    /// An attachment (non-note file).
    Attachment,
}

/// Read-only details of an attachment, for the attachment view: its identity,
/// size, last-modified time, extension, and previewable content.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentDetails {
    /// The attachment's vault path.
    pub path: VaultPath,
    /// File size in bytes (the true on-disk size, even when the preview is
    /// truncated).
    pub size: u64,
    /// Last-modified time, in whole seconds since the Unix epoch.
    pub modified_secs: u64,
    /// Lowercased file extension, if any (`png`, `pdf`, …; `None` for
    /// extension-less files like `LICENSE`).
    pub extension: Option<String>,
    /// The previewable content.
    pub content: AttachmentContent,
}

/// The previewable content of an attachment.
#[derive(Debug, Clone, PartialEq)]
pub enum AttachmentContent {
    /// A text file, decoded as UTF-8. `truncated` is `true` when the file
    /// exceeded the preview cap and `text` is a prefix.
    Text {
        /// The decoded text (capped).
        text: String,
        /// Whether the file was longer than the preview cap.
        truncated: bool,
    },
    /// A binary (non-text) file — no preview is offered.
    Binary,
}

/// Builder for [`VaultBrowseOptions`]; see [`NoteVault::browse_vault_stream`]
/// and its collected form [`NoteVault::browse_vault`].
pub struct VaultBrowseOptionsBuilder {
    path: VaultPath,
    validation: NotesValidation,
    recursive: bool,
}

impl VaultBrowseOptionsBuilder {
    /// Starts a builder rooted at `path`, with defaults otherwise.
    pub fn new(path: &VaultPath) -> Self {
        Self::default().path(path.clone())
    }

    /// Finalizes the options.
    pub fn build(self) -> VaultBrowseOptions {
        VaultBrowseOptions {
            path: self.path,
            validation: self.validation,
            recursive: self.recursive,
        }
    }

    /// Sets the path to browse from.
    pub fn path(mut self, path: VaultPath) -> Self {
        self.path = path;
        self
    }

    /// Sets whether the browse descends into subdirectories.
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    /// Sets how thoroughly each note is re-validated against the index during
    /// the browse.
    pub fn validation(mut self, validation: NotesValidation) -> Self {
        self.validation = validation;
        self
    }
}

impl Default for VaultBrowseOptionsBuilder {
    fn default() -> Self {
        Self {
            path: VaultPath::root(),
            validation: NotesValidation::None,
            recursive: false,
        }
    }
}

/// What a browse walks: the subtree root, how deep, and how strictly each
/// note is re-validated against the index on the way. See
/// [`NoteVault::browse_vault`] and [`NoteVault::browse_vault_stream`].
#[derive(Debug, Clone, PartialEq)]
pub struct VaultBrowseOptions {
    path: VaultPath,
    validation: NotesValidation,
    recursive: bool,
}

impl Display for VaultBrowseOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Vault Browse Options - [Path: `{}`|Validation Type: `{}`|Recursive: `{}`]",
            self.path, self.validation, self.recursive
        )
    }
}

/// How thoroughly a sync pass checks whether each note has changed before
/// updating its index entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotesValidation {
    /// Compares a content hash — detects any change, at the cost of reading
    /// each file.
    Full,
    /// Compares the file size — cheaper, but misses same-size edits.
    Fast,
    /// Only checks whether the note exists.
    None,
}

impl Display for NotesValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                NotesValidation::Full => "Full",
                NotesValidation::Fast => "Fast",
                NotesValidation::None => "None",
            }
        )
    }
}

#[cfg(test)]
mod tests;
