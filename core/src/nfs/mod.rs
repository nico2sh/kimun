mod backup;
pub mod filename;
pub mod saved_searches;
pub mod vault_id;
mod vault_path;
use std::{
    fmt::Display,
    hash::Hash,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use ignore::{WalkBuilder, WalkParallel};
use twox_hash::XxHash64;

use super::{error::FSError, DirectoryDetails, NoteDetails};

use super::utilities::path_to_string;

pub(crate) use backup::backup_note;
use backup::rename_path;
pub use vault_path::{with_note_extension, VaultPath, PATH_SEPARATOR};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VaultEntry {
    pub path: VaultPath,
    pub path_string: String,
    pub data: EntryData,
}

impl AsRef<str> for VaultEntry {
    fn as_ref(&self) -> &str {
        self.path_string.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum EntryData {
    Note(NoteEntryData),
    Directory(DirectoryEntryData),
    Attachment,
}

/// The kind of entry at a vault path, resolved by a filesystem stat. The single
/// discriminator callers switch on to choose the matching file operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A `.md` note.
    Note,
    /// A directory.
    Directory,
    /// Any other (non-note) file.
    Attachment,
}

/// The one classification rule for a vault entry: a directory, a `.md` note, or
/// any other file (an attachment). Shared by the index walk ([`VaultEntry`])
/// and the public `entry_kind` door so the two can never disagree on what a
/// path is.
pub(crate) fn classify(metadata: &std::fs::Metadata, path: &VaultPath) -> EntryKind {
    if metadata.is_dir() {
        EntryKind::Directory
    } else if path.is_note() {
        EntryKind::Note
    } else {
        EntryKind::Attachment
    }
}

/// Lightweight metadata for an indexed note: enough to detect changes without
/// reading the note's contents. Produced from filesystem metadata as the vault
/// is walked.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct NoteEntryData {
    /// The note's vault path, stored flattened (no `.`/`..` components).
    pub path: VaultPath,
    /// File size in bytes. Cheap first-pass signal that a note changed.
    pub size: u64,
    /// Last-modified time, in whole seconds since the Unix epoch.
    pub modified_secs: u64,
}

impl NoteEntryData {
    #[cfg(test)]
    pub async fn load_details<P: AsRef<Path>>(
        &self,
        workspace_path: P,
        path: &VaultPath,
    ) -> Result<NoteDetails, FSError> {
        let content = load_note(workspace_path, path).await?;
        Ok(NoteDetails::new(path, content))
    }

    /// Reads the file at `os_path` directly (no case-insensitive resolution).
    /// Use when the real on-disk path is already known (e.g. from the walker).
    pub(crate) fn load_details_from_os_path(&self, os_path: &Path) -> Result<NoteDetails, FSError> {
        let bytes = std::fs::read(os_path)?;
        let text = String::from_utf8(bytes)?;
        Ok(NoteDetails::new(&self.path, text))
    }

    async fn from_os_path(path: &VaultPath, file_path: &Path) -> Result<NoteEntryData, FSError> {
        let metadata = tokio::fs::metadata(file_path).await?;
        Ok(Self::from_metadata(path, &metadata))
    }

    fn from_metadata(path: &VaultPath, metadata: &std::fs::Metadata) -> NoteEntryData {
        let (size, modified_secs) = size_and_mtime(metadata);
        NoteEntryData {
            path: path.flatten(),
            size,
            modified_secs,
        }
    }
}

/// Extracts `(size_bytes, modified_unix_secs)` from filesystem metadata. A
/// missing or pre-epoch mtime yields `0` (rather than panicking). Shared by
/// note and attachment reads so the two never drift on how size/mtime are read.
fn size_and_mtime(metadata: &std::fs::Metadata) -> (u64, u64) {
    let modified_secs = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (metadata.len(), modified_secs)
}

/// Metadata for an indexed directory. A directory carries no content of its
/// own, so its vault path is all that needs tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectoryEntryData {
    /// The directory's vault path.
    pub path: VaultPath,
}
impl DirectoryEntryData {
    /// Builds the public [`DirectoryDetails`] view of this directory entry.
    pub fn get_details<P: AsRef<Path>>(&self) -> DirectoryDetails {
        DirectoryDetails {
            path: self.path.clone(),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) enum VaultEntryDetails {
    Note(NoteDetails),
    #[allow(dead_code)]
    Directory(DirectoryDetails),
    None,
}

#[cfg(test)]
impl VaultEntryDetails {
    pub fn get_title(&mut self) -> String {
        match self {
            VaultEntryDetails::Note(note_details) => note_details.get_title(),
            VaultEntryDetails::Directory(_) => String::new(),
            VaultEntryDetails::None => String::new(),
        }
    }
}

impl VaultEntry {
    #[cfg(test)]
    pub async fn new<P: AsRef<Path>>(workspace_path: P, path: VaultPath) -> Result<Self, FSError> {
        let os_path = resolve_path_on_disk(&workspace_path, &path).await;
        let metadata = tokio::fs::metadata(&os_path)
            .await
            .map_err(|e| Self::map_metadata_err(e, &os_path))?;
        Self::assemble(path, &metadata)
    }

    #[cfg(test)]
    pub async fn from_path<P: AsRef<Path>, F: AsRef<Path>>(
        workspace_path: P,
        full_path: F,
    ) -> Result<Self, FSError> {
        let note_path = VaultPath::from_path(&workspace_path, &full_path)?;
        let os_path = full_path.as_ref();
        let metadata = tokio::fs::metadata(os_path)
            .await
            .map_err(|e| Self::map_metadata_err(e, os_path))?;
        Self::assemble(note_path, &metadata)
    }

    /// Sync sibling of `from_path`. Used by the parallel-walker visitor where
    /// the OS path is already known and the calling thread is synchronous.
    pub(crate) fn from_path_sync<P: AsRef<Path>, F: AsRef<Path>>(
        workspace_path: P,
        full_path: F,
    ) -> Result<Self, FSError> {
        let note_path = VaultPath::from_path(&workspace_path, &full_path)?;
        let os_path = full_path.as_ref();
        let metadata =
            std::fs::metadata(os_path).map_err(|e| Self::map_metadata_err(e, os_path))?;
        Self::assemble(note_path, &metadata)
    }

    fn map_metadata_err(e: std::io::Error, os_path: &Path) -> FSError {
        match e.kind() {
            std::io::ErrorKind::NotFound => FSError::NoFileOrDirectoryFound {
                path: path_to_string(os_path),
            },
            _ => FSError::ReadFileError(e),
        }
    }

    fn assemble(path: VaultPath, metadata: &std::fs::Metadata) -> Result<Self, FSError> {
        let data = match classify(metadata, &path) {
            EntryKind::Directory => EntryData::Directory(DirectoryEntryData { path: path.clone() }),
            EntryKind::Note => EntryData::Note(NoteEntryData::from_metadata(&path, metadata)),
            EntryKind::Attachment => EntryData::Attachment,
        };
        let path_string = path.to_string();
        Ok(VaultEntry {
            path,
            path_string,
            data,
        })
    }
}

impl Display for VaultEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.data {
            EntryData::Note(_details) => write!(f, "[NOT] {}", self.path),
            EntryData::Directory(_details) => write!(f, "[DIR] {}", self.path),
            EntryData::Attachment => write!(f, "[ATT]"),
        }
    }
}

pub(crate) fn hash_text<S: AsRef<str>>(text: S) -> u64 {
    XxHash64::oneshot(42, text.as_ref().as_bytes())
}

/// Resolves a VaultPath to the real PathBuf on disk by matching each component
/// case-insensitively. When a component doesn't exist on disk yet, the stored
/// (lowercase) name is used for the remainder of the path.
///
/// Fast path: stored paths are always lowercase, so `vault_path.to_pathbuf` is
/// the canonical form. We try it directly first; only fall back to the
/// per-slice walk when something exists on disk under a different case
/// (legacy mixed-case files imported from outside Kimun).
pub(crate) async fn resolve_path_on_disk<P: AsRef<Path>>(
    workspace_path: P,
    vault_path: &VaultPath,
) -> PathBuf {
    let canonical = vault_path.to_pathbuf(&workspace_path);
    if matches!(tokio::fs::try_exists(&canonical).await, Ok(true)) {
        return canonical;
    }
    let mut current = workspace_path.as_ref().to_path_buf();
    for slice in &vault_path.flatten().slices {
        let name = slice.to_string();
        let real_name = async {
            let mut entries = tokio::fs::read_dir(&current).await.ok()?;
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_name().to_string_lossy().to_lowercase() == name {
                    return Some(entry.file_name().to_string_lossy().into_owned());
                }
            }
            None
        }
        .await
        .unwrap_or(name);
        current = current.join(real_name);
    }
    current
}

/// Sync variant of `resolve_path_on_disk` for use in non-async contexts.
pub(crate) fn resolve_path_on_disk_sync<P: AsRef<Path>>(
    workspace_path: P,
    vault_path: &VaultPath,
) -> PathBuf {
    let canonical = vault_path.to_pathbuf(&workspace_path);
    if canonical.exists() {
        return canonical;
    }
    let mut current = workspace_path.as_ref().to_path_buf();
    for slice in &vault_path.flatten().slices {
        let name = slice.to_string();
        let real_name = std::fs::read_dir(&current)
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .find(|e| e.file_name().to_string_lossy().to_lowercase() == name)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
            })
            .unwrap_or(name);
        current = current.join(real_name);
    }
    current
}

/// Walks the vault directory tree and returns a human-readable description of
/// every pair of entries that collide when lowercased (e.g. "note.md" vs "Note.md").
/// Returns an empty Vec if the vault is clean.
pub(crate) fn check_case_conflicts<P: AsRef<Path>>(workspace_path: P) -> Vec<String> {
    let root = workspace_path.as_ref();
    check_conflicts_in_dir(root, root)
}

fn check_conflicts_in_dir(workspace_root: &Path, dir: &Path) -> Vec<String> {
    let mut conflicts = Vec::new();
    let mut seen: std::collections::HashMap<String, std::ffi::OsString> =
        std::collections::HashMap::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return conflicts,
    };

    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        // skip hidden entries, consistent with the vault's filter_files behaviour
        if name_str.starts_with('.') {
            continue;
        }
        let lower = name_str.to_lowercase();
        if let Some(existing) = seen.get(&lower) {
            let rel = dir.strip_prefix(workspace_root).unwrap_or(dir);
            let rel_str = rel.to_string_lossy();
            let location = if rel_str.is_empty() {
                PATH_SEPARATOR.to_string()
            } else {
                format!("{}{}", PATH_SEPARATOR, rel_str)
            };
            conflicts.push(format!(
                "\"{}\" conflicts with \"{}\" in {}",
                name_str,
                existing.to_string_lossy(),
                location
            ));
        } else {
            seen.insert(lower, name);
        }
        // Use file_type() rather than is_dir() to avoid following symlinks,
        // which could cause unbounded recursion on symlink loops.
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                subdirs.push(entry.path());
            }
        }
    }

    // Recurse into all subdirectories, including both sides of a conflicting pair,
    // so that deeper conflicts inside them are also surfaced.
    for subdir in subdirs {
        conflicts.extend(check_conflicts_in_dir(workspace_root, &subdir));
    }

    conflicts
}

/// Loads a note from disk, if the file doesn't exist, returns a FSError::NotePathNotFound
/// Returns the note's text. If you want the details, use NoteDetails::from_content
pub(crate) async fn load_note<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<String, FSError> {
    let os_path = resolve_path_on_disk(&workspace_path, path).await;
    match tokio::fs::read(&os_path).await {
        Ok(file) => {
            let text = String::from_utf8(file)?;
            Ok(text)
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => Err(FSError::VaultPathNotFound {
                path: path.to_owned(),
            }),
            _ => Err(FSError::ReadFileError(e)),
        },
    }
}

/// Creates a new directory at `path`. Returns `FSError::AlreadyExists` if the
/// directory (or any case-insensitive variant) is already present.
pub(crate) async fn create_directory<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<DirectoryEntryData, FSError> {
    path.ensure_directory()?;

    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match tokio::fs::create_dir(&full_path).await {
        Ok(()) => Ok(DirectoryEntryData {
            path: path.to_owned(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(FSError::AlreadyExists {
            path: path.to_owned(),
        }),
        Err(e) => Err(FSError::ReadFileError(e)),
    }
}

/// Writes raw bytes (e.g. an image attachment) at `path` under the workspace,
/// creating parent directories as needed. Unlike [`save_note`], does not require
/// the path to be a note file and bypasses the case-insensitive note resolver.
pub(crate) async fn save_attachment<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
    bytes: &[u8],
) -> Result<(), FSError> {
    let full_path = path.flatten().to_pathbuf(workspace_path);
    if let Some(parent) = full_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&full_path, bytes).await?;
    Ok(())
}

/// Largest text-attachment preview read into memory. Files past this are
/// truncated to a prefix; binary detection still only ever reads this far, so a
/// multi-GB file can never blow the heap.
pub(crate) const ATTACHMENT_PREVIEW_CAP: usize = 10 * 1024 * 1024;

/// First-pass read used to classify text vs binary. A binary file almost
/// always reveals a NUL byte or invalid UTF-8 within this window, so we can
/// reject it without reading the rest — a multi-GB binary costs one 64 KiB
/// read, not a full [`ATTACHMENT_PREVIEW_CAP`] read.
const ATTACHMENT_SNIFF_BYTES: usize = 64 * 1024;

/// Decoded content of an attachment read from disk.
pub(crate) enum FileText {
    /// Valid UTF-8, no NUL bytes. `truncated` when the file exceeded the cap.
    Text { text: String, truncated: bool },
    /// Binary (NUL byte or invalid UTF-8). No preview.
    Binary,
}

/// Raw read of an attachment: its size/mtime plus the decoded preview content.
pub(crate) struct AttachmentRead {
    pub size: u64,
    pub modified_secs: u64,
    pub content: FileText,
}

/// Text-vs-binary heuristic, the same call git/ripgrep make: a buffer is text
/// iff it holds no NUL byte and is valid UTF-8. A buffer that is valid UTF-8 up
/// to a final incomplete multi-byte char (the cap split it mid-character) is
/// still text — we keep the valid prefix; a genuine invalid byte means binary.
fn decode_attachment_text(buf: &[u8]) -> Option<String> {
    if buf.contains(&0) {
        return None;
    }
    match std::str::from_utf8(buf) {
        Ok(s) => Some(s.to_owned()),
        // `error_len() == None` means the error is a truncated trailing char.
        Err(e) if e.error_len().is_none() => Some(
            std::str::from_utf8(&buf[..e.valid_up_to()])
                .unwrap()
                .to_owned(),
        ),
        Err(_) => None,
    }
}

/// Whether a sniff window looks binary: a NUL byte, or invalid UTF-8 that is a
/// genuine bad byte rather than a multi-byte char the window split. Unlike
/// [`decode_attachment_text`], a trailing-truncated char here is NOT binary —
/// the window may simply have cut mid-character, and the rest is read next.
fn sniff_is_binary(buf: &[u8]) -> bool {
    if buf.contains(&0) {
        return true;
    }
    match std::str::from_utf8(buf) {
        Ok(_) => false,
        Err(e) => e.error_len().is_some(),
    }
}

/// Stats an attachment and reads up to [`ATTACHMENT_PREVIEW_CAP`] bytes,
/// classifying the content as text (with the decoded preview) or binary.
pub(crate) async fn read_attachment<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<AttachmentRead, FSError> {
    use tokio::io::AsyncReadExt;

    let os_path = resolve_path_on_disk(&workspace_path, path).await;
    let meta = match tokio::fs::metadata(&os_path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FSError::VaultPathNotFound {
                path: path.to_owned(),
            });
        }
        Err(e) => return Err(FSError::ReadFileError(e)),
    };
    let (size, modified_secs) = size_and_mtime(&meta);

    let file = tokio::fs::File::open(&os_path)
        .await
        .map_err(FSError::ReadFileError)?;

    // Phase 1: sniff the first window. A binary file reveals itself here, so we
    // never read the full cap just to reject it.
    let mut reader = file.take(ATTACHMENT_SNIFF_BYTES as u64);
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .await
        .map_err(FSError::ReadFileError)?;
    if sniff_is_binary(&buf) {
        return Ok(AttachmentRead {
            size,
            modified_secs,
            content: FileText::Binary,
        });
    }

    // Phase 2: text so far — read the remainder up to one byte past the cap, so
    // a file exactly at the cap is not flagged truncated and anything larger is.
    let remaining = (ATTACHMENT_PREVIEW_CAP + 1).saturating_sub(buf.len());
    reader
        .into_inner()
        .take(remaining as u64)
        .read_to_end(&mut buf)
        .await
        .map_err(FSError::ReadFileError)?;
    let truncated = buf.len() > ATTACHMENT_PREVIEW_CAP;
    buf.truncate(ATTACHMENT_PREVIEW_CAP);

    // Decode the whole buffer: a NUL only appearing past the sniff window still
    // makes it binary.
    let content = match decode_attachment_text(&buf) {
        Some(text) => FileText::Text { text, truncated },
        None => FileText::Binary,
    };
    Ok(AttachmentRead {
        size,
        modified_secs,
        content,
    })
}

/// Resolves `path` and returns its filesystem metadata, mapping a missing file
/// to [`FSError::VaultPathNotFound`]. Used to classify an entry's kind.
pub(crate) async fn metadata_at<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<std::fs::Metadata, FSError> {
    let os_path = resolve_path_on_disk(&workspace_path, path).await;
    match tokio::fs::metadata(&os_path).await {
        Ok(m) => Ok(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(FSError::VaultPathNotFound {
            path: path.to_owned(),
        }),
        Err(e) => Err(FSError::ReadFileError(e)),
    }
}

/// Renames/moves an attachment (any non-note file) on disk. Plain filesystem
/// rename — no index or link rewriting, since attachments are not indexed and
/// not part of the note-link graph.
pub(crate) async fn rename_attachment<P: AsRef<Path>>(
    workspace_path: P,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), FSError> {
    rename_path(workspace_path, from, to).await
}

/// Deletes an attachment file. Plain `remove_file` — no index cleanup.
pub(crate) async fn delete_attachment<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<(), FSError> {
    remove_file_at(workspace_path, path).await
}

pub(crate) async fn save_note<P: AsRef<Path>, S: AsRef<str>>(
    workspace_path: P,
    path: &VaultPath,
    text: S,
) -> Result<NoteEntryData, FSError> {
    path.ensure_note()?;
    // Resolve the full path case-insensitively so an existing `MyNote.md` is
    // written in place rather than creating a new lowercase `mynote.md` alongside it.
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    if let Some(base_path) = full_path.parent() {
        tokio::fs::create_dir_all(base_path).await?;
    }
    tokio::fs::write(&full_path, text.as_ref().as_bytes()).await?;

    let entry = NoteEntryData::from_os_path(path, &full_path).await?;
    Ok(entry)
}

/// Creates a new note at `path` exclusively. Returns `FSError::AlreadyExists` if
/// any file (case-insensitive) already occupies the resolved path.
pub(crate) async fn create_note_exclusive<P: AsRef<Path>, S: AsRef<str>>(
    workspace_path: P,
    path: &VaultPath,
    text: S,
) -> Result<NoteEntryData, FSError> {
    path.ensure_note()?;
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    if let Some(base_path) = full_path.parent() {
        tokio::fs::create_dir_all(base_path).await?;
    }
    let mut file = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&full_path)
        .await
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(FSError::AlreadyExists {
                path: path.to_owned(),
            });
        }
        Err(e) => return Err(FSError::ReadFileError(e)),
    };
    use tokio::io::AsyncWriteExt;
    file.write_all(text.as_ref().as_bytes()).await?;
    file.flush().await?;
    drop(file);

    NoteEntryData::from_os_path(path, &full_path).await
}

pub(crate) async fn rename_note<P: AsRef<Path>>(
    workspace_path: P,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), FSError> {
    from.ensure_note()?;
    to.ensure_note()?;
    rename_path(workspace_path, from, to).await
}

pub(crate) async fn rename_directory<P: AsRef<Path>>(
    workspace_path: P,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), FSError> {
    from.ensure_directory()?;
    to.ensure_directory()?;
    rename_path(workspace_path, from, to).await
}

pub(crate) async fn delete_note<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<(), FSError> {
    remove_file_at(workspace_path, path).await
}

/// Resolves `path` and removes the single file there. Shared by note and
/// attachment deletion, which differ only in their lib-level index/backup
/// handling, not in the filesystem step.
async fn remove_file_at<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<(), FSError> {
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    tokio::fs::remove_file(full_path).await?;
    Ok(())
}

/// Create `dir` and all missing parents. No-op if it already exists.
pub(crate) fn ensure_dir(dir: &Path) -> Result<(), FSError> {
    std::fs::create_dir_all(dir).map_err(FSError::ReadFileError)
}

/// Returns true if anything (file or directory) exists at the resolved
/// disk path for `path`. Cheaper than `load_note` when the contents are
/// not needed.
pub(crate) async fn path_exists<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<bool, FSError> {
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    Ok(tokio::fs::try_exists(&full_path).await?)
}

pub(crate) async fn delete_directory<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<(), FSError> {
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    tokio::fs::remove_dir_all(full_path).await?;
    Ok(())
}

fn filter_files(dir: &ignore::DirEntry) -> bool {
    // Prune dotfile / dot-directory entries (e.g. the hidden `.kimun` backups
    // dir) so they never enter the index. `path().starts_with(".")` does NOT
    // work here — the walker root is an absolute path, so an entry's path never
    // begins with "."; check the entry's own name instead. The `ignore` crate's
    // default hidden filter also covers these, but excluding them explicitly
    // keeps the walk correct even if that default is ever disabled.
    dir.file_name()
        .to_str()
        .map(|name| !name.starts_with('.'))
        .unwrap_or(true)
}

pub(crate) fn list_directories<P: AsRef<Path>>(
    base_path: P,
    path: &VaultPath,
    recursive: bool,
) -> Result<Vec<super::DirectoryDetails>, FSError> {
    let base_path = base_path.as_ref();
    let os_path = resolve_path_on_disk_sync(base_path, path);
    let walker = WalkBuilder::new(&os_path)
        .max_depth(if recursive { None } else { Some(1) })
        .filter_entry(filter_files)
        .build();

    let mut dirs = Vec::new();
    for entry in walker.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() && entry_path != os_path {
            let vault_path = VaultPath::from_path(base_path, entry_path)?;
            dirs.push(super::DirectoryDetails { path: vault_path });
        }
    }
    Ok(dirs)
}

pub(crate) fn get_file_walker<P: AsRef<Path>>(
    base_path: P,
    path: &VaultPath,
    recurse: bool,
) -> WalkParallel {
    let w = WalkBuilder::new(resolve_path_on_disk_sync(base_path, path))
        .max_depth(if recurse { None } else { Some(1) })
        .filter_entry(filter_files)
        // .threads(0)
        .build_parallel();

    w
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{decode_attachment_text, save_attachment, sniff_is_binary};

    #[test]
    fn decode_attachment_text_accepts_plain_utf8() {
        assert_eq!(
            decode_attachment_text("héllo".as_bytes()),
            Some("héllo".to_string())
        );
    }

    #[test]
    fn decode_attachment_text_rejects_nul_byte() {
        assert_eq!(decode_attachment_text(&[b'a', 0, b'b']), None);
    }

    #[test]
    fn decode_attachment_text_rejects_invalid_utf8() {
        // 0xFF is never valid UTF-8 and is not a truncated trailing char.
        assert_eq!(decode_attachment_text(&[b'a', 0xFF, b'b']), None);
    }

    #[test]
    fn sniff_is_binary_flags_nul_and_invalid_utf8_but_not_split_char() {
        assert!(!sniff_is_binary(b"plain ascii text"));
        assert!(sniff_is_binary(&[b'a', 0, b'b']), "NUL byte is binary");
        assert!(
            sniff_is_binary(&[b'a', 0xFF, b'b']),
            "a genuine invalid byte is binary"
        );
        // "é" = 0xC3 0xA9; a window cut after 0xC3 is a split char, not binary.
        assert!(
            !sniff_is_binary(&[b'a', b'b', 0xC3]),
            "a trailing truncated multi-byte char is still text"
        );
    }

    #[test]
    fn decode_attachment_text_keeps_prefix_when_split_mid_char() {
        // "é" is 0xC3 0xA9; cut after 0xC3 simulates the cap splitting a char.
        let mut bytes = b"ab".to_vec();
        bytes.push(0xC3);
        assert_eq!(
            decode_attachment_text(&bytes),
            Some("ab".to_string()),
            "a truncated trailing multi-byte char keeps the valid prefix as text"
        );
    }

    /// Returns true if the filesystem at `dir` is case-sensitive.
    /// Used to skip "no duplicate lowercase entry" assertions on macOS and other
    /// platforms that use a case-insensitive filesystem by default.
    fn is_case_sensitive_fs(dir: &Path) -> bool {
        // Write a probe file with a known uppercase name, then check whether the
        // lowercase variant resolves to the same entry or is absent.
        let upper = dir.join("__CaseSensitivityProbe__");
        std::fs::write(&upper, "").unwrap();
        let result = !dir.join("__casesensitivityprobe__").exists();
        std::fs::remove_file(&upper).unwrap();
        result
    }

    use crate::{
        error::FSError,
        nfs::{
            create_directory, delete_directory, delete_note, rename_directory, rename_note,
            save_note, DirectoryEntryData, EntryData, VaultEntry, VaultEntryDetails,
        },
        DirectoryDetails, NoteDetails,
    };

    use super::{load_note, VaultPath};

    #[tokio::test]
    async fn test_file_not_exists() {
        let path = VaultPath::new("don't exist");
        let res = load_note(std::env::current_dir().unwrap(), &path).await;

        let result = if let Err(e) = res {
            matches!(e, FSError::VaultPathNotFound { path: _ })
        } else {
            false
        };

        assert!(result);
    }

    #[tokio::test]
    async fn create_a_note() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let note_path = VaultPath::new("note.md");
        let note_text = "this is an empty note".to_string();

        let res = save_note(workspace_path, &note_path, &note_text).await;
        if let Err(e) = &res {
            panic!("Error saving note: {e}")
        }

        let note = load_note(workspace_path, &note_path).await;
        if let Err(e) = &note {
            panic!("Error loading note: {e}")
        }
        assert_eq!(note.unwrap(), note_text);

        let del_res = delete_note(workspace_path, &note_path).await;
        if let Err(e) = &del_res {
            panic!("Error deleting note: {e}")
        }
        assert!(load_note(workspace_path, &note_path).await.is_err());
    }

    #[tokio::test]
    async fn move_a_note() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let note_path = VaultPath::new("note.md");
        let dest_note_path = VaultPath::new("directory/moved_note.md");
        let note_text = "this is an empty note".to_string();

        let res = save_note(workspace_path, &note_path, &note_text).await;
        if let Err(e) = &res {
            panic!("Error saving note: {e}")
        }
        let note = load_note(workspace_path, &note_path).await;
        if let Err(e) = &note {
            panic!("Error loading note: {e}")
        }
        assert_eq!(note.as_ref().unwrap().to_owned(), note_text);

        let ren_res = rename_note(workspace_path, &note_path, &dest_note_path).await;
        if let Err(e) = &ren_res {
            panic!("Error renaming note: {e}")
        }
        let moved_note = load_note(workspace_path, &dest_note_path).await;
        if let Err(e) = &moved_note {
            panic!("Error loading note: {e}")
        }
        assert_eq!(note.unwrap(), moved_note.unwrap());
        assert!(load_note(workspace_path, &note_path).await.is_err());

        let del_res = delete_note(workspace_path, &dest_note_path).await;
        if let Err(e) = &del_res {
            panic!("Error deleting note: {e}")
        }
        assert!(load_note(workspace_path, &dest_note_path).await.is_err());

        let del_res = delete_directory(workspace_path, &dest_note_path.get_parent_path().0).await;
        if let Err(e) = &del_res {
            panic!("Error deleting directory: {e}")
        }
    }

    #[tokio::test]
    async fn move_a_directory() -> Result<(), FSError> {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let from_note_dir = VaultPath::new("old_dir");
        let from_note_path = from_note_dir.append(&VaultPath::new("note.md"));
        let dest_note_dir = VaultPath::new("new_dir/two_levels");
        let dest_note_path = dest_note_dir.append(&VaultPath::new("note.md"));
        let note_text = "this is an empty note".to_string();

        save_note(workspace_path, &from_note_path, &note_text).await?;
        let note = load_note(workspace_path, &from_note_path).await?;
        assert_eq!(note, note_text);

        rename_directory(workspace_path, &from_note_dir, &dest_note_dir).await?;
        let moved_note = load_note(workspace_path, &dest_note_path).await?;
        assert_eq!(note, moved_note);
        assert!(load_note(workspace_path, &from_note_dir).await.is_err());

        delete_note(workspace_path, &dest_note_path).await?;
        assert!(load_note(workspace_path, &dest_note_path).await.is_err());

        let first_level = dest_note_path.get_parent_path().0;
        let second_level = first_level.get_parent_path().0;
        delete_directory(workspace_path, &first_level).await?;
        delete_directory(workspace_path, &second_level).await?;

        Ok(())
    }

    // Additional comprehensive tests for NFS module

    #[tokio::test]
    async fn test_vault_entry_new_with_directory() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let dir_path = VaultPath::new("test_directory");

        // Create directory first
        tokio::fs::create_dir_all(workspace_path.join("test_directory"))
            .await
            .ok();

        let result = VaultEntry::new(workspace_path, dir_path.clone()).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        assert_eq!(entry.path, dir_path);
        assert_eq!(entry.path_string, dir_path.to_string());

        match entry.data {
            EntryData::Directory(dir_data) => {
                assert_eq!(dir_data.path, dir_path);
            }
            _ => panic!("Expected Directory entry data"),
        }

        // Cleanup
        tokio::fs::remove_dir_all(workspace_path.join("test_directory"))
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_vault_entry_new_with_note() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("test_note.md");
        let note_content = "# Test Note\n\nThis is a test.";

        // Create note first
        save_note(workspace_path, &note_path, note_content)
            .await
            .unwrap();

        let result = VaultEntry::new(workspace_path, note_path.clone()).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        assert_eq!(entry.path, note_path);

        match entry.data {
            EntryData::Note(note_data) => {
                assert_eq!(note_data.path, note_path);
                assert!(note_data.size > 0);
                assert!(note_data.modified_secs > 0);
            }
            _ => panic!("Expected Note entry data"),
        }

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
    }

    #[tokio::test]
    async fn test_vault_entry_new_with_attachment() {
        let workspace_path = Path::new("testdata");
        let attachment_path = VaultPath::new("test.txt");

        // Create a text file (attachment)
        tokio::fs::create_dir_all(workspace_path).await.ok();
        tokio::fs::write(workspace_path.join("test.txt"), "test content")
            .await
            .unwrap();

        let result = VaultEntry::new(workspace_path, attachment_path.clone()).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        match entry.data {
            EntryData::Attachment => (),
            _ => panic!("Expected Attachment entry data"),
        }

        // Cleanup
        tokio::fs::remove_file(workspace_path.join("test.txt"))
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_vault_entry_new_with_nonexistent_path() {
        let workspace_path = Path::new("testdata");
        let nonexistent_path = VaultPath::new("does_not_exist.md");

        let result = VaultEntry::new(workspace_path, nonexistent_path).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            FSError::NoFileOrDirectoryFound { .. } => (),
            _ => panic!("Expected NoFileOrDirectoryFound error"),
        }
    }

    #[tokio::test]
    async fn test_vault_entry_from_path() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("from_path_test.md");
        let note_content = "Test content";

        // Create note
        save_note(workspace_path, &note_path, note_content)
            .await
            .unwrap();

        let full_path = workspace_path.join("from_path_test.md");
        let result = VaultEntry::from_path(workspace_path, &full_path).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        assert_eq!(entry.path, note_path.clone().absolute());

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
    }

    #[tokio::test]
    async fn test_vault_entry_display() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("display_test.md");
        let dir_path = VaultPath::new("display_dir");
        let attachment_path = VaultPath::new("display.txt");

        // Test note display
        save_note(workspace_path, &note_path, "content")
            .await
            .unwrap();
        let note_entry = VaultEntry::new(workspace_path, note_path.clone())
            .await
            .unwrap();
        let note_display = format!("{}", note_entry);
        assert!(note_display.contains("[NOT]"));
        assert!(note_display.contains(&note_path.to_string()));

        // Test directory display
        tokio::fs::create_dir_all(workspace_path.join("display_dir"))
            .await
            .ok();
        let dir_entry = VaultEntry::new(workspace_path, dir_path.clone())
            .await
            .unwrap();
        let dir_display = format!("{}", dir_entry);
        assert!(dir_display.contains("[DIR]"));
        assert!(dir_display.contains(&dir_path.to_string()));

        // Test attachment display
        tokio::fs::write(workspace_path.join("display.txt"), "content")
            .await
            .ok();
        let attachment_entry = VaultEntry::new(workspace_path, attachment_path.clone())
            .await
            .unwrap();
        let attachment_display = format!("{}", attachment_entry);
        assert!(attachment_display.contains("[ATT]"));

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
        tokio::fs::remove_dir_all(workspace_path.join("display_dir"))
            .await
            .ok();
        tokio::fs::remove_file(workspace_path.join("display.txt"))
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_note_entry_data_load_details() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("details_test.md");
        let note_content = "# Test\n\nContent here";

        save_note(workspace_path, &note_path, note_content)
            .await
            .unwrap();
        let entry = VaultEntry::new(workspace_path, note_path.clone())
            .await
            .unwrap();

        if let EntryData::Note(note_data) = entry.data {
            let details_result = note_data.load_details(workspace_path, &note_path).await;
            assert!(details_result.is_ok());

            let details = details_result.unwrap();
            assert_eq!(details.path, note_path);
            assert_eq!(details.raw_text, note_content);
        } else {
            panic!("Expected Note entry data");
        }

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
    }

    #[test]
    fn test_directory_entry_data_get_details() {
        let dir_path = VaultPath::new("test_dir");
        let dir_data = DirectoryEntryData {
            path: dir_path.clone(),
        };

        let details = dir_data.get_details::<PathBuf>();
        assert_eq!(details.path, dir_path);
    }

    #[test]
    fn test_vault_entry_details_get_title() {
        let note_path = VaultPath::new("test.md");
        let note_content = "# My Title\n\nContent";
        let note_details = NoteDetails::new(&note_path, note_content);

        let mut note_entry_details = VaultEntryDetails::Note(note_details);
        let title = note_entry_details.get_title();
        assert_eq!(title, "My Title");

        let dir_path = VaultPath::new("test_dir");
        let dir_details = DirectoryDetails { path: dir_path };
        let mut dir_entry_details = VaultEntryDetails::Directory(dir_details);
        let dir_title = dir_entry_details.get_title();
        assert_eq!(dir_title, "");

        let mut none_details = VaultEntryDetails::None;
        let none_title = none_details.get_title();
        assert_eq!(none_title, "");
    }

    #[test]
    fn test_hash_text() {
        use super::hash_text;

        let text1 = "Hello, world!";
        let text2 = "Hello, world!";
        let text3 = "Different text";

        let hash1 = hash_text(text1);
        let hash2 = hash_text(text2);
        let hash3 = hash_text(text3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert!(hash1 > 0);
    }

    #[tokio::test]
    async fn test_create_directory_with_note_path() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("invalid.md");

        let result = create_directory(workspace_path, &note_path).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            FSError::InvalidPath { message, .. } => {
                assert_eq!(message, "The path is not a directory");
            }
            _ => panic!("Expected InvalidPath error"),
        }
    }

    #[tokio::test]
    async fn save_attachment_writes_bytes_and_creates_parent_dirs() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        let path = VaultPath::new("/assets/img.png");
        let bytes = b"\x89PNG\r\n\x1a\n stub".to_vec();

        save_attachment(workspace, &path, &bytes).await.unwrap();

        let on_disk = workspace.join("assets").join("img.png");
        let read_back = tokio::fs::read(&on_disk).await.unwrap();
        assert_eq!(read_back, bytes);
    }

    #[tokio::test]
    async fn test_save_note_with_directory_path() {
        let workspace_path = Path::new("testdata");
        let dir_path = VaultPath::new("directory");
        let content = "test content";

        let result = save_note(workspace_path, &dir_path, content).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            FSError::InvalidPath { message, .. } => {
                assert_eq!(message, "The path is not a note");
            }
            _ => panic!("Expected InvalidPath error"),
        }
    }

    #[tokio::test]
    async fn test_rename_note_with_invalid_paths() {
        let workspace_path = Path::new("testdata");
        let dir_path = VaultPath::new("directory");
        let note_path = VaultPath::new("note.md");

        // Test renaming from directory (should fail)
        let result = rename_note(workspace_path, &dir_path, &note_path).await;
        assert!(result.is_err());

        // Test renaming to directory (should fail)
        let result = rename_note(workspace_path, &note_path, &dir_path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rename_directory_with_invalid_paths() {
        let workspace_path = Path::new("testdata");
        let dir_path = VaultPath::new("directory");
        let note_path = VaultPath::new("note.md");

        // Test renaming from note (should fail)
        let result = rename_directory(workspace_path, &note_path, &dir_path).await;
        assert!(result.is_err());

        // Test renaming to note (should fail)
        let result = rename_directory(workspace_path, &dir_path, &note_path).await;
        assert!(result.is_err());
    }

    // ── Case-insensitive disk resolution tests ────────────────────────────────

    #[tokio::test]
    async fn resolve_finds_uppercase_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal"))
            .await
            .unwrap();

        let result = super::resolve_path_on_disk(tmp.path(), &VaultPath::new("/journal")).await;
        assert_eq!(result, tmp.path().join("Journal"));
    }

    #[tokio::test]
    async fn resolve_finds_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "hi")
            .await
            .unwrap();

        let result =
            super::resolve_path_on_disk(tmp.path(), &VaultPath::new("/projects/mynote.md")).await;
        assert_eq!(result, tmp.path().join("Projects").join("MyNote.md"));
    }

    #[tokio::test]
    async fn resolve_uses_lowercase_for_nonexistent_path() {
        let tmp = tempfile::TempDir::new().unwrap();

        let result =
            super::resolve_path_on_disk(tmp.path(), &VaultPath::new("/newdir/note.md")).await;
        assert_eq!(result, tmp.path().join("newdir").join("note.md"));
    }

    #[test]
    fn resolve_sync_finds_uppercase_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("Archive")).unwrap();

        let result = super::resolve_path_on_disk_sync(tmp.path(), &VaultPath::new("/archive"));
        assert_eq!(result, tmp.path().join("Archive"));
    }

    #[tokio::test]
    async fn load_note_finds_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Journal").join("MyNote.md"), "# Hello")
            .await
            .unwrap();

        let text = super::load_note(tmp.path(), &VaultPath::new("/journal/mynote.md"))
            .await
            .unwrap();
        assert_eq!(text, "# Hello");
    }

    #[tokio::test]
    async fn save_note_writes_to_existing_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Journal").join("MyNote.md"), "original")
            .await
            .unwrap();

        save_note(tmp.path(), &VaultPath::new("/journal/mynote.md"), "updated")
            .await
            .unwrap();

        // The uppercase file should be updated
        let content = tokio::fs::read_to_string(tmp.path().join("Journal").join("MyNote.md"))
            .await
            .unwrap();
        assert_eq!(content, "updated");

        // On case-sensitive filesystems: no duplicate lowercase entries should exist.
        // On case-insensitive filesystems (e.g. macOS default APFS), 'Journal' and
        // 'journal' are the same path so these assertions are not meaningful.
        if is_case_sensitive_fs(tmp.path()) {
            assert!(!tmp.path().join("Journal").join("mynote.md").exists());
            assert!(!tmp.path().join("journal").exists());
        }
    }

    #[tokio::test]
    async fn save_note_in_uppercase_parent_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects"))
            .await
            .unwrap();

        save_note(tmp.path(), &VaultPath::new("/projects/new.md"), "content")
            .await
            .unwrap();

        // File should land inside the existing uppercase directory
        assert!(tmp.path().join("Projects").join("new.md").exists());
        // On case-sensitive filesystems: no duplicate lowercase directory should exist.
        if is_case_sensitive_fs(tmp.path()) {
            assert!(!tmp.path().join("projects").exists());
        }
    }

    #[tokio::test]
    async fn delete_note_removes_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal"))
            .await
            .unwrap();
        let file = tmp.path().join("Journal").join("MyNote.md");
        tokio::fs::write(&file, "bye").await.unwrap();

        delete_note(tmp.path(), &VaultPath::new("/journal/mynote.md"))
            .await
            .unwrap();

        assert!(!file.exists());
    }

    #[tokio::test]
    async fn delete_directory_removes_uppercase_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Archive"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Archive").join("note.md"), "x")
            .await
            .unwrap();

        delete_directory(tmp.path(), &VaultPath::new("/archive"))
            .await
            .unwrap();

        assert!(!tmp.path().join("Archive").exists());
    }

    #[tokio::test]
    async fn rename_note_finds_uppercase_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "data")
            .await
            .unwrap();

        rename_note(
            tmp.path(),
            &VaultPath::new("/projects/mynote.md"),
            &VaultPath::new("/projects/renamed.md"),
        )
        .await
        .unwrap();

        assert!(tmp.path().join("Projects").join("renamed.md").exists());
        assert!(!tmp.path().join("Projects").join("MyNote.md").exists());
    }

    #[tokio::test]
    async fn rename_note_into_uppercase_parent() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Inbox"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Inbox").join("note.md"), "data")
            .await
            .unwrap();
        tokio::fs::create_dir(tmp.path().join("Archive"))
            .await
            .unwrap();

        rename_note(
            tmp.path(),
            &VaultPath::new("/inbox/note.md"),
            &VaultPath::new("/archive/note.md"),
        )
        .await
        .unwrap();

        assert!(tmp.path().join("Archive").join("note.md").exists());
        // On case-sensitive filesystems: no duplicate lowercase directory should exist.
        if is_case_sensitive_fs(tmp.path()) {
            assert!(!tmp.path().join("archive").exists());
        }
    }

    #[tokio::test]
    async fn rename_directory_finds_uppercase_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("OldName"))
            .await
            .unwrap();

        rename_directory(
            tmp.path(),
            &VaultPath::new("/oldname"),
            &VaultPath::new("/newname"),
        )
        .await
        .unwrap();

        assert!(tmp.path().join("newname").exists());
        assert!(!tmp.path().join("OldName").exists());
    }

    #[tokio::test]
    async fn vault_entry_from_path_uses_lowercase_vault_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "# Title")
            .await
            .unwrap();

        let entry =
            VaultEntry::from_path(tmp.path(), tmp.path().join("Projects").join("MyNote.md"))
                .await
                .unwrap();

        // VaultPath is always lowercase even though the disk file has uppercase
        assert_eq!(entry.path.to_string(), "/projects/mynote.md");
        assert!(matches!(entry.data, EntryData::Note(_)));
    }

    #[tokio::test]
    async fn vault_entry_new_finds_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects"))
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "# Title")
            .await
            .unwrap();

        let entry = VaultEntry::new(tmp.path(), VaultPath::new("/projects/mynote.md"))
            .await
            .unwrap();

        assert_eq!(entry.path.to_string(), "/projects/mynote.md");
        assert!(matches!(entry.data, EntryData::Note(_)));
    }
}
