//! Per-workspace open-file history.
//!
//! Atomic writes (write to .tmp then rename) avoid partial writes
//! corrupting the file on crash mid-edit.

use std::io::{BufRead, BufReader};
use std::path::Path;

use kimun_core::nfs::VaultPath;
use kimun_core::system::{self, SystemPath};

pub const LAST_PATH_HISTORY_SIZE: usize = 50;

/// Load history from `path`. Missing file → empty. Malformed lines skipped.
/// Never returns an error: history is non-critical and IO failures are logged.
pub fn load_history(path: &Path) -> Vec<VaultPath> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!("failed to open history file {:?}: {}", path, e);
            return Vec::new();
        }
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let p = VaultPath::new(trimmed);
        if !p.to_string().is_empty() {
            out.push(p);
        }
    }
    out
}

/// Push `path` to the front of the history at `file_path`. Dedups, truncates
/// to LAST_PATH_HISTORY_SIZE, atomic write. No-op if `path` is already at the
/// front (common case: re-opening the same note).
pub fn push_history(file_path: &SystemPath, path: &VaultPath) -> Result<(), system::SystemError> {
    // Dedup with `is_like` (ignores relative/absolute form) so a note
    // reopened via different-form paths isn't stored twice; the entry is stored
    // in whatever form it arrived, to avoid rewriting existing history files.
    let mut existing = load_history(file_path.as_path());
    if existing.first().is_some_and(|f| f.is_like(path)) {
        return Ok(());
    }
    existing.retain(|p| !p.is_like(path));
    existing.insert(0, path.clone());
    if existing.len() > LAST_PATH_HISTORY_SIZE {
        existing.truncate(LAST_PATH_HISTORY_SIZE);
    }
    write_atomic(file_path, &existing)
}

fn write_atomic(file_path: &SystemPath, paths: &[VaultPath]) -> Result<(), system::SystemError> {
    let mut body = String::new();
    for p in paths {
        body.push_str(&p.to_string());
        body.push('\n');
    }
    // The tmp-then-rename dance itself lives in `system`: it is the same
    // recipe every host-scoped writer needs, and getting it wrong is silent.
    system::replace_atomically(file_path.as_path(), body.as_bytes())
}
