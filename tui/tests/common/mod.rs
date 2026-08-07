// tui/tests/common/mod.rs
//
// Helpers shared by the integration tests. Every test binary compiles this
// module separately and uses a different subset of it, hence the blanket
// dead-code allow.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use kimun_core::{NoteVault, VaultConfig};

/// Where test vaults keep their index, instead of the default
/// `<workspace>/kimun.sqlite`.
///
/// Two reasons, both about not colliding with the code under test: loading a
/// Phase 1 config migrates `kimun.sqlite` into the cache dir, and Windows
/// refuses to move a file whose handle is still open — which a live test
/// vault's connection pool holds. The leading dot also keeps the file out of
/// the vault walk.
pub const TEST_INDEX_FILE: &str = ".test-index.kimuncache";

/// [`VaultConfig`] for a test vault rooted at `dir`, indexed at
/// [`TEST_INDEX_FILE`].
pub fn test_vault_config(dir: &Path) -> VaultConfig {
    VaultConfig::new(dir).with_db_path(dir.join(TEST_INDEX_FILE))
}

/// Opens a test vault at `dir` with its database initialised.
///
/// Close it (`vault.close().await`) before the [`TempDir`] drops. The pool
/// holds the index file open — plus its `-wal`/`-shm` sidecars — and dropping
/// a pool only *schedules* the close; Windows then cannot `remove_dir_all` the
/// directory, an error `tempfile` swallows, so the leaked temp directory is
/// silent.
///
/// [`TempDir`]: tempfile::TempDir
pub async fn open_test_vault(dir: &Path) -> NoteVault {
    let vault = NoteVault::new(test_vault_config(dir))
        .await
        .expect("failed to create vault");
    vault
        .validate_and_init()
        .await
        .expect("failed to init vault");
    vault
}

/// Builds a genuinely absolute path for the host from `/`-separated
/// components.
///
/// A literal like `"/nonexistent/path"` is absolute on Unix but merely
/// *rooted* on Windows, where [`Path::is_absolute`] also wants a prefix
/// (`C:\`). Settings treat a rooted-but-prefixless path as relative and rebase
/// it onto the config directory, so a test written with the literal stops
/// testing what its name says.
///
/// [`Path::is_absolute`]: std::path::Path::is_absolute
pub fn absolute(unix_style: &str) -> PathBuf {
    let trimmed = unix_style.trim_start_matches('/');
    if cfg!(windows) {
        PathBuf::from(format!("C:\\{}", trimmed.replace('/', "\\")))
    } else {
        PathBuf::from(format!("/{trimmed}"))
    }
}

/// [`absolute`] as the contents of a TOML string literal — Windows separators
/// have to survive TOML's own escaping.
pub fn absolute_toml(unix_style: &str) -> String {
    absolute(unix_style).to_string_lossy().replace('\\', "\\\\")
}
