//! Per-workspace open-file history.
//!
//! Stored as plain UTF-8 text, one vault path per line, newest first.
//! Atomic writes (write to .tmp then rename) avoid partial writes
//! corrupting the file on crash mid-edit.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use kimun_core::nfs::VaultPath;

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

/// Push `path` to the front of the history at `file_path`. Dedups existing
/// occurrences. Truncates to LAST_PATH_HISTORY_SIZE. Atomic write.
/// Creates parent dir if missing.
pub fn push_history(file_path: &Path, path: &VaultPath) -> std::io::Result<()> {
    let mut existing = load_history(file_path);
    existing.retain(|p| p != path);
    existing.insert(0, path.clone());
    if existing.len() > LAST_PATH_HISTORY_SIZE {
        existing.truncate(LAST_PATH_HISTORY_SIZE);
    }
    write_atomic(file_path, &existing)
}

fn write_atomic(file_path: &Path, paths: &[VaultPath]) -> std::io::Result<()> {
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file_path.with_extension("txt.tmp");
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        for p in paths {
            writeln!(f, "{}", p)?;
        }
        f.sync_all()
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, file_path)
}
