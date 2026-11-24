//! Attachments on [`NoteVault`]: any non-note file in the vault. They are
//! never indexed and never backed up, and renaming one does not rewrite the
//! links to it.

use std::time::SystemTime;

use crate::error::VaultError;
use crate::nfs::{self, VaultPath};
use crate::note_rename::rename_dest_err;
use crate::{AttachmentContent, AttachmentDetails, NoteVault, DEFAULT_ASSETS_PATH};

impl NoteVault {
    /// Default attachments directory (e.g. `/assets`) inside the workspace.
    pub fn default_attachments_path(&self) -> VaultPath {
        VaultPath::new(DEFAULT_ASSETS_PATH)
    }

    /// Builds a candidate path for a new attachment under
    /// [`Self::default_attachments_path`], using `prefix` and `ext` plus the current
    /// unix-nanosecond timestamp for uniqueness. Nanoseconds (rather than
    /// millis) make same-instant collisions vanishingly unlikely for
    /// human-driven actions like clipboard paste.
    ///
    /// Does not check for collisions; callers that need stronger uniqueness
    /// guarantees should retry with [`Self::exists`] or use a different strategy.
    pub fn generate_attachment_path(&self, prefix: &str, ext: &str) -> VaultPath {
        let ts = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let filename = format!("{prefix}_{ts}.{ext}");
        self.default_attachments_path()
            .append(&VaultPath::new(filename))
    }

    /// Every attachment in the vault, as canonical absolute paths in no
    /// particular order. An attachment is any file that is not a note.
    /// Hidden entries are skipped. An old index file still kept inside the
    /// vault is listed too, as a browse of the vault lists it. Each call
    /// walks the vault tree afresh: nothing is cached, and the index is
    /// never read. Resolve a bare `![[name]]` image embed with it
    /// ([`crate::note::resolve_image_embed`]).
    pub fn list_attachments(&self) -> Vec<VaultPath> {
        nfs::list_attachments(self.workspace_path())
    }

    /// Writes an attachment (raw bytes — e.g. an encoded PNG) to `path` under
    /// the workspace. Creates parent directories as needed. The attachment is
    /// not added to the notes index.
    pub async fn save_attachment(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), VaultError> {
        nfs::save_attachment(self.workspace_path(), path, bytes).await?;
        Ok(())
    }

    /// Reads an attachment's [`AttachmentDetails`] — size, last-modified, file
    /// extension, and a preview of its content (the decoded text for text files,
    /// capped; nothing for binary files). The attachment is never indexed.
    pub async fn get_attachment_details(
        &self,
        path: &VaultPath,
    ) -> Result<AttachmentDetails, VaultError> {
        let path = path.flatten();
        let read = nfs::read_attachment(self.workspace_path(), &path).await?;
        let extension = attachment_extension(&path);
        let content = match read.content {
            nfs::FileText::Text { text, truncated } => AttachmentContent::Text { text, truncated },
            nfs::FileText::Binary => AttachmentContent::Binary,
        };
        Ok(AttachmentDetails {
            path,
            size: read.size,
            modified_secs: read.modified_secs,
            extension,
            content,
        })
    }

    /// Renames or moves an attachment (any non-note file). Plain filesystem
    /// rename: unlike [`Self::rename_note`], it does **not** rewrite the
    /// embed/link references to the attachment in notes, since attachments are
    /// not part of the note-link graph. Fails if `to` already exists.
    pub async fn rename_attachment(
        &self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();
        nfs::rename_attachment(self.workspace_path(), &from, &to)
            .await
            .map_err(rename_dest_err)?;
        Ok(())
    }

    /// Deletes an attachment file. No index or backup involvement — attachments
    /// are not indexed.
    pub async fn delete_attachment(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        nfs::delete_attachment(self.workspace_path(), &path).await?;
        Ok(())
    }
}

/// Lowercased file extension of an attachment path, if any (`diagram.png` →
/// `png`). `VaultPath` already lowercases its components.
fn attachment_extension(path: &VaultPath) -> Option<String> {
    let (_, name) = path.get_parent_path();
    std::path::Path::new(&name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
}
