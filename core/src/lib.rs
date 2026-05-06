pub(crate) mod db;
pub mod error;
pub mod nfs;
pub mod note;
pub mod utilities;
pub use db::DBStatus;
pub use utilities::{app_log_dir, ensure_dir_exists};

use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
    sync::{
        mpsc::{Receiver, Sender},
        Arc,
    },
    time::{Duration, SystemTime},
};

use chrono::{NaiveDate, Utc};
use db::VaultDB;
use error::{DBError, FSError, VaultError};
use log::debug;
use nfs::{visitor::NoteListVisitorBuilder, NoteEntryData, VaultPath};
use note::{ContentChunk, NoteContentData, NoteDetails};
use utilities::path_to_string;

use crate::nfs::DirectoryEntryData;

pub const DEFAULT_JOURNAL_PATH: &str = "/journal";
pub const DEFAULT_INBOX_PATH: &str = "/inbox";
pub const DEFAULT_ASSETS_PATH: &str = "/assets";

/// Maximum number of concurrent FS read/write tasks during backlink rewriting.
/// Caps file-descriptor pressure on hub-style notes with thousands of links.
/// Sized well below typical soft `ulimit -n` (256 on macOS, 1024 on Linux)
/// while still parallelizing enough to hide per-syscall latency.
const BACKLINK_IO_CONCURRENCY: usize = 32;

pub struct IndexReport {
    pub start: SystemTime,
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
    pub workspace_path: std::path::PathBuf,
    pub db_path: Option<std::path::PathBuf>,
}

impl VaultConfig {
    pub fn new(workspace_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workspace_path: workspace_path.into(),
            db_path: None,
        }
    }

    pub fn with_db_path(mut self, db_path: impl Into<std::path::PathBuf>) -> Self {
        self.db_path = Some(db_path.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct NoteVault {
    /// Stored as `Arc<Path>` (not `Arc<PathBuf>`) because (a) it impls
    /// `AsRef<Path>` directly so it can be passed to nfs helpers without
    /// extra deref, (b) `Arc::clone` is a refcount bump for fan-out tasks
    /// (backlink rewrites, indexing).
    workspace_path: Arc<Path>,
    journal_path: VaultPath,
    inbox_path: VaultPath,
    vault_db: VaultDB,
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
    /// Make sure you call `NoteVault::init_and_validate(&self)` to initialize the DB index if
    /// needed.
    pub async fn new(config: VaultConfig) -> Result<Self, VaultError> {
        debug!("Creating new vault Instance");
        let workspace_path = config.workspace_path;
        if !workspace_path.exists() {
            return Err(VaultError::VaultPathNotFound {
                path: path_to_string(&workspace_path),
            })?;
        }
        if !workspace_path.is_dir() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: path_to_string(&workspace_path),
                message: "Path provided is not a directory".to_string(),
            }))?;
        };

        let db_path = config
            .db_path
            .unwrap_or_else(|| workspace_path.join(crate::db::DB_FILE));
        let vault_db = VaultDB::new(&db_path).await?;
        let note_vault = Self {
            workspace_path: Arc::from(workspace_path.as_path()),
            journal_path: VaultPath::new(DEFAULT_JOURNAL_PATH),
            inbox_path: VaultPath::new(DEFAULT_INBOX_PATH),
            vault_db,
        };
        Ok(note_vault)
    }

    /// OS path to the workspace root (filesystem root of this vault).
    pub fn workspace_path(&self) -> &Path {
        &self.workspace_path
    }

    /// Test-only handle to the underlying SQLite pool. Used by migration
    /// tests that need to mutate stored state directly to simulate older
    /// schema versions.
    #[cfg(test)]
    pub(crate) fn db_pool(&self) -> &sqlx::SqlitePool {
        self.vault_db.pool()
    }

    pub async fn validate(&self) -> Result<DBStatus, VaultError> {
        self.vault_db.check_db().await.map_err(VaultError::DBError)
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
    /// On init and validate it verifies the DB index to make sure:
    ///
    /// 1. It exists
    /// 2. It is valid.
    /// 3. Its schema is updated
    ///
    /// Then does a quick scan of the workspace directory to update the index if there are new or
    /// missing notes.
    /// This can be slow on large vaults.
    pub async fn validate_and_init(&self) -> Result<IndexReport, VaultError> {
        self.fail_on_case_conflicts().await?;
        debug!("Initializing DB and validating it");
        let db_result = self.validate().await;
        match db_result {
            Ok(check_res) => {
                match check_res {
                    db::DBStatus::Ready => {
                        // We only check if there are new notes
                        self.index_notes(NotesValidation::None).await
                    }
                    db::DBStatus::Outdated => self.recreate_index().await,
                    db::DBStatus::NotValid => self.recreate_index().await,
                    db::DBStatus::FileNotFound => {
                        // No need to validate, no data there
                        self.recreate_index().await
                    }
                }
            }
            Err(e) => {
                debug!("Error validating the DB, rebuilding it: {}", e);
                self.recreate_index().await
            }
        }
    }

    /// Deletes the db file and recreates the index.
    /// On Windows, the pool must be closed before the file can be deleted,
    /// so this method closes the pool first. After calling this method,
    /// the NoteVault instance should be discarded and a new one created.
    pub async fn force_rebuild(&self) -> Result<IndexReport, VaultError> {
        let db_path = self.vault_db.get_db_path();
        // Close the pool to release file handles before deleting.
        // This is required on Windows where open handles prevent file deletion.
        self.vault_db.close().await?;
        // Delete the db file via the nfs module.
        nfs::remove_path(&db_path)?;
        // Note: the pool is closed at this point. The caller should create
        // a new NoteVault instance if further DB operations are needed.
        // recreate_index will reconnect via the pool's rwc mode which
        // recreates the file.
        self.recreate_index().await
    }

    /// Deletes all the cached data from the DB by destroying the tables
    /// and recreates the index
    /// This is similar to a force rebuild but instead of deleting the db file
    /// it only deletes the tables.
    pub async fn recreate_index(&self) -> Result<IndexReport, VaultError> {
        self.fail_on_case_conflicts().await?;
        let index_report = IndexReport::new();
        debug!("Initializing DB from Vault request");
        db::init_db(self.vault_db.pool()).await?;
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
        let workspace_path = self.workspace_path.clone();
        create_index_for(
            &workspace_path,
            self.vault_db.pool(),
            &VaultPath::root(),
            validation_mode,
        )
        .await?;
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

    pub fn journal_path(&self) -> &VaultPath {
        &self.journal_path
    }

    pub fn inbox_path(&self) -> &VaultPath {
        &self.inbox_path
    }

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

    pub async fn journal_entry(&self) -> Result<(NoteDetails, String), VaultError> {
        let (title, note_path) = self.get_todays_journal();
        let content = self
            .load_or_create_note(&note_path, Some(format!("# {}\n\n", title)))
            .await?;
        let details = NoteDetails::new(&note_path, &content);
        Ok((details, content))
    }

    fn get_todays_journal(&self) -> (String, VaultPath) {
        let today = Utc::now();
        let today_string = today.format("%Y-%m-%d").to_string();

        (
            today_string.clone(),
            self.journal_path
                .append(&VaultPath::note_path_from(&today_string))
                .absolute(),
        )
    }

    // Returns a NaiveDate if the note path is a valid journal entry
    pub fn journal_date(&self, note_path: &VaultPath) -> Option<NaiveDate> {
        if !note_path.is_note() {
            return None;
        }

        let (parent, _) = note_path.get_parent_path();
        if parent.eq(&self.journal_path) {
            let name = note_path.get_clean_name();
            NaiveDate::parse_from_str(&name, "%Y-%m-%d").ok()
        } else {
            None
        }
    }

    /// Loads the note at `path` if it exists; otherwise creates it with `default_text`
    /// (or empty if `None`) and returns that text.
    pub async fn load_or_create_note(
        &self,
        path: &VaultPath,
        default_text: Option<String>,
    ) -> Result<String, VaultError> {
        match nfs::load_note(self.workspace_path(), path).await {
            Ok(text) => Ok(text),
            Err(e) if e.is_not_found() => {
                let text = default_text.unwrap_or_default();
                self.create_note(path, &text).await?;
                Ok(text)
            }
            Err(e) => Err(e.into()),
        }
    }

    // Loads the note's content, returns the text
    // If the file doesn't exist you will get a VaultError::FSError with a
    // FSError::NotePathNotFound as the source, you can use that to
    // lazy create a note, or use the load_or_create_note function instead
    pub async fn get_note_text(&self, path: &VaultPath) -> Result<String, VaultError> {
        let text = nfs::load_note(self.workspace_path(), path).await?;
        Ok(text)
    }

    // Loads a note, returning its details that contain path, raw text and more
    // If the file doesn't exist you will get a VaultError::FSError with a
    // FSError::NotePathNotFound as the source, you can use that to
    // lazy create a note, or use the load_or_create_note function instead
    pub async fn load_note(&self, path: &VaultPath) -> Result<NoteDetails, VaultError> {
        let text = self.get_note_text(path).await?;
        Ok(NoteDetails::new(path, text))
    }

    pub async fn get_note_chunks(
        &self,
        path: &VaultPath,
    ) -> Result<HashMap<VaultPath, Vec<ContentChunk>>, VaultError> {
        let a = db::get_notes_sections(self.vault_db.pool(), path, false).await?;
        Ok(a)
    }

    // Search notes using a search syntax
    pub async fn search_notes<S: AsRef<str>>(
        &self,
        search_query: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let search_query = search_query.as_ref();
        let a = db::search_terms(self.vault_db.pool(), search_query).await?;
        Ok(a)
    }

    /// Get notes under the given path. When `recursive` is false, only direct
    /// children are returned.
    pub async fn get_notes(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let notes = db::get_notes(self.vault_db.pool(), path, recursive).await?;
        Ok(notes)
    }

    // Get all notes
    pub async fn get_all_notes(&self) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let a = db::get_all_notes(self.vault_db.pool()).await?;
        Ok(a)
    }
    pub fn path_to_pathbuf(&self, path: &VaultPath) -> PathBuf {
        path.to_pathbuf(self.workspace_path())
    }

    pub async fn browse_vault(&self, options: VaultBrowseOptions) -> Result<(), VaultError> {
        let start = std::time::SystemTime::now();
        debug!("> Start fetching files with Options:\n{}", options);

        let cached_notes =
            db::get_notes(self.vault_db.pool(), &options.path, options.recursive).await?;

        let builder = NoteListVisitorBuilder::new(
            self.workspace_path(),
            options.validation,
            cached_notes,
            Some(options.sender.clone()),
        );
        let walker = nfs::get_file_walker(
            self.workspace_path.clone(),
            &options.path,
            options.recursive,
        );
        let builder = run_walker_blocking(walker, builder).await?;
        let results = builder.into_results();

        let mut tx = self.vault_db.pool().begin().await?;
        db::insert_notes(&mut tx, &results.to_add).await?;
        db::delete_notes(&mut tx, &results.to_delete).await?;
        db::update_notes(&mut tx, &results.to_modify).await?;
        tx.commit().await?;

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
        let (md_text, mut links) =
            note::content_extractor::get_markdown_and_links(&note.path, &note.raw_text);
        // Since this function is intended to return content ready to be rendered
        // We need the full path of the image links, so any markdown processor can find the image,
        // the full path can only be resolved from here as we have the vault path
        let (md_text, image_links) =
            note::content_extractor::process_image_links(&md_text, |alt_text, raw_path| {
                let resolved = if note::is_remote_url(raw_path) {
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

    /// Returns all notes that contain a link pointing to `path`.
    /// Matches both absolute vault paths and bare filename links (wikilinks).
    pub async fn get_backlinks(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        Ok(db::get_backlinks(self.vault_db.pool(), path).await?)
    }

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
        let content_data = note_details.get_content_data();
        db::save_note(self.vault_db.pool(), &entry_data, &note_details).await?;
        Ok((entry_data, content_data))
    }

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

    pub async fn save_note<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        let entry_data = nfs::save_note(self.workspace_path(), path, &text).await?;
        let note_details = NoteDetails::new(path, text);
        let content_data = note_details.get_content_data();
        db::save_note(self.vault_db.pool(), &entry_data, &note_details).await?;
        Ok((entry_data, content_data))
    }

    /// Default attachments directory (e.g. `/assets`) inside the workspace.
    pub fn default_attachments_path(&self) -> VaultPath {
        VaultPath::new(DEFAULT_ASSETS_PATH)
    }

    /// Builds a candidate path for a new attachment under
    /// [`default_attachments_path`], using `prefix` and `ext` plus the current
    /// unix-nanosecond timestamp for uniqueness. Nanoseconds (rather than
    /// millis) make same-instant collisions vanishingly unlikely for
    /// human-driven actions like clipboard paste.
    ///
    /// Does not check for collisions; callers that need stronger uniqueness
    /// guarantees should retry with [`exists`] or use a different strategy.
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

    /// If the path looks like a specific note (has the note extension), search by name;
    /// otherwise treat it as a directory/path query that may return many results.
    pub async fn open_or_search(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        debug!("PATH: {}", path);
        let (_parent, name) = path.get_parent_path();
        if path.is_note_file() {
            Ok(db::search_note_by_name(self.vault_db.pool(), name).await?)
        } else {
            Ok(db::search_note_by_path(self.vault_db.pool(), path).await?)
        }
    }

    pub async fn delete_note(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        path.ensure_note()?;

        // Delete in DB first so the index never points at a missing file.
        let mut tx = self.vault_db.pool().begin().await?;
        db::delete_notes(&mut tx, std::slice::from_ref(&path)).await?;
        tx.commit().await?;

        nfs::delete_note(self.workspace_path(), &path).await?;

        Ok(())
    }

    pub async fn delete_directory(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        path.ensure_directory()?;

        let mut tx = self.vault_db.pool().begin().await?;
        db::delete_directories(&mut tx, std::slice::from_ref(&path)).await?;
        tx.commit().await?;

        nfs::delete_directory(self.workspace_path(), &path).await?;

        Ok(())
    }

    pub async fn rename_note(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();

        // 1. Read every backlink file (excluding the source itself), computing
        //    rewritten contents in memory. No FS mutations yet — failure
        //    here aborts cleanly.
        let updates = self.read_backlink_rewrites(&from, &to).await?;

        // 2. Rename the source note on disk. If this fails, backlinks remain
        //    untouched and the DB is unchanged — clean abort.
        nfs::rename_note(self.workspace_path(), &from, &to)
            .await
            .map_err(rename_dest_err)?;

        // 3. Write the rewritten backlink files (concurrency-bounded). Returns
        //    paired (NoteEntryData, String) tuples, consuming `updates` so the
        //    text is not cloned again.
        let mut notes_with_text = self.write_backlink_rewrites(updates).await?;

        // 3a. Rewrite self-links inside the renamed file at its new location
        //     (the source was excluded from the backlinks list to avoid the
        //     "create new file at old path" hazard).
        if let Some(updated) = self.rewrite_self_links(&from, &to).await? {
            notes_with_text.push(updated);
        }

        // 4. Single DB transaction: rename the source row + update each
        //    backlink's chunks/links. If this commit fails, FS is consistent
        //    with the rename but DB is stale — next index pass corrects.
        let mut tx = self.vault_db.pool().begin().await?;
        db::rename_note(&mut tx, &from, &to).await?;
        db::update_notes(&mut tx, &notes_with_text).await?;
        tx.commit().await?;

        Ok(())
    }

    /// Reads the renamed source file at `to`, rewrites any links pointing to
    /// `from` into `to`, and writes the file back. Returns the updated entry
    /// for DB update, or `None` if no self-links existed.
    async fn rewrite_self_links(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<Option<(NoteEntryData, String)>, VaultError> {
        let text = nfs::load_note(self.workspace_path(), to).await?;
        let (updated, changed) = note::content_extractor::replace_note_links(&text, from, to);
        if !changed {
            return Ok(None);
        }
        let entry = nfs::save_note(self.workspace_path(), to, &updated).await?;
        Ok(Some((entry, updated)))
    }

    /// Loads every note that links to `from`, rewrites its links to `to`,
    /// returns only the entries whose content actually changed. I/O is
    /// concurrency-bounded so a hub note with thousands of backlinks won't
    /// exhaust the OS file-descriptor limit.
    async fn read_backlink_rewrites(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<Vec<(VaultPath, String)>, VaultError> {
        // Drop the source itself if it backlinks to itself — those self-links
        // are handled separately by `rewrite_self_links` after the FS rename,
        // so the source's body isn't written to its old location here (which
        // would resurrect a file at `from`).
        let backlinks: Vec<_> = db::get_backlinks(self.vault_db.pool(), from)
            .await?
            .into_iter()
            .filter(|(e, _)| e.path != *from)
            .collect();
        if backlinks.is_empty() {
            return Ok(Vec::new());
        }
        let workspace = self.workspace_path.clone();
        let from = Arc::new(from.clone());
        let to = Arc::new(to.clone());
        let stream = futures_util::stream::iter(backlinks.into_iter().map(|(entry_data, _)| {
            let workspace = workspace.clone();
            let from = from.clone();
            let to = to.clone();
            async move {
                let text = nfs::load_note(&workspace, &entry_data.path).await?;
                let (updated, changed) =
                    note::content_extractor::replace_note_links(&text, &from, &to);
                Ok::<_, VaultError>(changed.then_some((entry_data.path, updated)))
            }
        }));
        use futures_util::stream::StreamExt;
        let mut stream = stream.buffered(BACKLINK_IO_CONCURRENCY);
        let mut updates = Vec::new();
        while let Some(item) = stream.next().await {
            if let Some(entry) = item? {
                updates.push(entry);
            }
        }
        Ok(updates)
    }

    /// Writes the rewritten backlink files concurrency-bounded. Consumes
    /// `updates` so each file's text is moved into its task without cloning,
    /// then returns the paired `(NoteEntryData, String)` results in input
    /// order ready for `db::update_notes`.
    async fn write_backlink_rewrites(
        &self,
        updates: Vec<(VaultPath, String)>,
    ) -> Result<Vec<(NoteEntryData, String)>, VaultError> {
        if updates.is_empty() {
            return Ok(Vec::new());
        }
        let workspace = self.workspace_path.clone();
        let mut futures = Vec::with_capacity(updates.len());
        for (path, text) in updates {
            let workspace = workspace.clone();
            futures.push(async move {
                let entry = nfs::save_note(&workspace, &path, &text).await?;
                Ok::<_, VaultError>((entry, text))
            });
        }
        use futures_util::stream::StreamExt;
        let cap = futures.len();
        let mut stream = futures_util::stream::iter(futures).buffered(BACKLINK_IO_CONCURRENCY);
        let mut out = Vec::with_capacity(cap);
        while let Some(item) = stream.next().await {
            out.push(item?);
        }
        Ok(out)
    }

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

        let mut tx = self.vault_db.pool().begin().await?;
        db::rename_directory(&mut tx, &from, &to).await?;
        tx.commit().await?;

        Ok(())
    }
}

/// Runs the synchronous parallel walker on a blocking thread so the async
/// runtime is not stalled while the entire vault subtree is enumerated.
async fn run_walker_blocking(
    walker: ignore::WalkParallel,
    builder: NoteListVisitorBuilder,
) -> Result<NoteListVisitorBuilder, VaultError> {
    tokio::task::spawn_blocking(move || {
        let mut builder = builder;
        walker.visit(&mut builder);
        builder
    })
    .await
    .map_err(|e| VaultError::TaskJoin(format!("vault walker: {}", e)))
}

fn rename_dest_err(e: FSError) -> VaultError {
    match e {
        FSError::AlreadyExists { path } => VaultError::FSError(FSError::InvalidPath {
            path: path.to_string(),
            message: "Destination path already exists".to_string(),
        }),
        other => VaultError::FSError(other),
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirectoryDetails {
    pub path: VaultPath,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub path: VaultPath,
    pub rtype: ResultType,
}

impl SearchResult {
    pub fn note(path: &VaultPath, content_data: &NoteContentData) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Note(content_data.to_owned()),
        }
    }
    pub fn directory(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Directory,
        }
    }
    pub fn attachment(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Attachment,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResultType {
    Note(NoteContentData),
    Directory,
    Attachment,
}

pub struct VaultBrowseOptionsBuilder {
    path: VaultPath,
    validation: NotesValidation,
    recursive: bool,
}

impl VaultBrowseOptionsBuilder {
    pub fn new(path: &VaultPath) -> Self {
        Self::default().path(path.clone())
    }

    pub fn build(self) -> (VaultBrowseOptions, Receiver<SearchResult>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            VaultBrowseOptions {
                path: self.path,
                validation: self.validation,
                recursive: self.recursive,
                sender,
            },
            receiver,
        )
    }

    pub fn path(mut self, path: VaultPath) -> Self {
        self.path = path;
        self
    }

    pub fn recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

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

#[derive(Debug, Clone)]
/// Options to traverse the Notes
/// You need a sync::mpsc::Sender to use a channel to receive the entries
pub struct VaultBrowseOptions {
    path: VaultPath,
    validation: NotesValidation,
    recursive: bool,
    sender: Sender<SearchResult>,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotesValidation {
    Full,
    Fast,
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

async fn create_index_for<P>(
    workspace_path: P,
    pool: &sqlx::SqlitePool,
    path: &VaultPath,
    validation_mode: NotesValidation,
) -> Result<(), DBError>
where
    P: AsRef<Path> + Send,
{
    debug!("Indexing subtree at {}", path);
    let workspace_path = workspace_path.as_ref();
    let walker = nfs::get_file_walker(workspace_path, path, true);

    let cached_notes = db::get_notes(pool, path, true).await?;
    let builder = NoteListVisitorBuilder::new(workspace_path, validation_mode, cached_notes, None);
    let builder = run_walker_blocking(walker, builder)
        .await
        .map_err(|e| match e {
            VaultError::DBError(e) => e,
            other => DBError::Other(other.to_string()),
        })?;
    let results = builder.into_results();

    let mut tx = pool.begin().await?;
    db::delete_notes(&mut tx, &results.to_delete).await?;
    db::insert_notes(&mut tx, &results.to_add).await?;
    db::update_notes(&mut tx, &results.to_modify).await?;
    tx.commit().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::time::Duration;
    use tempfile::TempDir;

    // Helper: build a NoteVault pointing at a temp directory (no DB needed for pure-text tests).
    async fn make_vault(dir: &std::path::Path) -> NoteVault {
        NoteVault::new(VaultConfig::new(dir)).await.unwrap()
    }

    #[tokio::test]
    async fn get_markdown_and_links_resolves_relative_image() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        std::fs::create_dir_all(dir.path().join("directory")).unwrap();
        std::fs::write(dir.path().join("directory/note.md"), "![alt](../photo.png)").unwrap();

        let md_note = vault
            .get_markdown_and_links(&VaultPath::new("/directory/note.md"))
            .await
            .unwrap();

        let expected_os_path = dir.path().join("photo.png").display().to_string();
        assert_eq!(md_note.text, format!("![alt]({})", expected_os_path));
        assert_eq!(1, md_note.links.len());
        let link = &md_note.links[0];
        assert_eq!(link.ltype, note::LinkType::Image(expected_os_path));
        assert_eq!(link.text, "alt");
        assert_eq!(link.raw_link, "../photo.png");
    }

    #[tokio::test]
    async fn get_markdown_and_links_resolves_absolute_vault_image() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        std::fs::create_dir_all(dir.path().join("notes")).unwrap();
        std::fs::write(
            dir.path().join("notes/note.md"),
            "![banner](/assets/banner.png)",
        )
        .unwrap();

        let md_note = vault
            .get_markdown_and_links(&VaultPath::new("/notes/note.md"))
            .await
            .unwrap();

        let expected_os_path = dir
            .path()
            .join("assets")
            .join("banner.png")
            .display()
            .to_string();
        assert_eq!(md_note.text, format!("![banner]({})", expected_os_path));
        assert!(matches!(
            &md_note.links[0].ltype,
            note::LinkType::Image(p) if *p == expected_os_path
        ));
    }

    #[tokio::test]
    async fn get_markdown_and_links_keeps_external_image_url() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        let url = "https://example.com/img.png";
        std::fs::write(dir.path().join("note.md"), format!("![remote]({})", url)).unwrap();

        let md_note = vault
            .get_markdown_and_links(&VaultPath::new("/note.md"))
            .await
            .unwrap();

        assert_eq!(md_note.text, format!("![remote]({})", url));
        assert!(matches!(
            &md_note.links[0].ltype,
            note::LinkType::Image(p) if p == url
        ));
        assert_eq!(md_note.links[0].raw_link, url);
    }

    #[tokio::test]
    async fn get_markdown_and_links_mixed_content() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        std::fs::write(
            dir.path().join("note.md"),
            "[[Other Note]] [link](other.md) ![img](photo.png) #tag",
        )
        .unwrap();

        let md_note = vault
            .get_markdown_and_links(&VaultPath::new("/note.md"))
            .await
            .unwrap();

        assert_eq!(
            1,
            md_note
                .links
                .iter()
                .filter(|l| matches!(l.ltype, note::LinkType::Image(_)))
                .count()
        );
        assert_eq!(
            2,
            md_note
                .links
                .iter()
                .filter(|l| matches!(l.ltype, note::LinkType::Note(_)))
                .count()
        );
        assert_eq!(
            1,
            md_note
                .links
                .iter()
                .filter(|l| matches!(l.ltype, note::LinkType::Hashtag))
                .count()
        );
    }

    // ---- rename_note: backlink rewriting integration tests ----

    /// Create a small vault with a DB, write two notes, index them, then rename one
    /// and assert that the other note's content and DB links are updated.
    async fn setup_vault_with_notes(dir: &std::path::Path) -> NoteVault {
        let vault = NoteVault::new(VaultConfig::new(dir)).await.unwrap();
        vault.validate_and_init().await.unwrap();
        vault
    }

    #[tokio::test]
    async fn rename_note_updates_wikilink_in_backlink() {
        let dir = TempDir::new().unwrap();
        let vault = setup_vault_with_notes(dir.path()).await;

        // Create the note that will be renamed
        vault
            .save_note(&VaultPath::new("/target.md"), "# Target note")
            .await
            .unwrap();
        // Create a note that links to it via wikilink
        vault
            .save_note(
                &VaultPath::new("/referrer.md"),
                "# Referrer\nSee [[target]].",
            )
            .await
            .unwrap();

        vault
            .rename_note(
                &VaultPath::new("/target.md"),
                &VaultPath::new("/renamed.md"),
            )
            .await
            .unwrap();

        // The referrer file on disk must now use [[renamed]]
        let updated = nfs::load_note(dir.path(), &VaultPath::new("/referrer.md"))
            .await
            .unwrap();
        assert!(
            updated.contains("[[renamed]]"),
            "expected [[renamed]] in: {updated}"
        );
        assert!(
            !updated.contains("[[target]]"),
            "old wikilink still present in: {updated}"
        );
    }

    #[tokio::test]
    async fn rename_note_updates_markdown_link_in_backlink() {
        let dir = TempDir::new().unwrap();
        let vault = setup_vault_with_notes(dir.path()).await;

        vault
            .save_note(&VaultPath::new("/target.md"), "# Target note")
            .await
            .unwrap();
        vault
            .save_note(
                &VaultPath::new("/referrer.md"),
                "# Referrer\n[link](/target.md) end.",
            )
            .await
            .unwrap();

        vault
            .rename_note(
                &VaultPath::new("/target.md"),
                &VaultPath::new("/renamed.md"),
            )
            .await
            .unwrap();

        let updated = nfs::load_note(dir.path(), &VaultPath::new("/referrer.md"))
            .await
            .unwrap();
        assert!(
            updated.contains("[link](/renamed.md)"),
            "expected updated link in: {updated}"
        );
        assert!(
            !updated.contains("/target.md"),
            "old path still present in: {updated}"
        );
    }

    #[tokio::test]
    async fn rename_note_does_not_touch_unrelated_notes() {
        let dir = TempDir::new().unwrap();
        let vault = setup_vault_with_notes(dir.path()).await;

        vault
            .save_note(&VaultPath::new("/target.md"), "# Target")
            .await
            .unwrap();
        vault
            .save_note(
                &VaultPath::new("/unrelated.md"),
                "# Unrelated\nNo links here.",
            )
            .await
            .unwrap();

        vault
            .rename_note(
                &VaultPath::new("/target.md"),
                &VaultPath::new("/renamed.md"),
            )
            .await
            .unwrap();

        let unrelated = nfs::load_note(dir.path(), &VaultPath::new("/unrelated.md"))
            .await
            .unwrap();
        assert_eq!(unrelated, "# Unrelated\nNo links here.");
    }

    #[tokio::test]
    async fn rename_note_handles_self_link() {
        let dir = TempDir::new().unwrap();
        let vault = setup_vault_with_notes(dir.path()).await;

        vault
            .save_note(
                &VaultPath::new("/target.md"),
                "# Target\nSee [[target]] here.",
            )
            .await
            .unwrap();

        vault
            .rename_note(
                &VaultPath::new("/target.md"),
                &VaultPath::new("/renamed.md"),
            )
            .await
            .unwrap();

        // Source no longer exists at the old path.
        assert!(
            !dir.path().join("target.md").exists(),
            "old file should be gone"
        );
        // New file exists with the self-link rewritten.
        let body = nfs::load_note(dir.path(), &VaultPath::new("/renamed.md"))
            .await
            .unwrap();
        assert!(
            body.contains("[[renamed]]"),
            "expected self-link rewritten in: {body}"
        );
        assert!(
            !body.contains("[[target]]"),
            "old self-link still present in: {body}"
        );

        // DB should have exactly one row for the renamed note.
        let all = vault.get_all_notes().await.unwrap();
        assert_eq!(all.len(), 1, "expected single DB row, got: {:?}", all);
    }

    #[test]
    fn test_index_report_finish() {
        let mut report = IndexReport::new();

        // Sleep for a small amount to ensure duration is non-zero
        std::thread::sleep(Duration::from_millis(10));

        report.finish();

        // Check that duration is now set and non-zero
        assert!(report.duration > Duration::default());
        assert!(report.duration.as_millis() >= 10);
    }

    #[tokio::test]
    async fn test_note_vault_new_with_nonexistent_path() {
        let nonexistent_path = "/this/path/does/not/exist";
        let result = NoteVault::new(VaultConfig::new(nonexistent_path)).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            VaultError::VaultPathNotFound { path } => {
                assert_eq!(path, nonexistent_path);
            }
            _ => panic!("Expected VaultPathNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_note_vault_new_with_file_instead_of_directory() {
        // Create a temporary file
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let file_path = temp_file.path();

        let result = NoteVault::new(VaultConfig::new(file_path)).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            VaultError::FSError(FSError::InvalidPath { message, .. }) => {
                assert_eq!(message, "Path provided is not a directory");
            }
            _ => panic!("Expected FSError::InvalidPath"),
        }
    }

    #[tokio::test]
    async fn test_note_vault_new_with_valid_directory() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        let result = NoteVault::new(VaultConfig::new(dir_path)).await;

        assert!(result.is_ok());
        let vault = result.unwrap();
        assert_eq!(vault.workspace_path(), dir_path);
        assert_eq!(vault.journal_path, VaultPath::new(DEFAULT_JOURNAL_PATH));
    }

    #[tokio::test]
    async fn test_get_todays_journal() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(temp_dir.path()))
            .await
            .unwrap();

        let (title, note_path) = vault.get_todays_journal();

        // Check that title matches today's date format
        let today = Utc::now();
        let expected_title = today.format("%Y-%m-%d").to_string();
        assert_eq!(title, expected_title);

        // Check that the path is correct
        let expected_path = vault
            .journal_path
            .append(&VaultPath::note_path_from(&expected_title))
            .absolute();
        assert_eq!(note_path, expected_path);
    }

    #[tokio::test]
    async fn test_journal_date_with_valid_journal_note() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(temp_dir.path()))
            .await
            .unwrap();

        // Create a journal note path
        let journal_note_path = vault
            .journal_path
            .append(&VaultPath::note_path_from("2023-12-25"))
            .absolute();

        let result = vault.journal_date(&journal_note_path);

        assert!(result.is_some());
        let date = result.unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2023, 12, 25).unwrap());
    }

    #[tokio::test]
    async fn test_journal_date_with_invalid_date_format() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(temp_dir.path()))
            .await
            .unwrap();

        // Create a note path with invalid date format
        let invalid_journal_path = vault
            .journal_path
            .append(&VaultPath::note_path_from("invalid-date"))
            .absolute();

        let result = vault.journal_date(&invalid_journal_path);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_journal_date_with_non_journal_path() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(temp_dir.path()))
            .await
            .unwrap();

        // Create a note path outside of journal directory
        let non_journal_path = VaultPath::new("/other/2023-12-25.md");

        let result = vault.journal_date(&non_journal_path);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_journal_date_with_non_note_path() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(temp_dir.path()))
            .await
            .unwrap();

        // Create a directory path (not a note)
        let directory_path = vault.journal_path.append(&VaultPath::new("2023-12-25"));

        let result = vault.journal_date(&directory_path);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_path_to_pathbuf() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(temp_dir.path()))
            .await
            .unwrap();

        let vault_path = VaultPath::new("/test/note.md");
        let result = vault.path_to_pathbuf(&vault_path);

        let expected = vault_path.to_pathbuf(&vault.workspace_path);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_directory_details() {
        let path = VaultPath::new("/test/directory");
        let details = DirectoryDetails { path: path.clone() };

        assert_eq!(details.path, path);
    }

    #[test]
    fn test_search_result_note() {
        let path = VaultPath::new("/test/note.md");
        let content_data = NoteContentData::new("Test Note".to_string(), 12345);
        let result = SearchResult::note(&path, &content_data);

        assert_eq!(result.path, path);
        match result.rtype {
            ResultType::Note(data) => assert_eq!(data, content_data),
            _ => panic!("Expected Note result type"),
        }
    }

    #[test]
    fn test_search_result_directory() {
        let path = VaultPath::new("/test/directory");
        let result = SearchResult::directory(&path);

        assert_eq!(result.path, path);
        match result.rtype {
            ResultType::Directory => (),
            _ => panic!("Expected Directory result type"),
        }
    }

    #[test]
    fn test_search_result_attachment() {
        let path = VaultPath::new("/test/image.png");
        let result = SearchResult::attachment(&path);

        assert_eq!(result.path, path);
        match result.rtype {
            ResultType::Attachment => (),
            _ => panic!("Expected Attachment result type"),
        }
    }

    #[test]
    fn test_result_type_equality() {
        let content_data = NoteContentData::new("Test Note".to_string(), 12345);
        let note_type1 = ResultType::Note(content_data.clone());
        let note_type2 = ResultType::Note(content_data);
        let directory_type = ResultType::Directory;
        let attachment_type = ResultType::Attachment;

        assert_eq!(note_type1, note_type2);
        assert_eq!(directory_type, ResultType::Directory);
        assert_eq!(attachment_type, ResultType::Attachment);
        assert_ne!(directory_type, attachment_type);
    }

    #[test]
    fn test_vault_browse_options_builder_default() {
        let builder = VaultBrowseOptionsBuilder::default();

        // We can't directly inspect private fields, but we can test the build result
        let (options, _receiver) = builder.build();

        assert_eq!(options.path, VaultPath::root());
        assert_eq!(options.validation, NotesValidation::None);
        assert!(!options.recursive);
    }

    #[test]
    fn test_vault_browse_options_builder_new() {
        let test_path = VaultPath::new("/test/path");
        let builder = VaultBrowseOptionsBuilder::new(&test_path);

        let (options, _receiver) = builder.build();

        assert_eq!(options.path, test_path);
        assert_eq!(options.validation, NotesValidation::None);
        assert!(!options.recursive);
    }

    #[test]
    fn test_vault_browse_options_builder_path() {
        let initial_path = VaultPath::new("/initial");
        let new_path = VaultPath::new("/new/path");

        let builder = VaultBrowseOptionsBuilder::new(&initial_path).path(new_path.clone());

        let (options, _receiver) = builder.build();

        assert_eq!(options.path, new_path);
    }

    #[test]
    fn test_vault_browse_options_builder_recursive() {
        let path = VaultPath::new("/test");

        let builder = VaultBrowseOptionsBuilder::new(&path).recursive(true);
        let (options, _receiver) = builder.build();
        assert!(options.recursive);

        let builder = VaultBrowseOptionsBuilder::new(&path).recursive(false);
        let (options, _receiver) = builder.build();
        assert!(!options.recursive);
    }

    #[test]
    fn test_vault_browse_options_builder_validation_modes() {
        let path = VaultPath::new("/test");

        for v in [
            NotesValidation::Full,
            NotesValidation::Fast,
            NotesValidation::None,
        ] {
            let builder = VaultBrowseOptionsBuilder::new(&path).validation(v);
            let (options, _receiver) = builder.build();
            assert_eq!(options.validation, v);
        }
    }

    #[test]
    fn test_vault_browse_options_builder_chaining() {
        let path = VaultPath::new("/test");
        let new_path = VaultPath::new("/new");

        let builder = VaultBrowseOptionsBuilder::new(&path)
            .path(new_path.clone())
            .recursive(true)
            .validation(NotesValidation::Full);

        let (options, _receiver) = builder.build();

        assert_eq!(options.path, new_path);
        assert!(options.recursive);
        assert_eq!(options.validation, NotesValidation::Full);
    }

    #[test]
    fn test_vault_browse_options_build_returns_channel() {
        let path = VaultPath::new("/test");
        let builder = VaultBrowseOptionsBuilder::new(&path);

        let (_options, receiver) = builder.build();

        // Test that the receiver is valid by checking if it's ready to receive
        // (it should be empty initially)
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_notes_validation_display() {
        assert_eq!(format!("{}", NotesValidation::Full), "Full");
        assert_eq!(format!("{}", NotesValidation::Fast), "Fast");
        assert_eq!(format!("{}", NotesValidation::None), "None");
    }

    #[test]
    fn test_vault_browse_options_display() {
        let path = VaultPath::new("/test/path");
        let builder = VaultBrowseOptionsBuilder::new(&path)
            .recursive(true)
            .validation(NotesValidation::Full);

        let (options, _receiver) = builder.build();
        let display_string = format!("{}", options);

        assert!(display_string.contains("Path: `/test/path`"));
        assert!(display_string.contains("Validation Type: `Full`"));
        assert!(display_string.contains("Recursive: `true`"));
    }

    #[test]
    fn test_default_journal_path_constant() {
        assert_eq!(DEFAULT_JOURNAL_PATH, "/journal");
    }

    // Verifies that validate_and_init rejects a vault containing case-insensitive
    // path conflicts (e.g. note.md vs Note.md, projects/ vs Projects/).
    // Linux only: macOS and Windows filesystems are case-insensitive by default,
    // so creating note.md + Note.md would silently overwrite rather than produce two files.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn rejects_vault_with_case_conflicts() {
        let tmp = TempDir::new().unwrap();
        // file conflict at root
        std::fs::write(tmp.path().join("note.md"), "lowercase").unwrap();
        std::fs::write(tmp.path().join("Note.md"), "uppercase").unwrap();
        // directory conflict at root
        std::fs::create_dir(tmp.path().join("projects")).unwrap();
        std::fs::create_dir(tmp.path().join("Projects")).unwrap();

        let vault = NoteVault::new(VaultConfig::new(tmp.path())).await.unwrap();
        let result = vault.validate_and_init().await;

        match result {
            Err(VaultError::CaseConflict { conflicts }) => {
                assert_eq!(
                    conflicts.len(),
                    2,
                    "expected 2 conflicts, got: {:?}",
                    conflicts
                );
                let joined = conflicts.join("\n");
                assert!(
                    joined.contains("note.md") && joined.contains("Note.md"),
                    "expected note.md conflict in list, got: {}",
                    joined
                );
                assert!(
                    joined.contains("projects") && joined.contains("Projects"),
                    "expected projects conflict in list, got: {}",
                    joined
                );
            }
            other => panic!(
                "expected CaseConflict, got: {}",
                match other {
                    Ok(_) => "Ok(_)".to_string(),
                    Err(e) => format!("Err({})", e),
                }
            ),
        }
    }

    #[tokio::test]
    async fn quick_note_creates_timestamped_note_in_inbox() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        vault.validate_and_init().await.unwrap();

        let details = vault.quick_note("my quick thought").await.unwrap();
        let (parent, _) = details.path.get_parent_path();
        assert!(parent.to_string().contains("inbox"));

        let text = vault.get_note_text(&details.path).await.unwrap();
        assert_eq!(text, "my quick thought");
    }

    #[tokio::test]
    async fn quick_note_resolves_conflicts() {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        vault.validate_and_init().await.unwrap();

        let d1 = vault.quick_note("first").await.unwrap();
        let d2 = vault.quick_note("second").await.unwrap();

        assert_ne!(d1.path, d2.path);
        assert_eq!(vault.get_note_text(&d1.path).await.unwrap(), "first");
        assert_eq!(vault.get_note_text(&d2.path).await.unwrap(), "second");
    }

    #[tokio::test]
    async fn quick_note_uses_custom_inbox_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        vault.validate_and_init().await.unwrap();
        vault.set_inbox_path(VaultPath::new("/capture"));

        let details = vault.quick_note("test").await.unwrap();
        let (parent, _) = details.path.get_parent_path();
        assert!(parent.to_string().contains("capture"));
    }

    #[tokio::test]
    async fn create_note_errors_when_file_exists() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        vault.validate_and_init().await.unwrap();

        let path = VaultPath::new("/already.md");
        vault.create_note(&path, "first").await.unwrap();

        match vault.create_note(&path, "second").await {
            Err(VaultError::NoteExists { path: p }) => assert_eq!(p, path.flatten()),
            other => panic!("expected NoteExists, got {:?}", other.err()),
        }

        // The original content must be intact (no overwrite).
        let text = vault.get_note_text(&path).await.unwrap();
        assert_eq!(text, "first");
    }

    #[tokio::test]
    async fn create_directory_errors_when_dir_exists() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        vault.validate_and_init().await.unwrap();

        let path = VaultPath::new("/projects");
        vault.create_directory(&path).await.unwrap();

        match vault.create_directory(&path).await {
            Err(VaultError::DirectoryExists { path: p }) => assert_eq!(p, path),
            other => panic!("expected DirectoryExists, got {:?}", other.err()),
        }
    }

    #[tokio::test]
    async fn rename_note_errors_when_dest_exists() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        vault.validate_and_init().await.unwrap();

        let from = VaultPath::new("/source.md");
        let to = VaultPath::new("/dest.md");
        vault.create_note(&from, "src").await.unwrap();
        vault.create_note(&to, "dst").await.unwrap();

        match vault.rename_note(&from, &to).await {
            Err(VaultError::FSError(FSError::InvalidPath { message, .. })) => {
                assert_eq!(message, "Destination path already exists");
            }
            other => panic!("expected destination-exists error, got {:?}", other.err()),
        }

        // Both files unchanged.
        assert_eq!(vault.get_note_text(&from).await.unwrap(), "src");
        assert_eq!(vault.get_note_text(&to).await.unwrap(), "dst");
    }

    /// Indexing a multi-level directory tree should pick up notes at every depth
    /// in a single pass (recursive walk + single transaction).
    #[tokio::test(flavor = "multi_thread")]
    async fn validate_and_init_indexes_nested_tree() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        std::fs::create_dir_all(root.join("dir1/sub")).unwrap();
        std::fs::write(root.join("a.md"), "# A").unwrap();
        std::fs::write(root.join("dir1/b.md"), "# B").unwrap();
        std::fs::write(root.join("dir1/sub/c.md"), "# C").unwrap();

        let vault = NoteVault::new(VaultConfig::new(root)).await.unwrap();
        vault.validate_and_init().await.unwrap();

        let all = vault.get_all_notes().await.unwrap();
        let names: Vec<String> = all.iter().map(|(e, _)| e.path.to_string()).collect();

        assert_eq!(all.len(), 3, "expected 3 notes, got: {:?}", names);
        assert!(names.iter().any(|p| p.ends_with("/a.md")), "{:?}", names);
        assert!(
            names.iter().any(|p| p.ends_with("/dir1/b.md")),
            "{:?}",
            names
        );
        assert!(
            names.iter().any(|p| p.ends_with("/dir1/sub/c.md")),
            "{:?}",
            names
        );
    }

    /// On a stored DB version older than the current `VERSION`, `check_db`
    /// must report `Outdated` and `validate_and_init` must drop + rebuild the
    /// index. After migration, stale `>`-separated breadcrumb rows are gone
    /// and the new `\x1f` separator is in place.
    #[tokio::test(flavor = "multi_thread")]
    async fn validate_and_init_migrates_outdated_db() {
        use sqlx::Row;

        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n## Sub\nbody text").unwrap();

        // Bring the DB up at the current version with one indexed note.
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        vault.validate_and_init().await.unwrap();
        assert!(vault.validate().await.unwrap().is_ready());

        // Force the schema backwards: stamp version `0.4` and rewrite stored
        // breadcrumbs in the legacy `>`-joined form to simulate a vault
        // upgraded across the separator change.
        let pool = vault.db_pool();
        sqlx::query("UPDATE appData SET value = '0.4' WHERE name = 'version'")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE notesContent SET breadcrumb = REPLACE(breadcrumb, x'1f', '>')")
            .execute(pool)
            .await
            .unwrap();

        // Sanity: the stale row really does contain `>`.
        let stale: Vec<String> =
            sqlx::query("SELECT breadcrumb FROM notesContent WHERE breadcrumb != ''")
                .fetch_all(pool)
                .await
                .unwrap()
                .into_iter()
                .map(|r| r.try_get("breadcrumb").unwrap())
                .collect();
        assert!(
            stale.iter().any(|b| b.contains('>')),
            "expected legacy `>` separator in: {:?}",
            stale
        );

        // Migration: validate flags Outdated, then validate_and_init rebuilds.
        assert_eq!(vault.validate().await.unwrap(), DBStatus::Outdated);
        vault.validate_and_init().await.unwrap();
        assert!(vault.validate().await.unwrap().is_ready());

        // Post-migration: no row carries the legacy separator; non-empty
        // breadcrumbs use `\x1f`.
        let pool = vault.db_pool();
        let after: Vec<String> =
            sqlx::query("SELECT breadcrumb FROM notesContent WHERE breadcrumb != ''")
                .fetch_all(pool)
                .await
                .unwrap()
                .into_iter()
                .map(|r| r.try_get("breadcrumb").unwrap())
                .collect();
        assert!(
            after.iter().all(|b| !b.contains('>')),
            "stale `>` separator survived migration: {:?}",
            after
        );

        // The note is still indexed and `get_note_chunks` exposes a sane
        // `breadcrumb_last` (no `>` artifacts).
        let chunks = vault
            .get_note_chunks(&VaultPath::new("/note.md"))
            .await
            .unwrap();
        let leaves: Vec<String> = chunks
            .values()
            .flatten()
            .filter_map(|c| c.breadcrumb_last().map(|s| s.to_string()))
            .collect();
        assert!(
            leaves.iter().any(|l| l == "Note" || l == "Sub"),
            "expected Note/Sub leaves, got: {:?}",
            leaves
        );
    }
}

#[cfg(test)]
mod vault_config_tests {
    use super::VaultConfig;
    use std::path::PathBuf;

    #[test]
    fn new_sets_workspace_and_no_db_path() {
        let cfg = VaultConfig::new("/tmp/ws");
        assert_eq!(cfg.workspace_path, PathBuf::from("/tmp/ws"));
        assert!(cfg.db_path.is_none());
    }

    #[test]
    fn with_db_path_overrides_default() {
        let cfg = VaultConfig::new("/tmp/ws").with_db_path("/var/cache/foo.kimuncache");
        assert_eq!(
            cfg.db_path.as_deref(),
            Some(std::path::Path::new("/var/cache/foo.kimuncache"))
        );
    }

    #[tokio::test]
    async fn note_vault_new_uses_vault_config_with_legacy_default() {
        use crate::{NoteVault, VaultConfig};
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(tmp.path())).await.unwrap();
        let expected = tmp.path().join("kimun.sqlite");
        assert!(
            expected.exists(),
            "legacy DB path should be used when db_path is None"
        );
        drop(vault);
    }

    #[tokio::test]
    async fn note_vault_new_with_explicit_db_path_uses_override() {
        use crate::{NoteVault, VaultConfig};
        let workspace = tempfile::TempDir::new().unwrap();
        let cache_dir = tempfile::TempDir::new().unwrap();
        let custom_db = cache_dir.path().join("my-vault.kimuncache");
        let vault = NoteVault::new(VaultConfig::new(workspace.path()).with_db_path(&custom_db))
            .await
            .unwrap();
        assert!(custom_db.exists());
        assert!(!workspace.path().join("kimun.sqlite").exists());
        drop(vault);
    }
}
