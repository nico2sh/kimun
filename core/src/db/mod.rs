// pub mod async_db;
mod search_terms;

use std::path::{Path, PathBuf};

use log::{debug, error};
use rusqlite::{config::DbConfig, params, Connection, Transaction};
use rusqlite::{params_from_iter, OpenFlags, OptionalExtension};
use search_terms::SearchTerms;

use crate::note::{NoteContentData, NoteDetails};

use super::error::DBError;

use super::{nfs::NoteEntryData, VaultPath};

const VERSION: &str = "0.2";
const DB_FILE: &str = "notes.sqlite";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct VaultDB {
    workspace_path: PathBuf,
}

pub enum DBStatus {
    Ready,
    Outdated,
    NotValid,
    FileNotFound,
}

impl VaultDB {
    pub(super) fn new<P: AsRef<Path>>(workspace_path: P) -> Self {
        Self {
            workspace_path: workspace_path.as_ref().to_owned(),
        }
    }

    /// Executes a function with a connection, the connection is closed right
    /// after the funciton closes
    pub fn call<F, R>(&self, function: F) -> Result<R, DBError>
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<R, DBError> + 'static + Send,
    {
        let mut conn = ConnectionBuilder::build(&self.workspace_path)?;

        let res = function(&mut conn);
        conn.close().map_err(|(_conn, e)| e)?;

        res
    }

    pub fn get_db_path(&self) -> PathBuf {
        self.workspace_path.join(DB_FILE)
    }

    pub fn check_db(&self) -> Result<DBStatus, DBError> {
        let db_path = self.get_db_path();
        let conn_res = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI,
        );
        match conn_res {
            Ok(mut conn) => {
                let ver = self.current_schema_version(&mut conn)?.unwrap_or_default();
                debug!("DB Version: {}, current DB Version: {}", ver, VERSION);
                let status = if ver == VERSION {
                    debug!("DB up to date");
                    DBStatus::Ready
                } else {
                    debug!("DB outdated");
                    DBStatus::Outdated
                };
                conn.close().map_err(|(_conn, e)| e)?;
                Ok(status)
            }
            Err(e) => {
                if let Some(error_code) = e.sqlite_error_code() {
                    match error_code {
                        rusqlite::ErrorCode::CannotOpen => Ok(DBStatus::FileNotFound),
                        rusqlite::ErrorCode::NotADatabase => Ok(DBStatus::NotValid),
                        _ => Err(e)?,
                    }
                } else {
                    Err(e)?
                }
            }
        }
    }

    fn current_schema_version(&self, conn: &mut Connection) -> Result<Option<String>, DBError> {
        let mut stmt = conn.prepare("SELECT value FROM appData WHERE name = 'version'")?;
        let ver = stmt
            .query_row([], |row| {
                let ver: String = row.get(0)?;
                Ok(ver)
            })
            .optional()?;

        Ok(ver)
    }
}

/// Deletes all tables and recreates them
pub fn init_db(connection: &mut Connection) -> Result<(), DBError> {
    debug!("Deleting DB");
    delete_db(connection)?;
    debug!("Creating Tables");
    create_tables(connection)
}

fn _close_connection(connection: Connection) -> Result<(), DBError> {
    // debug!("Closing Database");
    Ok(connection.close().map_err(|(_conn, error)| error)?)
}

fn delete_db(connection: &mut Connection) -> Result<(), DBError> {
    let mut stmt = connection.prepare("SELECT name FROM sqlite_schema WHERE type = 'table'")?;
    let mut table_rows = stmt.query([])?;
    let mut tables = vec![];

    while let Some(row) = table_rows.next()? {
        let table_name: String = row.get(0)?;

        tables.push(table_name);
    }

    for table in tables {
        // Can't use params for tables or columns
        // so we use format!
        connection
            .execute(&format!("DROP TABLE '{}'", table), [])
            .or_else(|e| {
                if table.contains("_") {
                    // Some virtual tables ar automatically deleted
                    debug!("Error for table {}: {}", table, e);
                    Ok(0)
                } else {
                    Err(DBError::DBError(e))
                }
            })?;
    }

    connection.execute("VACUUM", [])?;
    Ok(())
}

fn create_tables(connection: &mut Connection) -> Result<(), DBError> {
    let tx = connection.transaction()?;

    tx.execute(
        "CREATE TABLE appData (
            name TEXT PRIMARY KEY,
            value TEXT
        )",
        (), // empty list of parameters.
    )?;
    tx.execute(
        "INSERT INTO appData (name, value) VALUES (?1, ?2)",
        ["version", VERSION],
    )?;

    // Storing hash as a string, as SQlite doesn't like
    // unsigned 64bit integers, alternatively we could
    // have used signed numbers by substracting the half
    // of the max value, but that looks like a worse conversion
    tx.execute(
        "CREATE TABLE notes (
            path TEXT PRIMARY KEY,
            title TEXT,
            hash TEXT,
            size INTEGER,
            modified INTEGER,
            basePath TEXT,
            noteName TEXT
        )",
        (), // empty list of parameters.
    )?;
    tx.execute(
        "CREATE VIRTUAL TABLE notesContent USING fts4(
            path,
            breadcrumb,
            text
        )",
        (), // empty list of parameters.
    )?;

    tx.commit()?;

    Ok(())
}

pub fn search_terms<S: AsRef<str>>(
    connection: &mut Connection,
    query: S,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let search_terms = SearchTerms::from_query_string(query);
    let mut var_num = 1;
    let base_sql = "SELECT notesContent.path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path";
    let mut params = vec![];
    let mut queries = vec![];
    if !search_terms.terms.is_empty() {
        let terms_sql = format!("{} WHERE notesContent MATCH ?{}", base_sql, var_num);
        queries.push(terms_sql);
        params.push(search_terms.terms.join(" "));
        var_num += 1;
    }
    if !search_terms.breadcrumb.is_empty() {
        let terms_sql = format!(
            "{} WHERE notesContent.breadcrumb MATCH ?{}",
            base_sql, var_num
        );
        queries.push(terms_sql);
        params.push(search_terms.breadcrumb.join(" "));
        var_num += 1;
    }
    if !search_terms.path.is_empty() {
        let terms_sql = format!("{} WHERE notesContent.path MATCH ?{}", base_sql, var_num);
        queries.push(terms_sql);
        params.push(search_terms.path.join(" "));
    }

    if queries.is_empty() {
        debug!("No query provided");
        return Ok(vec![]);
    }

    let sql = queries.join(" INTERSECT ");
    debug!("QUERY: {}", sql);

    let params = params_from_iter(params);
    let mut stmt = connection.prepare(&sql)?;
    let res = stmt
        .query_map(params, |row| {
            let path: String = row.get(0)?;
            let title = row.get(1)?;
            let size = row.get(2)?;
            let modified = row.get(3)?;
            let hash: String = row.get(4)?;
            let note_path = VaultPath::new(&path);
            let data = NoteEntryData {
                path: note_path.clone(),
                size,
                modified_secs: modified,
            };
            let det = NoteContentData::new(title, hash.parse().unwrap());
            Ok((data, det))
        })?
        .map(|el| el.map_err(DBError::DBError))
        .collect::<Result<Vec<(NoteEntryData, NoteContentData)>, DBError>>()?;
    Ok(res)
}

fn note_exists(connection: &mut Connection, path: &VaultPath) -> Result<bool, DBError> {
    let sql = "SELECT count(*) FROM notes where path = ?1";
    let mut stmt = connection.prepare(sql)?;
    let res = stmt.query_row([path.to_string()], |row| row.get(0))?;
    match res {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DBError::Other(format!(
            "Unexpected error, the DB contains more than one ({}) entry for path {}",
            res, path
        ))),
    }
}

pub fn get_notes(
    connection: &mut Connection,
    path: &VaultPath,
    recursive: bool,
) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
    let sql = if recursive {
        "SELECT path, title, size, modified, hash, noteName FROM notes where basePath LIKE (?1 || '%')"
    } else {
        "SELECT path, title, size, modified, hash, noteName FROM notes where basePath = ?1"
    };
    let mut stmt = connection.prepare(sql)?;
    let res = stmt
        .query_map([path.to_string()], |row| {
            let path: String = row.get(0)?;
            let title = row.get(1)?;
            let size = row.get(2)?;
            let modified = row.get(3)?;
            let hash: String = row.get(4)?;
            let note_path = VaultPath::new(&path);
            let data = NoteEntryData {
                path: note_path.clone(),
                size,
                modified_secs: modified,
            };
            let det = NoteContentData::new(title, hash.parse().unwrap());
            Ok((data, det))
        })?
        .map(|el| el.map_err(DBError::DBError))
        .collect::<Result<Vec<(NoteEntryData, NoteContentData)>, DBError>>()?;
    Ok(res)
}

pub fn insert_notes(tx: &Transaction, notes: &Vec<(NoteEntryData, String)>) -> Result<(), DBError> {
    if !notes.is_empty() {
        debug!("Inserting {} notes", notes.len());
        for (entry_data, content_data) in notes {
            insert_note(tx, entry_data, content_data)?;
        }
    }
    Ok(())
}

pub fn update_notes(tx: &Transaction, notes: &Vec<(NoteEntryData, String)>) -> Result<(), DBError> {
    if !notes.is_empty() {
        debug!("Updating {} notes", notes.len());
        for (entry_data, content_data) in notes {
            update_note(tx, entry_data, content_data)?;
        }
    }
    Ok(())
}

pub fn delete_notes(tx: &Transaction, paths: &Vec<VaultPath>) -> Result<(), DBError> {
    if !paths.is_empty() {
        for path in paths {
            delete_note(tx, path)?;
        }
    }
    Ok(())
}

pub fn save_note<S: AsRef<str>>(
    connection: &mut Connection,
    entry_data: &NoteEntryData,
    text: S,
) -> Result<(), DBError> {
    let exists = note_exists(connection, &entry_data.path)?;
    let tx = connection.transaction()?;
    if exists {
        update_note(&tx, entry_data, text)
    } else {
        insert_note(&tx, entry_data, text)
    }?;
    tx.commit()?;
    Ok(())
}

fn insert_note<S: AsRef<str>>(
    tx: &Transaction,
    entry_data: &NoteEntryData,
    text: S,
) -> Result<(), DBError> {
    let (parent_path, name) = entry_data.path.get_parent_path();
    let note_details = NoteDetails::new(&entry_data.path, text);
    if let Err(e) = tx.execute(
        "INSERT INTO notes (path, title, size, modified, hash, basePath, noteName) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![entry_data.path.to_string(), note_details.data.title, entry_data.size, entry_data.modified_secs, note_details.data.hash.to_string(), parent_path.to_string(), name],
    ){
        error!("Error inserting note: {}\nDetails: {}", e, note_details);
    }
    for chunk in &note_details.content_chunks {
        let breadcrumb = chunk.get_breadcrumb();
        let chunk_text = &chunk.text;
        tx.execute(
            "INSERT INTO notesContent (path, breadcrumb, text) VALUES (?1, ?2, ?3)",
            params![entry_data.path.to_string(), breadcrumb, chunk_text],
        )?;
    }

    Ok(())
}

fn update_note<S: AsRef<str>>(
    tx: &Transaction,
    entry_data: &NoteEntryData,
    text: S,
) -> Result<(), DBError> {
    let note_details = NoteDetails::new(&entry_data.path, text);
    let title = note_details.data.title.clone();
    let hash = note_details.data.hash.to_string();
    let path = entry_data.path.clone();
    tx.execute(
        "UPDATE notes SET title = ?2, size = ?3, modified = ?4, hash = ?5 WHERE path = ?1",
        params![
            path.to_string(),
            title,
            entry_data.size,
            entry_data.modified_secs,
            hash
        ],
    )?;
    tx.execute(
        "DELETE FROM notesContent WHERE path = ?1",
        params![path.to_string()],
    )?;
    for chunk in &note_details.content_chunks {
        let breadcrumb = chunk.get_breadcrumb();
        let chunk_text = &chunk.text;
        tx.execute(
            "INSERT INTO notesContent (path, breadcrumb, text) VALUES (?1, ?2, ?3)",
            params![entry_data.path.to_string(), breadcrumb, chunk_text],
        )?;
    }

    Ok(())
}

fn delete_note(tx: &Transaction, path: &VaultPath) -> Result<(), DBError> {
    tx.execute(
        "DELETE FROM notes WHERE path = ?1",
        params![path.to_string()],
    )?;
    tx.execute(
        "DELETE FROM notesContent WHERE path = ?1",
        params![path.to_string()],
    )?;

    Ok(())
}

pub fn delete_directories(tx: &Transaction, directories: &Vec<VaultPath>) -> Result<(), DBError> {
    if !directories.is_empty() {
        for directory in directories {
            delete_directory(tx, directory)?;
        }
    }
    Ok(())
}

fn delete_directory(tx: &Transaction, directory_path: &VaultPath) -> Result<(), DBError> {
    let path_string = directory_path.to_string();
    let sql1 = "DELETE FROM notes WHERE path LIKE (?1 || '%')";
    let sql2 = "DELETE FROM notesContent WHERE path LIKE (?1 || '%')";

    tx.execute(sql1, params![path_string])?;
    tx.execute(sql2, params![path_string])?;

    Ok(())
}

pub struct ConnectionBuilder {}

impl ConnectionBuilder {
    pub fn build<P: AsRef<Path>>(workspace_path: P) -> Result<Connection, DBError> {
        // debug!("Opening Database");
        let db_path = workspace_path.as_ref().join(DB_FILE);
        let connection = Connection::open(&db_path)?;
        let _c = connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER, true)?;
        Ok(connection)
    }
}
