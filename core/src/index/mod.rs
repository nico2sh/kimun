pub(crate) mod search_terms;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use log::{debug, error};
use search_terms::{OrderBy, SearchTerms};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::{Row, Sqlite, Transaction};

use crate::note::{ContentChunk, LinkType, NoteContentData, NoteDetails};

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
const VERSION: &str = "0.10";
pub(crate) const DB_FILE: &str = "kimun.sqlite";

/// The diff a vault sync walk produces and [`NoteIndex::apply`] consumes in
/// one atomic operation — the currency crossing the index's interface.
/// The order of `to_add` and `to_modify` is non-deterministic: they are
/// populated by parallel walker threads and entries land in the order each
/// thread completes its file read.
pub struct IndexDiff {
    pub to_add: Vec<(NoteEntryData, String)>,
    pub to_modify: Vec<(NoteEntryData, String)>,
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
    /// `true` when [`open`](Self::open) found a missing/outdated/invalid
    /// schema and recreated it (self-heal), leaving a valid but *empty*
    /// index. See ADR-0007.
    healed: bool,
}

impl NoteIndex {
    /// Opens the index at `db_path`, self-healing the schema: when the stored
    /// index is missing, outdated, or invalid, the tables are silently
    /// recreated, leaving a valid but empty index that the next sync pass
    /// fills (ADR-0007). [`ready`](Self::ready) reports whether a heal
    /// happened.
    pub(crate) async fn open<P: AsRef<Path>>(db_path: P) -> Result<Self, DBError> {
        let db_path = db_path.as_ref().to_owned();
        if let Some(parent) = db_path.parent() {
            crate::nfs::ensure_dir(parent).map_err(|e| DBError::Other(e.to_string()))?;
        }
        let connection_string = format!("sqlite:{}?mode=rwc", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect(&connection_string)
            .await?;

        // A schema-version check that *errors* (rather than reporting stale)
        // is treated as invalid too — e.g. a corrupted-but-present file —
        // matching the pre-ADR-0007 behaviour where a failing validation fell
        // through to a rebuild. Only a failing rebuild propagates.
        let current = Self::schema_is_current(&pool).await.unwrap_or_else(|e| {
            debug!("Index schema check failed ({e}) — treating as invalid");
            false
        });
        let healed = if current {
            false
        } else {
            debug!("Index schema missing/outdated/invalid — recreating");
            init_db(&pool).await?;
            true
        };

        Ok(Self { pool, healed })
    }

    /// `false` when [`open`](Self::open) just healed the schema (the index is
    /// valid but empty until a sync pass fills it). Fast paths use this to
    /// refuse to operate against an empty index without paying for a sync.
    pub(crate) fn ready(&self) -> bool {
        !self.healed
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
    /// but empty. Callers are expected to run a full sync pass afterwards.
    pub(crate) async fn recreate(&self) -> Result<(), DBError> {
        init_db(&self.pool).await
    }

    /// Applies a sync diff — adds, modifications, deletions — in one atomic
    /// operation.
    pub(crate) async fn apply(&self, diff: IndexDiff) -> Result<(), DBError> {
        let mut tx = self.pool.begin().await?;
        delete_notes(&mut tx, &diff.to_delete).await?;
        insert_notes(&mut tx, &diff.to_add).await?;
        update_notes(&mut tx, &diff.to_modify).await?;
        tx.commit().await?;
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
        let mut tx = self.pool.begin().await?;
        rename_note(&mut tx, from, to).await?;
        update_notes(&mut tx, rewritten).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn rename_directory(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<(), DBError> {
        let mut tx = self.pool.begin().await?;
        rename_directory(&mut tx, from, to).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn delete_notes(&self, paths: &[VaultPath]) -> Result<(), DBError> {
        let mut tx = self.pool.begin().await?;
        delete_notes(&mut tx, paths).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn delete_directories(
        &self,
        directories: &[VaultPath],
    ) -> Result<(), DBError> {
        let mut tx = self.pool.begin().await?;
        delete_directories(&mut tx, directories).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn save_note(
        &self,
        entry_data: &NoteEntryData,
        note_details: &NoteDetails,
    ) -> Result<(), DBError> {
        save_note(&self.pool, entry_data, note_details).await
    }

    pub(crate) async fn search<S: AsRef<str>>(
        &self,
        search_query: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        search_terms(&self.pool, search_query).await
    }

    pub(crate) async fn search_note_by_name<S: AsRef<str>>(
        &self,
        name: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        search_note_by_name(&self.pool, name).await
    }

    pub(crate) async fn search_note_by_path(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        search_note_by_path(&self.pool, path).await
    }

    pub(crate) async fn get_notes(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        get_notes(&self.pool, path, recursive).await
    }

    pub(crate) async fn get_all_notes(
        &self,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        get_all_notes(&self.pool).await
    }

    pub(crate) async fn get_backlinks(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        get_backlinks(&self.pool, path).await
    }

    pub(crate) async fn get_notes_sections(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<HashMap<VaultPath, Vec<ContentChunk>>, DBError> {
        get_notes_sections(&self.pool, path, recursive).await
    }

    pub(crate) async fn list_labels(&self) -> Result<Vec<String>, DBError> {
        list_labels(&self.pool).await
    }

    pub(crate) async fn label_counts(&self) -> Result<Vec<(String, i64)>, DBError> {
        label_counts(&self.pool).await
    }

    pub(crate) async fn notes_with_label(&self, name: &str) -> Result<Vec<VaultPath>, DBError> {
        notes_with_label(&self.pool, name).await
    }

    pub(crate) async fn suggest_notes_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<NoteSuggestion>, DBError> {
        suggest_notes_by_prefix(&self.pool, prefix, limit).await
    }

    pub(crate) async fn suggest_tags_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<TagSuggestion>, DBError> {
        suggest_tags_by_prefix(&self.pool, prefix, limit).await
    }
}

#[cfg(test)]
impl NoteIndex {
    /// Test-only pool accessor — index-internal tests exercise SQL and the
    /// query builders directly through this internal seam.
    fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Test-only: release the pool's file handles promptly.
    async fn close(&self) {
        self.pool.close().await;
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

async fn get_all_notes(
    pool: &SqlitePool,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let query = format!("SELECT DISTINCT {} FROM notes", NOTE_COLUMNS);
    let rows = sqlx::query(&query).fetch_all(pool).await?;
    rows.iter().map(row_to_note_entry).collect()
}

async fn list_labels(pool: &SqlitePool) -> Result<Vec<String>, DBError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT name FROM labels")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
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
    pub name: String,
    pub path: VaultPath,
}

/// A tag suggestion for the autocomplete popup. `usage_count` is computed
/// per-query via `COUNT(*) GROUP BY name` over the `labels` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSuggestion {
    pub label: String,
    pub usage_count: u32,
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
async fn suggest_notes_by_prefix(
    pool: &SqlitePool,
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
        .fetch_all(pool)
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
async fn suggest_tags_by_prefix(
    pool: &SqlitePool,
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
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(label, cnt)| TagSuggestion {
            label,
            usage_count: cnt.max(0) as u32,
        })
        .collect())
}

async fn label_counts(pool: &SqlitePool) -> Result<Vec<(String, i64)>, DBError> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT name, COUNT(*) as cnt FROM labels GROUP BY name ORDER BY name")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

async fn notes_with_label(pool: &SqlitePool, name: &str) -> Result<Vec<VaultPath>, DBError> {
    let normalized = name.to_lowercase();
    let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM labels WHERE name = ?")
        .bind(&normalized)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(p,)| VaultPath::new(p)).collect())
}

async fn search_terms<S: AsRef<str>>(
    pool: &SqlitePool,
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

    let rows = sql_query.fetch_all(pool).await?;

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

async fn search_note_by_name<S: AsRef<str>>(
    pool: &SqlitePool,
    name: S,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let name = name.as_ref().to_lowercase();
    let sql = format!("SELECT {} FROM notes where noteName = ?", NOTE_COLUMNS);
    let rows = sqlx::query(&sql).bind(&name).fetch_all(pool).await?;

    rows.iter().map(row_to_note_entry).collect()
}

async fn search_note_by_path(
    pool: &SqlitePool,
    path: &VaultPath,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let sql = format!("SELECT {} FROM notes where path = ?", NOTE_COLUMNS);
    let path_string = path.to_string();
    let rows = sqlx::query(&sql).bind(&path_string).fetch_all(pool).await?;

    // Should always return one or zero
    rows.iter().map(row_to_note_entry).collect()
}

async fn get_notes(
    pool: &SqlitePool,
    path: &VaultPath,
    recursive: bool,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let (where_clause, bind_value) = if recursive {
        (
            "basePath LIKE (? || '%') ESCAPE '\\'".to_string(),
            escape_like_pattern(&path.to_string()),
        )
    } else {
        ("basePath = ?".to_string(), path.to_string())
    };
    let sql = format!("SELECT {} FROM notes where {}", NOTE_COLUMNS, where_clause);
    let rows = sqlx::query(&sql).bind(bind_value).fetch_all(pool).await?;

    rows.iter().map(row_to_note_entry).collect()
}

/// Backlinks of a *specific* note: notes whose body links to exactly this note,
/// matched by its full path OR its bare filename (wikilinks stored without a
/// path). This is intentionally narrower than the `>`/`lk:` search filter
/// (see [`link_subquery`]), which matches a name in *any* folder; keep the two
/// in step on the stored-form invariant they share (lowercased, `.md`-suffixed
/// destinations, bare-relative or relative/absolute path).
async fn get_backlinks(
    pool: &SqlitePool,
    path: &VaultPath,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
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
        .fetch_all(pool)
        .await?;

    rows.iter().map(row_to_note_entry).collect()
}

async fn get_notes_sections(
    pool: &SqlitePool,
    path: &VaultPath,
    recursive: bool,
) -> Result<HashMap<VaultPath, Vec<ContentChunk>>, DBError> {
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
            escape_like_pattern(&path.to_string()),
        )
    } else {
        // Only notes directly in this directory (basePath join)
        ("SELECT nc.path, nc.breadcrumb, nc.text FROM notesContent nc JOIN notes n ON nc.path = n.path WHERE n.basePath = ?".to_string(), path.to_string())
    };

    let rows = sqlx::query(&sql).bind(bind_value).fetch_all(pool).await?;

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
) -> Result<(), DBError> {
    // The caller already parsed `note_details`, so reuse it instead of paying
    // for another `NoteDetails::new` clone of the raw text.
    let data = note_details.get_content_data();
    let (chunks, links) = note_details.get_chunks_and_links();
    let label_count = links
        .iter()
        .filter(|l| matches!(l.ltype, LinkType::Hashtag))
        .count();
    let mut batch = NoteBatch::with_capacity(1, chunks.len(), links.len(), label_count);
    batch.push(entry_data, data, chunks, links);

    let mut tx = pool.begin().await?;
    batch.flush(&mut tx).await?;
    tx.commit().await?;
    Ok(())
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
        // re-borrowed for each parse pass below. The free functions take the
        // text by `AsRef<str>` and keep it borrowed.
        let data = crate::note::content_extractor::get_content_data(text);
        let (chunks, links) =
            crate::note::content_extractor::get_chunks_and_links(&entry_data.path, text);
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
        let (parent_path, name) = entry_data.path.get_parent_path();
        self.paths.push(entry_data.path.to_string());
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
    let from = {
        let s = from.to_string();
        if s.ends_with(PATH_SEPARATOR) {
            s
        } else {
            s + &PATH_SEPARATOR.to_string()
        }
    };
    let to = {
        let s = to.to_string();
        if s.ends_with(PATH_SEPARATOR) {
            s
        } else {
            s + &PATH_SEPARATOR.to_string()
        }
    };

    let from_escaped = escape_like_pattern(&from);

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
    let path_str = directory_path.to_string();
    let normalized = if path_str.ends_with(PATH_SEPARATOR) {
        path_str
    } else {
        format!("{path_str}{PATH_SEPARATOR}")
    };
    let pattern = escape_like_pattern(&normalized);

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
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_parent_dir_for_db_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("nested/dir/cache.kimuncache");
        // Parent dir does not exist yet.
        assert!(!nested.parent().unwrap().exists());

        let db = super::NoteIndex::open(&nested).await.unwrap();
        assert!(nested.parent().unwrap().exists());
        assert!(nested.exists());
        // A fresh file has no schema — open must have healed it.
        assert!(!db.ready());
        db.close().await;
    }

    #[test]
    fn test_search_terms_query_empty() {
        let (sql, params) = build_search_sql_query("");
        assert_eq!(sql, "");
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_search_terms_query_simple_terms() {
        let (sql, params) = build_search_sql_query("foo bar");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"foo\" \"bar\"");
    }

    #[test]
    fn test_search_terms_query_single_term() {
        let (sql, params) = build_search_sql_query("keyword");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_breadcrumb_only() {
        let (sql, params) = build_search_sql_query("@heading");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"heading\"");
    }

    #[test]
    fn test_search_terms_query_breadcrumb_with_in() {
        let (sql, params) = build_search_sql_query("in:section");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"section\"");
    }

    #[test]
    fn test_search_terms_query_multiple_breadcrumbs() {
        let (sql, params) = build_search_sql_query("@heading1 in:heading2");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"heading1\" \"heading2\"");
    }

    #[test]
    fn test_search_terms_query_path_only() {
        let (sql, params) = build_search_sql_query("=filename");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?1 ESCAPE '\\'"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "%filename%");
    }

    #[test]
    fn test_search_terms_query_path_with_at() {
        let (sql, params) = build_search_sql_query("name:directory");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?1 ESCAPE '\\'"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "%directory%");
    }

    #[test]
    fn test_search_terms_query_multiple_paths() {
        let (sql, params) = build_search_sql_query("=file1 name:file2");
        // Same-type operators AND together (consistent with #, <, >, and the
        // documented "all terms are ANDed" precedence).
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?1 ESCAPE '\\' AND notes.noteName LIKE ?2 ESCAPE '\\'"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "%file1%");
        assert_eq!(params[1], "%file2%");
    }

    #[test]
    fn test_search_terms_query_terms_and_breadcrumb() {
        let (sql, params) = build_search_sql_query("keyword @section");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "\"keyword\"");
        assert_eq!(params[1], "\"section\"");
    }

    #[test]
    fn test_search_terms_query_terms_and_path() {
        let (sql, params) = build_search_sql_query("keyword =file");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?2 ESCAPE '\\'"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "\"keyword\"");
        assert_eq!(params[1], "%file%");
    }

    #[test]
    fn test_search_terms_query_breadcrumb_and_path() {
        let (sql, params) = build_search_sql_query("@heading =file");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?2 ESCAPE '\\'"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "\"heading\"");
        assert_eq!(params[1], "%file%");
    }

    #[test]
    fn test_search_terms_query_all_combined() {
        let (sql, params) = build_search_sql_query("keyword @heading =file");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?3 ESCAPE '\\'"
        );
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], "\"keyword\"");
        assert_eq!(params[1], "\"heading\"");
        assert_eq!(params[2], "%file%");
    }

    #[test]
    fn test_search_terms_query_quoted_terms() {
        let (sql, params) = build_search_sql_query("\"exact phrase\" keyword");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"exact phrase\" \"keyword\"");
    }

    #[test]
    fn test_search_terms_query_order_by_title_asc() {
        let (sql, params) = build_search_sql_query("keyword or:title");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_order_by_title_desc() {
        let (sql, params) = build_search_sql_query("keyword -or:title");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_order_by_filename_asc() {
        let (sql, params) = build_search_sql_query("keyword or:filename");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_order_by_file_shorthand() {
        let (sql, params) = build_search_sql_query("keyword or:f");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_order_by_title_shorthand() {
        let (sql, params) = build_search_sql_query("keyword or:t");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_multiple_order_by() {
        let (sql, params) = build_search_sql_query("keyword ^title -^filename");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_complex_with_order() {
        let (sql, params) = build_search_sql_query("keyword @section =file ^title");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?3 ESCAPE '\\'"
        );
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], "\"keyword\"");
        assert_eq!(params[1], "\"section\"");
        assert_eq!(params[2], "%file%");
    }

    #[test]
    fn test_search_terms_query_only_order_by() {
        let (sql, params) = build_search_sql_query("^title");
        assert_eq!(sql, "");
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_search_terms_query_invalid_order_by_field() {
        let (sql, params) = build_search_sql_query("keyword ^invalid");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"keyword\"");
    }

    #[test]
    fn test_search_terms_query_whitespace_handling() {
        let (sql, params) = build_search_sql_query("  keyword   @section  ");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "\"keyword\"");
        assert_eq!(params[1], "\"section\"");
    }

    #[test]
    fn test_fts4_mixed_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query("meeting -cancelled");

        // Should use NOT IN subquery approach instead of FTS4 native exclusion
        assert!(sql.contains("notesContent MATCH"));
        assert!(sql.contains("NOT IN"));
        assert!(sql.contains(
            "SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH"
        ));
        // params: first is the excluded term (NOT IN subquery), second is the positive term
        assert_eq!(params.len(), 2);
        assert!(params.contains(&"\"cancelled\"".to_string()));
        assert!(params.contains(&"\"meeting\"".to_string()));

        assert!(sql.contains("SELECT DISTINCT"));
    }

    #[test]
    fn test_exclusion_only_sql_generation() {
        // Critical test: exclusion-only queries MUST use NOT IN, not pure FTS4 MATCH
        let (sql, params) = build_search_sql_query("-cancelled");

        // Should NOT contain pure FTS4 exclusion (which is invalid)
        assert!(!sql.contains("MATCH \"-cancelled\""));
        // Should use NOT IN subquery approach
        assert!(sql.contains("NOT IN"));
        assert!(sql.contains(
            "SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH"
        ));
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "\"cancelled\"");
    }

    #[test]
    fn test_breadcrumb_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query("@project -@draft");

        // Positive breadcrumb is a column-scoped MATCH; the exclusion is a
        // robust NOT IN subquery (not the old, broken inline `breadcrumb: -term`).
        assert!(sql.contains("notesContent.breadcrumb MATCH ?1"));
        assert!(sql.contains(
            "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent.breadcrumb MATCH ?2)"
        ));
        assert_eq!(
            params,
            vec!["\"project\"".to_string(), "\"draft\"".to_string()]
        );
    }

    #[test]
    fn test_like_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query("=2024 -=draft");

        // Should generate filename query with positive and negative conditions
        assert!(sql.contains("notes.noteName LIKE"));
        assert!(sql.contains("notes.noteName NOT LIKE"));
        assert!(params.contains(&"%2024%".to_string()));
        assert!(params.contains(&"%draft%".to_string()));
    }

    #[test]
    fn test_exclusion_only_like_query() {
        let (sql, params) = build_search_sql_query("-=draft -=temp");

        // Exclusion-only should still generate valid WHERE clause
        assert!(sql.contains("notes.noteName NOT LIKE"));
        // The new format embeds % in the param, not in the SQL template
        assert!(!sql.contains("NOT LIKE ('%'"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_path_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query("/projects -/archive");

        assert!(sql.contains("notes.basePath LIKE"));
        assert!(sql.contains("notes.basePath NOT LIKE"));
        assert!(params.contains(&"projects".to_string()));
        assert!(params.contains(&"archive".to_string()));
    }

    #[test]
    fn test_exclusion_only_path_query() {
        let (sql, params) = build_search_sql_query("-/draft -/temp");

        assert!(sql.contains("notes.basePath NOT LIKE"));
        assert!(!sql.contains("notes.basePath LIKE ('/'"));
        assert_eq!(params.len(), 2);
    }

    #[tokio::test]
    async fn labels_table_exists_after_create_tables() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&path).await.unwrap();

        let row: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='table' AND name='labels'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(row.0, 1, "labels table should exist");

        // labels_by_name was removed in 0.7; the PK autoindex covers it.
        let idx_name: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='index' AND name='labels_by_name'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(
            idx_name.0, 0,
            "labels_by_name index must not exist (dropped in 0.7)"
        );

        let idx_path: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='index' AND name='labels_by_path'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(idx_path.0, 1, "labels_by_path index should exist");

        db.close().await;
    }

    #[tokio::test]
    async fn labels_are_persisted_on_note_insert() {
        use crate::nfs::{NoteEntryData, VaultPath};

        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let path = VaultPath::note_path_from("/n.md");
        let body = "Title\n\nbody with #foo and #Foo and #bar".to_string();
        let entry = NoteEntryData {
            path: path.clone(),
            size: body.len() as u64,
            modified_secs: 0,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, body)])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT name, path FROM labels ORDER BY name")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("bar".to_string(), path.to_string()),
                ("foo".to_string(), path.to_string()),
            ],
            "labels stored deduped + lowercased"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn reindexing_a_note_drops_removed_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};

        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let path = VaultPath::note_path_from("/n.md");
        let body_v1 = "before #draft #keep".to_string();
        let entry_v1 = NoteEntryData {
            path: path.clone(),
            size: body_v1.len() as u64,
            modified_secs: 0,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry_v1, body_v1)])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let body_v2 = "after #keep only".to_string();
        let entry_v2 = NoteEntryData {
            path: path.clone(),
            size: body_v2.len() as u64,
            modified_secs: 1,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::update_notes(&mut tx, &[(entry_v2, body_v2)])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM labels WHERE path = ? ORDER BY name")
                .bind(path.to_string())
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            rows.into_iter().map(|(n,)| n).collect::<Vec<_>>(),
            vec!["keep".to_string()],
            "reindex must drop labels no longer present"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn labels_are_removed_on_note_delete() {
        use crate::nfs::{NoteEntryData, VaultPath};

        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let path = VaultPath::note_path_from("/n.md");
        let body = "x #drop".to_string();
        let entry = NoteEntryData {
            path: path.clone(),
            size: body.len() as u64,
            modified_secs: 0,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, body)])
            .await
            .unwrap();
        super::delete_notes(&mut tx, std::slice::from_ref(&path))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM labels WHERE path = ?")
            .bind(path.to_string())
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 0);

        db.close().await;
    }

    #[test]
    fn test_search_terms_query_label_only() {
        let (sql, params) = build_search_sql_query("#important");
        assert_eq!(params, vec!["important".to_string()]);
        assert!(
            sql.contains("FROM notes") && sql.contains("labels"),
            "query should join notes with labels: {}",
            sql
        );
    }

    #[test]
    fn test_search_terms_query_two_labels_intersect() {
        let (sql, params) = build_search_sql_query("#a #b");
        assert_eq!(params.len(), 2);
        assert!(
            sql.contains("INTERSECT"),
            "two labels should INTERSECT: {}",
            sql
        );
    }

    #[test]
    fn test_search_terms_query_links_only() {
        let (sql, params) = build_search_sql_query("<projects");
        assert_eq!(params, vec!["projects.md".to_string()]);
        assert!(
            sql.contains("FROM notes")
                && sql.contains("SELECT source FROM links")
                && sql.contains("notes.path IN"),
            "backlinks query should select sources from links: {}",
            sql
        );
        // Bare name (no wildcard) matches the indexed dest_name column with
        // plain equality — no leading-`%` scan.
        assert!(
            sql.contains("dest_name = ?1"),
            "expected indexed dest_name equality: {}",
            sql
        );
    }

    #[test]
    fn test_search_terms_query_links_long_form() {
        let (_sql, params) = build_search_sql_query("lk:projects");
        assert_eq!(params, vec!["projects.md".to_string()]);
    }

    #[test]
    fn test_search_terms_query_links_path_qualified() {
        let (sql, params) = build_search_sql_query("<work/projects");
        assert_eq!(params, vec!["work/projects.md".to_string()]);
        // Path-qualified anchors to the full path (relative or absolute) via
        // indexed equality on `destination`, not the bare-name column.
        assert!(
            sql.contains("destination = ?1 OR destination = ('/' || ?1)"),
            "expected path-anchored equality: {}",
            sql
        );
        assert!(!sql.contains("dest_name"));
    }

    #[test]
    fn test_search_terms_query_links_wildcard() {
        let (sql, params) = build_search_sql_query("<proj*");
        assert_eq!(params, vec!["proj%.md".to_string()]);
        // Wildcard bare name uses a prefix LIKE on the indexed dest_name column.
        assert!(
            sql.contains("dest_name LIKE ?1 ESCAPE '\\'"),
            "expected dest_name LIKE for wildcard: {}",
            sql
        );
    }

    #[test]
    fn test_search_terms_query_links_extension_optional() {
        let (_sql, params) = build_search_sql_query("<projects.md");
        assert_eq!(params, vec!["projects.md".to_string()]);
    }

    #[test]
    fn test_search_terms_query_excluded_links() {
        let (sql, params) = build_search_sql_query("-<draft");
        assert_eq!(params, vec!["draft.md".to_string()]);
        assert!(
            sql.contains("notes.path NOT IN (SELECT source FROM links"),
            "excluded backlinks should use NOT IN: {}",
            sql
        );
    }

    #[test]
    fn test_search_terms_query_two_links_intersect() {
        let (sql, params) = build_search_sql_query("<a <b");
        assert_eq!(params.len(), 2);
        assert!(
            sql.contains("INTERSECT"),
            "two backlinks should INTERSECT: {}",
            sql
        );
    }

    #[test]
    fn test_search_terms_query_links_combined_with_operators() {
        // Free-text term + backlink + label all compose via INTERSECT.
        let (sql, params) = build_search_sql_query("meeting <spec #urgent");
        assert_eq!(sql.matches("INTERSECT").count(), 2);
        assert!(sql.contains("notesContent MATCH"));
        assert!(sql.contains("SELECT source FROM links"));
        assert!(sql.contains("FROM labels WHERE name"));
        // Params follow the fan-out order: content term, label, then backlink.
        assert_eq!(
            params,
            vec![
                "\"meeting\"".to_string(),
                "urgent".to_string(),
                "spec.md".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn search_combining_links_with_other_operators() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entries: Vec<(NoteEntryData, String)> = vec![
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/work/a.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "# Tasks\n[[spec]] meeting #urgent".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/b.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "[[spec]] casual".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/c.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "#urgent only, no link".to_string(),
            ),
        ];

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let paths = |results: &[(NoteEntryData, NoteContentData)]| {
            let mut p: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
            p.sort();
            p
        };

        // backlink + free-text term.
        let r = super::search_terms(db.pool(), "<spec meeting")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

        // backlink + label.
        let r = super::search_terms(db.pool(), "<spec #urgent")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

        // backlink + excluded label.
        let r = super::search_terms(db.pool(), "<spec -#urgent")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/b.md".to_string()]);

        // backlink + path filter.
        let r = super::search_terms(db.pool(), "<spec /work").await.unwrap();
        assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

        // backlink + section (breadcrumb) filter.
        let r = super::search_terms(db.pool(), "<spec @tasks")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

        // backlink + filename filter.
        let r = super::search_terms(db.pool(), "<spec =b").await.unwrap();
        assert_eq!(paths(&r), vec!["/b.md".to_string()]);

        // label without link still matches the non-linking note.
        let r = super::search_terms(db.pool(), "#urgent -spec")
            .await
            .unwrap();
        assert!(paths(&r).contains(&"/c.md".to_string()));

        db.close().await;
    }

    #[tokio::test]
    async fn multiple_filename_terms_are_anded() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entries: Vec<(NoteEntryData, String)> = vec![
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/report-2024.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "x".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/report-2023.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "y".to_string(),
            ),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        // =report =2024 must match ONLY the file containing both, not either.
        let r = super::search_terms(db.pool(), "=report =2024")
            .await
            .unwrap();
        let paths: Vec<String> = r.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(paths, vec!["/report-2024.md".to_string()]);

        db.close().await;
    }

    #[tokio::test]
    async fn link_search_follows_rename() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entry = NoteEntryData {
            path: VaultPath::note_path_from("/a.md"),
            size: 10,
            modified_secs: 0,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, "see [[target]]".to_string())])
            .await
            .unwrap();
        // Rename the linked-to note; links (destination + dest_name) must follow.
        super::rename_note(
            &mut tx,
            &VaultPath::note_path_from("/target.md"),
            &VaultPath::note_path_from("/renamed.md"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let r = super::search_terms(db.pool(), "<renamed").await.unwrap();
        let paths: Vec<String> = r.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(paths, vec!["/a.md".to_string()]);

        // The old name no longer matches.
        let r = super::search_terms(db.pool(), "<target").await.unwrap();
        assert!(r.is_empty());

        db.close().await;
    }

    #[tokio::test]
    async fn search_by_link_returns_linking_notes() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entries: Vec<(NoteEntryData, String)> = vec![
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/index.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "links [[projects]] and [[work/spec]]".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/b.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "see [[projects]]".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/c.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "no links here".to_string(),
            ),
        ];

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let paths = |results: &[(NoteEntryData, NoteContentData)]| {
            let mut p: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
            p.sort();
            p
        };

        // Notes that link to "projects" (backlinks).
        let r = super::search_terms(db.pool(), "<projects").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/b.md".to_string(), "/index.md".to_string()]
        );

        // Extension optional.
        let r = super::search_terms(db.pool(), "<projects.md")
            .await
            .unwrap();
        assert_eq!(
            paths(&r),
            vec!["/b.md".to_string(), "/index.md".to_string()]
        );

        // Bare name matches a note in a subfolder (name-anywhere).
        let r = super::search_terms(db.pool(), "<spec").await.unwrap();
        assert_eq!(paths(&r), vec!["/index.md".to_string()]);

        // Path-qualified match.
        let r = super::search_terms(db.pool(), "<work/spec").await.unwrap();
        assert_eq!(paths(&r), vec!["/index.md".to_string()]);

        // Wildcard.
        let r = super::search_terms(db.pool(), "<proj*").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/b.md".to_string(), "/index.md".to_string()]
        );

        // Exclusion: all notes that do NOT link to projects (index and b both link it).
        let r = super::search_terms(db.pool(), "-<projects").await.unwrap();
        assert_eq!(paths(&r), vec!["/c.md".to_string()]);

        // Unknown target → no results.
        let r = super::search_terms(db.pool(), "<nonexistent")
            .await
            .unwrap();
        assert!(r.is_empty());

        db.close().await;
    }

    #[tokio::test]
    async fn search_by_forward_link_returns_targets() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        // A links to B and C; B and C link nowhere; D links to A.
        let entries: Vec<(NoteEntryData, String)> = vec![
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/a.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "see [[b]] and [[c]]".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/b.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "b body".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/c.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "c body".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/d.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "points to [[a]]".to_string(),
            ),
        ];

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let paths = |results: &[(NoteEntryData, NoteContentData)]| {
            let mut p: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
            p.sort();
            p
        };

        // Forward links of A: the notes A links *to* (B and C).
        let r = super::search_terms(db.pool(), ">a").await.unwrap();
        assert_eq!(paths(&r), vec!["/b.md".to_string(), "/c.md".to_string()]);

        // Long form.
        let r = super::search_terms(db.pool(), "fwd:a").await.unwrap();
        assert_eq!(paths(&r), vec!["/b.md".to_string(), "/c.md".to_string()]);

        // Backlinks of B: the notes that link *to* B (A).
        let r = super::search_terms(db.pool(), "<b").await.unwrap();
        assert_eq!(paths(&r), vec!["/a.md".to_string()]);

        // Forward links of D: A.
        let r = super::search_terms(db.pool(), ">d").await.unwrap();
        assert_eq!(paths(&r), vec!["/a.md".to_string()]);

        // Exclusion: notes that are NOT forward links of A (everything but B and C).
        let r = super::search_terms(db.pool(), "->a").await.unwrap();
        assert_eq!(paths(&r), vec!["/a.md".to_string(), "/d.md".to_string()]);

        // A note with no outgoing links has no forward links.
        let r = super::search_terms(db.pool(), ">b").await.unwrap();
        assert!(r.is_empty());

        db.close().await;
    }

    #[tokio::test]
    async fn fts_content_and_breadcrumb_combinations() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let mk = |p: &str, body: &str| {
            (
                NoteEntryData {
                    path: VaultPath::note_path_from(p),
                    size: 10,
                    modified_secs: 0,
                },
                body.to_string(),
            )
        };
        let entries = vec![
            // "meeting" under a "Work" heading, also says "done".
            mk("/a.md", "# Work\nmeeting notes, all done"),
            // "meeting" but under "Personal", not "Work".
            mk("/b.md", "# Personal\nmeeting with a friend"),
            // "Work" heading but no "meeting".
            mk("/c.md", "# Work\nbudget review"),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let paths = |r: &[(NoteEntryData, NoteContentData)]| {
            let mut p: Vec<String> = r.iter().map(|(e, _)| e.path.to_string()).collect();
            p.sort();
            p
        };

        // content AND breadcrumb (both must hold).
        let r = super::search_terms(db.pool(), "meeting @work")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/a.md".to_string()]);

        // two content terms AND (only /a.md has both "meeting" and "notes").
        let r = super::search_terms(db.pool(), "meeting notes")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/a.md".to_string()]);

        // content positive + content exclusion.
        let r = super::search_terms(db.pool(), "meeting -done")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/b.md".to_string()]);

        // breadcrumb positive + content exclusion.
        let r = super::search_terms(db.pool(), "@work -budget")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/a.md".to_string()]);

        // breadcrumb positive + breadcrumb exclusion.
        let r = super::search_terms(db.pool(), "@work -@personal")
            .await
            .unwrap();
        assert_eq!(paths(&r), vec!["/a.md".to_string(), "/c.md".to_string()]);

        // pure content exclusion (no positives anywhere).
        let r = super::search_terms(db.pool(), "-meeting").await.unwrap();
        assert_eq!(paths(&r), vec!["/c.md".to_string()]);

        // pure breadcrumb exclusion.
        let r = super::search_terms(db.pool(), "-@work").await.unwrap();
        assert_eq!(paths(&r), vec!["/b.md".to_string()]);

        db.close().await;
    }

    #[tokio::test]
    async fn search_by_label_returns_matching_notes() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entries: Vec<(NoteEntryData, String)> = vec![
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/a.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "a #important #todo".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/b.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "b #todo".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/c.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "c plain".to_string(),
            ),
        ];

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let results = super::search_terms(db.pool(), "#important").await.unwrap();
        let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(paths, vec!["/a.md".to_string()]);

        let results = super::search_terms(db.pool(), "#important #todo")
            .await
            .unwrap();
        let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(paths, vec!["/a.md".to_string()]);

        let results = super::search_terms(db.pool(), "#nope").await.unwrap();
        assert!(results.is_empty());

        db.close().await;
    }

    #[tokio::test]
    async fn label_search_uses_index() {
        // Confirms the PK autoindex (sqlite_autoindex_labels_1) is used for
        // label lookups after the explicit labels_by_name index was dropped in
        // 0.7. A hashtag filter must not degrade to a full table scan.
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entry = NoteEntryData {
            path: VaultPath::note_path_from("/a.md"),
            size: 10,
            modified_secs: 0,
        };
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, "x #important".to_string())])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let (sql, _) = super::build_search_sql_query("#important");
        let plan_sql = format!("EXPLAIN QUERY PLAN {}", sql);
        let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(&plan_sql)
            .bind("important")
            .fetch_all(db.pool())
            .await
            .unwrap();
        let plan_text = rows
            .iter()
            .map(|(_, _, _, detail)| detail.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        // The PK autoindex covers WHERE name = ? lookups on (name, path).
        // No explicit labels_by_name index any more (removed in 0.7).
        // Accept any sqlite_autoindex_labels_ suffix to tolerate DROP+CREATE migration changes.
        assert!(
            plan_text.contains("sqlite_autoindex_labels_"),
            "expected PK autoindex on labels in plan: {}",
            plan_text
        );

        db.close().await;
    }

    #[tokio::test]
    async fn rename_note_updates_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let from = VaultPath::note_path_from("/old.md");
        let to = VaultPath::note_path_from("/new.md");
        let entry = NoteEntryData {
            path: from.clone(),
            size: 10,
            modified_secs: 0,
        };
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, "x #foo".to_string())])
            .await
            .unwrap();
        super::rename_note(&mut tx, &from, &to).await.unwrap();
        tx.commit().await.unwrap();

        let old_rows: (i64,) = sqlx::query_as("SELECT count(*) FROM labels WHERE path = ?")
            .bind(from.to_string())
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(old_rows.0, 0, "no label rows should remain at old path");

        let new_rows: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM labels WHERE path = ? ORDER BY name")
                .bind(to.to_string())
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            new_rows.into_iter().map(|(n,)| n).collect::<Vec<_>>(),
            vec!["foo".to_string()],
        );

        db.close().await;
    }

    #[tokio::test]
    async fn rename_directory_updates_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let note_path = VaultPath::note_path_from("/old_dir/note.md");
        let entry = NoteEntryData {
            path: note_path.clone(),
            size: 10,
            modified_secs: 0,
        };
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, "x #moved".to_string())])
            .await
            .unwrap();
        super::rename_directory(
            &mut tx,
            &VaultPath::new("/old_dir"),
            &VaultPath::new("/new_dir"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, path FROM labels")
            .fetch_all(db.pool())
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![("moved".to_string(), "/new_dir/note.md".to_string())],
        );

        db.close().await;
    }

    #[tokio::test]
    async fn delete_directory_removes_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let note_path = VaultPath::note_path_from("/sub/note.md");
        let entry = NoteEntryData {
            path: note_path.clone(),
            size: 10,
            modified_secs: 0,
        };
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, "x #gone".to_string())])
            .await
            .unwrap();
        super::delete_directories(&mut tx, &[VaultPath::new("/sub")])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM labels")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 0);

        db.close().await;
    }

    #[tokio::test]
    async fn delete_directory_with_underscore_does_not_touch_siblings() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let target = VaultPath::note_path_from("/my_dir/a.md");
        let sibling = VaultPath::note_path_from("/myXdir/b.md");
        let entries = vec![
            (
                NoteEntryData {
                    path: target.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "x #t".to_string(),
            ),
            (
                NoteEntryData {
                    path: sibling.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "y #s".to_string(),
            ),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        super::delete_directories(&mut tx, &[VaultPath::new("/my_dir")])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let remaining: Vec<(String,)> = sqlx::query_as("SELECT path FROM notes ORDER BY path")
            .fetch_all(db.pool())
            .await
            .unwrap();
        assert_eq!(
            remaining.into_iter().map(|(p,)| p).collect::<Vec<_>>(),
            vec![sibling.to_string()],
            "sibling /myXdir/b.md must be untouched"
        );

        let sibling_label: (i64,) = sqlx::query_as("SELECT count(*) FROM labels WHERE path = ?")
            .bind(sibling.to_string())
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(sibling_label.0, 1, "sibling label preserved");

        db.close().await;
    }

    #[test]
    fn escape_like_pattern_escapes_metacharacters() {
        assert_eq!(super::escape_like_pattern("/my_dir/"), "/my\\_dir/");
        assert_eq!(super::escape_like_pattern("/a%b/"), "/a\\%b/");
        assert_eq!(super::escape_like_pattern("/a\\b/"), "/a\\\\b/");
        assert_eq!(super::escape_like_pattern("/normal/"), "/normal/");
    }

    /// Verify that `escape_like_pattern` leaves `*` and `.` untouched — a
    /// prerequisite for the escape-then-replace order in the wildcard branch.
    #[test]
    fn escape_like_pattern_leaves_star_and_dot_untouched() {
        assert_eq!(super::escape_like_pattern("task*"), "task*");
        assert_eq!(super::escape_like_pattern("task*.md"), "task*.md");
        assert_eq!(super::escape_like_pattern("*report.md"), "*report.md");
    }

    /// SQL-shape unit test: confirm the bound parameter produced for `=task*`
    /// is `task%.md` and for plain `=task` is `%task%`.
    #[test]
    fn filename_wildcard_produces_correct_pattern_param() {
        // Wildcard term: =task*  → param should be "task%.md"
        let (_, params) = build_search_sql_query("=task*");
        assert_eq!(
            params,
            vec!["task%.md".to_string()],
            "=task* must produce bound param 'task%.md'"
        );

        // Non-wildcard term: =task  → param should be "%task%"
        let (_, params) = build_search_sql_query("=task");
        assert_eq!(
            params,
            vec!["%task%".to_string()],
            "=task must produce bound param '%task%'"
        );

        // Suffix wildcard: =*report → param should be "%report.md"
        let (_, params) = build_search_sql_query("=*report");
        assert_eq!(
            params,
            vec!["%report.md".to_string()],
            "=*report must produce bound param '%report.md'"
        );

        // Mid wildcard: =ta*sk → param should be "ta%sk.md"
        let (_, params) = build_search_sql_query("=ta*sk");
        assert_eq!(
            params,
            vec!["ta%sk.md".to_string()],
            "=ta*sk must produce bound param 'ta%sk.md'"
        );
    }

    #[tokio::test]
    async fn search_by_filename_wildcard() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entries: Vec<(NoteEntryData, String)> = vec![
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/task.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "x".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/tasks.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "y".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/weekly-report.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "z".to_string(),
            ),
            (
                NoteEntryData {
                    path: VaultPath::note_path_from("/other.md"),
                    size: 10,
                    modified_secs: 0,
                },
                "w".to_string(),
            ),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let paths = |results: &[(NoteEntryData, NoteContentData)]| {
            let mut p: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
            p.sort();
            p
        };

        // Substring (non-wildcard): =task → task.md and tasks.md
        let r = super::search_terms(db.pool(), "=task").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/task.md".to_string(), "/tasks.md".to_string()],
            "=task must match task.md and tasks.md as substrings"
        );

        // Prefix wildcard: =task* → task.md and tasks.md, NOT weekly-report.md
        let r = super::search_terms(db.pool(), "=task*").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/task.md".to_string(), "/tasks.md".to_string()],
            "=task* must match task.md and tasks.md, not weekly-report.md"
        );

        // Suffix wildcard: =*report → weekly-report.md only
        let r = super::search_terms(db.pool(), "=*report").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/weekly-report.md".to_string()],
            "=*report must match only weekly-report.md"
        );

        // Exclusion with wildcard: -=task* → other.md and weekly-report.md
        let r = super::search_terms(db.pool(), "-=task*").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/other.md".to_string(), "/weekly-report.md".to_string()],
            "-=task* must exclude task.md and tasks.md"
        );

        db.close().await;
    }

    #[tokio::test]
    async fn search_by_path_wildcard() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let mk = |p: &str| NoteEntryData {
            path: VaultPath::note_path_from(p),
            size: 10,
            modified_secs: 0,
        };
        let entries: Vec<(NoteEntryData, String)> = vec![
            (mk("/work/a.md"), "a".to_string()),
            (mk("/work/sub/b.md"), "b".to_string()),
            (mk("/personal/c.md"), "c".to_string()),
            (mk("/d.md"), "d".to_string()),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let paths = |results: &[(NoteEntryData, NoteContentData)]| {
            let mut p: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
            p.sort();
            p
        };

        // Prefix (non-wildcard) is unchanged: /work matches the folder + subfolders.
        let r = super::search_terms(db.pool(), "/work").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/work/a.md".to_string(), "/work/sub/b.md".to_string()],
        );

        // Wildcard prefix: /wo* behaves like the prefix form.
        let r = super::search_terms(db.pool(), "/wo*").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/work/a.md".to_string(), "/work/sub/b.md".to_string()],
        );

        // Suffix wildcard on the folder path: /*sub → only notes whose folder ends in "sub".
        let r = super::search_terms(db.pool(), "/*sub").await.unwrap();
        assert_eq!(paths(&r), vec!["/work/sub/b.md".to_string()]);

        // Subfolder wildcard: /work/* → only notes strictly under /work/.
        let r = super::search_terms(db.pool(), "/work/*").await.unwrap();
        assert_eq!(paths(&r), vec!["/work/sub/b.md".to_string()]);

        // Excluded wildcard: -/wo* drops everything under /work.
        let r = super::search_terms(db.pool(), "-/wo*").await.unwrap();
        assert_eq!(
            paths(&r),
            vec!["/d.md".to_string(), "/personal/c.md".to_string()],
        );

        db.close().await;
    }

    #[tokio::test]
    async fn delete_directory_no_trailing_slash_does_not_match_sibling_prefix() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let target = VaultPath::note_path_from("/notes/a.md");
        let sibling = VaultPath::note_path_from("/notes_archive/b.md");
        let entries = vec![
            (
                NoteEntryData {
                    path: target.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "x".to_string(),
            ),
            (
                NoteEntryData {
                    path: sibling.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "y".to_string(),
            ),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        super::delete_directories(&mut tx, &[VaultPath::new("/notes")])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM notes ORDER BY path")
            .fetch_all(db.pool())
            .await
            .unwrap();
        let paths: Vec<String> = rows.into_iter().map(|(p,)| p).collect();
        assert_eq!(
            paths,
            vec![sibling.to_string()],
            "sibling /notes_archive/ must not be deleted"
        );
        db.close().await;
    }

    #[tokio::test]
    async fn path_search_with_underscore_does_not_treat_as_wildcard() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let target = VaultPath::note_path_from("/my_notes/a.md");
        let sibling = VaultPath::note_path_from("/myXnotes/b.md");
        let entries = vec![
            (
                NoteEntryData {
                    path: target.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "x".to_string(),
            ),
            (
                NoteEntryData {
                    path: sibling.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "y".to_string(),
            ),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        // pt:my_notes search must only match /my_notes/, not /myXnotes/.
        let results = super::search_terms(db.pool(), "pt:my_notes").await.unwrap();
        let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(
            paths,
            vec![target.to_string()],
            "underscore must be literal in path search"
        );
        db.close().await;
    }

    #[tokio::test]
    async fn filename_search_with_underscore_does_not_treat_as_wildcard() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let target = VaultPath::note_path_from("/my_note.md");
        let sibling = VaultPath::note_path_from("/myXnote.md");
        let entries = vec![
            (
                NoteEntryData {
                    path: target.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "x".to_string(),
            ),
            (
                NoteEntryData {
                    path: sibling.clone(),
                    size: 10,
                    modified_secs: 0,
                },
                "y".to_string(),
            ),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        let results = super::search_terms(db.pool(), "=my_note").await.unwrap();
        let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(
            paths,
            vec![target.to_string()],
            "underscore must be literal in filename search"
        );
        db.close().await;
    }

    #[tokio::test]
    async fn fts_term_with_metachar_does_not_error() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entry = NoteEntryData {
            path: VaultPath::note_path_from("/a.md"),
            size: 10,
            modified_secs: 0,
        };
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, "some meeting note".to_string())])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // Each of these would have produced an FTS4 syntax error before the fix.
        for q in &[
            "(meeting",
            "*",
            "meet*ing",
            "title:value",
            "a^b",
            "<",
            ">",
            "=",
            "@",
            "-",
            "-<",
            "->",
            "in:",
            "name:",
        ] {
            let res = super::search_terms(db.pool(), q).await;
            assert!(
                res.is_ok(),
                "query {:?} must not error; got {:?}",
                q,
                res.err()
            );
        }

        db.close().await;
    }

    #[tokio::test]
    async fn breadcrumb_term_with_metachar_does_not_error() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::NoteIndex::open(&db_path).await.unwrap();

        let entry = NoteEntryData {
            path: VaultPath::note_path_from("/a.md"),
            size: 10,
            modified_secs: 0,
        };
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, "# Heading\n\ntext".to_string())])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        for q in &["@(heading", "@*", "in:title:", ">(heading", ">*"] {
            let res = super::search_terms(db.pool(), q).await;
            assert!(
                res.is_ok(),
                "breadcrumb query {:?} must not error; got {:?}",
                q,
                res.err()
            );
        }

        db.close().await;
    }

    #[cfg(test)]
    mod note_columns_consistency {
        #[test]
        fn note_columns_is_path_plus_rest() {
            assert_eq!(
                super::super::NOTE_COLUMNS,
                format!("path, {}", super::super::NOTE_COLUMNS_REST),
                "NOTE_COLUMNS must equal 'path, ' + NOTE_COLUMNS_REST"
            );
        }
    }

    /// On a stored DB version older than the current `VERSION`, reopening the
    /// vault must self-heal the schema (ADR-0007): the index comes back valid
    /// but empty, `index_ready` reports `false`, and the next sync pass
    /// (`validate_and_init`) refills it. After the heal, stale `>`-separated
    /// breadcrumb rows are gone and the new `\x1f` separator is in place.
    #[tokio::test(flavor = "multi_thread")]
    async fn reopen_self_heals_outdated_schema() {
        use crate::{NoteVault, VaultConfig};
        use sqlx::Row;

        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n## Sub\nbody text").unwrap();

        // Bring the index up at the current version with one indexed note.
        {
            let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
            vault.validate_and_init().await.unwrap();
            // A brand-new index is healed-on-open, hence not ready; reopening
            // it below (current version) must report ready.

            // Force the schema backwards: stamp version `0.4` and rewrite
            // stored breadcrumbs in the legacy `>`-joined form to simulate a
            // vault upgraded across the separator change.
            let pool = vault.index.pool();
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
            vault.index.close().await;
        }

        // Reopen: the outdated schema is healed silently; the probe reports
        // not-ready until a sync pass fills the empty index.
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        assert!(!vault.index_ready(), "healed index must not report ready");
        vault.validate_and_init().await.unwrap();

        // Post-heal: no row carries the legacy separator; non-empty
        // breadcrumbs use `\x1f`.
        let pool = vault.index.pool();
        let after: Vec<String> =
            sqlx::query("SELECT breadcrumb FROM notesContent WHERE breadcrumb != ''")
                .fetch_all(pool)
                .await
                .unwrap()
                .into_iter()
                .map(|r| r.try_get("breadcrumb").unwrap())
                .collect();
        assert!(
            !after.is_empty(),
            "expected reindexed breadcrumb rows after heal"
        );
        assert!(
            after.iter().all(|b| !b.contains('>')),
            "stale `>` separator survived the heal: {:?}",
            after
        );

        // A second reopen with a current schema must report ready.
        vault.index.close().await;
        drop(vault);
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        assert!(vault.index_ready(), "current schema must report ready");
    }

    /// `open` on a current-version schema must not heal: `ready` is `true`
    /// and existing rows survive.
    #[tokio::test]
    async fn open_preserves_current_schema() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");

        // First open heals the fresh file into a current schema.
        let first = super::NoteIndex::open(&db_path).await.unwrap();
        assert!(!first.ready());
        sqlx::query("INSERT INTO appData (name, value) VALUES ('marker', 'kept')")
            .execute(first.pool())
            .await
            .unwrap();
        first.close().await;

        // Second open sees a current schema: no heal, data intact.
        let second = super::NoteIndex::open(&db_path).await.unwrap();
        assert!(second.ready());
        let marker: Option<String> =
            sqlx::query_scalar("SELECT value FROM appData WHERE name = 'marker'")
                .fetch_optional(second.pool())
                .await
                .unwrap();
        assert_eq!(marker.as_deref(), Some("kept"));
        second.close().await;
    }
}
