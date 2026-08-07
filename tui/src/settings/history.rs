//! A workspace's open-file history: the notes most recently opened in it,
//! newest first.
//!
//! The counterpart of [`IndexFile`](kimun_core::IndexFile) for the other
//! per-workspace artifact — same shape (the type owns its own naming, and
//! moving or deleting a workspace goes through it), but it lives here rather
//! than in core because "which notes did I look at last" is a TUI concern.
//! Core knows nothing about it.

use std::io::{BufRead, BufReader};

use kimun_core::nfs::VaultPath;
use kimun_core::system::{self, SystemError, SystemPath};

pub const LAST_PATH_HISTORY_SIZE: usize = 50;

/// Extension of a workspace's history file.
const HISTORY_FILE_EXT: &str = "txt";

/// A workspace's history file on this machine.
///
/// Plain text, one [`VaultPath`] per line. Non-critical by design: a missing,
/// unreadable or half-written file costs the user an ordering, not a note, so
/// reads never fail — they return what they could parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryFile {
    path: SystemPath,
}

impl HistoryFile {
    /// The history for `workspace_name` inside `dir`.
    ///
    /// The naming rule lives here, so a workspace resolves to the same file
    /// from every caller — the same reason `IndexFile::in_dir` exists.
    /// `workspace_name` must already be a valid filename.
    pub fn in_dir(dir: &SystemPath, workspace_name: &str) -> Self {
        Self {
            path: dir.join(format!("{workspace_name}.{HISTORY_FILE_EXT}")),
        }
    }

    /// The file's path, for callers that need to report it.
    pub fn path(&self) -> &SystemPath {
        &self.path
    }

    /// Whether the file exists. A workspace that has never been opened has no
    /// history, which is not an error.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// The stored paths, newest first. Missing file, unreadable file or
    /// malformed lines all yield what could be read; failures are logged.
    pub fn load(&self) -> Vec<VaultPath> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                tracing::warn!("failed to open history file {}: {}", self.path, e);
                return Vec::new();
            }
        };
        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| {
                let candidate = VaultPath::new(line.trim());
                (!candidate.to_string().is_empty()).then_some(candidate)
            })
            .collect()
    }

    /// Moves `path` to the front, dropping any earlier occurrence and
    /// truncating to [`LAST_PATH_HISTORY_SIZE`]. A no-op when it is already at
    /// the front — the common case of reopening the note you are editing.
    pub fn push(&self, path: &VaultPath) -> Result<(), SystemError> {
        // Dedup with `is_like` (ignores relative/absolute form) so a note
        // reopened via different-form paths isn't stored twice; the entry is
        // stored in whatever form it arrived, to avoid rewriting existing
        // history files.
        let mut existing = self.load();
        if existing.first().is_some_and(|f| f.is_like(path)) {
            return Ok(());
        }
        existing.retain(|p| !p.is_like(path));
        existing.insert(0, path.clone());
        existing.truncate(LAST_PATH_HISTORY_SIZE);
        self.write(&existing)
    }

    /// Replaces the file's contents with `paths`.
    ///
    /// Atomic, because this rewrites the whole file on every note the user
    /// opens: a crash mid-write would otherwise leave a truncated history. The
    /// tmp-then-rename recipe itself lives in `system` — it is the same one
    /// every host-scoped writer needs, and getting it wrong is silent.
    pub fn write(&self, paths: &[VaultPath]) -> Result<(), SystemError> {
        let mut body = String::new();
        for path in paths {
            body.push_str(&path.to_string());
            body.push('\n');
        }
        system::replace_atomically(self.path.as_path(), body.as_bytes())
    }

    /// Moves this history file to `dest`, for a workspace being renamed.
    ///
    /// Refuses rather than overwrites: an existing destination aborts before
    /// anything moves. A source that does not exist is a no-op — a workspace
    /// with no history still renames.
    pub fn move_to(&self, dest: &HistoryFile) -> Result<(), SystemError> {
        if !self.exists() {
            return Ok(());
        }
        if dest.exists() {
            return Err(SystemError::AlreadyExists {
                path: dest.path.to_string(),
            });
        }
        system::move_file(self.path.as_path(), dest.path.as_path())
    }

    /// Deletes this history file. A missing file is not an error.
    pub fn remove(&self) -> Result<(), SystemError> {
        system::remove_file(self.path.as_path())
    }
}

impl std::fmt::Display for HistoryFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)
    }
}
