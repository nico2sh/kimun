use thiserror::Error;

use crate::nfs::VaultPath;

// use super::db::async_sqlite::Command;

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("Path {path} doesn't exist")]
    VaultPathNotFound { path: String },
    #[error("Path {path} is not a directory")]
    PathIsNotDirectory { path: VaultPath },
    #[error("DB Error: {0}")]
    DBError(#[from] DBError),
    #[error("FS Error: {0}")]
    FSError(#[from] FSError),
    #[error("Note already exists at: {path}")]
    NoteExists { path: VaultPath },
}

#[derive(Error, Debug)]
pub enum FSError {
    #[error("IO Error: {0}")]
    ReadFileError(#[from] std::io::Error),
    #[error("Decoding Error: {0}")]
    EncodingError(#[from] std::string::FromUtf8Error),
    #[error("No File or Directory found at {path}")]
    NoFileOrDirectoryFound { path: String },
    #[error("Invalid path {path}")]
    InvalidPath { path: String },
    #[error("Path doesn't exists at: {path}")]
    VaultPathNotFound { path: VaultPath },
}

#[derive(Error, Debug, PartialEq)]
pub enum DBError {
    #[error("Database Error: {0}")]
    DBError(#[from] rusqlite::Error),
    #[error("Error DB Connection Closed")]
    DBConnectionClosed,
    #[error("Error Querying Data: {0}")]
    QueryError(String),
    #[error("Error reading cached notes in the DB: {0}")]
    NonCritical(String),
    #[error("DB related error: {0}")]
    Other(String),
}
