//! Vault ID: a stable identifier persisted in the vault under
//! `.kimun/vault-id`, so it travels with the notes and survives renames and
//! moves. It keys the vault's collection on the RAG server (see adr/0020).
//! All filesystem access lives here per the project rule that fs ops belong
//! in `nfs`.

use std::path::Path;
use std::str::FromStr;

use uuid::Uuid;

use crate::error::FSError;

/// A vault's stable identity — a UUID generated once and kept in `.kimun/`.
/// Opaque to callers; its only jobs are to compare equal across reopens and to
/// serialize to/from the on-disk string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VaultId(Uuid);

impl VaultId {
    /// Generates a fresh random Vault ID.
    pub fn new_random() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for VaultId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hyphenated form, the canonical on-disk and on-the-wire representation.
        write!(f, "{}", self.0)
    }
}

fn vault_id_path(workspace_path: &Path) -> std::path::PathBuf {
    workspace_path.join(".kimun").join("vault-id")
}

/// Reads the vault's [`VaultId`], creating and persisting a fresh one under
/// `.kimun/vault-id` when none exists yet. A malformed or orphaned-empty
/// stored id is replaced, so the file self-heals rather than wedging the
/// vault.
pub async fn read_or_create_vault_id(workspace_path: &Path) -> Result<VaultId, FSError> {
    let path = vault_id_path(workspace_path);
    // Fast path, no locking: [`settle_vault_id`] publishes atomically (temp
    // file + rename), so a reader sees either the previous or the new
    // complete content — never a partial write.
    if let Ok(body) = tokio::fs::read_to_string(&path).await {
        if let Ok(uuid) = Uuid::from_str(body.trim()) {
            return Ok(VaultId(uuid));
        }
    }
    // Missing, empty, or corrupt — settle it on a blocking thread (std fs +
    // a blocking OS lock).
    tokio::task::spawn_blocking(move || settle_vault_id(&path))
        .await
        .map_err(|e| FSError::ReadFileError(std::io::Error::other(e)))?
}

/// Decides the vault id when the file is missing, empty, or corrupt.
///
/// Every mutation is serialized on an OS file lock (`vault-id.lock`, held for
/// the duration of this function and released by the OS even if the holder
/// crashes), so exactly one process settles the id per contention epoch and
/// everyone else re-reads that id under the same lock. A lock-free
/// rename/create election was tried first and abandoned: any process that
/// once observed corrupt content could re-run the election later and evict a
/// freshly-healed valid id, splitting the vault across two server collections
/// (adr/0020). A held lock has no such window, and lock staleness cannot
/// occur because the OS drops it with the process.
fn settle_vault_id(path: &Path) -> Result<VaultId, FSError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false) // the file is only a lock anchor; content irrelevant
        .write(true)
        .open(path.with_extension("lock"))?;
    lock_file.lock()?; // held until `lock_file` drops

    // Re-read under the lock: whoever held it before us may already have
    // settled the id.
    match std::fs::read_to_string(path) {
        Ok(body) => {
            let trimmed = body.trim();
            if let Ok(uuid) = Uuid::from_str(trimmed) {
                return Ok(VaultId(uuid));
            }
            if trimmed.is_empty() {
                // Every writer holds this lock, so an empty file seen here is
                // an orphan from a crash between create and write — not a
                // live writer mid-write. Clear it so the publish below works
                // on every platform (Windows' rename won't replace).
                std::fs::remove_file(path)?;
            } else {
                // Corrupt: keep the evidence as `vault-id.corrupt` instead of
                // silently destroying it.
                let backup = path.with_extension("corrupt");
                let _ = std::fs::remove_file(&backup);
                std::fs::rename(path, &backup)?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(FSError::ReadFileError(e)),
    }

    // The path is vacant; publish a fresh id atomically so lock-free
    // fast-path readers can never observe a partial write.
    let id = VaultId::new_random();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, id.to_string())?;
    std::fs::rename(&tmp, path)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_and_persists_on_first_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let id = read_or_create_vault_id(dir.path()).await.unwrap();
        assert!(dir.path().join(".kimun").join("vault-id").exists());
        // Second read returns the same id from disk.
        let again = read_or_create_vault_id(dir.path()).await.unwrap();
        assert_eq!(id, again);
    }

    #[tokio::test]
    async fn corrupt_file_self_heals() {
        let dir = tempfile::TempDir::new().unwrap();
        let kimun = dir.path().join(".kimun");
        tokio::fs::create_dir_all(&kimun).await.unwrap();
        tokio::fs::write(kimun.join("vault-id"), "not-a-uuid")
            .await
            .unwrap();
        let id = read_or_create_vault_id(dir.path()).await.unwrap();
        // Rewritten to a valid id, stable thereafter.
        let again = read_or_create_vault_id(dir.path()).await.unwrap();
        assert_eq!(id, again);
        // The corrupt content was moved aside, not silently destroyed.
        let backup = tokio::fs::read_to_string(kimun.join("vault-id.corrupt"))
            .await
            .unwrap();
        assert_eq!(backup, "not-a-uuid");
    }

    #[tokio::test]
    async fn orphaned_empty_file_self_heals() {
        // A creator crashing between create_new and write_all leaves a
        // zero-byte file; with nobody left to fill it, the empty-file wait
        // must fall through to the heal election instead of wedging forever.
        let dir = tempfile::TempDir::new().unwrap();
        let kimun = dir.path().join(".kimun");
        tokio::fs::create_dir_all(&kimun).await.unwrap();
        tokio::fs::write(kimun.join("vault-id"), "").await.unwrap();

        let id = read_or_create_vault_id(dir.path()).await.unwrap();
        let again = read_or_create_vault_id(dir.path()).await.unwrap();
        assert_eq!(id, again);
    }

    #[tokio::test]
    async fn straggler_settle_never_discards_a_valid_id() {
        // The straggler scenario: a process that observed corrupt/missing
        // content reaches the settle path AFTER another process already wrote
        // a valid id. The under-lock re-read must adopt that id, not mint a
        // fresh one (which would split the vault across two collections).
        let dir = tempfile::TempDir::new().unwrap();
        let settled = read_or_create_vault_id(dir.path()).await.unwrap();

        let path = vault_id_path(dir.path());
        let straggler = settle_vault_id(&path).unwrap();
        assert_eq!(straggler, settled, "settle must adopt the existing id");
        // And the id on disk is unchanged.
        assert_eq!(read_or_create_vault_id(dir.path()).await.unwrap(), settled);
    }

    #[tokio::test]
    async fn concurrent_heals_of_a_corrupt_file_converge_on_one_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let kimun = dir.path().join(".kimun");
        tokio::fs::create_dir_all(&kimun).await.unwrap();
        tokio::fs::write(kimun.join("vault-id"), "not-a-uuid")
            .await
            .unwrap();

        // Concurrent readers of the same corrupt file must all converge on a
        // single healed id — the rename election guarantees one healer, the
        // exclusive create guarantees one writer.
        let workspace = dir.path().to_path_buf();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let workspace = workspace.clone();
                tokio::spawn(async move { read_or_create_vault_id(&workspace).await.unwrap() })
            })
            .collect();
        let mut ids = Vec::new();
        for handle in handles {
            ids.push(handle.await.unwrap());
        }
        assert!(
            ids.windows(2).all(|w| w[0] == w[1]),
            "all concurrent readers must get the same id: {ids:?}"
        );
        // And the winner's id is what persists on disk.
        let on_disk = read_or_create_vault_id(&workspace).await.unwrap();
        assert_eq!(on_disk, ids[0]);
    }
}
