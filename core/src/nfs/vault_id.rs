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
/// `.kimun/vault-id` when none exists yet. A malformed stored id is treated as
/// absent and replaced, so a corrupt file self-heals rather than wedging the
/// vault.
pub async fn read_or_create_vault_id(workspace_path: &Path) -> Result<VaultId, FSError> {
    let path = vault_id_path(workspace_path);
    match tokio::fs::read_to_string(&path).await {
        Ok(body) => {
            let trimmed = body.trim();
            if let Ok(uuid) = Uuid::from_str(trimmed) {
                return Ok(VaultId(uuid));
            }
            if trimmed.is_empty() {
                // Empty file means a concurrent creator is mid-write (create_new
                // makes the file before write_all fills it). Don't overwrite —
                // that would reopen the split-id race. Go through the exclusive
                // path, which reads the winner's id back.
                return create_vault_id_exclusive(&path).await;
            }
            // Non-empty but unparseable — genuinely corrupt.
            heal_corrupt_vault_id(&path).await
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            create_vault_id_exclusive(&path).await
        }
        Err(e) => Err(FSError::ReadFileError(e)),
    }
}

/// Heals a corrupt (non-empty, unparseable) vault-id file without reopening
/// the split-id race: an atomic `rename` moves the corrupt file aside, and —
/// because a rename succeeds for exactly one process — elects a single healer.
/// Winner and losers alike then converge through the exclusive-create path,
/// whose `create_new` can only succeed once, so every concurrent caller ends
/// up with the same id. The corrupt content is kept as `vault-id.corrupt` for
/// inspection instead of being silently overwritten.
async fn heal_corrupt_vault_id(path: &Path) -> Result<VaultId, FSError> {
    let backup = path.with_extension("corrupt");
    // Clear any stale backup first: Windows' rename refuses to replace an
    // existing destination (harmlessly racy — the backup is best-effort).
    let _ = tokio::fs::remove_file(&backup).await;
    match tokio::fs::rename(path, &backup).await {
        // We won the election and moved the corrupt file aside…
        Ok(()) => {}
        // …or another healer got there first; either way the id now comes
        // from the exclusive create below.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(FSError::ReadFileError(e)),
    }
    create_vault_id_exclusive(path).await
}

/// Creates the vault-id file exclusively so two concurrent first-openers can't
/// each persist a different id (a split id would orphan a RAG collection).
/// Whoever wins the create writes theirs; anyone who loses the race reads the
/// winner's id back.
async fn create_vault_id_exclusive(path: &Path) -> Result<VaultId, FSError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let id = VaultId::new_random();
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
    {
        Ok(mut file) => {
            use tokio::io::AsyncWriteExt;
            file.write_all(id.to_string().as_bytes()).await?;
            file.flush().await?;
            Ok(id)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Lost the create race. Read the winner's id back, tolerating a
            // brief window where the file exists but the winner has not written
            // yet (empty).
            for attempt in 0..50 {
                let body = tokio::fs::read_to_string(path).await?;
                let trimmed = body.trim();
                if let Ok(uuid) = Uuid::from_str(trimmed) {
                    return Ok(VaultId(uuid));
                }
                if !trimmed.is_empty() {
                    break; // non-empty & invalid → genuinely corrupt
                }
                if attempt < 49 {
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
            }
            Err(FSError::SerializationError(format!(
                "invalid vault id at {path:?}"
            )))
        }
        Err(e) => Err(FSError::ReadFileError(e)),
    }
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
