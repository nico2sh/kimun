use thiserror::Error;

// use super::db::async_sqlite::Command;

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("Path {path:?} doesn't exist")]
    PathNotFound { path: String },
    #[error("Path {path:?} is not a directory")]
    PathIsNotDirectory { path: String },
    #[error("DB Error: {0}")]
    DBError(#[from] DBError),
    #[error("IO Error: {0}")]
    ReadFileError(#[from] std::io::Error),
    #[error("Decoding Error: {0}")]
    EncodingError(#[from] std::string::FromUtf8Error),
    #[error("No File or Directory found at {path:?}")]
    NoFileOrDirectoryFound { path: String },
    #[error("Invalid path {path:?}")]
    InvalidPath { path: String },
}

#[derive(Error, Debug)]
pub enum DBError {
    #[error("Error reading DB: {0}")]
    DBError(#[from] rusqlite::Error),
    #[error("Error DB Connection Closed")]
    DBConnectionClosed,
    #[error("Error Querying Data: {0}")]
    QueryError(String),
    #[error("Error reading Filesystem: {0}")]
    NonCritical(String),
    #[error("Async Oneshot Receive Message Error: {0}")]
    AsyncCall(#[from] futures_channel::oneshot::Canceled),
    #[error("Sync Receive Message Error: {0}")]
    RcvCall(#[from] crossbeam_channel::RecvError),
    // #[error("Sync Receive Message Error: {0}")]
    // SendCall(#[from] crossbeam_channel::SendError<Command>),
    #[error("DB related error: {0}")]
    Other(String),
}
