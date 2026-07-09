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
            if let Ok(uuid) = Uuid::from_str(body.trim()) {
                return Ok(VaultId(uuid));
            }
            // Corrupt/empty file — fall through and rewrite a fresh id.
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(FSError::ReadFileError(e)),
    }

    let id = VaultId::new_random();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, id.to_string()).await?;
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
    }
}
