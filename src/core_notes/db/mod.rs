// pub mod async_db;

use std::path::{Path, PathBuf};

use log::{debug, error};
use rusqlite::{config::DbConfig, params, Connection, Transaction};

use super::error::DBError;

use super::{
    nfs::{DirectoryEntryData, NoteEntryData},
    NotePath,
};
use super::{DirectoryDetails, NoteDetails};

const VERSION: &str = "0.1";
const DB_FILE: &str = "notes.sqlite";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct VaultDB {
    workspace_path: PathBuf,
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
        let mut conn = ConnectionBuilder::new(&self.workspace_path).build()?;
        let res = function(&mut conn);
        conn.close().map_err(|(_conn, e)| e)?;

        res
    }
}

pub fn init_db(connection: &mut Connection) -> Result<(), DBError> {
    delete_db(connection)?;
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
        // debug!("Table to delete: {}", table_name);

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
            name VARCHAR(255) PRIMARY KEY,
            value VARCHAR(255)
        )",
        (), // empty list of parameters.
    )?;
    tx.execute(
        "INSERT INTO appData (name, value) VALUES (?1, ?2)",
        ["version", VERSION],
    )?;

    tx.execute(
        "CREATE TABLE notes (
            path VARCHAR(255) PRIMARY KEY,
            title VARCHAR(255),
            size INTEGER,
            modified INTEGER,
            hash INTEGER,
            basePath VARCHAR(255),
            noteName VARCHAR(255)
        )",
        (), // empty list of parameters.
    )?;
    tx.execute(
        "CREATE TABLE directories (
            path VARCHAR(255) PRIMARY KEY,
            basePath VARCHAR(255)
        )",
        (), // empty list of parameters.
    )?;
    tx.execute(
        "CREATE VIRTUAL TABLE notesContent USING fts4(
            path,
            content
        )",
        (), // empty list of parameters.
    )?;
    tx.execute(
        "CREATE VIRTUAL TABLE notes_terms USING fts4aux(notesContent);",
        (), // empty list of parameters.
    )?;

    tx.commit()?;

    Ok(())
}

pub fn search_terms<S: AsRef<str>>(
    connection: &mut Connection,
    terms: S,
    include_path: bool,
) -> Result<Vec<(NoteEntryData, NoteDetails)>, DBError> {
    let sql = if include_path {
        "SELECT notesContent.path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
    } else {
        "SELECT notesContent.path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE content MATCH ?1"
    };

    let mut stmt = connection.prepare(sql)?;
    let res = stmt
        .query_map([terms.as_ref()], |row| {
            let path: String = row.get(0)?;
            let title = row.get(1)?;
            let size = row.get(2)?;
            let modified = row.get(3)?;
            let hash: i64 = row.get(4)?;
            let note_path = NotePath::from(&path);
            let data = NoteEntryData {
                path: note_path.clone(),
                size,
                modified_secs: modified,
            };
            let det = NoteDetails::new(note_path, u32::try_from(hash).unwrap(), title, None);
            Ok((data, det))
        })?
        .map(|el| el.map_err(DBError::DBError))
        .collect::<Result<Vec<(NoteEntryData, NoteDetails)>, DBError>>()?;
    Ok(res)
}

pub fn get_notes(
    connection: &mut Connection,
    path: &NotePath,
    recursive: bool,
) -> Result<Vec<(NoteEntryData, NoteDetails)>, DBError> {
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
            let hash: i64 = row.get(4)?;
            let note_path = NotePath::from(&path);
            let data = NoteEntryData {
                path: note_path.clone(),
                size,
                modified_secs: modified,
            };
            let det = NoteDetails::new(note_path, u32::try_from(hash).unwrap(), title, None);
            Ok((data, det))
        })?
        .map(|el| el.map_err(DBError::DBError))
        .collect::<Result<Vec<(NoteEntryData, NoteDetails)>, DBError>>()?;
    Ok(res)
}

pub fn get_directories<P: AsRef<Path>>(
    connection: &mut Connection,
    base_path: P,
    path: &NotePath,
) -> Result<Vec<(DirectoryEntryData, DirectoryDetails)>, DBError> {
    // debug!("getting directories");
    let mut stmt = connection.prepare("SELECT path FROM directories where basePath = ?1")?;
    let res = stmt
        .query_map([path.to_string()], |row| {
            let path: String = row.get(0)?;
            let note_path = NotePath::from(&path);
            let data = DirectoryEntryData {
                path: note_path.clone(),
            };
            let det = DirectoryDetails {
                base_path: base_path.as_ref().to_path_buf(),
                path: note_path,
            };
            Ok((data, det))
        })?
        .map(|el| el.map_err(DBError::DBError))
        .collect::<Result<Vec<(DirectoryEntryData, DirectoryDetails)>, DBError>>()?;
    Ok(res)
}

pub fn insert_notes<P: AsRef<Path>>(
    tx: &Transaction,
    base_path: P,
    notes: &Vec<(NoteEntryData, NoteDetails)>,
) -> Result<(), DBError> {
    if !notes.is_empty() {
        debug!("Inserting {} notes", notes.len());
        for (data, details) in notes {
            let mut details = details.clone();
            insert_note(tx, base_path.as_ref(), data, &mut details)?;
        }
    }
    Ok(())
}

pub fn update_notes<P: AsRef<Path>>(
    tx: &Transaction,
    base_path: P,
    notes: &Vec<(NoteEntryData, NoteDetails)>,
) -> Result<(), DBError> {
    if !notes.is_empty() {
        debug!("Updating {} notes", notes.len());
        for (data, details) in notes {
            let mut details = details.clone();
            update_note(tx, base_path.as_ref(), data, &mut details)?;
        }
    }
    Ok(())
}

pub fn delete_notes(tx: &Transaction, paths: &Vec<NotePath>) -> Result<(), DBError> {
    if !paths.is_empty() {
        for path in paths {
            delete_note(tx, path)?;
        }
    }
    Ok(())
}

fn insert_note<P: AsRef<Path>>(
    tx: &Transaction,
    base_path: P,
    data: &NoteEntryData,
    details: &mut NoteDetails,
) -> Result<(), DBError> {
    let (parent_path, name) = details.path.get_parent_path();
    if let Err(e) = tx.execute(
        "INSERT INTO notes (path, title, size, modified, hash, basePath, noteName) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![details.path.to_string(), details.get_title(), data.size, data.modified_secs, details.data.hash, parent_path.to_string(), name],
    ){
        error!("Error inserting note {}", e);
    }
    tx.execute(
        "INSERT INTO notesContent (path, content) VALUES (?1, ?2)",
        params![
            details.path.to_string(),
            details.get_content(base_path).unwrap_or_default()
        ],
    )?;

    Ok(())
}

fn update_note<P: AsRef<Path>>(
    tx: &Transaction,
    base_path: P,
    data: &NoteEntryData,
    details: &mut NoteDetails,
) -> Result<(), DBError> {
    let title = details.get_title();
    let hash = details.data.hash;
    let content = details.get_content(base_path).unwrap_or_default();
    let path = details.path.clone();
    tx.execute(
        "UPDATE notes SET title = ?2, size = ?3, modified = ?4, hash = ?5 WHERE path = ?1",
        params![
            path.to_string(),
            title,
            data.size,
            data.modified_secs,
            i64::from(hash)
        ],
    )?;
    tx.execute(
        "UPDATE notesContent SET content = ?2 WHERE path = ?1",
        params![path.to_string(), content],
    )?;

    Ok(())
}

fn delete_note(tx: &Transaction, path: &NotePath) -> Result<(), DBError> {
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

pub fn delete_directories(tx: &Transaction, directories: &Vec<NotePath>) -> Result<(), DBError> {
    if !directories.is_empty() {
        for directory in directories {
            delete_directory(tx, directory)?;
        }
    }
    Ok(())
}

pub fn _insert_directories(tx: &Transaction, directories: &Vec<NotePath>) -> Result<(), DBError> {
    if !directories.is_empty() {
        for directory in directories {
            insert_directory(tx, directory)?;
        }
    }
    Ok(())
}

pub fn insert_directory(tx: &Transaction, path: &NotePath) -> Result<(), DBError> {
    tx.execute(
        "INSERT OR IGNORE INTO directories (path, basePath) VALUES (?1, ?2)",
        params![path.to_string(), path.get_parent_path().0.to_string()],
    )?;

    Ok(())
}

fn delete_directory(tx: &Transaction, directory_path: &NotePath) -> Result<(), DBError> {
    let path_string = directory_path.to_string();
    let sql1 = "DELETE FROM notes WHERE path LIKE (?1 || '%')";
    let sql2 = "DELETE FROM notesContent WHERE path LIKE (?1 || '%')";
    let sql3 = "DELETE FROM directories WHERE path LIKE (?1 || '%')";

    tx.execute(sql1, params![path_string])?;
    tx.execute(sql2, params![path_string])?;
    tx.execute(sql3, params![path_string])?;

    Ok(())
}

// pub fn execute_in_transaction(
//     connection: &mut Connection,
//     fun: Box<dyn Fn(&Transaction) -> Result<(), DBErrors>>,
// ) -> Result<(), DBErrors> {
//     let tx = connection.transaction()?;
//     fun(&tx)?;
//     tx.commit()?;
//     Ok(())
// }

// We use a builder to create connection in a thread
pub struct ConnectionBuilder {
    workspace_path: PathBuf,
}

impl ConnectionBuilder {
    pub fn new<P: AsRef<Path>>(workspace_path: P) -> Self {
        Self {
            workspace_path: workspace_path.as_ref().into(),
        }
    }

    pub fn build(&self) -> Result<Connection, DBError> {
        // debug!("Opening Database");
        let db_path = self.workspace_path.join(DB_FILE);
        let connection = Connection::open(&db_path)?;
        let _c = connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER, true)?;
        Ok(connection)
    }
}
