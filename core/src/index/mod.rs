pub(crate) mod search_terms;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use log::{debug, error};
use search_terms::{OrderBy, SearchTerms};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

pub(crate) mod file;
use sqlx::{Row, Sqlite, Transaction};

use crate::note::{ContentChunk, LinkType, NoteContentData, NoteDetails};

/// A note change reported by the `NoteIndex` the moment it is recorded, for
/// consumers outside core (the RAG client). Thin by design — it carries a path,
/// a content hash, and the kind of change, never chunk text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteChange {
    /// A note was created or its content rewritten. `hash` is the note's
    /// full-text [`NoteContentData::hash`].
    Upsert { path: VaultPath, hash: u64 },
    /// A note was removed from the index.
    Delete { path: VaultPath },
}

/// Observer of index mutations, registered zero-or-one on a vault via
/// [`NoteVault::set_index_observer`](crate::NoteVault::set_index_observer). The
/// index calls [`on_change`](IndexObserver::on_change) synchronously right after
/// a write commits, so an implementation must be cheap and non-blocking — no
/// network, no `await`; fold the event into a queue and drain it elsewhere.
pub trait IndexObserver: Send + Sync + std::fmt::Debug {
    fn on_change(&self, change: &NoteChange);
}

fn row_to_note_entry(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<(NoteEntryData, NoteContentData), DBError> {
    let path: String = row.try_get("path")?;
    let title: String = row.try_get("title")?;
    let size: i64 = row.try_get("size")?;
    let modified: i64 = row.try_get("modified")?;
    let hash: String = row.try_get("hash")?;

    let hash_val: u64 = hash.parse().unwrap_or_else(|e| {
        // A non-numeric hash means a corrupt row (or schema drift). Falling
        // back to 0 lets indexing continue but flags the issue loudly so the
        // operator can rebuild the index.
        log::warn!(
            "Non-numeric hash in DB for {}: {} ({}). Treating as 0.",
            path,
            hash,
            e
        );
        0
    });

    let note_path = VaultPath::new(&path);
    let entry = NoteEntryData {
        path: note_path,
        size: size as u64,
        modified_secs: modified as u64,
    };
    let content = NoteContentData::new(title, hash_val);
    Ok((entry, content))
}

use super::error::DBError;

/// All columns after `path` for `SELECT … FROM notes` queries. Used to build
/// qualified column lists without `.split_once` + `.unwrap()`.
const NOTE_COLUMNS_REST: &str = "title, size, modified, hash, noteName";

/// Column list shared by every `SELECT … FROM notes` query that maps rows
/// through `row_to_note_entry`. Order must match the `try_get` calls there.
const NOTE_COLUMNS: &str = "path, title, size, modified, hash, noteName";

/// Prefixes each comma-separated column name in `cols` with `prefix.`, useful
/// for join queries that disambiguate which table a column comes from.
fn qualify_columns(prefix: &str, cols: &str) -> String {
    cols.split(", ")
        .map(|c| format!("{}.{}", prefix, c))
        .collect::<Vec<_>>()
        .join(", ")
}

use super::{
    nfs::{with_note_extension, NoteEntryData, PATH_SEPARATOR},
    VaultPath,
};

// 0.10: Added `links(source)` and `notes(noteName)` indexes so the forward-link
//       filter `>`/`fwd:` and bare-name source resolution are index-served
//       instead of full scans. Bump forces a clean reindex.
// 0.8: Tightened hashtag word-boundary rule — `##tag`, `#tag#more`, and
//      similar adjacent-`#` patterns are no longer treated as labels. Bump
//      forces a clean reindex so the `labels` table drops the stale rows
//      that the old extractor produced.
// 0.7: Dropped the redundant `labels_by_name` index (the PK autoindex
//      sqlite_autoindex_labels_1 already covers WHERE name = ? lookups).
//      Bump forces a clean reindex so existing 0.6 installs drop the dead
//      index on next launch.
// 0.6: Added `labels` table populated from hashtags in note bodies. Bump
//      forces a clean reindex so the table is filled for existing vaults.
// 0.5: BREADCRUMB_SEP changed from `>` to `\x1f`. Bump forces a clean
//      reindex so stale rows with the old separator are rewritten.
// 0.9: Added `dest_name` column + index to `links` (bare lowercased filename
//      of each link destination) so the `>`/`lk:` link filter matches notes
//      by name with an indexed lookup instead of a leading-`%` scan. Bump
//      forces a clean reindex so the column is populated for existing vaults.
// 0.11: Note paths are now stored in one canonical (vault-absolute) form.
// Existing vaults may hold rows written in relative form (or
//       relative+absolute duplicates) that canonical reads no longer match.
//       Bump forces a clean reindex so every row is rewritten canonical and
//       stale duplicates are dropped.
const VERSION: &str = "0.11";
pub(crate) const DB_FILE: &str = "kimun.sqlite";

/// The diff a vault sync walk produces and `NoteIndex::apply` consumes in
/// one atomic operation — the currency crossing the index's interface.
/// The order of `to_add` and `to_modify` is non-deterministic: they are
/// populated by parallel walker threads and entries land in the order each
/// thread completes its file read.
#[derive(Default)]
pub struct IndexDiff {
    /// Notes present in the vault but absent from the index, each paired with
    /// its full text content for FTS insertion.
    pub to_add: Vec<(NoteEntryData, String)>,
    /// Notes present in both the vault and the index whose content has
    /// changed, each paired with its current text content.
    pub to_modify: Vec<(NoteEntryData, String)>,
    /// Notes present in the index but no longer on disk, to be removed.
    pub to_delete: Vec<VaultPath>,
}

/// The searchable index of the vault — search, suggestions, backlinks, and
/// the index's own lifecycle. The interface speaks in notes, queries, and
/// note links; SQLite, sqlx, transactions, and schema versioning are
/// implementation and never cross it. Atomicity is carried by composite
/// operations ([`apply`](Self::apply), [`rename_note`](Self::rename_note))
/// rather than by exposing transactions.
#[derive(Debug, Clone)]
pub(crate) struct NoteIndex {
    pool: SqlitePool,
    /// `true` while the index is valid but possibly *empty*: set when
    /// [`open`](Self::open) recreated a missing/outdated/invalid schema
    /// (self-heal) or when [`recreate`](Self::recreate) dropped the
    /// tables, cleared by [`mark_synced`](Self::mark_synced) once a vault
    /// sync pass has filled the index. Shared across clones (like the pool)
    /// so every handle agrees on readiness.
    healed: Arc<AtomicBool>,
    /// The registered index observer, if any. Shared across clones (like the
    /// pool) so every handle emits to the same consumer; `None` until a caller
    /// registers one, in which case emission is a no-op.
    observer: Arc<RwLock<Option<Arc<dyn IndexObserver>>>>,
}

impl NoteIndex {
    /// Opens the index at `db_path`, self-healing the schema: when the stored
    /// index is missing, outdated, or invalid, the tables are silently
    /// recreated, leaving a valid but empty index that the next sync pass
    /// fills. [`ready`](Self::ready) reports whether a heal
    /// happened.
    pub(crate) async fn open(index_file: &file::IndexFile) -> Result<Self, DBError> {
        let db_path = index_file.path().as_path().to_owned();
        if let Some(parent) = index_file.path().parent() {
            crate::system::ensure_dir(&parent).map_err(|e| DBError::Other(e.to_string()))?;
        }
        // The path is handed to sqlx as a path, never spliced into a
        // `sqlite:{}?mode=rwc` URL. A formatted URL has to survive URL parsing,
        // so the first `?` in it starts the query string — and on Windows every
        // canonicalized path opens with the verbatim prefix `\\?\`, which made
        // the rest of the path parse as a bogus query parameter and failed
        // every open. `?` and `#` in a vault's own directory name would do the
        // same on any platform. `create_if_missing` is what `mode=rwc` meant.
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options)
            .await?;

        // Only a *readable* schema that is missing or stale heals (the
        // "no such table" case is mapped to `Ok(false)` inside the probe).
        // A probe that errors — SQLITE_BUSY from a concurrent process,
        // transient I/O — propagates and fails the open: silently dropping
        // the tables of a healthy index on a transient error would destroy
        // a valid cache.
        let healed = if Self::schema_is_current(&pool).await? {
            false
        } else {
            debug!("Index schema missing/outdated/invalid — recreating");
            init_db(&pool).await?;
            true
        };

        Ok(Self {
            pool,
            healed: Arc::new(AtomicBool::new(healed)),
            observer: Arc::new(RwLock::new(None)),
        })
    }

    /// Registers the index observer, replacing any previous one. Shared across
    /// clones of this index.
    pub(crate) fn set_observer(&self, observer: Arc<dyn IndexObserver>) {
        *self.observer.write().unwrap() = Some(observer);
    }

    /// Removes the registered observer (if any), so it stops receiving events.
    pub(crate) fn clear_observer(&self) {
        *self.observer.write().unwrap() = None;
    }

    /// Removes the observer only if it is the exact one passed in (by identity).
    /// Lets a consumer deregister its own observer on teardown without wiping a
    /// newer one that has since replaced it.
    pub(crate) fn clear_observer_if(&self, observer: &Arc<dyn IndexObserver>) {
        let mut guard = self.observer.write().unwrap();
        if guard.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, observer)) {
            *guard = None;
        }
    }

    /// Whether an observer is registered. Lets callers skip building events
    /// (e.g. re-hashing every note in a bulk `apply`) when nobody listens.
    fn has_observer(&self) -> bool {
        self.observer.read().unwrap().is_some()
    }

    /// Hands `change` to the registered observer, if any. A no-op when none is
    /// registered. Paths are normalised to canonical vault-relative form so a
    /// note has one identity regardless of the write path that produced it.
    fn emit(&self, change: NoteChange) {
        if let Some(observer) = self.observer.read().unwrap().as_ref() {
            observer.on_change(&change);
        }
    }

    /// Emits an `Upsert`, normalising the path to canonical vault-relative form.
    fn emit_upsert(&self, path: &VaultPath, hash: u64) {
        self.emit(NoteChange::Upsert {
            path: path.canonical(),
            hash,
        });
    }

    /// Emits a `Delete`, normalising the path to canonical vault-relative form.
    fn emit_delete(&self, path: &VaultPath) {
        self.emit(NoteChange::Delete {
            path: path.canonical(),
        });
    }

    /// `false` when the schema was healed ([`open`](Self::open)) or dropped
    /// ([`recreate`](Self::recreate)) and no sync pass has filled the index
    /// since. Fast paths use this to refuse to operate against an empty
    /// index without paying for a sync.
    pub(crate) fn ready(&self) -> bool {
        // Relaxed: the flag is advisory — it gates whether callers bother
        // with a sync, it does not publish index contents (the SQLite pool
        // provides the real synchronization). No happens-before is implied.
        !self.healed.load(Ordering::Relaxed)
    }

    /// Records that a vault sync pass completed: the index now mirrors the
    /// vault on disk, so [`ready`](Self::ready) reports `true` from here on.
    pub(crate) fn mark_synced(&self) {
        self.healed.store(false, Ordering::Relaxed);
    }

    /// `true` when the stored schema version matches [`VERSION`].
    async fn schema_is_current(pool: &SqlitePool) -> Result<bool, DBError> {
        let version: Option<String> =
            sqlx::query_scalar("SELECT value FROM appData WHERE name = 'version'")
                .fetch_optional(pool)
                .await
                .or_else(|e| {
                    // No appData table at all — fresh or foreign file.
                    if e.to_string().contains("no such table") {
                        return Ok(None);
                    }
                    Err(e)
                })?;
        match version {
            Some(v) => {
                debug!("DB Version: {}, current DB Version: {}", v, VERSION);
                Ok(v == VERSION)
            }
            None => Ok(false),
        }
    }

    /// Drops every table and recreates the schema, leaving the index valid
    /// but empty — [`ready`](Self::ready) reports `false` until the full
    /// sync pass that callers are expected to run afterwards
    /// [`mark_synced`](Self::mark_synced)s.
    pub(crate) async fn recreate(&self) -> Result<(), DBError> {
        init_db(&self.pool).await?;
        self.healed.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Applies a sync diff — adds, modifications, deletions — in one atomic
    /// operation.
    pub(crate) async fn apply(&self, diff: IndexDiff) -> Result<(), DBError> {
        let mut tx = self.pool.begin().await?;
        delete_notes(&mut tx, &diff.to_delete).await?;
        insert_notes(&mut tx, &diff.to_add).await?;
        update_notes(&mut tx, &diff.to_modify).await?;
        tx.commit().await?;
        // Skip event construction (notably re-hashing every added/modified note)
        // when nothing is listening — the common case for non-RAG users.
        if self.has_observer() {
            for path in &diff.to_delete {
                self.emit_delete(path);
            }
            for (entry, text) in diff.to_add.iter().chain(diff.to_modify.iter()) {
                self.emit_upsert(&entry.path, NoteDetails::content_data_of(text).hash);
            }
        }
        Ok(())
    }

    /// Renames a note's index rows and updates the rewritten backlink
    /// victims' chunks/links, atomically.
    pub(crate) async fn rename_note(
        &self,
        from: &VaultPath,
        to: &VaultPath,
        rewritten: &[(NoteEntryData, String)],
    ) -> Result<(), DBError> {
        let from = from.canonical();
        let to = to.canonical();
        let mut tx = self.pool.begin().await?;
        // Capture the moving note's hash before the rows change, so observers
        // can be told about the note under its new path (a rename leaves the
        // content — and therefore the hash — untouched).
        let moved_hash = if self.has_observer() {
            note_hash(&mut tx, &from).await?
        } else {
            None
        };
        rename_note(&mut tx, &from, &to).await?;
        update_notes(&mut tx, rewritten).await?;
        tx.commit().await?;
        if self.has_observer() {
            if let Some(hash) = moved_hash {
                self.emit_delete(&from);
                self.emit_upsert(&to, hash);
            }
            // The backlink victims' content changed: their links were
            // rewritten to the new name.
            for (entry, text) in rewritten {
                self.emit_upsert(&entry.path, NoteDetails::content_data_of(text).hash);
            }
        }
        Ok(())
    }

    pub(crate) async fn rename_directory(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<(), DBError> {
        let from = from.canonical();
        let to = to.canonical();
        let mut tx = self.pool.begin().await?;
        // Capture the affected notes before the prefix rewrite, so observers
        // learn both sides of every move.
        let moved = if self.has_observer() {
            notes_under(&mut tx, &from).await?
        } else {
            Vec::new()
        };
        rename_directory(&mut tx, &from, &to).await?;
        tx.commit().await?;
        let from_prefix = dir_prefix(&from);
        let to_prefix = dir_prefix(&to);
        for (path, hash) in moved {
            self.emit_delete(&path);
            // Mirror the SQL prefix rewrite to obtain the post-rename path.
            if let Some(rest) = path.to_string().strip_prefix(&from_prefix) {
                self.emit_upsert(&VaultPath::new(format!("{to_prefix}{rest}")), hash);
            }
        }
        Ok(())
    }

    pub(crate) async fn delete_notes(&self, paths: &[VaultPath]) -> Result<(), DBError> {
        let canonical: Vec<VaultPath> = paths.iter().map(|p| p.canonical()).collect();
        let mut tx = self.pool.begin().await?;
        delete_notes(&mut tx, &canonical).await?;
        tx.commit().await?;
        for path in &canonical {
            self.emit_delete(path);
        }
        Ok(())
    }

    pub(crate) async fn delete_directories(
        &self,
        directories: &[VaultPath],
    ) -> Result<(), DBError> {
        let canonical: Vec<VaultPath> = directories.iter().map(|p| p.canonical()).collect();
        let mut tx = self.pool.begin().await?;
        // Capture the contained notes before the rows go, so observers get a
        // Delete per note — same contract as delete_notes above.
        let mut removed = Vec::new();
        if self.has_observer() {
            for directory in &canonical {
                removed.extend(notes_under(&mut tx, directory).await?);
            }
        }
        delete_directories(&mut tx, &canonical).await?;
        tx.commit().await?;
        for (path, _) in removed {
            self.emit_delete(&path);
        }
        Ok(())
    }

    /// Indexes one saved note and returns its computed content data (title
    /// + hash), so callers never parse the note a second time.
    pub(crate) async fn save_note(
        &self,
        entry_data: &NoteEntryData,
        note_details: &NoteDetails,
    ) -> Result<NoteContentData, DBError> {
        let data = save_note(&self.pool, entry_data, note_details).await?;
        self.emit_upsert(&entry_data.path, data.hash);
        Ok(data)
    }

    pub(crate) async fn search<S: AsRef<str>>(
        &self,
        search_query: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        let search_query = search_query.as_ref();
        let search_terms = SearchTerms::from_query_string(search_query);
        let (query, params) = build_search_sql_query_inner(&search_terms);
        let order_by = search_terms.order_by;

        if query.is_empty() {
            debug!("No query provided");
            return Ok(vec![]);
        }

        debug!("QUERY: {}", query);

        let mut sql_query = sqlx::query(&query);
        for param in params {
            sql_query = sql_query.bind(param);
        }

        let rows = sql_query.fetch_all(&self.pool).await?;

        let mut result: Vec<(NoteEntryData, NoteContentData)> = rows
            .iter()
            .map(row_to_note_entry)
            .collect::<Result<_, _>>()?;

        if !order_by.is_empty() {
            result.sort_by(|(a_entry, a_content), (b_entry, b_content)| {
                for ob in &order_by {
                    let ord = match ob {
                        OrderBy::Title { asc } => {
                            let cmp = a_content
                                .title
                                .to_lowercase()
                                .cmp(&b_content.title.to_lowercase());
                            if *asc {
                                cmp
                            } else {
                                cmp.reverse()
                            }
                        }
                        OrderBy::FileName { asc } => {
                            let cmp = a_entry.path.to_string().cmp(&b_entry.path.to_string());
                            if *asc {
                                cmp
                            } else {
                                cmp.reverse()
                            }
                        }
                    };
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        Ok(result)
    }

    pub(crate) async fn search_note_by_name<S: AsRef<str>>(
        &self,
        name: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        let name = name.as_ref().to_lowercase();
        let sql = format!("SELECT {} FROM notes where noteName = ?", NOTE_COLUMNS);
        let rows = sqlx::query(&sql).bind(&name).fetch_all(&self.pool).await?;

        rows.iter().map(row_to_note_entry).collect()
    }

    pub(crate) async fn search_note_by_path(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        let path = path.canonical();
        let sql = format!("SELECT {} FROM notes where path = ?", NOTE_COLUMNS);
        let path_string = path.to_string();
        let rows = sqlx::query(&sql)
            .bind(&path_string)
            .fetch_all(&self.pool)
            .await?;

        // Should always return one or zero
        rows.iter().map(row_to_note_entry).collect()
    }

    pub(crate) async fn get_notes(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        let path = path.canonical();
        let (where_clause, bind_value) = if recursive {
            // The note's own `path`, not `basePath`: `basePath` is stored
            // without a trailing separator, so a `<dir>/`-prefixed LIKE would
            // miss the directory's direct children. Matching the full path
            // against the same `dir_prefix` the rename/delete wrappers use
            // keeps `/foo` from also matching a sibling `/foobar/`.
            (
                "path LIKE (? || '%') ESCAPE '\\'".to_string(),
                escape_like_pattern(&dir_prefix(&path)),
            )
        } else {
            ("basePath = ?".to_string(), path.to_string())
        };
        let sql = format!("SELECT {} FROM notes where {}", NOTE_COLUMNS, where_clause);
        let rows = sqlx::query(&sql)
            .bind(bind_value)
            .fetch_all(&self.pool)
            .await?;

        rows.iter().map(row_to_note_entry).collect()
    }

    pub(crate) async fn get_all_notes(
        &self,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        let query = format!("SELECT DISTINCT {} FROM notes", NOTE_COLUMNS);
        let rows = sqlx::query(&query).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_note_entry).collect()
    }

    /// Backlinks of a *specific* note: notes whose body links to exactly this note,
    /// matched by its full path OR its bare filename (wikilinks stored without a
    /// path). This is intentionally narrower than the `>`/`lk:` search filter
    /// (see [`link_subquery`]), which matches a name in *any* folder; keep the two
    /// in step on the stored-form invariant they share (lowercased, `.md`-suffixed
    /// destinations, bare-relative or relative/absolute path).
    pub(crate) async fn get_backlinks(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        let path = path.canonical();
        // Match notes that link to the full path OR by filename only (wikilinks stored without path)
        let sql = format!(
            "SELECT DISTINCT {cols} \
             FROM notes n \
             JOIN links l ON n.path = l.source \
             WHERE l.destination = ? OR l.destination = ?",
            cols = qualify_columns("n", NOTE_COLUMNS),
        );
        let rows = sqlx::query(&sql)
            .bind(path.to_string())
            .bind(path.get_name())
            .fetch_all(&self.pool)
            .await?;

        rows.iter().map(row_to_note_entry).collect()
    }

    pub(crate) async fn get_notes_sections(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<HashMap<VaultPath, Vec<ContentChunk>>, DBError> {
        let path = path.canonical();
        let mut result = HashMap::new();
        let (sql, bind_value) = if path.is_note() {
            // Exact note path
            (
                "SELECT path, breadcrumb, text FROM notesContent WHERE path = ?".to_string(),
                path.to_string(),
            )
        } else if recursive {
            // All notes under this directory tree
            (
                "SELECT path, breadcrumb, text FROM notesContent WHERE path LIKE (? || '%') ESCAPE '\\'".to_string(),
                escape_like_pattern(&dir_prefix(&path)),
            )
        } else {
            // Only notes directly in this directory (basePath join)
            ("SELECT nc.path, nc.breadcrumb, nc.text FROM notesContent nc JOIN notes n ON nc.path = n.path WHERE n.basePath = ?".to_string(), path.to_string())
        };

        let rows = sqlx::query(&sql)
            .bind(bind_value)
            .fetch_all(&self.pool)
            .await?;

        for row in rows {
            let path: String = row.try_get("path")?;
            let breadcrumb: String = row.try_get("breadcrumb")?;
            let text: String = row.try_get("text")?;

            let path = VaultPath::new(path);
            let chunk = ContentChunk { breadcrumb, text };
            result.entry(path).or_insert_with(Vec::new).push(chunk);
        }

        Ok(result)
    }

    pub(crate) async fn list_labels(&self) -> Result<Vec<String>, DBError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT name FROM labels")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }

    pub(crate) async fn label_counts(&self) -> Result<Vec<(String, i64)>, DBError> {
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT name, COUNT(*) as cnt FROM labels GROUP BY name ORDER BY name")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub(crate) async fn notes_with_label(&self, name: &str) -> Result<Vec<VaultPath>, DBError> {
        let normalized = name.to_lowercase();
        let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM labels WHERE name = ?")
            .bind(&normalized)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|(p,)| VaultPath::new(p)).collect())
    }

    /// Returns notes whose `noteName` starts with `prefix` (case-insensitive),
    /// capped at `limit`. Empty prefix returns the top `limit` notes by name.
    ///
    /// Results are ordered alphabetically by name. Notes that share a name are
    /// both returned as separate rows; callers (the autocomplete UI) are
    /// responsible for disambiguating them via `path`.
    ///
    /// The returned `name` is the note's filename with the extension stripped
    /// (via `VaultPath::get_clean_name`) — i.e. the exact text a wikilink
    /// targets. Filenames in the index are already lowercased on insert
    /// (see `VaultPathSlice::new`), so callers get lowercase names back.
    pub(crate) async fn suggest_notes_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<NoteSuggestion>, DBError> {
        let pattern = format!("{}%", escape_like_pattern(&prefix.to_lowercase()));
        // `noteName` is lowercased on insert, so `LIKE` against a lowercased
        // pattern is naturally case-insensitive; the explicit `LOWER()` is a
        // defensive belt-and-braces against any future code path that might
        // insert mixed case.
        let sql = "SELECT path \
                   FROM notes \
                   WHERE LOWER(noteName) LIKE ?1 ESCAPE '\\' \
                   ORDER BY noteName ASC, path ASC \
                   LIMIT ?2";
        let rows: Vec<(String,)> = sqlx::query_as(sql)
            .bind(&pattern)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(path,)| {
                let vault_path = VaultPath::new(path);
                let name = vault_path.get_clean_name();
                NoteSuggestion {
                    name,
                    path: vault_path,
                }
            })
            .collect())
    }

    /// Returns tag labels whose name starts with `prefix` (case-insensitive),
    /// each paired with how many notes carry the tag, capped at `limit`. Empty
    /// prefix returns the top `limit` tags by usage.
    ///
    /// The `labels` table is stored lowercased, so prefix matching is naturally
    /// case-insensitive once we lowercase the input. Ranking is `usage_count
    /// DESC, label ASC` so the most-used tags surface first.
    pub(crate) async fn suggest_tags_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<TagSuggestion>, DBError> {
        let pattern = format!("{}%", escape_like_pattern(&prefix.to_lowercase()));
        let sql = "SELECT name, COUNT(*) AS cnt \
                   FROM labels \
                   WHERE name LIKE ?1 ESCAPE '\\' \
                   GROUP BY name \
                   ORDER BY cnt DESC, name ASC \
                   LIMIT ?2";
        let rows: Vec<(String, i64)> = sqlx::query_as(sql)
            .bind(&pattern)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(label, cnt)| TagSuggestion {
                label,
                usage_count: cnt.max(0) as u32,
            })
            .collect())
    }
}

impl NoteIndex {
    /// Closes the pool, releasing the index file's handles now rather than
    /// whenever the last clone happens to drop.
    ///
    /// Dropping a pool schedules the close; it does not perform it, so the file
    /// can still be open afterwards. On Windows that is the difference between
    /// being able to rename or delete the index file and getting "The process
    /// cannot access the file because it is being used by another process".
    pub(crate) async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
impl NoteIndex {
    /// The schema seam: for tests of the schema itself — the version stamp,
    /// self-heal on open, query plans — which assert facts the interface
    /// cannot observe. Behaviour tests never use it: they apply an
    /// [`IndexDiff`] and query (CONTEXT.md, NoteIndex; ADR-0008 as amended).
    fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Deletes all tables and recreates them
async fn init_db(pool: &SqlitePool) -> Result<(), DBError> {
    debug!("Deleting DB");
    delete_db(pool).await?;
    debug!("Creating Tables");
    create_tables(pool).await
}

async fn delete_db(pool: &SqlitePool) -> Result<(), DBError> {
    let rows = sqlx::query("SELECT name FROM sqlite_schema WHERE type = 'table'")
        .fetch_all(pool)
        .await?;

    let mut tables = vec![];
    for row in rows {
        let table_name: String = row.try_get("name")?;
        tables.push(table_name);
    }

    for table in tables {
        // Can't use params for tables or columns, so we use format!
        let drop_query = format!("DROP TABLE '{}'", table);
        match sqlx::query(&drop_query).execute(pool).await {
            Ok(_) => {}
            Err(e) => {
                if table.contains("_") {
                    // Some virtual tables are automatically deleted
                    debug!("Error for table {}: {}", table, e);
                } else {
                    return Err(DBError::DBError(e));
                }
            }
        }
    }

    sqlx::query("VACUUM").execute(pool).await?;
    Ok(())
}

async fn create_tables(pool: &SqlitePool) -> Result<(), DBError> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE appData (
            name TEXT PRIMARY KEY,
            value TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO appData (name, value) VALUES (?, ?)")
        .bind("version")
        .bind(VERSION)
        .execute(&mut *tx)
        .await?;

    // Storing hash as a string, as SQLite doesn't like
    // unsigned 64bit integers, alternatively we could
    // have used signed numbers by subtracting the half
    // of the max value
    sqlx::query(
        "CREATE TABLE notes (
            path TEXT PRIMARY KEY,
            title TEXT,
            hash TEXT,
            size INTEGER,
            modified INTEGER,
            basePath TEXT,
            noteName TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE links (
            source TEXT,
            destination TEXT,
            dest_name TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX backlinks
            ON links(destination)",
    )
    .execute(&mut *tx)
    .await?;

    // Backs the `<`/`lk:` backlink filter's name-anywhere match (folder-independent
    // bare filename), so it never has to scan with a leading-`%` LIKE.
    sqlx::query(
        "CREATE INDEX links_by_dest_name
            ON links(dest_name)",
    )
    .execute(&mut *tx)
    .await?;

    // Backs the `>`/`fwd:` forward-link filter, which filters/joins on
    // `links.source`, so it never has to full-scan the links table.
    sqlx::query(
        "CREATE INDEX links_by_source
            ON links(source)",
    )
    .execute(&mut *tx)
    .await?;

    // Backs bare-name source resolution (the `>`/`fwd:` filter joins links
    // back to `notes.noteName`), so the join is index-served instead of a
    // full scan.
    sqlx::query(
        "CREATE INDEX notes_by_name
            ON notes(noteName)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE VIRTUAL TABLE notesContent USING fts4(
            path,
            breadcrumb,
            text
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE labels (
            name TEXT NOT NULL,
            path TEXT NOT NULL,
            PRIMARY KEY (name, path)
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX labels_by_path
            ON labels(path)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}

/// Joins the positive and negative conditions of one operator class. Positives
/// AND together (each same-type term must match, matching the documented
/// "all terms are ANDed" precedence and the `#`/`>`/`<` operators); negatives
/// already AND.
fn combine_conditions(positive: Vec<String>, negative: Vec<String>) -> Option<String> {
    match (positive.is_empty(), negative.is_empty()) {
        (true, true) => None,
        (false, true) => Some(positive.join(" AND ")),
        (true, false) => Some(negative.join(" AND ")),
        (false, false) => Some(format!(
            "{} AND {}",
            positive.join(" AND "),
            negative.join(" AND ")
        )),
    }
}

fn build_like_conditions(
    positive_terms: &[String],
    negative_terms: &[String],
    pos_condition_fn: impl Fn(usize) -> String,
    neg_condition_fn: impl Fn(usize) -> String,
    var_num: &mut usize,
    params: &mut Vec<String>,
    push_term_fn: impl Fn(&String) -> String,
) -> Option<String> {
    let mut positive_conditions = vec![];
    let mut negative_conditions = vec![];

    for term in positive_terms {
        if !term.is_empty() {
            positive_conditions.push(pos_condition_fn(*var_num));
            params.push(push_term_fn(term));
            *var_num += 1;
        }
    }

    for term in negative_terms {
        if !term.is_empty() {
            negative_conditions.push(neg_condition_fn(*var_num));
            params.push(push_term_fn(term));
            *var_num += 1;
        }
    }

    combine_conditions(positive_conditions, negative_conditions)
}

/// Base query for the search fan-out. Aliases `notes.path` to `path` so the
/// shared `row_to_note_entry` mapper finds all `NOTE_COLUMNS` keys. First
/// column is qualified to disambiguate the `notesContent`/`notes` join; the
/// rest are unique to `notes` and need no prefix.
static SEARCH_BASE_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "SELECT DISTINCT notes.path as path, {} FROM notesContent JOIN notes ON notesContent.path = notes.path",
        NOTE_COLUMNS_REST
    )
});

fn search_base_sql() -> &'static str {
    &SEARCH_BASE_SQL
}

fn build_search_sql_query_inner(search_terms: &SearchTerms) -> (String, Vec<String>) {
    let mut var_num = 1;
    let mut params: Vec<String> = vec![];
    let mut queries: Vec<String> = vec![];

    add_fts_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_filename_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_path_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_labels_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_links_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_forward_links_query(search_terms, &mut var_num, &mut params, &mut queries);

    if queries.is_empty() {
        debug!("No query provided");
        return (String::new(), vec![]);
    }
    (queries.join(" INTERSECT "), params)
}

/// Free-text + breadcrumb FTS branches. Content (whole-row) and breadcrumb
/// (heading-path column) are *separate* INTERSECT branches: FTS4 allows only
/// one `MATCH` per virtual table per SELECT, and its in-MATCH column filter
/// (`breadcrumb:"x"`) is unreliable across builds, so the two cannot be folded
/// into a single scan. Within each branch, the positive `MATCH` is ANDed with
/// `NOT IN` subqueries for that field's exclusions (FTS4 has no reliable
/// pure-negative / inline `-term`, so a subquery is used uniformly).
fn add_fts_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    add_fts_field_query(
        &s.terms,
        &s.excluded_terms,
        "notesContent",
        fts4_quote,
        var_num,
        params,
        queries,
    );
    add_fts_field_query(
        &s.breadcrumb,
        &s.excluded_breadcrumb,
        "notesContent.breadcrumb",
        fts4_quote,
        var_num,
        params,
        queries,
    );
}

/// Emits one FTS branch for a single field (`notesContent` for content,
/// `notesContent.breadcrumb` for headings): a positive `MATCH` (all positive
/// terms space-joined into one query) ANDed with one `NOT IN` subquery per
/// excluded term. Pure-exclusion (no positives) drops the leading `MATCH`.
fn add_fts_field_query(
    positives: &[String],
    excludeds: &[String],
    match_target: &str,
    quote: impl Fn(&str) -> String,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    if positives.is_empty() && excludeds.is_empty() {
        return;
    }

    let mut conditions: Vec<String> = vec![];

    if !positives.is_empty() {
        conditions.push(format!("{} MATCH ?{}", match_target, var_num));
        params.push(
            positives
                .iter()
                .map(|t| quote(t))
                .collect::<Vec<_>>()
                .join(" "),
        );
        *var_num += 1;
    }

    for term in excludeds {
        conditions.push(format!(
            "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE {} MATCH ?{})",
            match_target, var_num
        ));
        params.push(quote(term));
        *var_num += 1;
    }

    queries.push(format!(
        "{} WHERE {}",
        search_base_sql(),
        conditions.join(" AND ")
    ));
}

fn add_filename_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    if s.filename.is_empty() && s.excluded_filename.is_empty() {
        return;
    }
    if let Some(final_where) = build_like_conditions(
        &s.filename,
        &s.excluded_filename,
        |n| format!("notes.noteName LIKE ?{} ESCAPE '\\'", n),
        |n| format!("notes.noteName NOT LIKE ?{} ESCAPE '\\'", n),
        var_num,
        params,
        |t: &String| {
            if t.contains('*') {
                // Explicit wildcard: extension-aware whole-name match, * → %.
                // Escape first (so literal % / _ stay escaped), then * → %.
                escape_like_pattern(&with_note_extension(t)).replace('*', "%")
            } else {
                // Substring match (unchanged behaviour).
                format!("%{}%", escape_like_pattern(t))
            }
        },
    ) {
        queries.push(format!("{} WHERE {}", search_base_sql(), final_where));
    }
}

fn add_path_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    if s.path.is_empty() && s.excluded_path.is_empty() {
        return;
    }
    let positive_conditions = path_term_conditions(&s.path, var_num, params, true);
    let negative_conditions = path_term_conditions(&s.excluded_path, var_num, params, false);
    if let Some(final_where) = combine_conditions(positive_conditions, negative_conditions) {
        queries.push(format!("{} WHERE {}", search_base_sql(), final_where));
    }
}

/// Notes-only base SELECT (no `notesContent` join) so membership-style filters
/// (labels, links) don't pay an FTS scan. No `DISTINCT`: `notes.path` is the
/// primary key, so every notes-only branch already yields unique paths (and
/// `INTERSECT` dedups across branches regardless). Same columns as
/// `SEARCH_BASE_SQL` so INTERSECT branches line up.
static NOTES_BASE_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "SELECT notes.path as path, {} FROM notes",
        NOTE_COLUMNS_REST
    )
});

fn notes_base_sql() -> &'static str {
    &NOTES_BASE_SQL
}

/// Fan-out shared by membership-style operators (labels, links): each positive
/// term becomes its own INTERSECT branch (`notes.path IN (subquery)`); excluded
/// terms are bundled into one notes-only SELECT chaining `NOT IN (subquery)` so
/// the INTERSECT machinery still composes. `mk_subquery(term, var_num, params)`
/// returns the inner `SELECT <col> FROM …` for one term (pushing its bind
/// params and advancing `var_num`), or `None` to skip a degenerate term.
fn add_membership_query<F>(
    positives: &[String],
    excludeds: &[String],
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
    mk_subquery: F,
) where
    F: Fn(&str, &mut usize, &mut Vec<String>) -> Option<String>,
{
    for term in positives {
        if let Some(sub) = mk_subquery(term, var_num, params) {
            queries.push(format!(
                "{} WHERE notes.path IN ({})",
                notes_base_sql(),
                sub
            ));
        }
    }

    if excludeds.is_empty() {
        return;
    }
    let mut clauses = Vec::with_capacity(excludeds.len());
    for term in excludeds {
        if let Some(sub) = mk_subquery(term, var_num, params) {
            clauses.push(format!("notes.path NOT IN ({})", sub));
        }
    }
    if !clauses.is_empty() {
        queries.push(format!(
            "{} WHERE {}",
            notes_base_sql(),
            clauses.join(" AND ")
        ));
    }
}

fn add_labels_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    // Each label is matched via the labels PK autoindex.
    add_membership_query(
        &s.labels,
        &s.excluded_labels,
        var_num,
        params,
        queries,
        |label, var_num, params| {
            let sub = format!("SELECT path FROM labels WHERE name = ?{}", var_num);
            params.push(label.to_string());
            *var_num += 1;
            Some(sub)
        },
    );
}

/// Builds the `SELECT source FROM links WHERE …` subquery for one link-filter
/// target (`>`/`lk:`). Matching is by note name, extension optional,
/// case-insensitive, with `*` wildcards.
///
/// A bare name matches a link in *any* folder via the indexed `dest_name`
/// column (the folder-independent basename) — no leading-`%` scan. A slash in
/// the target anchors it to a full path via indexed equality on `destination`
/// (covering both the relative and absolute stored forms). Wildcards fall back
/// to `LIKE`, but the pattern is prefix-anchored so the index can still help
/// (e.g. `proj*`).
///
/// This is the name-anywhere counterpart to [`get_backlinks`], which matches a
/// *specific* note by its exact full path or bare name. Both rely on the same
/// stored-form invariant: link destinations are lowercased, carry the note
/// extension, and are either a bare relative name or a relative/absolute path.
/// Returns `None` for an empty target.
/// Normalized pieces of a link-filter target, shared by [`link_subquery`]
/// (backlinks) and [`forward_link_subquery`] (forward links). Only the SQL
/// column names differ between the two; the normalization is identical.
struct LinkTarget {
    /// Lowercased, extension-applied note name/path used as the bound param.
    name: String,
    /// `true` when the target contains a path separator (anchor to full path).
    is_path_qualified: bool,
    /// `true` when the target contains a `*` wildcard (use `LIKE`).
    has_wildcard: bool,
}

/// Normalize a link-filter target: trim/lowercase, strip a leading separator,
/// detect path-qualified / wildcard, and apply the note extension. Returns
/// `None` for an empty target.
///
/// A leading separator only signals "absolute"; both stored forms are matched
/// anyway, so it is stripped before normalizing.
fn normalize_link_target(target: &str) -> Option<LinkTarget> {
    let lowered = target.trim().to_lowercase();
    let stripped = lowered.strip_prefix(PATH_SEPARATOR).unwrap_or(&lowered);
    if stripped.is_empty() {
        return None;
    }
    let is_path_qualified = stripped.contains(PATH_SEPARATOR);
    let has_wildcard = stripped.contains('*');
    let name = with_note_extension(stripped);
    Some(LinkTarget {
        name,
        is_path_qualified,
        has_wildcard,
    })
}

fn link_subquery(target: &str, var_num: &mut usize, params: &mut Vec<String>) -> Option<String> {
    let LinkTarget {
        name,
        is_path_qualified,
        has_wildcard,
    } = normalize_link_target(target)?;

    let body = if has_wildcard {
        // Escape LIKE metacharacters in the literal, then turn user `*` into
        // the SQL `%` wildcard. Destinations never contain `*`, so this is safe.
        let pattern = escape_like_pattern(&name).replace('*', "%");
        params.push(pattern);
        let body = if is_path_qualified {
            format!(
                "destination LIKE ?{n} ESCAPE '\\' OR destination LIKE ('/' || ?{n}) ESCAPE '\\'",
                n = var_num
            )
        } else {
            format!("dest_name LIKE ?{n} ESCAPE '\\'", n = var_num)
        };
        *var_num += 1;
        body
    } else if is_path_qualified {
        // Indexed equality on the full path (relative or absolute stored form).
        params.push(name);
        let body = format!(
            "destination = ?{n} OR destination = ('/' || ?{n})",
            n = var_num
        );
        *var_num += 1;
        body
    } else {
        // Indexed equality on the folder-independent basename.
        params.push(name);
        let body = format!("dest_name = ?{n}", n = var_num);
        *var_num += 1;
        body
    };
    Some(format!("SELECT source FROM links WHERE {body}"))
}

/// Backlinks filter (`<` / `lk:`). Each positive target is its own INTERSECT
/// branch (AND semantics, like labels); exclusions are bundled into a single
/// notes-only SELECT chaining `NOT IN`.
fn add_links_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    add_membership_query(
        &s.links,
        &s.excluded_links,
        var_num,
        params,
        queries,
        link_subquery,
    );
}

/// Builds the subquery of NOTE PATHS that are link *destinations* of the
/// note(s) named `target` — i.e. the forward-links of `target` (`>` / `fwd:`).
///
/// Where [`link_subquery`] selects the *sources* of links pointing at a name,
/// this selects the *destinations* of links emitted by the note(s) matching the
/// name. The destination column is heterogeneous (a bare relative name, or a
/// relative/absolute path), so resolving it back to a concrete note path is
/// done by joining `notes n2` and matching on all stored forms:
///   - `l.dest_name = n2.noteName` (folder-independent bare basename, both
///     carry the `.md` extension), or
///   - `l.destination = n2.path` (relative stored form), or
///   - `l.destination = '/' || n2.path` (absolute stored form).
///
/// The *source* note is matched by name exactly as [`link_subquery`] matches a
/// target: bare name → indexed equality on `src.noteName` (the lowercased
/// basename, `.md`-suffixed); path-qualified → equality on `src.path` (relative
/// or absolute); wildcards → `LIKE`. Returns `None` for an empty target.
fn forward_link_subquery(
    target: &str,
    var_num: &mut usize,
    params: &mut Vec<String>,
) -> Option<String> {
    let LinkTarget {
        name,
        is_path_qualified,
        has_wildcard,
    } = normalize_link_target(target)?;

    let src_match = if has_wildcard {
        let pattern = escape_like_pattern(&name).replace('*', "%");
        params.push(pattern);
        let body = if is_path_qualified {
            format!(
                "src.path LIKE ?{n} ESCAPE '\\' OR src.path LIKE ('/' || ?{n}) ESCAPE '\\'",
                n = var_num
            )
        } else {
            format!("src.noteName LIKE ?{n} ESCAPE '\\'", n = var_num)
        };
        *var_num += 1;
        body
    } else if is_path_qualified {
        params.push(name);
        let body = format!("src.path = ?{n} OR src.path = ('/' || ?{n})", n = var_num);
        *var_num += 1;
        body
    } else {
        params.push(name);
        let body = format!("src.noteName = ?{n}", n = var_num);
        *var_num += 1;
        body
    };

    Some(format!(
        "SELECT n2.path FROM notes n2 \
         JOIN links l ON (l.dest_name = n2.noteName \
                          OR l.destination = n2.path \
                          OR l.destination = ('/' || n2.path)) \
         JOIN notes src ON src.path = l.source \
         WHERE {src_match}"
    ))
}

/// Forward-links filter (`>` / `fwd:`). Mirrors [`add_links_query`] but over
/// `forward_links`/`excluded_forward_links` with [`forward_link_subquery`], so
/// forward-link branches INTERSECT/compose like every other membership filter.
fn add_forward_links_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    add_membership_query(
        &s.forward_links,
        &s.excluded_forward_links,
        var_num,
        params,
        queries,
        forward_link_subquery,
    );
}

/// Builds basePath conditions for path-style search terms. A trailing
/// `PATH_SEPARATOR` means an exact directory match; otherwise the term is a
/// prefix. `positive` selects the operator family (`=` / `LIKE` vs.
/// `!=` / `NOT LIKE`).
fn path_term_conditions(
    terms: &[String],
    var_num: &mut usize,
    params: &mut Vec<String>,
    positive: bool,
) -> Vec<String> {
    let mut out = vec![];
    for term in terms {
        if term.is_empty() {
            continue;
        }
        let (cond, value) = if term.contains('*') {
            // Explicit wildcard: anchor at the leading separator, translate the
            // user's `*` into the SQL `%` wildcard (escape first so any literal
            // `%`/`_` stays literal). No auto-appended `%` — the `*` placement
            // fully controls matching (e.g. `/work*` = prefix, `/wo*k` = infix).
            let op = if positive { "LIKE" } else { "NOT LIKE" };
            (
                format!("notes.basePath {} ('/' || ?{}) ESCAPE '\\'", op, var_num),
                escape_like_pattern(term).replace('*', "%"),
            )
        } else {
            match term.strip_suffix(PATH_SEPARATOR) {
                Some(absolute) => {
                    let op = if positive { "=" } else { "!=" };
                    (
                        format!("notes.basePath {} ('/' || ?{})", op, var_num),
                        absolute.to_string(),
                    )
                }
                None => {
                    let op = if positive { "LIKE" } else { "NOT LIKE" };
                    (
                        format!(
                            "notes.basePath {} ('/' || ?{} || '%') ESCAPE '\\'",
                            op, var_num
                        ),
                        escape_like_pattern(term),
                    )
                }
            }
        };
        out.push(cond);
        params.push(value);
        *var_num += 1;
    }
    out
}

#[cfg(test)]
fn build_search_sql_query<S: AsRef<str>>(query: S) -> (String, Vec<String>) {
    let search_terms = SearchTerms::from_query_string(query);
    build_search_sql_query_inner(&search_terms)
}

/// A note suggestion for the autocomplete popup.
///
/// `name` is the note's filename without extension — the string a wikilink
/// actually targets, since wikilinks are stored by name, not by full vault
/// path (see `get_backlinks` and the surrounding `noteName` column).
/// `path` is carried so the UI can disambiguate when multiple notes share a
/// name, but the wikilink target inserted on accept is `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSuggestion {
    /// The note's filename without extension — the exact text a wikilink
    /// targets, since wikilinks are stored by name rather than by full path.
    pub name: String,
    /// The note's full vault path, so the UI can disambiguate when several
    /// notes share a `name`. The link inserted on accept is still `name`.
    pub path: VaultPath,
}

/// A tag suggestion for the autocomplete popup. `usage_count` is computed
/// per-query via `COUNT(*) GROUP BY name` over the `labels` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSuggestion {
    /// The label text (lowercased, as stored in the `labels` table).
    pub label: String,
    /// How many notes carry this label, computed per-query via
    /// `COUNT(*) GROUP BY name`, so the UI can rank common tags first.
    pub usage_count: u32,
}

async fn insert_notes(
    tx: &mut Transaction<'_, Sqlite>,
    notes: &[(NoteEntryData, String)],
) -> Result<(), DBError> {
    if notes.is_empty() {
        return Ok(());
    }
    debug!("Inserting {} notes", notes.len());
    upsert_notes_batched(tx, notes).await
}

async fn update_notes(
    tx: &mut Transaction<'_, Sqlite>,
    notes: &[(NoteEntryData, String)],
) -> Result<(), DBError> {
    if notes.is_empty() {
        return Ok(());
    }
    debug!("Updating {} notes", notes.len());
    upsert_notes_batched(tx, notes).await
}

async fn delete_notes(
    tx: &mut Transaction<'_, Sqlite>,
    paths: &[VaultPath],
) -> Result<(), DBError> {
    if paths.is_empty() {
        return Ok(());
    }
    let path_strings: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
    bulk_delete_in(tx, "notes", &["path"], &path_strings).await?;
    bulk_delete_in(tx, "notesContent", &["path"], &path_strings).await?;
    bulk_delete_in(tx, "links", &["source", "destination"], &path_strings).await?;
    bulk_delete_in(tx, "labels", &["path"], &path_strings).await?;
    Ok(())
}

async fn save_note(
    pool: &SqlitePool,
    entry_data: &NoteEntryData,
    note_details: &NoteDetails,
) -> Result<NoteContentData, DBError> {
    // Parse once and hand the computed content data back to the caller, so
    // the full-text hash + title extraction is never done twice per save.
    let data = note_details.get_content_data();
    let (chunks, links) = note_details.get_chunks_and_links();
    let label_count = links
        .iter()
        .filter(|l| matches!(l.ltype, LinkType::Hashtag))
        .count();
    let mut batch = NoteBatch::with_capacity(1, chunks.len(), links.len(), label_count);
    batch.push(entry_data, data.clone(), chunks, links);

    let mut tx = pool.begin().await?;
    batch.flush(&mut tx).await?;
    tx.commit().await?;
    Ok(data)
}

// SQLite default parameter limit is 999. Stay under for safety.
const SQLITE_PARAM_BUDGET: usize = 900;

struct NoteRow {
    path_idx: usize,
    title: String,
    size: i64,
    modified: i64,
    hash: String,
    base_path: String,
    name: String,
}

struct ChunkRow {
    path_idx: usize,
    breadcrumb: String,
    text: String,
}

struct LinkRow {
    path_idx: usize,
    destination: String,
    /// Bare lowercased filename of `destination` (folder-independent), indexed
    /// to back the link filter's name-anywhere match. See `link_subquery`.
    dest_name: String,
}

struct LabelRow {
    path_idx: usize,
    name: String,
}

/// Bulk-upserts a slice of notes plus their chunks and links inside the given
/// transaction. Each note's raw text is parsed once; chunks/links are bound by
/// `path_idx` into a shared `paths` table to avoid per-row clones. Inserts
/// chunk via `bulk_insert` so binds-per-statement stay under
/// `SQLITE_PARAM_BUDGET`.
async fn upsert_notes_batched(
    tx: &mut Transaction<'_, Sqlite>,
    notes: &[(NoteEntryData, String)],
) -> Result<(), DBError> {
    if notes.is_empty() {
        return Ok(());
    }
    let mut batch = NoteBatch::with_capacity(notes.len(), 0, 0, notes.len() * 4);
    for (entry_data, text) in notes {
        // Avoid `NoteDetails::new` — it would clone the raw text purely to be
        // re-borrowed for each parse pass below. The borrowed-text associated
        // functions take the text by `AsRef<str>` and keep it borrowed.
        let data = NoteDetails::content_data_of(text);
        let (chunks, links) = NoteDetails::chunks_and_links_of(&entry_data.path, text);
        batch.push(entry_data, data, chunks, links);
    }
    batch.flush(tx).await
}

/// Accumulates the per-note row sets for a multi-note write. `paths` holds
/// each note's path once; chunk and link rows reference paths by index, so
/// no path string is cloned per row.
struct NoteBatch {
    paths: Vec<String>,
    notes: Vec<NoteRow>,
    chunks: Vec<ChunkRow>,
    links: Vec<LinkRow>,
    labels: Vec<LabelRow>,
}

impl NoteBatch {
    fn with_capacity(notes: usize, chunks: usize, links: usize, labels: usize) -> Self {
        Self {
            paths: Vec::with_capacity(notes),
            notes: Vec::with_capacity(notes),
            chunks: Vec::with_capacity(chunks),
            links: Vec::with_capacity(links),
            labels: Vec::with_capacity(labels),
        }
    }

    fn push(
        &mut self,
        entry_data: &NoteEntryData,
        data: NoteContentData,
        chunks: Vec<ContentChunk>,
        links: Vec<crate::note::NoteLink>,
    ) {
        let idx = self.paths.len();
        // Store every note under its canonical vault-relative key so the index
        // never holds mixed relative/absolute forms of the same note,
        // regardless of the path form the caller (or the walker) supplied.
        let canonical_path = entry_data.path.canonical();
        let (parent_path, name) = canonical_path.get_parent_path();
        self.paths.push(canonical_path.to_string());
        self.notes.push(NoteRow {
            path_idx: idx,
            title: data.title,
            size: entry_data.size as i64,
            modified: entry_data.modified_secs as i64,
            hash: data.hash.to_string(),
            base_path: parent_path.to_string(),
            name,
        });
        for c in chunks {
            self.chunks.push(ChunkRow {
                path_idx: idx,
                breadcrumb: c.breadcrumb,
                text: c.text,
            });
        }
        for l in &links {
            match &l.ltype {
                LinkType::Note(p) => {
                    self.links.push(LinkRow {
                        path_idx: idx,
                        destination: p.to_string(),
                        // Already lowercased by VaultPathSlice; folder-independent.
                        dest_name: p.get_name(),
                    });
                }
                LinkType::Hashtag => {
                    let normalized = l.text.to_lowercase();
                    if !normalized.is_empty() {
                        self.labels.push(LabelRow {
                            path_idx: idx,
                            name: normalized,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    async fn flush(self, tx: &mut Transaction<'_, Sqlite>) -> Result<(), DBError> {
        bulk_upsert_note_rows(tx, &self.notes, &self.paths).await?;
        bulk_delete_in(tx, "notesContent", &["path"], &self.paths).await?;
        bulk_delete_in(tx, "links", &["source"], &self.paths).await?;
        bulk_delete_in(tx, "labels", &["path"], &self.paths).await?;
        bulk_insert(tx, &self.chunks, &self.paths).await?;
        bulk_insert(tx, &self.links, &self.paths).await?;
        bulk_insert(tx, &self.labels, &self.paths).await?;
        Ok(())
    }
}

async fn bulk_upsert_note_rows(
    tx: &mut Transaction<'_, Sqlite>,
    rows: &[NoteRow],
    paths: &[String],
) -> Result<(), DBError> {
    bulk_insert(tx, rows, paths).await.map_err(|e| match e {
        DBError::DBError(inner) => {
            error!("Error upserting {} notes: {}", rows.len(), inner);
            DBError::DBError(inner)
        }
        other => other,
    })
}

fn placeholders(rows: usize, cols: usize) -> String {
    let one = format!("({})", vec!["?"; cols].join(", "));
    std::iter::repeat_n(one.as_str(), rows)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `DELETE FROM <table> WHERE <col1> IN (?, ?, …) [OR <col2> IN (...) …]`,
/// chunked by parameter budget. With multiple columns each value is bound
/// once per column; budget halves accordingly.
///
/// `table` and `columns` are interpolated into the SQL — never accept
/// untrusted input here. The `&'static str` bound prevents passing
/// caller-derived strings.
async fn bulk_delete_in(
    tx: &mut Transaction<'_, Sqlite>,
    table: &'static str,
    columns: &[&'static str],
    values: &[String],
) -> Result<(), DBError> {
    if values.is_empty() || columns.is_empty() {
        return Ok(());
    }
    let max_per_chunk = SQLITE_PARAM_BUDGET / columns.len();
    for chunk in values.chunks(max_per_chunk) {
        let ph = vec!["?"; chunk.len()].join(", ");
        let where_clause = columns
            .iter()
            .map(|c| format!("{} IN ({})", c, ph))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!("DELETE FROM {} WHERE {}", table, where_clause);
        let mut q = sqlx::query(&sql);
        for _ in columns {
            for v in chunk {
                q = q.bind(v);
            }
        }
        q.execute(&mut **tx).await?;
    }
    Ok(())
}

/// Trait for rows that can be batch-inserted via `bulk_insert`. Each impl
/// provides the SQL framing constants and a per-row `bind_to` method.
trait BulkInsertRow {
    /// Statement prefix ending in `VALUES `.
    const HEADER: &'static str;
    /// Optional clause appended after the placeholders (e.g. `ON CONFLICT …`).
    const FOOTER: &'static str;
    /// Number of `?` placeholders per row.
    const COLS: usize;

    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        paths: &'q [String],
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>;
}

impl BulkInsertRow for NoteRow {
    const HEADER: &'static str =
        "INSERT INTO notes (path, title, size, modified, hash, basePath, noteName) VALUES ";
    const FOOTER: &'static str = " ON CONFLICT(path) DO UPDATE SET \
                                   title = excluded.title, \
                                   size = excluded.size, \
                                   modified = excluded.modified, \
                                   hash = excluded.hash";
    const COLS: usize = 7;

    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        paths: &'q [String],
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        q.bind(&paths[self.path_idx])
            .bind(&self.title)
            .bind(self.size)
            .bind(self.modified)
            .bind(&self.hash)
            .bind(&self.base_path)
            .bind(&self.name)
    }
}

impl BulkInsertRow for ChunkRow {
    const HEADER: &'static str = "INSERT INTO notesContent (path, breadcrumb, text) VALUES ";
    const FOOTER: &'static str = "";
    const COLS: usize = 3;

    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        paths: &'q [String],
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        q.bind(&paths[self.path_idx])
            .bind(&self.breadcrumb)
            .bind(&self.text)
    }
}

impl BulkInsertRow for LinkRow {
    const HEADER: &'static str = "INSERT INTO links (source, destination, dest_name) VALUES ";
    const FOOTER: &'static str = "";
    const COLS: usize = 3;

    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        paths: &'q [String],
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        q.bind(&paths[self.path_idx])
            .bind(&self.destination)
            .bind(&self.dest_name)
    }
}

impl BulkInsertRow for LabelRow {
    const HEADER: &'static str = "INSERT INTO labels (name, path) VALUES ";
    const FOOTER: &'static str = " ON CONFLICT(name, path) DO NOTHING";
    const COLS: usize = 2;

    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        paths: &'q [String],
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        q.bind(&self.name).bind(&paths[self.path_idx])
    }
}

/// Generic chunked multi-row INSERT. Builds `<HEADER>(?, …), (?, …)<FOOTER>`,
/// chunking so binds-per-statement stays under `SQLITE_PARAM_BUDGET`.
async fn bulk_insert<R: BulkInsertRow>(
    tx: &mut Transaction<'_, Sqlite>,
    rows: &[R],
    paths: &[String],
) -> Result<(), DBError> {
    if rows.is_empty() {
        return Ok(());
    }
    let max_rows = SQLITE_PARAM_BUDGET / R::COLS;
    for chunk in rows.chunks(max_rows) {
        let sql = format!(
            "{}{}{}",
            R::HEADER,
            placeholders(chunk.len(), R::COLS),
            R::FOOTER
        );
        let mut q = sqlx::query(&sql);
        for r in chunk {
            q = r.bind_to(q, paths);
        }
        q.execute(&mut **tx).await?;
    }
    Ok(())
}

/// Wraps a user-supplied FTS4 term in double quotes so SQLite treats it
/// as a literal phrase, neutralising any FTS4 metacharacters the user
/// may have typed (`(`, `)`, `*`, `"`, `:`, etc.) that would otherwise
/// cause SQLite to reject the query at runtime.
fn fts4_quote(term: &str) -> String {
    let escaped = term.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

/// A directory path in the form the prefix `LIKE` predicates use: its
/// canonical string with a trailing separator, so `<prefix> || '%'` matches
/// exactly the rows under (not merely named like) the directory.
fn dir_prefix(path: &VaultPath) -> String {
    let s = path.to_string();
    if s.ends_with(PATH_SEPARATOR) {
        s
    } else {
        format!("{s}{PATH_SEPARATOR}")
    }
}

/// The stored content hash of one indexed note, or `None` when the path has
/// no row. Read inside the caller's transaction so it reflects pre-mutation
/// state.
async fn note_hash(
    tx: &mut Transaction<'_, Sqlite>,
    path: &VaultPath,
) -> Result<Option<u64>, DBError> {
    let hash: Option<String> = sqlx::query_scalar("SELECT hash FROM notes WHERE path = ?")
        .bind(path.to_string())
        .fetch_optional(&mut **tx)
        .await?;
    // A non-numeric hash is a corrupt row; 0 keeps the observer informed and
    // merely forces the consumer to treat the note as changed.
    Ok(hash.map(|h| h.parse().unwrap_or(0)))
}

/// `(path, hash)` of every indexed note under `directory` (recursively),
/// captured inside the caller's transaction so the rename/delete wrappers can
/// emit observer events for exactly the rows their SQL is about to touch.
async fn notes_under(
    tx: &mut Transaction<'_, Sqlite>,
    directory: &VaultPath,
) -> Result<Vec<(VaultPath, u64)>, DBError> {
    let pattern = escape_like_pattern(&dir_prefix(directory));
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT path, hash FROM notes WHERE path LIKE (? || '%') ESCAPE '\\'")
            .bind(&pattern)
            .fetch_all(&mut **tx)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(path, hash)| (VaultPath::new(&path), hash.parse().unwrap_or(0)))
        .collect())
}

/// Escapes SQLite LIKE pattern metacharacters (`\`, `%`, `_`) in `s` so the
/// result can be used as a safe literal prefix before appending `%`.
/// Must be paired with `ESCAPE '\\'` in the SQL clause.
fn escape_like_pattern(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            other => out.push(other),
        }
    }
    out
}

async fn rename_note(
    tx: &mut Transaction<'_, Sqlite>,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), DBError> {
    let old_note_name = from.get_name();
    let (new_base_path, new_note_name) = to.get_parent_path();

    sqlx::query("UPDATE notes SET path = ?, basePath = ?, noteName = ? WHERE path = ?")
        .bind(to.to_string())
        .bind(new_base_path.to_string())
        .bind(&new_note_name)
        .bind(from.to_string())
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE notesContent SET path = ? WHERE path = ?")
        .bind(to.to_string())
        .bind(from.to_string())
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE links SET source = ? WHERE source = ?")
        .bind(to.to_string())
        .bind(from.to_string())
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE links SET destination = ?, dest_name = ? WHERE destination = ?")
        .bind(to.to_string())
        .bind(&new_note_name)
        .bind(from.to_string())
        .execute(&mut **tx)
        .await?;

    // Update links that reference the note by filename only (wikilinks without path)
    sqlx::query("UPDATE links SET destination = ?, dest_name = ? WHERE destination = ?")
        .bind(&new_note_name)
        .bind(&new_note_name)
        .bind(&old_note_name)
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE labels SET path = ? WHERE path = ?")
        .bind(to.to_string())
        .bind(from.to_string())
        .execute(&mut **tx)
        .await?;

    Ok(())
}

async fn rename_directory(
    tx: &mut Transaction<'_, Sqlite>,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), DBError> {
    // Stored (no trailing separator) form for exact basePath matches, and the
    // prefix form for everything nested deeper.
    let from_base = from.to_string();
    let to_base = to.to_string();
    let from = dir_prefix(from);
    let to = dir_prefix(to);

    let from_escaped = escape_like_pattern(&from);

    // Direct children: their basePath equals `from` exactly (basePath is
    // stored without a trailing separator), so the prefix LIKE below cannot
    // match them.
    sqlx::query(
        "UPDATE notes SET path = ? || SUBSTR(path, LENGTH(?) + 1), basePath = ? WHERE basePath = ?",
    )
    .bind(&to)
    .bind(&from)
    .bind(&to_base)
    .bind(&from_base)
    .execute(&mut **tx)
    .await?;

    let notes_sql = "UPDATE notes SET path = ? || SUBSTR(path, LENGTH(?) + 1), basePath = ? || SUBSTR(basePath, LENGTH(?) + 1) WHERE basePath LIKE (? || '%') ESCAPE '\\'";
    sqlx::query(notes_sql)
        .bind(&to)
        .bind(&from)
        .bind(&to)
        .bind(&from)
        .bind(&from_escaped)
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE notesContent SET path = ? || SUBSTR(path, LENGTH(?) + 1) WHERE path LIKE (? || '%') ESCAPE '\\'")
        .bind(&to)
        .bind(&from)
        .bind(&from_escaped)
        .execute(&mut **tx)
        .await?;

    sqlx::query(
        "UPDATE links SET source = ? || SUBSTR(source, LENGTH(?) + 1) WHERE source LIKE (? || '%') ESCAPE '\\'",
    )
    .bind(&to)
    .bind(&from)
    .bind(&from_escaped)
    .execute(&mut **tx)
    .await?;

    sqlx::query("UPDATE links SET destination = ? || SUBSTR(destination, LENGTH(?) + 1) WHERE destination LIKE (? || '%') ESCAPE '\\'")
        .bind(&to)
        .bind(&from)
        .bind(&from_escaped)
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE labels SET path = ? || SUBSTR(path, LENGTH(?) + 1) WHERE path LIKE (? || '%') ESCAPE '\\'")
        .bind(&to)
        .bind(&from)
        .bind(&from_escaped)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

async fn delete_directories(
    tx: &mut Transaction<'_, Sqlite>,
    directories: &[VaultPath],
) -> Result<(), DBError> {
    if !directories.is_empty() {
        for directory in directories {
            delete_directory(tx, directory).await?;
        }
    }
    Ok(())
}

async fn delete_directory(
    tx: &mut Transaction<'_, Sqlite>,
    directory_path: &VaultPath,
) -> Result<(), DBError> {
    let pattern = escape_like_pattern(&dir_prefix(directory_path));

    sqlx::query("DELETE FROM notes WHERE path LIKE (? || '%') ESCAPE '\\'")
        .bind(&pattern)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM notesContent WHERE path LIKE (? || '%') ESCAPE '\\'")
        .bind(&pattern)
        .execute(&mut **tx)
        .await?;

    // Clear both sides of the links table — outbound (source) and inbound
    // (destination) — so backlinks pointing to deleted notes don't linger.
    sqlx::query("DELETE FROM links WHERE source LIKE (? || '%') ESCAPE '\\' OR destination LIKE (? || '%') ESCAPE '\\'")
        .bind(&pattern)
        .bind(&pattern)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM labels WHERE path LIKE (? || '%') ESCAPE '\\'")
        .bind(&pattern)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests;
