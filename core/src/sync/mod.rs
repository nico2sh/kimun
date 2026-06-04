//! VaultSync — brings the NoteIndex in step with the vault on disk.
//!
//! One call ([`VaultSync::run`]) owns the whole pipeline: read the cached
//! entries from the index, walk the subtree in parallel, diff against the
//! cache under a validation mode, and apply the resulting [`IndexDiff`] —
//! optionally streaming discovered entries to the caller as they are found.
//! The parallel walker, its thread-state plumbing, and the async/blocking
//! bridge are implementation details and never cross this interface.

mod visitor;

use std::path::Path;
use std::sync::mpsc::Sender;

use log::debug;

use crate::error::VaultError;
use crate::index::NoteIndex;
use crate::nfs::{self, VaultPath};
use crate::{NotesValidation, SearchResult};

use visitor::NoteListVisitorBuilder;

/// The sync pipeline over one vault: a [`NoteIndex`] plus the workspace root
/// it mirrors. Cheap to construct per call.
pub(crate) struct VaultSync<'a> {
    index: &'a NoteIndex,
    workspace_path: &'a Path,
}

impl<'a> VaultSync<'a> {
    pub(crate) fn new(index: &'a NoteIndex, workspace_path: &'a Path) -> Self {
        Self {
            index,
            workspace_path,
        }
    }

    /// Syncs the subtree at `path` into the index: cached entries are read,
    /// the filesystem is walked in parallel, every note is validated against
    /// the cache under `validation`, and the resulting [`IndexDiff`] is
    /// applied atomically. When `sender` is given, every discovered entry
    /// (note, directory, attachment) is streamed to it as the walk finds it.
    ///
    /// [`IndexDiff`]: crate::index::IndexDiff
    pub(crate) async fn run(
        &self,
        path: &VaultPath,
        recursive: bool,
        validation: NotesValidation,
        sender: Option<Sender<SearchResult>>,
    ) -> Result<(), VaultError> {
        debug!("Syncing subtree at {}", path);
        let cached_notes = self.index.get_notes(path, recursive).await?;
        let builder =
            NoteListVisitorBuilder::new(self.workspace_path, validation, cached_notes, sender);
        let walker = nfs::get_file_walker(self.workspace_path, path, recursive);
        let builder = run_walker_blocking(walker, builder).await?;
        self.index.apply(builder.into_diff()).await?;
        Ok(())
    }
}

/// Runs the synchronous parallel walker on a blocking thread so the async
/// runtime is not stalled while the vault subtree is enumerated.
async fn run_walker_blocking(
    walker: ignore::WalkParallel,
    builder: NoteListVisitorBuilder,
) -> Result<NoteListVisitorBuilder, VaultError> {
    tokio::task::spawn_blocking(move || {
        let mut builder = builder;
        walker.visit(&mut builder);
        builder
    })
    .await
    .map_err(|e| VaultError::TaskJoin(format!("vault walker: {}", e)))
}
