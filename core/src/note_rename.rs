//! NoteRename — renames a note and rewrites every note link pointing at it.
//!
//! One call, [`NoteRename::rename`], owns the whole operation: locks, the
//! filesystem move, the link rewrites, the index commit. The stages inside are
//! compiler-enforced — each consumes the previous, so running them out of
//! order is a compile error, not a broken vault:
//!
//! 1. *scout* — one index query for the notes linking to the source (the
//!    victims).
//! 2. *lock* — source, destination and every victim are locked for the rest
//!    of the rename (in a stable order, so two overlapping renames can't
//!    deadlock), so a concurrent in-process write to any of them can't
//!    interleave with what follows (lost update / stale backup).
//! 3. *prepare* — read every victim, rewrite its links in memory, and take a
//!    fail-closed backup of each note about to change. No filesystem
//!    mutation: a failure here aborts the rename cleanly.
//! 4. *move* — rename the source note on disk. If this fails, victims remain
//!    untouched and the index is unchanged: clean abort.
//! 5. *commit* — write the rewritten victims (concurrency-bounded) and rewrite
//!    the renamed note's self-links at its new path.
//! 6. *index* — one atomic index operation: rename the source rows and update
//!    each victim's chunks and links. If this fails, the filesystem is
//!    consistent with the rename but the index is stale — the next sync pass
//!    corrects it.
//!
//! Failure atomicity is therefore: nothing on disk changes before step 4;
//! after step 4 the vault is renamed and the rest is convergent.

use crate::system::SystemPath;

use futures_util::stream::StreamExt;

use crate::error::{FSError, VaultError};
use crate::index::NoteIndex;
use crate::nfs::{self, NoteEntryData, VaultPath};
use crate::note;
use crate::note_locks::NoteLocks;

/// Maximum number of concurrent FS read/write tasks while rewriting. Caps
/// file-descriptor pressure on hub-style notes with thousands of links.
/// Sized well below typical soft `ulimit -n` (256 on macOS, 1024 on Linux)
/// while still parallelizing enough to hide per-syscall latency.
const REWRITE_IO_CONCURRENCY: usize = 32;

/// Entry point: a rename over one vault. Cheap to construct per call.
pub(crate) struct NoteRename<'a> {
    index: &'a NoteIndex,
    workspace_path: &'a SystemPath,
    /// Whether `prepare` takes a pre-change backup of every note about to be
    /// rewritten (rename's collateral rewrites are automated edits).
    backup: bool,
    locks: &'a NoteLocks,
}

impl<'a> NoteRename<'a> {
    pub(crate) fn new(
        index: &'a NoteIndex,
        workspace_path: &'a SystemPath,
        backup: bool,
        locks: &'a NoteLocks,
    ) -> Self {
        Self {
            index,
            workspace_path,
            backup,
            locks,
        }
    }

    /// Renames the note `from` to `to`, rewriting links to it (wikilinks,
    /// Markdown links, and the note's own self-links) in every backlinking
    /// note so they keep pointing at the renamed note. Fails if `to` already
    /// exists. See the module docs for the stage order and what each failure
    /// leaves behind.
    ///
    /// Paths are flattened internally, so callers may pass them as-is.
    pub(crate) async fn rename(self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();
        let index = self.index;
        let workspace_path = self.workspace_path;
        let locks = self.locks;

        let scouted = self.scout(&from, &to).await?;
        let _guards = locks
            .lock_notes(
                std::iter::once(&from)
                    .chain(std::iter::once(&to))
                    .chain(scouted.victims().iter()),
            )
            .await;

        let prepared = scouted.prepare().await?;

        nfs::rename_note(workspace_path, &from, &to)
            .await
            .map_err(rename_dest_err)?;

        let notes_with_text = prepared.commit().await?;

        index.rename_note(&from, &to, &notes_with_text).await?;

        Ok(())
    }

    /// Stage 1: query the index once for the notes linking to `from`. The
    /// source itself is excluded — its self-links are rewritten at the new
    /// path during [`Prepared::commit`], never written back to `from` (which
    /// would resurrect a file at the old path).
    async fn scout(self, from: &VaultPath, to: &VaultPath) -> Result<Scouted<'a>, VaultError> {
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

/// Maps a "destination exists" filesystem error into the vault-level
/// "destination path already exists" error every rename reports.
pub(crate) fn rename_dest_err(e: FSError) -> VaultError {
    match e {
        FSError::AlreadyExists { path } => VaultError::FSError(FSError::InvalidPath {
            path: path.to_string(),
            message: "Destination path already exists".to_string(),
        }),
        other => VaultError::FSError(other),
    }
}

/// Stage 1 output: the victim list is known; nothing has been read or
/// written. The victims (plus source and destination) are locked before
/// [`prepare`](Self::prepare).
struct Scouted<'a> {
    base: NoteRename<'a>,
    from: VaultPath,
    to: VaultPath,
    victims: Vec<VaultPath>,
}

impl<'a> Scouted<'a> {
    /// The notes whose links will be rewritten. For lock acquisition.
    fn victims(&self) -> &[VaultPath] {
        &self.victims
    }

    /// Stage 3: read every victim and rewrite its links to `from` in memory,
    /// keeping only the ones whose content actually changed; then take a
    /// fail-closed backup of each changed victim plus the source (its
    /// self-links are rewritten at the new path during commit). I/O is
    /// concurrency-bounded. No filesystem mutation happens here — a failure
    /// aborts the rename cleanly.
    async fn prepare(self) -> Result<Prepared<'a>, VaultError> {
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

/// Stage 3 output: every rewritten body is in memory and backed up. The
/// source note is renamed on disk next, then [`commit`](Self::commit).
struct Prepared<'a> {
    workspace_path: &'a SystemPath,
    from: VaultPath,
    to: VaultPath,
    updates: Vec<(VaultPath, String)>,
}

impl Prepared<'_> {
    /// Stage 5: write the rewritten victims (concurrency-bounded, each
    /// file's text moved into its task without cloning), then rewrite any
    /// self-links inside the renamed note at its new location. Returns the
    /// updated `(entry, text)` pairs ready for the index commit.
    async fn commit(self) -> Result<Vec<(NoteEntryData, String)>, VaultError> {
        let Self {
            workspace_path,
            from,
            to,
            updates,
        } = self;

        let mut out = run_bounded(updates.into_iter().map(|(path, text)| async move {
            // The written body, not `text`: a CRLF note is stored with the
            // endings it had, and what gets indexed has to be what is on disk.
            let (entry, written) = nfs::save_note(workspace_path, &path, &text).await?;
            Ok((entry, written))
        }))
        .await?;

        // Self-links inside the renamed file, rewritten at its new location.
        let text = nfs::load_note(workspace_path, &to).await?;
        let (updated, changed) = note::replace_note_links(&text, &from, &to);
        if changed {
            let (entry, written) = nfs::save_note(workspace_path, &to, &updated).await?;
            out.push((entry, written));
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

#[cfg(test)]
mod tests {
    use super::NoteRename;
    use crate::index::NoteIndex;
    use crate::nfs::{self, VaultPath};
    use crate::note::NoteDetails;
    use crate::note_locks::NoteLocks;
    use crate::system::{sys, SystemPath};
    use crate::IndexFile;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        ws: SystemPath,
        index: NoteIndex,
        locks: NoteLocks,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let ws = sys(dir.path());
            let index = NoteIndex::open(&IndexFile::legacy_in_workspace(&ws))
                .await
                .unwrap();
            Self {
                _dir: dir,
                ws,
                index,
                locks: NoteLocks::default(),
            }
        }

        /// Writes a note to disk and indexes it, the way a saved note is.
        async fn note(&self, path: &str, text: &str) -> VaultPath {
            let path = VaultPath::new(path);
            let (entry, written) = nfs::save_note(&self.ws, &path, text).await.unwrap();
            self.index
                .save_note(&entry, &NoteDetails::new(&path, written))
                .await
                .unwrap();
            path
        }

        fn rename(&self, backup: bool) -> NoteRename<'_> {
            NoteRename::new(&self.index, &self.ws, backup, &self.locks)
        }

        async fn text(&self, path: &str) -> String {
            nfs::load_note(&self.ws, &VaultPath::new(path))
                .await
                .unwrap()
        }

        async fn backlinks(&self, path: &str) -> Vec<VaultPath> {
            self.index
                .get_backlinks(&VaultPath::new(path))
                .await
                .unwrap()
                .into_iter()
                .map(|(e, _)| e.path)
                .collect()
        }
    }

    #[tokio::test]
    async fn rename_moves_the_note_and_rewrites_victims_self_links_and_index() {
        let f = Fixture::new().await;
        let target = f
            .note("/target.md", "# Target\nSee [[target]] myself.")
            .await;
        f.note("/referrer.md", "See [[target]] and [link](target.md).")
            .await;
        f.note("/unrelated.md", "Nothing to see [[other]].").await;

        f.rename(false)
            .rename(&target, &VaultPath::new("/renamed.md"))
            .await
            .unwrap();

        assert!(!f.ws.join("target.md").exists());
        assert_eq!(
            f.text("/renamed.md").await,
            "# Target\nSee [[renamed]] myself."
        );
        assert_eq!(
            f.text("/referrer.md").await,
            "See [[renamed]] and [link](renamed.md)."
        );
        assert_eq!(f.text("/unrelated.md").await, "Nothing to see [[other]].");

        assert!(f.backlinks("/target.md").await.is_empty());
        let mut linkers = f.backlinks("/renamed.md").await;
        linkers.sort();
        assert_eq!(
            linkers,
            vec![
                VaultPath::new("/referrer.md"),
                VaultPath::new("/renamed.md")
            ]
        );
    }

    fn backups_dir(ws: &SystemPath) -> std::path::PathBuf {
        ws.as_path().join(".kimun").join("backups")
    }

    #[tokio::test]
    async fn rename_backs_up_victims_and_source_before_touching_disk() {
        let f = Fixture::new().await;
        let target = f.note("/target.md", "I am target").await;
        f.note("/referrer.md", "See [[target]].").await;
        f.note("/unrelated.md", "See [[other]].").await;

        f.rename(true)
            .rename(&target, &VaultPath::new("/renamed.md"))
            .await
            .unwrap();

        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let day = backups_dir(&f.ws).join(date);
        assert_eq!(
            std::fs::read_to_string(day.join("referrer.md")).unwrap(),
            "See [[target]]."
        );
        assert_eq!(
            std::fs::read_to_string(day.join("target.md")).unwrap(),
            "I am target"
        );
        assert!(
            !day.join("unrelated.md").exists(),
            "a note whose links did not change is not backed up"
        );
    }

    #[tokio::test]
    async fn rename_with_backups_off_writes_no_backup() {
        let f = Fixture::new().await;
        let target = f.note("/target.md", "I am target").await;
        f.note("/referrer.md", "See [[target]].").await;

        f.rename(false)
            .rename(&target, &VaultPath::new("/renamed.md"))
            .await
            .unwrap();

        assert!(!backups_dir(&f.ws).exists());
    }

    #[tokio::test]
    async fn rename_to_existing_destination_fails_leaving_victims_and_index_untouched() {
        let f = Fixture::new().await;
        let target = f.note("/target.md", "I am target").await;
        f.note("/taken.md", "already here").await;
        f.note("/referrer.md", "See [[target]].").await;

        let err = f
            .rename(false)
            .rename(&target, &VaultPath::new("/taken.md"))
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("already exists"),
            "unexpected error: {err}"
        );
        assert_eq!(f.text("/target.md").await, "I am target");
        assert_eq!(f.text("/taken.md").await, "already here");
        assert_eq!(f.text("/referrer.md").await, "See [[target]].");
        assert_eq!(
            f.backlinks("/target.md").await,
            vec![VaultPath::new("/referrer.md")]
        );
    }

    #[tokio::test]
    async fn unreadable_victim_aborts_before_any_disk_change() {
        let f = Fixture::new().await;
        let target = f.note("/target.md", "I am target").await;
        f.note("/referrer.md", "See [[target]].").await;
        f.note("/other.md", "Also [[target]].").await;
        // The index still lists referrer as a victim, but on disk it is now a
        // directory: reading it during prepare fails.
        let referrer = f.ws.as_path().join("referrer.md");
        std::fs::remove_file(&referrer).unwrap();
        std::fs::create_dir(&referrer).unwrap();

        f.rename(true)
            .rename(&target, &VaultPath::new("/renamed.md"))
            .await
            .unwrap_err();

        assert_eq!(f.text("/target.md").await, "I am target");
        assert!(!f.ws.as_path().join("renamed.md").exists());
        assert_eq!(f.text("/other.md").await, "Also [[target]].");
        assert!(
            !backups_dir(&f.ws).exists(),
            "no backup is taken when prepare aborts"
        );
        let mut linkers = f.backlinks("/target.md").await;
        linkers.sort();
        assert_eq!(
            linkers,
            vec![VaultPath::new("/other.md"), VaultPath::new("/referrer.md")]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unwritable_victim_fails_after_the_move_with_the_index_still_at_the_old_path() {
        use std::os::unix::fs::PermissionsExt;

        let f = Fixture::new().await;
        let target = f.note("/target.md", "I am target").await;
        f.note("/referrer.md", "See [[target]].").await;
        let referrer = f.ws.as_path().join("referrer.md");
        std::fs::set_permissions(&referrer, std::fs::Permissions::from_mode(0o444)).unwrap();

        let result = f
            .rename(false)
            .rename(&target, &VaultPath::new("/renamed.md"))
            .await;
        // Restore so the temp dir can be cleaned up even if the assertions fail.
        std::fs::set_permissions(&referrer, std::fs::Permissions::from_mode(0o644)).unwrap();
        result.unwrap_err();

        // The move happened; the victim keeps its old link; the index was not
        // committed, so it still describes the old path.
        assert!(!f.ws.as_path().join("target.md").exists());
        assert_eq!(f.text("/renamed.md").await, "I am target");
        assert_eq!(f.text("/referrer.md").await, "See [[target]].");
        assert_eq!(
            f.backlinks("/target.md").await,
            vec![VaultPath::new("/referrer.md")]
        );
        assert!(f.backlinks("/renamed.md").await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_renames_sharing_a_victim_both_land() {
        let f = Fixture::new().await;
        let a = f.note("/a.md", "A").await;
        let b = f.note("/b.md", "B").await;
        f.note("/hub.md", "[[a]] and [[b]]").await;

        let a2 = VaultPath::new("/a2.md");
        let b2 = VaultPath::new("/b2.md");
        let (ra, rb) = tokio::join!(
            f.rename(false).rename(&a, &a2),
            f.rename(false).rename(&b, &b2),
        );
        ra.unwrap();
        rb.unwrap();

        assert_eq!(f.text("/hub.md").await, "[[a2]] and [[b2]]");
        assert_eq!(f.backlinks("/a2.md").await, vec![VaultPath::new("/hub.md")]);
        assert_eq!(f.backlinks("/b2.md").await, vec![VaultPath::new("/hub.md")]);
    }
}
