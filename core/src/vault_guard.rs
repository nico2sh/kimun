//! Source protection on [`NoteVault`]: the one place that decides whether an
//! automated edit may change an entry in the sources folder that
//! `.kimun/wiki.toml` declares (ADR 0068). A vault built without
//! [`VaultConfig::protect_sources`](crate::VaultConfig::protect_sources) skips
//! every check, so the human editor is never bound.

use crate::error::VaultError;
use crate::nfs::VaultPath;
use crate::NoteVault;

impl NoteVault {
    /// The sources folder to protect, or `None` when protection is off or the
    /// vault has no declaration. A malformed declaration is an error, so the
    /// guards fail closed.
    async fn protected_sources(&self) -> Result<Option<VaultPath>, VaultError> {
        if !self.protect_sources {
            return Ok(None);
        }
        Ok(self.wiki_declaration().await?.map(|d| d.sources_path()))
    }

    /// Refuses `path` when it lies inside the protected sources folder.
    pub(crate) async fn ensure_not_protected(&self, path: &VaultPath) -> Result<(), VaultError> {
        match self.protected_sources().await? {
            Some(sources) => refuse_inside(&sources, path),
            None => Ok(()),
        }
    }

    /// Like [`Self::ensure_not_protected`], but only when a file already
    /// exists at `path`. Creating a new source stays allowed.
    pub(crate) async fn ensure_existing_not_protected(
        &self,
        path: &VaultPath,
    ) -> Result<(), VaultError> {
        match self.protected_sources().await? {
            Some(sources) if self.exists(path).await => refuse_inside(&sources, path),
            _ => Ok(()),
        }
    }

    /// Refuses a directory that is the protected sources folder, holds it, or
    /// lies inside it.
    pub(crate) async fn ensure_directory_not_protected(
        &self,
        dir: &VaultPath,
    ) -> Result<(), VaultError> {
        let Some(sources) = self.protected_sources().await? else {
            return Ok(());
        };
        let dir = dir.flatten();
        if dir.is_like(&sources) || sources.is_ancestor_of(&dir) {
            return Err(VaultError::SourceProtected { path: dir });
        }
        if dir.is_ancestor_of(&sources) {
            return Err(VaultError::SourceProtectedFolder { folder: dir });
        }
        Ok(())
    }

    /// Refuses a note rename when the note is a source, or when a source links
    /// to it. The rename rewrites the links in every backlinking note, so a
    /// linking source would change.
    pub(crate) async fn ensure_rename_not_protected(
        &self,
        from: &VaultPath,
    ) -> Result<(), VaultError> {
        let Some(sources) = self.protected_sources().await? else {
            return Ok(());
        };
        refuse_inside(&sources, from)?;
        for (entry, _) in self.get_backlinks(&from.flatten()).await? {
            let source = entry.path.flatten();
            if sources.is_ancestor_of(&source) {
                return Err(VaultError::SourceProtectedByLink {
                    page: from.flatten(),
                    linking_note: source,
                });
            }
        }
        Ok(())
    }
}

/// `SourceProtected` when `path` lies inside `sources`. The comparison goes
/// through `VaultPath`, so it ignores case.
fn refuse_inside(sources: &VaultPath, path: &VaultPath) -> Result<(), VaultError> {
    let path = path.flatten();
    if sources.is_ancestor_of(&path) {
        return Err(VaultError::SourceProtected { path });
    }
    Ok(())
}
