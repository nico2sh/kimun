// mod async_db;
mod search_terms;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use log::{debug, error};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::{Row, Sqlite, Transaction};
use search_terms::{OrderBy, SearchTerms};

use crate::note::{ContentChunk, LinkType, NoteContentData, NoteDetails};

fn row_to_note_entry(row: &sqlx::sqlite::SqliteRow) -> Result<(NoteEntryData, NoteContentData), DBError> {
    let path: String = row.try_get("path")?;
    let title: String = row.try_get("title")?;
    let size: i64 = row.try_get("size")?;
    let modified: i64 = row.try_get("modified")?;
    let hash: String = row.try_get("hash")?;

    let note_path = VaultPath::new(&path);
    let entry = NoteEntryData {
        path: note_path,
        size: size as u64,
        modified_secs: modified as u64,
    };
    let content = NoteContentData::new(title, hash.parse().unwrap_or(0));
    Ok((entry, content))
}

use super::error::DBError;

use super::{nfs::NoteEntryData, VaultPath};

const VERSION: &str = "0.4";
const DB_FILE: &str = "kimun.sqlite";

#[derive(Debug, Clone)]
pub(super) struct VaultDB {
    workspace_path: PathBuf,
    pool: SqlitePool,
}

pub enum DBStatus {
    Ready,
    Outdated,
    NotValid,
    FileNotFound,
}

impl VaultDB {
    pub(super) async fn new<P: AsRef<Path>>(workspace_path: P) -> Result<Self, DBError> {
        let workspace_path = workspace_path.as_ref().to_owned();
        let db_path = workspace_path.join(DB_FILE);
        let connection_string = format!("sqlite:{}?mode=rwc", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect(&connection_string)
            .await?;

        Ok(Self {
            workspace_path,
            pool,
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn get_db_path(&self) -> PathBuf {
        self.workspace_path.join(DB_FILE)
    }

    pub async fn check_db(&self) -> Result<DBStatus, DBError> {
        debug!("Checking the DB");

        let version: Option<String> = sqlx::query_scalar(
            "SELECT value FROM appData WHERE name = 'version'"
        )
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
            Ok(_) => {},
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
        )"
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
        )"
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE links (
            source TEXT,
            destination TEXT
        )"
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX backlinks
            ON links(destination)"
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE VIRTUAL TABLE notesContent USING fts4(
            path,
            breadcrumb,
            text
        )"
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}

pub fn build_search_sql_query<S: AsRef<str>>(query: S) -> (String, Vec<String>) {
    let search_terms = SearchTerms::from_query_string(query);
    let mut var_num = 1;
    let base_sql = "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path";
    let mut params = vec![];
    let mut queries = vec![];
    if !search_terms.terms.is_empty() || !search_terms.excluded_terms.is_empty() {
        if !search_terms.terms.is_empty() {
            // Positive content terms: create query with all positive terms + exclusions
            let mut fts_query_parts = vec![search_terms.terms.join(" ")];

            // Add excluded terms with FTS4 - prefix
            for excluded in &search_terms.excluded_terms {
                fts_query_parts.push(format!("-{}", excluded));
            }

            let terms_sql = format!("{} WHERE notesContent MATCH ?{}", base_sql, var_num);
            queries.push(terms_sql);
            params.push(fts_query_parts.join(" "));
            var_num += 1;
        } else if !search_terms.excluded_terms.is_empty() {
            // Exclusion-only content query: FTS4 doesn't support pure exclusions
            // Use NOT IN approach with subquery for each excluded term
            let mut exclusion_conditions = vec![];
            for excluded in &search_terms.excluded_terms {
                exclusion_conditions.push(format!(
                    "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH ?{})",
                    var_num
                ));
                params.push(excluded.clone());
                var_num += 1;
            }

            // Use base_sql to get all notes, then exclude matching ones
            let terms_sql = format!("{} WHERE {}", base_sql, exclusion_conditions.join(" AND "));
            queries.push(terms_sql);
        }
    }
    if !search_terms.breadcrumb.is_empty() || !search_terms.excluded_breadcrumb.is_empty() {
        if !search_terms.breadcrumb.is_empty() {
            if search_terms.excluded_breadcrumb.is_empty() {
                // Positive-only breadcrumb terms: use column-specific MATCH syntax
                let terms_sql = format!(
                    "{} WHERE notesContent.breadcrumb MATCH ?{}",
                    base_sql, var_num
                );
                queries.push(terms_sql);
                params.push(search_terms.breadcrumb.join(" "));
                var_num += 1;
            } else {
                // Positive breadcrumb terms with exclusions: use column-prefix syntax in notesContent MATCH
                let mut breadcrumb_parts = vec![];

                // Add positive breadcrumb terms with column prefix
                for breadcrumb in &search_terms.breadcrumb {
                    breadcrumb_parts.push(format!("breadcrumb: {}", breadcrumb));
                }

                // Add excluded breadcrumb terms with column prefix
                for excluded in &search_terms.excluded_breadcrumb {
                    breadcrumb_parts.push(format!("breadcrumb: -{}", excluded));
                }

                let terms_sql = format!("{} WHERE notesContent MATCH ?{}", base_sql, var_num);
                queries.push(terms_sql);
                params.push(breadcrumb_parts.join(" "));
                var_num += 1;
            }
        } else if !search_terms.excluded_breadcrumb.is_empty() {
            // Exclusion-only breadcrumb query: use NOT IN approach for breadcrumb column
            let mut exclusion_conditions = vec![];
            for excluded in &search_terms.excluded_breadcrumb {
                exclusion_conditions.push(format!(
                    "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH ?{})",
                    var_num
                ));
                params.push(format!("breadcrumb: {}", excluded));
                var_num += 1;
            }

            let terms_sql = format!("{} WHERE {}", base_sql, exclusion_conditions.join(" AND "));
            queries.push(terms_sql);
        }
    }
    if !search_terms.filename.is_empty() || !search_terms.excluded_filename.is_empty() {
        let mut positive_conditions = vec![];
        let mut negative_conditions = vec![];

        for filename in search_terms.filename {
            if !filename.is_empty() {
                positive_conditions.push(format!("notes.noteName LIKE ('%' || ?{} || '%')", var_num));
                params.push(filename);
                var_num += 1;
            }
        }

        for excluded in search_terms.excluded_filename {
            if !excluded.is_empty() {
                negative_conditions.push(format!("notes.noteName NOT LIKE ('%' || ?{} || '%')", var_num));
                params.push(excluded);
                var_num += 1;
            }
        }

        let final_where = match (positive_conditions.is_empty(), negative_conditions.is_empty()) {
            (false, false) => format!(
                "({}) AND ({})",
                positive_conditions.join(" OR "),
                negative_conditions.join(" AND ")
            ),
            (false, true) => positive_conditions.join(" OR "),
            (true, false) => negative_conditions.join(" AND "),
            (true, true) => unreachable!(),
        };

        let terms_sql = format!("{} WHERE {}", base_sql, final_where);
        queries.push(terms_sql);
    }
    if !search_terms.path.is_empty() || !search_terms.excluded_path.is_empty() {
        let mut positive_conditions = vec![];
        let mut negative_conditions = vec![];

        for path in search_terms.path {
            if !path.is_empty() {
                match path.strip_suffix("/") {
                    Some(absolute) => {
                        positive_conditions.push(format!("notes.basePath = ('/' || ?{})", var_num));
                        params.push(absolute.to_string());
                    }
                    None => {
                        positive_conditions.push(format!("notes.basePath LIKE ('/' || ?{} || '%')", var_num));
                        params.push(path.to_string());
                    }
                }
                var_num += 1;
            }
        }

        for excluded in search_terms.excluded_path {
            if !excluded.is_empty() {
                match excluded.strip_suffix("/") {
                    Some(absolute) => {
                        negative_conditions.push(format!("notes.basePath != ('/' || ?{})", var_num));
                        params.push(absolute.to_string());
                    }
                    None => {
                        negative_conditions.push(format!("notes.basePath NOT LIKE ('/' || ?{} || '%')", var_num));
                        params.push(excluded.to_string());
                    }
                }
                var_num += 1;
            }
        }

        let final_where = match (positive_conditions.is_empty(), negative_conditions.is_empty()) {
            (false, false) => format!(
                "({}) AND ({})",
                positive_conditions.join(" OR "),
                negative_conditions.join(" AND ")
            ),
            (false, true) => positive_conditions.join(" OR "),
            (true, false) => negative_conditions.join(" AND "),
            (true, true) => unreachable!(),
        };

        let terms_sql = format!("{} WHERE {}", base_sql, final_where);
        queries.push(terms_sql);
    }

    if queries.is_empty() {
        debug!("No query provided");
        return (String::new(), vec![]);
    }

    let sql = queries.join(" INTERSECT ");

    (sql, params)
}

pub async fn get_all_notes(
    pool: &SqlitePool,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let query = "SELECT DISTINCT path, title, size, modified, hash, noteName FROM notes";

    let rows = sqlx::query(query)
        .fetch_all(pool)
        .await?;

    rows.iter().map(row_to_note_entry).collect()
}

pub async fn search_terms<S: AsRef<str>>(
    pool: &SqlitePool,
    search_query: S,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let search_query = search_query.as_ref();
    let order_by = SearchTerms::from_query_string(search_query).order_by;
    let (query, params) = build_search_sql_query(search_query);

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

    let mut result: Vec<(NoteEntryData, NoteContentData)> = rows.iter().map(row_to_note_entry).collect::<Result<_, _>>()?;

    if !order_by.is_empty() {
        result.sort_by(|(a_entry, a_content), (b_entry, b_content)| {
            for ob in &order_by {
                let ord = match ob {
                    OrderBy::Title { asc } => {
                        let cmp = a_content.title.to_lowercase().cmp(&b_content.title.to_lowercase());
                        if *asc { cmp } else { cmp.reverse() }
                    }
                    OrderBy::FileName { asc } => {
                        let cmp = a_entry.path.to_string().cmp(&b_entry.path.to_string());
                        if *asc { cmp } else { cmp.reverse() }
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

async fn note_exists(pool: &SqlitePool, path: &VaultPath) -> Result<bool, DBError> {
    let sql = "SELECT count(*) FROM notes where path = ?";
    let count: i64 = sqlx::query_scalar(sql)
        .bind(path.to_string())
        .fetch_one(pool)
        .await?;

    match count {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DBError::Other(format!(
            "Unexpected error, the DB contains more than one ({}) entry for path {}",
            count, path
        ))),
    }
}

pub async fn search_note_by_name<S: AsRef<str>>(
    pool: &SqlitePool,
    name: S,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let name = name.as_ref().to_lowercase();
    let sql = "SELECT path, title, size, modified, hash, noteName FROM notes where noteName = ?";

    let rows = sqlx::query(sql)
        .bind(&name)
        .fetch_all(pool)
        .await?;

    rows.iter().map(row_to_note_entry).collect()
}

pub async fn search_note_by_path(
    pool: &SqlitePool,
    path: &VaultPath,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let sql = "SELECT path, title, size, modified, hash, noteName FROM notes where path = ?";
    let path_string = path.to_string();

    let rows = sqlx::query(sql)
        .bind(&path_string)
        .fetch_all(pool)
        .await?;

    // Should always return one or zero
    rows.iter().map(row_to_note_entry).collect()
}

pub async fn get_notes(
    pool: &SqlitePool,
    path: &VaultPath,
    recursive: bool,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let sql = if recursive {
        "SELECT path, title, size, modified, hash, noteName FROM notes where basePath LIKE (? || '%')"
    } else {
        "SELECT path, title, size, modified, hash, noteName FROM notes where basePath = ?"
    };

    let rows = sqlx::query(sql)
        .bind(path.to_string())
        .fetch_all(pool)
        .await?;

    rows.iter().map(row_to_note_entry).collect()
}

pub async fn get_backlinks(
    pool: &SqlitePool,
    path: &VaultPath,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    // Match notes that link to the full path OR by filename only (wikilinks stored without path)
    let sql = "SELECT DISTINCT n.path, n.title, n.size, n.modified, n.hash, n.noteName \
               FROM notes n \
               JOIN links l ON n.path = l.source \
               WHERE l.destination = ? OR l.destination = ?";

    let rows = sqlx::query(sql)
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
        ("SELECT path, breadcrumb, text FROM notesContent WHERE path = ?", path.to_string())
    } else if recursive {
        // All notes under this directory tree
        ("SELECT path, breadcrumb, text FROM notesContent WHERE path LIKE (? || '%')", path.to_string())
    } else {
        // Only notes directly in this directory (basePath join)
        ("SELECT nc.path, nc.breadcrumb, nc.text FROM notesContent nc JOIN notes n ON nc.path = n.path WHERE n.basePath = ?", path.to_string())
    };

    let rows = sqlx::query(sql)
        .bind(bind_value)
        .fetch_all(pool)
        .await?;

    for row in rows {
        let path: String = row.try_get("path")?;
        let breadcrumb: String = row.try_get("breadcrumb")?;
        let text: String = row.try_get("text")?;

        let path = VaultPath::new(path);
        let breadcrumb = breadcrumb
            .split(">")
            .map(|e| e.to_string())
            .collect::<Vec<String>>();

        let chunk = ContentChunk { breadcrumb, text };
        let chunks = result.entry(path).or_insert_with(Vec::new);
        chunks.push(chunk);
    }

    Ok(result)
}

pub async fn insert_notes(
    tx: &mut Transaction<'_, Sqlite>,
    notes: &[(NoteEntryData, String)],
) -> Result<(), DBError> {
    if !notes.is_empty() {
        debug!("Inserting {} notes", notes.len());
        for (entry_data, text) in notes {
            let note_details = NoteDetails::new(&entry_data.path, text);
            insert_note(tx, entry_data, &note_details).await?;
        }
    }
    Ok(())
}

pub async fn update_notes(
    tx: &mut Transaction<'_, Sqlite>,
    notes: &[(NoteEntryData, String)],
) -> Result<(), DBError> {
    if !notes.is_empty() {
        debug!("Updating {} notes", notes.len());
        for (entry_data, text) in notes {
            let note_details = NoteDetails::new(&entry_data.path, text);
            update_note(tx, entry_data, &note_details).await?;
        }
    }
    Ok(())
}

pub async fn delete_notes(
    tx: &mut Transaction<'_, Sqlite>,
    paths: &[VaultPath],
) -> Result<(), DBError> {
    if !paths.is_empty() {
        for path in paths {
            delete_note(tx, path).await?;
        }
    }
    Ok(())
}

pub async fn save_note(
    pool: &SqlitePool,
    entry_data: &NoteEntryData,
    note_details: &NoteDetails,
) -> Result<(), DBError> {
    let exists = note_exists(pool, &entry_data.path).await?;
    let mut tx = pool.begin().await?;
    if exists {
        update_note(&mut tx, entry_data, note_details).await
    } else {
        insert_note(&mut tx, entry_data, note_details).await
    }?;
    tx.commit().await?;
    Ok(())
}

async fn insert_note(
    tx: &mut Transaction<'_, Sqlite>,
    entry_data: &NoteEntryData,
    note_details: &NoteDetails,
) -> Result<(), DBError> {
    let (parent_path, name) = entry_data.path.get_parent_path();
    let path_string = entry_data.path.to_string();
    let data = note_details.get_content_data();

    sqlx::query(
        "INSERT INTO notes (path, title, size, modified, hash, basePath, noteName) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&path_string)
    .bind(&data.title)
    .bind(entry_data.size as i64)
    .bind(entry_data.modified_secs as i64)
    .bind(data.hash.to_string())
    .bind(parent_path.to_string())
    .bind(&name)
    .execute(&mut **tx)
    .await
    .map_err(|e| {
        error!("Error inserting note: {}\nDetails: {}", e, note_details);
        DBError::DBError(e)
    })?;

    let (chunks, links) = note_details.get_chunks_and_links();
    for chunk in &chunks {
        let breadcrumb = chunk.get_breadcrumb();
        let chunk_text = &chunk.text;
        sqlx::query("INSERT INTO notesContent (path, breadcrumb, text) VALUES (?, ?, ?)")
            .bind(&path_string)
            .bind(&breadcrumb)
            .bind(chunk_text)
            .execute(&mut **tx)
            .await?;
    }

    for link in &links {
        if let LinkType::Note(path) = &link.ltype {
            sqlx::query("INSERT INTO links (source, destination) VALUES (?, ?)")
                .bind(&path_string)
                .bind(path.to_string())
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}

async fn update_note(
    tx: &mut Transaction<'_, Sqlite>,
    entry_data: &NoteEntryData,
    note_details: &NoteDetails,
) -> Result<(), DBError> {
    let data = note_details.get_content_data();
    let title = data.title.clone();
    let hash = data.hash.to_string();
    let path_string = entry_data.path.to_string();

    sqlx::query("UPDATE notes SET title = ?, size = ?, modified = ?, hash = ? WHERE path = ?")
        .bind(&title)
        .bind(entry_data.size as i64)
        .bind(entry_data.modified_secs as i64)
        .bind(&hash)
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM notesContent WHERE path = ?")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    let (chunks, links) = note_details.get_chunks_and_links();
    for chunk in &chunks {
        let breadcrumb = chunk.get_breadcrumb();
        let chunk_text = &chunk.text;
        sqlx::query("INSERT INTO notesContent (path, breadcrumb, text) VALUES (?, ?, ?)")
            .bind(&path_string)
            .bind(&breadcrumb)
            .bind(chunk_text)
            .execute(&mut **tx)
            .await?;
    }

    sqlx::query("DELETE FROM links WHERE source = ?")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    for link in &links {
        if let LinkType::Note(path) = &link.ltype {
            sqlx::query("INSERT INTO links (source, destination) VALUES (?, ?)")
                .bind(&path_string)
                .bind(path.to_string())
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}

async fn delete_note(tx: &mut Transaction<'_, Sqlite>, path: &VaultPath) -> Result<(), DBError> {
    let path_string = path.to_string();

    sqlx::query("DELETE FROM notes WHERE path = ?")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM notesContent WHERE path = ?")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM links WHERE source = ?")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM links WHERE destination = ?")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    Ok(())
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

    Ok(())
}

pub async fn rename_directory(
    tx: &mut Transaction<'_, Sqlite>,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), DBError> {
    let from = {
        let s = from.to_string();
        if s.ends_with('/') {
            s
        } else {
            s + "/"
        }
    };
    let to = {
        let s = to.to_string();
        if s.ends_with('/') {
            s
        } else {
            s + "/"
        }
    };

    let notes_sql = "UPDATE notes SET path = ? || SUBSTR(path, LENGTH(?) + 1), basePath = ? || SUBSTR(basePath, LENGTH(?) + 1) WHERE basePath LIKE (? || '%')";
    sqlx::query(notes_sql)
        .bind(&to)
        .bind(&from)
        .bind(&to)
        .bind(&from)
        .bind(&from)
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE notesContent SET path = ? || SUBSTR(path, LENGTH(?) + 1) WHERE path LIKE (? || '%')")
        .bind(&to)
        .bind(&from)
        .bind(&from)
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE links SET source = ? || SUBSTR(source, LENGTH(?) + 1) WHERE source LIKE (? || '%')")
        .bind(&to)
        .bind(&from)
        .bind(&from)
        .execute(&mut **tx)
        .await?;

    sqlx::query("UPDATE links SET destination = ? || SUBSTR(destination, LENGTH(?) + 1) WHERE destination LIKE (? || '%')")
        .bind(&to)
        .bind(&from)
        .bind(&from)
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
    let path_string = directory_path.to_string();

    sqlx::query("DELETE FROM notes WHERE path LIKE (? || '%')")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM notesContent WHERE path LIKE (? || '%')")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    sqlx::query("DELETE FROM links WHERE source LIKE (? || '%')")
        .bind(&path_string)
        .execute(&mut **tx)
        .await?;

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(sql.contains("notesContent MATCH"));
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "meeting -cancelled");

        // Should generate single query with combined positive and negative terms
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
        assert!(sql.contains("SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH"));
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
}
