//! LinkRewrite — rewrites every note link pointing at a renamed note.
//!
//! Three compiler-enforced stages, with the caller's filesystem rename of
//! the source note sitting between the last two:
//!
//! 1. [`LinkRewrite::scout`] — one index query for the notes linking to the
//!    source (the victims). The caller locks them (plus source and
//!    destination) before going further.
//! 2. [`Scouted::prepare`] — read every victim, rewrite its links in memory,
//!    and take a fail-closed backup of each note about to change. No
//!    filesystem mutation: a failure here aborts the rename cleanly.
//! 3. [`Prepared::commit`] — after the source has been renamed on disk:
//!    write the rewritten victims (concurrency-bounded), rewrite the renamed
//!    note's self-links at its new path, and return the updated entries for
//!    the index commit.
//!
//! Each stage consumes the previous, so running them out of order is a
//! compile error, not a broken vault.

use std::path::Path;

use futures_util::stream::StreamExt;

use crate::error::VaultError;
use crate::index::NoteIndex;
use crate::nfs::{self, NoteEntryData, VaultPath};
use crate::note;

/// Maximum number of concurrent FS read/write tasks while rewriting. Caps
/// file-descriptor pressure on hub-style notes with thousands of links.
/// Sized well below typical soft `ulimit -n` (256 on macOS, 1024 on Linux)
/// while still parallelizing enough to hide per-syscall latency.
const REWRITE_IO_CONCURRENCY: usize = 32;

/// Entry point: a link-rewrite over one vault. Cheap to construct per call.
pub(crate) struct LinkRewrite<'a> {
    index: &'a NoteIndex,
    workspace_path: &'a Path,
    /// Whether `prepare` takes a pre-change backup of every note about to be
    /// rewritten (ADR-0002: rename's collateral rewrites are automated edits).
    backup: bool,
}

impl<'a> LinkRewrite<'a> {
    pub(crate) fn new(index: &'a NoteIndex, workspace_path: &'a Path, backup: bool) -> Self {
        Self {
            index,
            workspace_path,
            backup,
        }
    }

    /// Stage 1: query the index once for the notes linking to `from`. The
    /// source itself is excluded — its self-links are rewritten at the new
    /// path during [`Prepared::commit`], never written back to `from` (which
    /// would resurrect a file at the old path).
    ///
    /// Paths are flattened internally, so callers may pass them as-is.
    pub(crate) async fn scout(
        self,
        from: &VaultPath,
        to: &VaultPath,
    ) -> Result<Scouted<'a>, VaultError> {
        let from = from.flatten();
        let to = to.flatten();
        let victims: Vec<VaultPath> = self
            .index
            .get_backlinks(&from)
            .await?
            .into_iter()
            .map(|(e, _)| e.path)
            .filter(|p| *p != from)
            .collect();
        Ok(Scouted {
            base: self,
            from,
            to,
            victims,
        })
    }
}

/// Stage 1 output: the victim list is known; nothing has been read or
/// written. The caller locks the victims (plus source and destination)
/// before calling [`prepare`](Self::prepare).
pub(crate) struct Scouted<'a> {
    base: LinkRewrite<'a>,
    from: VaultPath,
    to: VaultPath,
    victims: Vec<VaultPath>,
}

impl<'a> Scouted<'a> {
    /// The notes whose links will be rewritten. For lock acquisition.
    pub(crate) fn victims(&self) -> &[VaultPath] {
        &self.victims
    }

    /// Stage 2: read every victim and rewrite its links to `from` in memory,
    /// keeping only the ones whose content actually changed; then take a
    /// fail-closed backup of each changed victim plus the source (its
    /// self-links are rewritten at the new path during commit). I/O is
    /// concurrency-bounded. No filesystem mutation happens here — a failure
    /// aborts the rename cleanly.
    pub(crate) async fn prepare(self) -> Result<Prepared<'a>, VaultError> {
        let Self {
            base,
            from,
            to,
            victims,
        } = self;

        let workspace = base.workspace_path;
        let updates: Vec<(VaultPath, String)> = run_bounded(victims.into_iter().map(|path| {
            let from = &from;
            let to = &to;
            async move {
                let text = nfs::load_note(workspace, &path).await?;
                let (updated, changed) = note::replace_note_links(&text, from, to);
                Ok(changed.then_some((path, updated)))
            }
        }))
        .await?
        .into_iter()
        .flatten()
        .collect();

        // Back up the pre-rewrite content of every note this rename will
        // modify — the changed victims, plus the source itself. Done before
        // any FS mutation so a backup failure aborts cleanly (fail-closed).
        // These writes go through nfs directly (not NoteVault::save_note),
        // so the backup gate is applied explicitly here.
        if base.backup {
            for path in updates.iter().map(|(p, _)| p).chain(std::iter::once(&from)) {
                nfs::backup_note(base.workspace_path, path).await?;
            }
        }

        Ok(Prepared {
            workspace_path: base.workspace_path,
            from,
            to,
            updates,
        })
    }
}

/// Stage 2 output: every rewritten body is in memory and backed up. The
/// caller renames the source note on disk, then calls
/// [`commit`](Self::commit).
pub(crate) struct Prepared<'a> {
    workspace_path: &'a Path,
    from: VaultPath,
    to: VaultPath,
    updates: Vec<(VaultPath, String)>,
}

impl Prepared<'_> {
    /// Stage 3: write the rewritten victims (concurrency-bounded, each
    /// file's text moved into its task without cloning), then rewrite any
    /// self-links inside the renamed note at its new location. Returns the
    /// updated `(entry, text)` pairs ready for the index commit.
    pub(crate) async fn commit(self) -> Result<Vec<(NoteEntryData, String)>, VaultError> {
        let Self {
            workspace_path,
            from,
            to,
            updates,
        } = self;

        let mut out = run_bounded(updates.into_iter().map(|(path, text)| async move {
            let entry = nfs::save_note(workspace_path, &path, &text).await?;
            Ok((entry, text))
        }))
        .await?;

        // Self-links inside the renamed file, rewritten at its new location.
        let text = nfs::load_note(workspace_path, &to).await?;
        let (updated, changed) = note::replace_note_links(&text, &from, &to);
        if changed {
            let entry = nfs::save_note(workspace_path, &to, &updated).await?;
            out.push((entry, updated));
        }

        Ok(out)
    }
}

/// Drives `futs` with at most [`REWRITE_IO_CONCURRENCY`] in flight and
/// collects the results, failing fast on the first error. The single
/// bounded-I/O loop both [`Scouted::prepare`] (reads) and
/// [`Prepared::commit`] (writes) drain through, so concurrency and
/// error-propagation behaviour cannot drift between the two stages.
async fn run_bounded<T>(
    futs: impl Iterator<Item = impl std::future::Future<Output = Result<T, VaultError>>>,
) -> Result<Vec<T>, VaultError> {
    let mut stream = futures_util::stream::iter(futs).buffered(REWRITE_IO_CONCURRENCY);
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item?);
    }
    Ok(out)
}
