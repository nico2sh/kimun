//! Deleting, renaming, and moving vault entries on [`NoteVault`]: notes,
//! directories, and (through [`NoteVault::entry_kind`]) attachments. A move is
//! a rename across directories. The note rename itself is `NoteRename`'s, in
//! `note_rename.rs`.

use crate::error::VaultError;
use crate::nfs::{self, pinned_notes, VaultPath};
use crate::note_rename::{rename_dest_err, NoteRename};
use crate::{EntryKind, NoteVault};

impl NoteVault {
    /// Deletes the note at `path` (backing it up first when backups are
    /// enabled). The index row is removed before the file, so the index never
    /// points at a missing file.
    pub async fn delete_note(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        path.ensure_note()?;
        let _guard = self.lock_note(&path).await;
        self.backup_if_enabled(&path).await?;

        // Delete in the index first so it never points at a missing file.
        self.index.delete_notes(std::slice::from_ref(&path)).await?;

        nfs::delete_note(self.workspace_path(), &path).await?;

        // Best-effort: a pin-file failure never fails a delete that already
        // succeeded on disk and in the index.
        pinned_notes::on_note_deleted(self.workspace_path(), &path).await;

        Ok(())
    }

    /// Deletes the directory at `path` and its contents, removing the
    /// corresponding index rows first.
    pub async fn delete_directory(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        path.ensure_directory()?;

        self.index
            .delete_directories(std::slice::from_ref(&path))
            .await?;

        nfs::delete_directory(self.workspace_path(), &path).await?;

        // Best-effort: a pin-file failure never fails a delete that already
        // succeeded on disk and in the index.
        pinned_notes::on_directory_deleted(self.workspace_path(), &path).await;

        Ok(())
    }

    /// Renames the note `from` to `to`, rewriting links to it (wikilinks,
    /// Markdown links, and the note's own self-links) in every backlinking note
    /// so they keep pointing at the renamed note. A name link that also leads
    /// to another note of the same name is ambiguous and left alone; a
    /// rewritten one names `to` by the shortest suffix that names it alone
    /// (at least as many folders as written); a move keeping a unique name
    /// rewrites no name link. After a move, the note's own relative links
    /// are rewritten from its new folder. Fails if `to` already exists.
    /// The whole operation — locks, filesystem move, link rewrites, index
    /// commit — is `NoteRename`'s (`note_rename.rs`); see its docs for the stage order and
    /// failure atomicity.
    pub async fn rename_note(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        NoteRename::new(
            &self.index,
            self.workspace_path(),
            self.backup,
            &self.note_locks,
        )
        .rename(from, to)
        .await
    }

    /// Renames the directory `from` to `to`, updating the index paths of all
    /// notes beneath it. Fails if `to` already exists. No link text is
    /// rewritten: path links into the folder, and wikilinks naming it as a
    /// folder part, break (a known gap).
    pub async fn rename_directory(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();

        nfs::rename_directory(self.workspace_path(), &from, &to)
            .await
            .map_err(rename_dest_err)?;

        // Best-effort: a pin-file failure never fails a rename that already
        // succeeded on disk. Fine before the index call too — that step
        // cannot roll back the filesystem move either.
        pinned_notes::on_directory_renamed(self.workspace_path(), &from, &to).await;

        self.index.rename_directory(&from, &to).await?;

        Ok(())
    }

    /// Classifies the entry at `path` with a single filesystem stat: a
    /// directory, a note (`.md`), or an attachment (any other file). The one
    /// door callers use to pick the right rename/delete path for a vault entry.
    /// Errors with [`VaultError::FSError`] / `VaultPathNotFound` when nothing
    /// exists at `path`.
    pub async fn entry_kind(&self, path: &VaultPath) -> Result<EntryKind, VaultError> {
        let path = path.flatten();
        let meta = nfs::metadata_at(self.workspace_path(), &path).await?;
        Ok(nfs::classify(&meta, &path))
    }

    /// Deletes the entry at `path`, dispatching on its [`EntryKind`]: a note
    /// (index- and backup-aware), a directory (recursive, reindexed), or an
    /// attachment (plain file). The one door the UI uses to delete any entry
    /// without classifying it itself.
    pub async fn delete_entry(&self, path: &VaultPath) -> Result<(), VaultError> {
        match self.entry_kind(path).await? {
            EntryKind::Note => self.delete_note(path).await,
            EntryKind::Directory => self.delete_directory(path).await,
            EntryKind::Attachment => self.delete_attachment(path).await,
        }
    }

    /// Renames or moves the entry at `from` to `to`, dispatching on `from`'s
    /// [`EntryKind`]: a note (rewrites backlinks), a directory (reindexes the
    /// notes beneath it), or an attachment (plain file rename — references to it
    /// are not rewritten). A move is just a cross-directory rename. The one door
    /// the UI uses to rename or move any entry without classifying it itself.
    pub async fn rename_entry(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        match self.entry_kind(from).await? {
            EntryKind::Note => self.rename_note(from, to).await,
            EntryKind::Directory => self.rename_directory(from, to).await,
            EntryKind::Attachment => self.rename_attachment(from, to).await,
        }
    }
}
