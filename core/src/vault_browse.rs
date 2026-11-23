//! Browsing the vault on [`NoteVault`]: the walk that lists entries and
//! syncs the index as it goes, and the directory listing.

use std::fmt::Display;
use std::path::PathBuf;

use futures_util::{stream, Stream, StreamExt, TryStreamExt};
use log::{debug, warn};

use crate::error::VaultError;
use crate::nfs::{self, VaultPath};
use crate::sync::VaultSync;
use crate::{DirectoryDetails, NoteVault, SearchResult};

impl NoteVault {
    /// Resolves a vault path to its real OS filesystem location under the
    /// workspace root.
    pub fn path_to_pathbuf(&self, path: &VaultPath) -> PathBuf {
        path.to_pathbuf(self.workspace_path())
    }

    /// Walks the vault per `options` and yields every entry it finds — note,
    /// directory or attachment — as a [`SearchResult`], in discovery order,
    /// as soon as the walker has processed it. Nothing happens until the
    /// stream is first polled; from then on the walk runs on its own task to
    /// completion, even if the stream is dropped early (the index sync must
    /// not be left half-applied). A sync error surfaces as the stream's final
    /// `Err` item. A recursive browse from the root doubles as a full index
    /// sync: by the time the stream ends, the index reports ready.
    ///
    /// Use this when rows should appear while a slow (e.g. network) directory
    /// is still being read; [`browse_vault`](Self::browse_vault) collects the
    /// same walk into one `Vec`.
    pub fn browse_vault_stream(
        &self,
        options: VaultBrowseOptions,
    ) -> impl Stream<Item = Result<SearchResult, VaultError>> + Send + 'static {
        let vault = self.clone();
        // `once(..).flatten()` defers the spawn to the first poll, so building
        // the stream needs no runtime and an unpolled stream walks nothing.
        stream::once(async move {
            let (sender, receiver) = futures_channel::mpsc::unbounded();
            // `sender` moves into the task and drops when the walk ends, which
            // is what closes `receiver` and lets `outcome` be polled.
            let walk = tokio::spawn(async move { vault.browse_into(options, sender).await });
            let outcome = stream::once(async move {
                match walk.await {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(Err(e)),
                    Err(e) => Some(Err(VaultError::TaskJoin(format!("vault browse: {}", e)))),
                }
            })
            .filter_map(std::future::ready);
            receiver.map(Ok).chain(outcome)
        })
        .flatten()
    }

    /// Walks the vault per `options` and returns every entry it finds — the
    /// collected form of [`browse_vault_stream`](Self::browse_vault_stream),
    /// same sync side effect included.
    pub async fn browse_vault(
        &self,
        options: VaultBrowseOptions,
    ) -> Result<Vec<SearchResult>, VaultError> {
        self.browse_vault_stream(options).try_collect().await
    }

    /// The browse walk itself: syncs the subtree into the index while sending
    /// each discovered entry to `sender`, then marks a whole-vault walk synced.
    async fn browse_into(
        &self,
        options: VaultBrowseOptions,
        sender: futures_channel::mpsc::UnboundedSender<SearchResult>,
    ) -> Result<(), VaultError> {
        let start = std::time::Instant::now();
        debug!("> Start fetching files with Options:\n{}", options);

        // Logged here as well as returned: when the consumer dropped the
        // stream early nobody polls the outcome, and a failed sync must not
        // vanish silently.
        if let Err(e) = VaultSync::new(&self.index, self.workspace_path())
            .run(
                &options.path,
                options.recursive,
                options.validation,
                Some(sender),
            )
            .await
        {
            warn!("browse of {} failed to sync the index: {}", options.path, e);
            return Err(e);
        }

        // A recursive browse from the root is a whole-vault sync: the index
        // now mirrors the disk, so the readiness probe must report true —
        // same as int_index_notes. A partial-subtree browse must NOT mark
        // synced (the rest of a healed index could still be empty).
        if options.recursive && options.path.is_root_or_empty() {
            self.index.mark_synced();
        }

        debug!(
            "> Files fetched in {} milliseconds",
            start.elapsed().as_millis()
        );

        Ok(())
    }

    /// Returns all subdirectories under `path`.
    /// Non-recursive returns only the immediate children; recursive returns the full tree.
    pub fn get_directories(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<DirectoryDetails>, VaultError> {
        Ok(nfs::list_directories(
            self.workspace_path(),
            path,
            recursive,
        )?)
    }
}

/// Builder for [`VaultBrowseOptions`]; see [`NoteVault::browse_vault_stream`]
/// and its collected form [`NoteVault::browse_vault`].
pub struct VaultBrowseOptionsBuilder {
    path: VaultPath,
    validation: NotesValidation,
    recursive: bool,
}

impl VaultBrowseOptionsBuilder {
    /// Starts a builder rooted at `path`, with defaults otherwise.
    pub fn new(path: &VaultPath) -> Self {
        Self::default().path(path.clone())
    }

    /// Finalizes the options.
    pub fn build(self) -> VaultBrowseOptions {
        VaultBrowseOptions {
            path: self.path,
            validation: self.validation,
            recursive: self.recursive,
        }
    }

    /// Sets the path to browse from.
    pub fn path(mut self, path: VaultPath) -> Self {
        self.path = path;
        self
    }

    /// Sets whether the browse descends into subdirectories.
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    /// Sets how thoroughly each note is re-validated against the index during
    /// the browse.
    pub fn validation(mut self, validation: NotesValidation) -> Self {
        self.validation = validation;
        self
    }
}

impl Default for VaultBrowseOptionsBuilder {
    fn default() -> Self {
        Self {
            path: VaultPath::root(),
            validation: NotesValidation::None,
            recursive: false,
        }
    }
}

/// What a browse walks: the subtree root, how deep, and how strictly each
/// note is re-validated against the index on the way. See
/// [`NoteVault::browse_vault`] and [`NoteVault::browse_vault_stream`].
#[derive(Debug, Clone, PartialEq)]
pub struct VaultBrowseOptions {
    pub(crate) path: VaultPath,
    pub(crate) validation: NotesValidation,
    pub(crate) recursive: bool,
}

impl Display for VaultBrowseOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Vault Browse Options - [Path: `{}`|Validation Type: `{}`|Recursive: `{}`]",
            self.path, self.validation, self.recursive
        )
    }
}

/// How thoroughly a sync pass checks whether each note has changed before
/// updating its index entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotesValidation {
    /// Compares a content hash — detects any change, at the cost of reading
    /// each file.
    Full,
    /// Compares the file size — cheaper, but misses same-size edits.
    Fast,
    /// Only checks whether the note exists.
    None,
}

impl Display for NotesValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                NotesValidation::Full => "Full",
                NotesValidation::Fast => "Fast",
                NotesValidation::None => "None",
            }
        )
    }
}
