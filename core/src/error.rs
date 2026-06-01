use thiserror::Error;

use crate::nfs::VaultPath;

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("Path {path} doesn't exist")]
    VaultPathNotFound { path: String },
    #[error("Path {path} is not a directory")]
    PathIsNotDirectory { path: VaultPath },
    #[error("DB Error: {0}")]
    DBError(#[from] DBError),
    #[error("File System Error: {0}")]
    FSError(#[from] FSError),
    #[error("Note already exists at: {path}")]
    NoteExists { path: VaultPath },
    #[error("Directory already exists at: {path}")]
    DirectoryExists { path: VaultPath },
    #[error("Text to replace not found in note: {path}")]
    ReplaceTextNotFound { path: VaultPath },
    #[error("Text to replace is not unique in note: {path}; replace every occurrence to proceed")]
    ReplaceTextNotUnique { path: VaultPath },
    #[error("Invalid regular expression '{pattern}': {message}")]
    InvalidRegex { pattern: String, message: String },
    #[error("Case-sensitivity conflicts detected in vault:\n{}", conflicts.join("\n"))]
    CaseConflict { conflicts: Vec<String> },
    #[error("Background task failed: {0}")]
    TaskJoin(String),
}

impl From<sqlx::Error> for VaultError {
    fn from(e: sqlx::Error) -> Self {
        VaultError::DBError(DBError::from(e))
    }
}

#[derive(Error, Debug)]
pub enum FSError {
    #[error("IO Error: {0}")]
    ReadFileError(#[from] std::io::Error),
    #[error("Decoding Error: {0}")]
    EncodingError(#[from] std::string::FromUtf8Error),
    #[error("No File or Directory found at {path}")]
    NoFileOrDirectoryFound { path: String },
    #[error("Invalid path {path}, {message}")]
    InvalidPath { path: String, message: String },
    #[error("Path doesn't exists at: {path}")]
    VaultPathNotFound { path: VaultPath },
    #[error("Path already exists at: {path}")]
    AlreadyExists { path: VaultPath },
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl FSError {
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            FSError::VaultPathNotFound { .. } | FSError::NoFileOrDirectoryFound { .. }
        )
    }
}

#[derive(Error, Debug)]
pub enum DBError {
    #[error("Database Error: {0}")]
    DBError(#[from] sqlx::Error),
    #[error("Error DB Connection Closed")]
    DBConnectionClosed,
    #[error("Error Querying Data: {0}")]
    QueryError(String),
    #[error("Error reading cached notes in the DB: {0}")]
    NonCritical(String),
    #[error("DB related error: {0}")]
    Other(String),
    #[error("Pool error: {0}")]
    PoolError(String),
}
