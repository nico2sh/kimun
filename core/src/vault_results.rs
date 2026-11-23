//! What [`NoteVault`](crate::NoteVault) methods return: the replace preview,
//! link resolution, browse entries, and attachment details.

use crate::nfs::{NoteEntryData, VaultPath};
use crate::note::NoteContentData;

/// Result of a dry-run replace ([`NoteVault::preview_replace`](crate::NoteVault::preview_replace)): how many matches
/// would be replaced, and the note's content after the replacement. Nothing is
/// written to disk.
#[derive(Debug, Clone)]
pub struct ReplacePreview {
    /// Number of matches that would be replaced.
    pub count: usize,
    /// The note's content after the replacement.
    pub content: String,
}

/// Where a note link leads ([`NoteVault::resolve_note_target`](crate::NoteVault::resolve_note_target)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    /// Exactly one note matches: its index entry and content data.
    Note(NoteEntryData, NoteContentData),
    /// A name several notes match, in index order.
    Ambiguous(Vec<(NoteEntryData, NoteContentData)>),
    /// No note matches: the canonical path a new note for the link would get.
    Missing(VaultPath),
}

/// A directory entry within the vault.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirectoryDetails {
    /// Path of the directory inside the vault.
    pub path: VaultPath,
}

/// A single entry produced by browsing the vault: a note, directory, or
/// attachment, identified by its path.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// Path of the entry inside the vault.
    pub path: VaultPath,
    /// What kind of entry this is, plus any content data for notes.
    pub rtype: ResultType,
}

impl SearchResult {
    /// Builds a note result carrying its content data.
    pub fn note(path: &VaultPath, content_data: &NoteContentData) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Note(content_data.to_owned()),
        }
    }
    /// Builds a directory result.
    pub fn directory(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Directory,
        }
    }
    /// Builds an attachment result.
    pub fn attachment(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Attachment,
        }
    }
}

/// Kind of a [`SearchResult`].
#[derive(Debug, Clone, PartialEq)]
pub enum ResultType {
    /// A note, with its parsed content data.
    Note(NoteContentData),
    /// A directory.
    Directory,
    /// An attachment (non-note file).
    Attachment,
}

/// Read-only details of an attachment, for the attachment view: its identity,
/// size, last-modified time, extension, and previewable content.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentDetails {
    /// The attachment's vault path.
    pub path: VaultPath,
    /// File size in bytes (the true on-disk size, even when the preview is
    /// truncated).
    pub size: u64,
    /// Last-modified time, in whole seconds since the Unix epoch.
    pub modified_secs: u64,
    /// Lowercased file extension, if any (`png`, `pdf`, …; `None` for
    /// extension-less files like `LICENSE`).
    pub extension: Option<String>,
    /// The previewable content.
    pub content: AttachmentContent,
}

/// The previewable content of an attachment.
#[derive(Debug, Clone, PartialEq)]
pub enum AttachmentContent {
    /// A text file, decoded as UTF-8. `truncated` is `true` when the file
    /// exceeded the preview cap and `text` is a prefix.
    Text {
        /// The decoded text (capped).
        text: String,
        /// Whether the file was longer than the preview cap.
        truncated: bool,
    },
    /// A binary (non-text) file — no preview is offered.
    Binary,
}
