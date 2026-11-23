// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::system::sys;
use crate::VaultConfig;

/// A host-absolute path from a `/`-separated literal: `/tmp/ws` is
/// absolute on unix but merely rooted on Windows, where `SystemPath`
/// rejects it.
fn host(unix_style: &str) -> std::path::PathBuf {
    let trimmed = unix_style.trim_start_matches('/');
    if cfg!(windows) {
        std::path::PathBuf::from(format!("C:\\{}", trimmed.replace('/', "\\")))
    } else {
        std::path::PathBuf::from(format!("/{trimmed}"))
    }
}

#[test]
fn new_sets_workspace_and_no_index_override() {
    let cfg = VaultConfig::new(sys(host("/tmp/ws")));
    assert_eq!(cfg.workspace_path.as_path(), host("/tmp/ws"));
    assert!(cfg.index.is_none());
}

#[test]
fn with_index_overrides_the_default_location() {
    let cache_dir = sys(host("/var/cache"));
    let cfg = VaultConfig::new(sys(host("/tmp/ws")))
        .with_index(crate::IndexFile::in_dir(&cache_dir, "foo"));
    assert_eq!(
        cfg.index.map(|i| i.path().to_string()),
        Some(cache_dir.join("foo.kimuncache").to_string())
    );
}

/// The CLI's JSON output needs a note's size and mtime; it asks the vault
/// rather than stat-ing the path, because the vault is what knows where a
/// `VaultPath` really lives.
#[tokio::test]
async fn note_entry_reports_size_and_mtime() {
    use crate::{nfs::VaultPath, NoteVault, VaultConfig};
    let tmp = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(tmp.path())))
        .await
        .unwrap();
    let path = VaultPath::note_path_from("note");
    vault.create_note(&path, "# Note\n").await.unwrap();

    let entry = vault.note_entry(&path).await.unwrap();

    assert_eq!(entry.path, path);
    assert_eq!(entry.size, "# Note\n".len() as u64);
    assert!(entry.modified_secs > 0, "mtime should be populated");

    let missing = vault.note_entry(&VaultPath::note_path_from("absent")).await;
    assert!(missing.is_err(), "a missing note must not report an entry");
    vault.close().await;
}

#[tokio::test]
async fn note_vault_new_uses_vault_config_with_legacy_default() {
    use crate::{NoteVault, VaultConfig};
    let tmp = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(tmp.path())))
        .await
        .unwrap();
    let expected = tmp.path().join("kimun.sqlite");
    assert!(
        expected.exists(),
        "legacy DB path should be used when db_path is None"
    );
    drop(vault);
}

#[tokio::test]
async fn note_vault_new_with_explicit_index_uses_override() {
    use crate::{IndexFile, NoteVault, VaultConfig};
    let workspace = tempfile::TempDir::new().unwrap();
    let cache_dir = tempfile::TempDir::new().unwrap();
    let index = IndexFile::in_dir(&crate::system::sys(cache_dir.path()), "my-vault");
    let vault = NoteVault::new(
        VaultConfig::new(crate::system::sys(workspace.path())).with_index(index.clone()),
    )
    .await
    .unwrap();
    assert!(index.exists());
    assert!(!workspace.path().join("kimun.sqlite").exists());
    drop(vault);
}
