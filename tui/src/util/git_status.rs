//! Workspace git-status summary for the status bar (display only — spec §12
//! keeps every other git interaction out of scope).
//!
//! This shells out to the user's `git`; it is repo metadata, not a vault
//! file operation, so it lives in the TUI rather than core.

use std::path::PathBuf;

/// Status-bar segment for the workspace's git state:
/// - `Some("git ✓")` — clean working tree
/// - `Some("git ●N")` — N changed paths
/// - `None` — `git` missing or the workspace is not a repository
pub async fn fetch(root: PathBuf) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["status", "--porcelain"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let dirty = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    Some(if dirty == 0 {
        "git ✓".to_string()
    } else {
        format!("git ●{dirty}")
    })
}
