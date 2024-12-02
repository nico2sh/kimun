use thiserror::Error;

#[derive(Error, Debug)]
pub enum UIError {
    #[error("Workspace Dialog closed")]
    DialogClosed,
}

#[derive(Error, Debug)]
pub enum NoteError {
    #[error("There was an error: {reason:?}")]
    ErrorWithReason { reason: String },
}

#[derive(Error, Debug)]
pub enum NoteInitError {
    #[error("Settings path not provided")]
    PathNotProvided,
    #[error("Path {path:?} doesn't exist")]
    PathNotFound { path: String },
    #[error("Path {path:?} is not a directory")]
    PathIsNotDirectory { path: String },
    #[error("DB doesn't exist at path {path:?}")]
    NoDBInPath { path: String },
    #[error("DB not valid at path {path:?}")]
    InvalidDBInPath { path: String },
    #[error("IO Error when {operation:?}")]
    IOError {
        #[source]
        source: std::io::Error,
        operation: String,
    },
    #[error("Error storing toml settings: {0}")]
    SettingsSerializationError(#[from] toml::ser::Error),
    #[error("Error reading toml settings: {0}")]
    SettingsDeserializationError(#[from] toml::de::Error),
}

#[derive(Error, Debug)]
pub enum IOErrors {
    #[error("IO Error: {0}")]
    ReadFileError(#[from] std::io::Error),
    #[error("Dir Walking Error: {0}")]
    DirWalkingFileError(#[from] ignore::Error),
    #[error("No File or Directory found at {path:?}")]
    NoFileOrDirectoryFound { path: String },
    #[error("Invalid path {path:?}")]
    InvalidPath { path: String },
    #[error("Error reading Directory: {message:?}")]
    DirectoryListError { message: String },
    #[error("Decoding Error: {0}")]
    EncodingError(#[from] std::string::FromUtf8Error),
}

#[derive(Error, Debug)]
pub enum DBErrors {
    #[error("Error reading DB: {0}")]
    DBError(#[from] rusqlite::Error),
    #[error("Error Querying Data: {0}")]
    QueryError(String),
    #[error("Error reading Filesystem: {0}")]
    NonCritical(String),
}

#[derive(Error, Debug, Clone)]
pub enum DialogErrors {
    #[error("Dialog Closed")]
    DialogClosed,
}
