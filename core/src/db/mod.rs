// mod async_db;
mod search_terms;

use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
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
    nfs::{NoteEntryData, PATH_SEPARATOR},
    VaultPath,
};

// 0.7: Dropped the redundant `labels_by_name` index (the PK autoindex
//      sqlite_autoindex_labels_1 already covers WHERE name = ? lookups).
//      Bump forces a clean reindex so existing 0.6 installs drop the dead
//      index on next launch.
// 0.6: Added `labels` table populated from hashtags in note bodies. Bump
//      forces a clean reindex so the table is filled for existing vaults.
// 0.5: BREADCRUMB_SEP changed from `>` to `\x1f`. Bump forces a clean
//      reindex so stale rows with the old separator are rewritten.
const VERSION: &str = "0.7";
pub(crate) const DB_FILE: &str = "kimun.sqlite";

#[derive(Debug, Clone)]
pub(super) struct VaultDB {
    db_path: PathBuf,
    pool: SqlitePool,
}

#[derive(Debug, PartialEq)]
pub enum DBStatus {
    Ready,
    Outdated,
    NotValid,
    #[allow(dead_code)]
    FileNotFound,
}

impl DBStatus {
    pub fn is_ready(&self) -> bool {
        DBStatus::Ready.eq(self)
    }
}

impl Display for DBStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DBStatus::Ready => write!(f, "DB is Ready"),
            DBStatus::Outdated => write!(f, "DB is an old version, needs to be rebuilt"),
            DBStatus::NotValid => write!(f, "DB file is not valid"),
            DBStatus::FileNotFound => write!(f, "No DB file found"),
        }
    }
}

impl VaultDB {
    pub(super) async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, DBError> {
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

        Ok(Self { db_path, pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn get_db_path(&self) -> PathBuf {
        self.db_path.clone()
    }

    pub async fn check_db(&self) -> Result<DBStatus, DBError> {
        debug!("Checking the DB");

        let version: Option<String> =
            sqlx::query_scalar("SELECT value FROM appData WHERE name = 'version'")
                .fetch_optional(&self.pool)
                .await
                .or_else(|e| {
                    // If the table doesn't exist, return FileNotFound
                    if e.to_string().contains("no such table") {
                        return Ok(None);
                    }
                    Err(e)
                })?;

        match version {
            Some(v) => {
                debug!("DB Version: {}, current DB Version: {}", v, VERSION);
                if v == VERSION {
                    debug!("DB up to date");
                    Ok(DBStatus::Ready)
                } else {
                    debug!("DB outdated");
                    Ok(DBStatus::Outdated)
                }
            }
            None => {
                debug!("DB not valid or not found");
                Ok(DBStatus::NotValid)
            }
        }
    }

    pub async fn close(&self) -> Result<(), DBError> {
        self.pool.close().await;
        Ok(())
    }
}

/// Deletes all tables and recreates them
pub async fn init_db(pool: &SqlitePool) -> Result<(), DBError> {
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
            destination TEXT
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

fn combine_conditions(positive: Vec<String>, negative: Vec<String>) -> Option<String> {
    match (positive.is_empty(), negative.is_empty()) {
        (true, true) => None,
        (false, true) => Some(positive.join(" OR ")),
        (true, false) => Some(negative.join(" AND ")),
        (false, false) => Some(format!(
            "({}) AND ({})",
            positive.join(" OR "),
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

fn add_exclusion_conditions(
    excluded_terms: &[String],
    var_num: &mut usize,
    exclusion_conditions: &mut Vec<String>,
    params: &mut Vec<String>,
) {
    for excluded in excluded_terms {
        exclusion_conditions.push(format!(
            "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH ?{})",
            var_num
        ));
        params.push(excluded.clone());
        *var_num += 1;
    }
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

    add_content_terms_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_breadcrumb_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_filename_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_path_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_labels_query(search_terms, &mut var_num, &mut params, &mut queries);

    if queries.is_empty() {
        debug!("No query provided");
        return (String::new(), vec![]);
    }
    (queries.join(" INTERSECT "), params)
}

/// Free-text content terms: positive matches use FTS4 `MATCH`; exclusions use
/// `NOT IN` subqueries because FTS4 doesn't support pure-negative queries.
fn add_content_terms_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    if s.terms.is_empty() && s.excluded_terms.is_empty() {
        return;
    }
    let mut exclusions = vec![];
    add_exclusion_conditions(&s.excluded_terms, var_num, &mut exclusions, params);

    if !s.terms.is_empty() {
        let where_clause = if exclusions.is_empty() {
            format!("notesContent MATCH ?{}", var_num)
        } else {
            format!(
                "notesContent MATCH ?{} AND {}",
                var_num,
                exclusions.join(" AND ")
            )
        };
        queries.push(format!("{} WHERE {}", search_base_sql(), where_clause));
        params.push(s.terms.join(" "));
        *var_num += 1;
    } else if !exclusions.is_empty() {
        // Pure exclusion: scan all notes, drop those matching the excluded terms.
        queries.push(format!(
            "{} WHERE {}",
            search_base_sql(),
            exclusions.join(" AND ")
        ));
    }
}

/// Breadcrumb (heading-path) FTS column. Positive-only uses the column-scoped
/// `MATCH`; mixed positive + exclusions inline column prefixes with `-`;
/// exclusion-only uses `NOT IN`.
fn add_breadcrumb_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    if s.breadcrumb.is_empty() && s.excluded_breadcrumb.is_empty() {
        return;
    }
    if !s.breadcrumb.is_empty() {
        if s.excluded_breadcrumb.is_empty() {
            queries.push(format!(
                "{} WHERE notesContent.breadcrumb MATCH ?{}",
                search_base_sql(),
                var_num
            ));
            params.push(s.breadcrumb.join(" "));
        } else {
            let mut parts = Vec::with_capacity(s.breadcrumb.len() + s.excluded_breadcrumb.len());
            for b in &s.breadcrumb {
                parts.push(format!("breadcrumb: {}", b));
            }
            for b in &s.excluded_breadcrumb {
                parts.push(format!("breadcrumb: -{}", b));
            }
            queries.push(format!(
                "{} WHERE notesContent MATCH ?{}",
                search_base_sql(),
                var_num
            ));
            params.push(parts.join(" "));
        }
        *var_num += 1;
        return;
    }
    // Exclusion-only: NOT IN with column-prefixed term per excluded breadcrumb.
    let mut exclusions = vec![];
    for excluded in &s.excluded_breadcrumb {
        exclusions.push(format!(
            "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH ?{})",
            var_num
        ));
        params.push(format!("breadcrumb: {}", excluded));
        *var_num += 1;
    }
    queries.push(format!(
        "{} WHERE {}",
        search_base_sql(),
        exclusions.join(" AND ")
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
        |n| format!("notes.noteName LIKE ('%' || ?{} || '%')", n),
        |n| format!("notes.noteName NOT LIKE ('%' || ?{} || '%')", n),
        var_num,
        params,
        |t| t.clone(),
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

/// Notes-only base SELECT (no `notesContent` join) so label-only queries
/// don't pay an FTS scan. Same columns as `SEARCH_BASE_SQL` so INTERSECT
/// branches line up.
static LABEL_BASE_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "SELECT DISTINCT notes.path as path, {} FROM notes",
        NOTE_COLUMNS_REST
    )
});

fn label_base_sql() -> &'static str {
    &LABEL_BASE_SQL
}

fn add_labels_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    // Each positive label is its own INTERSECT branch backed by the
    // labels_by_name index via an IN subquery against `labels`.
    for label in &s.labels {
        let q = format!(
            "{} WHERE notes.path IN (SELECT path FROM labels WHERE name = ?{})",
            label_base_sql(),
            var_num
        );
        queries.push(q);
        params.push(label.clone());
        *var_num += 1;
    }

    // Excluded labels: bundled into a single notes-only SELECT with a
    // chain of NOT IN clauses (so the INTERSECT machinery still composes).
    if !s.excluded_labels.is_empty() {
        let mut clauses = Vec::with_capacity(s.excluded_labels.len());
        for label in &s.excluded_labels {
            clauses.push(format!(
                "notes.path NOT IN (SELECT path FROM labels WHERE name = ?{})",
                var_num
            ));
            params.push(label.clone());
            *var_num += 1;
        }
        queries.push(format!(
            "{} WHERE {}",
            label_base_sql(),
            clauses.join(" AND ")
        ));
    }
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
        let (cond, value) = match term.strip_suffix(PATH_SEPARATOR) {
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
                    format!("notes.basePath {} ('/' || ?{} || '%') ESCAPE '\\'", op, var_num),
                    escape_like_pattern(term),
                )
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

pub async fn get_all_notes(
    pool: &SqlitePool,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let query = format!("SELECT DISTINCT {} FROM notes", NOTE_COLUMNS);
    let rows = sqlx::query(&query).fetch_all(pool).await?;
    rows.iter().map(row_to_note_entry).collect()
}

pub async fn list_labels(pool: &SqlitePool) -> Result<Vec<String>, DBError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT name FROM labels")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

pub async fn notes_with_label(
    pool: &SqlitePool,
    name: &str,
) -> Result<Vec<VaultPath>, DBError> {
    let normalized = name.to_lowercase();
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT path FROM labels WHERE name = ?")
            .bind(&normalized)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(p,)| VaultPath::new(p)).collect())
}

pub async fn search_terms<S: AsRef<str>>(
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

pub async fn search_note_by_name<S: AsRef<str>>(
    pool: &SqlitePool,
    name: S,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let name = name.as_ref().to_lowercase();
    let sql = format!("SELECT {} FROM notes where noteName = ?", NOTE_COLUMNS);
    let rows = sqlx::query(&sql).bind(&name).fetch_all(pool).await?;

    rows.iter().map(row_to_note_entry).collect()
}

pub async fn search_note_by_path(
    pool: &SqlitePool,
    path: &VaultPath,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let sql = format!("SELECT {} FROM notes where path = ?", NOTE_COLUMNS);
    let path_string = path.to_string();
    let rows = sqlx::query(&sql).bind(&path_string).fetch_all(pool).await?;

    // Should always return one or zero
    rows.iter().map(row_to_note_entry).collect()
}

pub async fn get_notes(
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
    let rows = sqlx::query(&sql)
        .bind(bind_value)
        .fetch_all(pool)
        .await?;

    rows.iter().map(row_to_note_entry).collect()
}

pub async fn get_backlinks(
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

pub async fn get_notes_sections(
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

pub async fn insert_notes(
    tx: &mut Transaction<'_, Sqlite>,
    notes: &[(NoteEntryData, String)],
) -> Result<(), DBError> {
    if notes.is_empty() {
        return Ok(());
    }
    debug!("Inserting {} notes", notes.len());
    upsert_notes_batched(tx, notes).await
}

pub async fn update_notes(
    tx: &mut Transaction<'_, Sqlite>,
    notes: &[(NoteEntryData, String)],
) -> Result<(), DBError> {
    if notes.is_empty() {
        return Ok(());
    }
    debug!("Updating {} notes", notes.len());
    upsert_notes_batched(tx, notes).await
}

pub async fn delete_notes(
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

pub async fn save_note(
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
    const HEADER: &'static str = "INSERT INTO links (source, destination) VALUES ";
    const FOOTER: &'static str = "";
    const COLS: usize = 2;

    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        paths: &'q [String],
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        q.bind(&paths[self.path_idx]).bind(&self.destination)
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

pub async fn rename_note(
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

    sqlx::query("UPDATE links SET destination = ? WHERE destination = ?")
        .bind(to.to_string())
        .bind(from.to_string())
        .execute(&mut **tx)
        .await?;

    // Update links that reference the note by filename only (wikilinks without path)
    sqlx::query("UPDATE links SET destination = ? WHERE destination = ?")
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

pub async fn rename_directory(
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

pub async fn delete_directories(
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
    async fn vault_db_new_creates_parent_dir_for_db_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("nested/dir/cache.kimuncache");
        // Parent dir does not exist yet.
        assert!(!nested.parent().unwrap().exists());

        let db = super::VaultDB::new(&nested).await.unwrap();
        assert_eq!(db.get_db_path(), nested);
        assert!(nested.parent().unwrap().exists());
        assert!(nested.exists());
        db.close().await.unwrap();
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
        assert_eq!(params[0], "foo bar");
    }

    #[test]
    fn test_search_terms_query_single_term() {
        let (sql, params) = build_search_sql_query("keyword");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_breadcrumb_only() {
        let (sql, params) = build_search_sql_query(">heading");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "heading");
    }

    #[test]
    fn test_search_terms_query_breadcrumb_with_in() {
        let (sql, params) = build_search_sql_query("in:section");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "section");
    }

    #[test]
    fn test_search_terms_query_multiple_breadcrumbs() {
        let (sql, params) = build_search_sql_query(">heading1 in:heading2");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "heading1 heading2");
    }

    #[test]
    fn test_search_terms_query_path_only() {
        let (sql, params) = build_search_sql_query("@filename");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ('%' || ?1 || '%')"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "filename");
    }

    #[test]
    fn test_search_terms_query_path_with_at() {
        let (sql, params) = build_search_sql_query("at:directory");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ('%' || ?1 || '%')"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "directory");
    }

    #[test]
    fn test_search_terms_query_multiple_paths() {
        let (sql, params) = build_search_sql_query("@file1 at:file2");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ('%' || ?1 || '%') OR notes.noteName LIKE ('%' || ?2 || '%')"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "file1");
        assert_eq!(params[1], "file2");
    }

    #[test]
    fn test_search_terms_query_terms_and_breadcrumb() {
        let (sql, params) = build_search_sql_query("keyword >section");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "keyword");
        assert_eq!(params[1], "section");
    }

    #[test]
    fn test_search_terms_query_terms_and_path() {
        let (sql, params) = build_search_sql_query("keyword @file");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ('%' || ?2 || '%')"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "keyword");
        assert_eq!(params[1], "file");
    }

    #[test]
    fn test_search_terms_query_breadcrumb_and_path() {
        let (sql, params) = build_search_sql_query(">heading @file");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ('%' || ?2 || '%')"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "heading");
        assert_eq!(params[1], "file");
    }

    #[test]
    fn test_search_terms_query_all_combined() {
        let (sql, params) = build_search_sql_query("keyword >heading @file");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ('%' || ?3 || '%')"
        );
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], "keyword");
        assert_eq!(params[1], "heading");
        assert_eq!(params[2], "file");
    }

    #[test]
    fn test_search_terms_query_quoted_terms() {
        let (sql, params) = build_search_sql_query("\"exact phrase\" keyword");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "exact phrase keyword");
    }

    #[test]
    fn test_search_terms_query_order_by_title_asc() {
        let (sql, params) = build_search_sql_query("keyword or:title");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_order_by_title_desc() {
        let (sql, params) = build_search_sql_query("keyword or:-title");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_order_by_filename_asc() {
        let (sql, params) = build_search_sql_query("keyword or:filename");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_order_by_file_shorthand() {
        let (sql, params) = build_search_sql_query("keyword or:f");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_order_by_title_shorthand() {
        let (sql, params) = build_search_sql_query("keyword or:t");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_multiple_order_by() {
        let (sql, params) = build_search_sql_query("keyword ^title ^-filename");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_complex_with_order() {
        let (sql, params) = build_search_sql_query("keyword >section @file ^title");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ('%' || ?3 || '%')"
        );
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], "keyword");
        assert_eq!(params[1], "section");
        assert_eq!(params[2], "file");
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
        assert_eq!(params[0], "keyword");
    }

    #[test]
    fn test_search_terms_query_whitespace_handling() {
        let (sql, params) = build_search_sql_query("  keyword   >section  ");
        assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], "keyword");
        assert_eq!(params[1], "section");
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
        assert!(params.contains(&"cancelled".to_string()));
        assert!(params.contains(&"meeting".to_string()));

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
        assert_eq!(params[0], "cancelled");
    }

    #[test]
    fn test_breadcrumb_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query(">project >-draft");

        assert!(sql.contains("notesContent MATCH"));
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "breadcrumb: project breadcrumb: -draft");
    }

    #[test]
    fn test_like_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query("@2024 @-draft");

        // Should generate filename query with positive and negative conditions
        assert!(sql.contains("notes.noteName LIKE"));
        assert!(sql.contains("notes.noteName NOT LIKE"));
        assert!(params.contains(&"2024".to_string()));
        assert!(params.contains(&"draft".to_string()));
    }

    #[test]
    fn test_exclusion_only_like_query() {
        let (sql, params) = build_search_sql_query("@-draft @-temp");

        // Exclusion-only should still generate valid WHERE clause
        assert!(sql.contains("notes.noteName NOT LIKE"));
        assert!(!sql.contains("notes.noteName LIKE ('%'")); // No positive conditions
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_path_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query("/projects /-archive");

        assert!(sql.contains("notes.basePath LIKE"));
        assert!(sql.contains("notes.basePath NOT LIKE"));
        assert!(params.contains(&"projects".to_string()));
        assert!(params.contains(&"archive".to_string()));
    }

    #[test]
    fn test_exclusion_only_path_query() {
        let (sql, params) = build_search_sql_query("/-draft /-temp");

        assert!(sql.contains("notes.basePath NOT LIKE"));
        assert!(!sql.contains("notes.basePath LIKE ('/'"));
        assert_eq!(params.len(), 2);
    }

    #[tokio::test]
    async fn labels_table_exists_after_create_tables() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

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
        assert_eq!(idx_name.0, 0, "labels_by_name index must not exist (dropped in 0.7)");

        let idx_path: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='index' AND name='labels_by_path'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(idx_path.0, 1, "labels_by_path index should exist");

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn labels_are_persisted_on_note_insert() {
        use crate::nfs::{NoteEntryData, VaultPath};

        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

        let path = VaultPath::note_path_from("/n.md");
        let body = "Title\n\nbody with #foo and #Foo and #bar".to_string();
        let entry = NoteEntryData {
            path: path.clone(),
            size: body.len() as u64,
            modified_secs: 0,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, body)]).await.unwrap();
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

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn reindexing_a_note_drops_removed_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};

        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

        let path = VaultPath::note_path_from("/n.md");
        let body_v1 = "before #draft #keep".to_string();
        let entry_v1 = NoteEntryData {
            path: path.clone(),
            size: body_v1.len() as u64,
            modified_secs: 0,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry_v1, body_v1)]).await.unwrap();
        tx.commit().await.unwrap();

        let body_v2 = "after #keep only".to_string();
        let entry_v2 = NoteEntryData {
            path: path.clone(),
            size: body_v2.len() as u64,
            modified_secs: 1,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::update_notes(&mut tx, &[(entry_v2, body_v2)]).await.unwrap();
        tx.commit().await.unwrap();

        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM labels WHERE path = ? ORDER BY name",
        )
        .bind(path.to_string())
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            rows.into_iter().map(|(n,)| n).collect::<Vec<_>>(),
            vec!["keep".to_string()],
            "reindex must drop labels no longer present"
        );

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn labels_are_removed_on_note_delete() {
        use crate::nfs::{NoteEntryData, VaultPath};

        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

        let path = VaultPath::note_path_from("/n.md");
        let body = "x #drop".to_string();
        let entry = NoteEntryData {
            path: path.clone(),
            size: body.len() as u64,
            modified_secs: 0,
        };

        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &[(entry, body)]).await.unwrap();
        super::delete_notes(&mut tx, std::slice::from_ref(&path)).await.unwrap();
        tx.commit().await.unwrap();

        let count: (i64,) =
            sqlx::query_as("SELECT count(*) FROM labels WHERE path = ?")
                .bind(path.to_string())
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count.0, 0);

        db.close().await.unwrap();
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
        assert!(sql.contains("INTERSECT"), "two labels should INTERSECT: {}", sql);
    }

    #[tokio::test]
    async fn search_by_label_returns_matching_notes() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

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

        let results = super::search_terms(db.pool(), "#important #todo").await.unwrap();
        let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(paths, vec!["/a.md".to_string()]);

        let results = super::search_terms(db.pool(), "#nope").await.unwrap();
        assert!(results.is_empty());

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn label_search_uses_index() {
        // Confirms the PK autoindex (sqlite_autoindex_labels_1) is used for
        // label lookups after the explicit labels_by_name index was dropped in
        // 0.7. A hashtag filter must not degrade to a full table scan.
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

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
        let rows: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(&plan_sql).bind("important").fetch_all(db.pool()).await.unwrap();
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

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn rename_note_updates_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

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

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn rename_directory_updates_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

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

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT name, path FROM labels")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![("moved".to_string(), "/new_dir/note.md".to_string())],
        );

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn delete_directory_removes_labels() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

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

        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn delete_directory_with_underscore_does_not_touch_siblings() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

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

        let remaining: Vec<(String,)> =
            sqlx::query_as("SELECT path FROM notes ORDER BY path")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            remaining.into_iter().map(|(p,)| p).collect::<Vec<_>>(),
            vec![sibling.to_string()],
            "sibling /myXdir/b.md must be untouched"
        );

        let sibling_label: (i64,) =
            sqlx::query_as("SELECT count(*) FROM labels WHERE path = ?")
                .bind(sibling.to_string())
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(sibling_label.0, 1, "sibling label preserved");

        db.close().await.unwrap();
    }

    #[test]
    fn escape_like_pattern_escapes_metacharacters() {
        assert_eq!(super::escape_like_pattern("/my_dir/"), "/my\\_dir/");
        assert_eq!(super::escape_like_pattern("/a%b/"), "/a\\%b/");
        assert_eq!(super::escape_like_pattern("/a\\b/"), "/a\\\\b/");
        assert_eq!(super::escape_like_pattern("/normal/"), "/normal/");
    }

    #[tokio::test]
    async fn delete_directory_no_trailing_slash_does_not_match_sibling_prefix() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

        let target = VaultPath::note_path_from("/notes/a.md");
        let sibling = VaultPath::note_path_from("/notes_archive/b.md");
        let entries = vec![
            (NoteEntryData { path: target.clone(), size: 10, modified_secs: 0 }, "x".to_string()),
            (NoteEntryData { path: sibling.clone(), size: 10, modified_secs: 0 }, "y".to_string()),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        super::delete_directories(&mut tx, &[VaultPath::new("/notes")]).await.unwrap();
        tx.commit().await.unwrap();

        let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM notes ORDER BY path")
            .fetch_all(db.pool())
            .await
            .unwrap();
        let paths: Vec<String> = rows.into_iter().map(|(p,)| p).collect();
        assert_eq!(paths, vec![sibling.to_string()], "sibling /notes_archive/ must not be deleted");
        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn path_search_with_underscore_does_not_treat_as_wildcard() {
        use crate::nfs::{NoteEntryData, VaultPath};
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("kimun.sqlite");
        let db = super::VaultDB::new(&db_path).await.unwrap();
        super::create_tables(db.pool()).await.unwrap();

        let target = VaultPath::note_path_from("/my_notes/a.md");
        let sibling = VaultPath::note_path_from("/myXnotes/b.md");
        let entries = vec![
            (NoteEntryData { path: target.clone(), size: 10, modified_secs: 0 }, "x".to_string()),
            (NoteEntryData { path: sibling.clone(), size: 10, modified_secs: 0 }, "y".to_string()),
        ];
        let mut tx = db.pool().begin().await.unwrap();
        super::insert_notes(&mut tx, &entries).await.unwrap();
        tx.commit().await.unwrap();

        // pt:my_notes search must only match /my_notes/, not /myXnotes/.
        let results = super::search_terms(db.pool(), "pt:my_notes").await.unwrap();
        let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
        assert_eq!(paths, vec![target.to_string()], "underscore must be literal in path search");
        db.close().await.unwrap();
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
}
