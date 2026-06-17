use thiserror::Error;

use crate::nfs::VaultPath;

/// Top-level error returned at the public API edge of a vault.
///
/// Wraps the lower-level [`FSError`] (filesystem boundary) and [`DBError`]
/// (index boundary) and adds the higher-level conditions that only make sense
/// in terms of vault operations, such as note/directory collisions and
/// in-note text replacement failures.
#[derive(Error, Debug)]
pub enum VaultError {
    /// The requested vault path does not exist (e.g. opening a vault rooted at
    /// a missing location).
    #[error("Path {path} doesn't exist")]
    VaultPathNotFound {
        /// The path that was expected to exist.
        path: String,
    },
    /// A path that was required to be a directory points to something else.
    #[error("Path {path} is not a directory")]
    PathIsNotDirectory {
        /// The path that is not a directory.
        path: VaultPath,
    },
    /// An index/database operation failed; see the wrapped [`DBError`].
    #[error("DB Error: {0}")]
    DBError(#[from] DBError),
    /// A filesystem operation failed; see the wrapped [`FSError`].
    #[error("File System Error: {0}")]
    FSError(#[from] FSError),
    /// Creating a note failed because one already exists at the target path.
    #[error("Note already exists at: {path}")]
    NoteExists {
        /// The path where the note already exists.
        path: VaultPath,
    },
    /// Creating a directory failed because one already exists at the target
    /// path.
    #[error("Directory already exists at: {path}")]
    DirectoryExists {
        /// The path where the directory already exists.
        path: VaultPath,
    },
    /// A text replacement found no occurrence of the search text in the note.
    #[error("Text to replace not found in note: {path}")]
    ReplaceTextNotFound {
        /// The note in which the text was not found.
        path: VaultPath,
    },
    /// A text replacement matched more than once but was asked to replace a
    /// single occurrence.
    #[error("Text to replace is not unique in note: {path}; replace every occurrence to proceed")]
    ReplaceTextNotUnique {
        /// The note in which the text matched multiple times.
        path: VaultPath,
    },
    /// A user-supplied regular expression failed to compile.
    #[error("Invalid regular expression '{pattern}': {message}")]
    InvalidRegex {
        /// The pattern that failed to compile.
        pattern: String,
        /// The compiler's explanation of the failure.
        message: String,
    },
    /// A vault scan found paths that collide once compared case-insensitively.
    #[error("Case-sensitivity conflicts detected in vault:\n{}", conflicts.join("\n"))]
    CaseConflict {
        /// Human-readable descriptions of each detected conflict.
        conflicts: Vec<String>,
    },
    /// A spawned background task panicked or was cancelled before completing.
    #[error("Background task failed: {0}")]
    TaskJoin(String),
}

impl From<sqlx::Error> for VaultError {
    fn from(e: sqlx::Error) -> Self {
        VaultError::DBError(DBError::from(e))
    }
}

impl VaultError {
    /// `true` when the failure means the requested note/path was not found.
    pub fn is_not_found(&self) -> bool {
        match self {
            VaultError::VaultPathNotFound { .. } => true,
            VaultError::FSError(e) => e.is_not_found(),
            _ => false,
        }
    }

    /// `true` when the failure is the caller's fault and actionable — a missing
    /// or already-existing note/directory, an absent or non-unique replacement
    /// target, an invalid regex or path — rather than an internal failure (DB,
    /// raw I/O, decoding, a panicked task).
    ///
    /// Surfaces decide presentation, not classification: the MCP server returns
    /// a user error as a tool error the model can react to (an internal one as a
    /// protocol error); the CLI prints a user error as a clean message with a
    /// distinct exit code (an internal one as a full report). One place to
    /// update when a variant is added — the exhaustive match below makes the
    /// decision compiler-enforced.
    pub fn is_user_error(&self) -> bool {
        match self {
            VaultError::VaultPathNotFound { .. }
            | VaultError::PathIsNotDirectory { .. }
            | VaultError::NoteExists { .. }
            | VaultError::DirectoryExists { .. }
            | VaultError::ReplaceTextNotFound { .. }
            | VaultError::ReplaceTextNotUnique { .. }
            | VaultError::InvalidRegex { .. } => true,
            VaultError::FSError(e) => e.is_user_error(),
            VaultError::DBError(_) | VaultError::CaseConflict { .. } | VaultError::TaskJoin(_) => {
                false
            }
        }
    }
}

/// Error at the filesystem boundary, raised by the `nfs` module.
///
/// Covers raw I/O failures, path validation, and on-disk note/directory
/// existence conflicts. The vault layer translates these into the
/// higher-level [`VaultError`] before they reach the public API.
#[derive(Error, Debug)]
pub enum FSError {
    /// An underlying `std::io` operation (read, write, create, rename) failed.
    #[error("IO Error: {0}")]
    ReadFileError(#[from] std::io::Error),
    /// File contents could not be decoded as UTF-8.
    #[error("Decoding Error: {0}")]
    EncodingError(#[from] std::string::FromUtf8Error),
    /// No file or directory exists at the expected location.
    #[error("No File or Directory found at {path}")]
    NoFileOrDirectoryFound {
        /// The path that could not be found.
        path: String,
    },
    /// A path failed validation for use as a vault note or directory path.
    #[error("Invalid path {path}, {message}")]
    InvalidPath {
        /// The offending path.
        path: String,
        /// Why the path was rejected.
        message: String,
    },
    /// A vault path was resolved but the corresponding entry is missing on
    /// disk.
    #[error("Path doesn't exists at: {path}")]
    VaultPathNotFound {
        /// The vault path that does not exist on disk.
        path: VaultPath,
    },
    /// An exclusive create failed because an entry already exists at the path.
    #[error("Path already exists at: {path}")]
    AlreadyExists {
        /// The path that already exists.
        path: VaultPath,
    },
    /// Reading or writing a serialized on-disk file (e.g. saved searches)
    /// failed to (de)serialize.
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl FSError {
    /// Returns `true` if this error means the target path was not found.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            FSError::VaultPathNotFound { .. } | FSError::NoFileOrDirectoryFound { .. }
        )
    }

    /// `true` when the failure stems from the caller's path input (missing,
    /// already-existing, or invalid) rather than an internal I/O, decoding, or
    /// serialization fault.
    pub fn is_user_error(&self) -> bool {
        matches!(
            self,
            FSError::NoFileOrDirectoryFound { .. }
                | FSError::VaultPathNotFound { .. }
                | FSError::AlreadyExists { .. }
                | FSError::InvalidPath { .. }
        )
    }
}

/// Error at the index boundary, raised when interacting with the SQLite store.
///
/// Wraps `sqlx` failures and the index-specific conditions the vault layer
/// can encounter while caching and querying note metadata.
#[derive(Error, Debug)]
pub enum DBError {
    /// An underlying `sqlx` operation against the SQLite database failed.
    #[error("Database Error: {0}")]
    DBError(#[from] sqlx::Error),
    /// An operation was attempted after the database connection was closed.
    #[error("Error DB Connection Closed")]
    DBConnectionClosed,
    /// A query executed but its result could not be processed as expected.
    #[error("Error Querying Data: {0}")]
    QueryError(String),
    /// Reading the cached notes failed in a way that does not invalidate the
    /// index and can be tolerated by the caller.
    #[error("Error reading cached notes in the DB: {0}")]
    NonCritical(String),
    /// An index-related failure that does not fit the other variants (e.g.
    /// preparing the database directory).
    #[error("DB related error: {0}")]
    Other(String),
    /// Acquiring or managing a connection from the connection pool failed.
    #[error("Pool error: {0}")]
    PoolError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nfs::VaultPath;

    #[test]
    fn user_errors_are_actionable_not_internal() {
        // Caller's fault → user error.
        assert!(VaultError::NoteExists { path: VaultPath::note_path_from("a") }.is_user_error());
        assert!(VaultError::ReplaceTextNotFound { path: VaultPath::note_path_from("a") }.is_user_error());
        assert!(VaultError::ReplaceTextNotUnique { path: VaultPath::note_path_from("a") }.is_user_error());
        assert!(
            VaultError::InvalidRegex { pattern: "(".into(), message: "x".into() }.is_user_error()
        );
        assert!(VaultError::FSError(FSError::VaultPathNotFound { path: VaultPath::note_path_from("a") }).is_user_error());

        // Internal failures → not user errors.
        assert!(!VaultError::DBError(DBError::DBConnectionClosed).is_user_error());
        assert!(!VaultError::TaskJoin("boom".into()).is_user_error());
        assert!(!VaultError::FSError(FSError::EncodingError(String::from_utf8(vec![0xff]).unwrap_err())).is_user_error());
    }

    #[test]
    fn not_found_recognized_through_the_fs_layer() {
        assert!(VaultError::FSError(FSError::VaultPathNotFound { path: VaultPath::note_path_from("a") }).is_not_found());
        assert!(VaultError::FSError(FSError::NoFileOrDirectoryFound { path: "a".into() }).is_not_found());
        assert!(!VaultError::NoteExists { path: VaultPath::note_path_from("a") }.is_not_found());
    }
}
