//! The index as a *file on the host*, rather than as an open connection pool.
//!
//! An index is not one file. SQLite runs in WAL mode here, so a live (or
//! uncleanly closed) index is `<name>.kimuncache` plus `-wal` and `-shm`
//! siblings, and moving or deleting only the first one either orphans the rest
//! or throws away the transactions the WAL still holds. That is knowledge
//! about *this* artifact, so it lives next to the index rather than in
//! whichever caller happens to be renaming a workspace.

use crate::system::{self, SystemError, SystemPath};

/// Extension of a workspace's index file.
///
/// Deliberately not `.sqlite`: the file is a rebuildable cache, and the name
/// should say so to anyone who finds one in a directory listing.
const INDEX_FILE_EXT: &str = "kimuncache";

/// Suffixes SQLite may keep beside an index file. `-wal` and `-shm` are the
/// WAL-mode pair; `-journal` is the rollback-mode equivalent, kept here so a
/// future change of journal mode cannot silently strand a file.
const SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// A workspace's index file on this machine — the whole artifact, sidecars
/// included.
///
/// Holding one says nothing about whether it exists or is open. What it does
/// say is that every operation on it treats the index as one unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexFile {
    path: SystemPath,
}

impl IndexFile {
    /// The index for `workspace_name` inside `dir`.
    ///
    /// The naming rule lives here so a workspace resolves to the same file
    /// from every caller. `workspace_name` must already be a valid filename
    /// (see [`crate::nfs::filename::validate_filename`]) — this joins, it does
    /// not sanitize.
    pub fn in_dir(dir: &SystemPath, workspace_name: &str) -> Self {
        Self {
            path: dir.join(format!("{workspace_name}.{INDEX_FILE_EXT}")),
        }
    }

    /// An index at an explicit path — for callers that were handed a path
    /// rather than a workspace name.
    pub fn at(path: SystemPath) -> Self {
        Self { path }
    }

    /// The pre-cache-directory location: an index sitting inside the vault it
    /// indexes. Still the default when [`VaultConfig`](crate::VaultConfig)
    /// names no index, and what the v2 → v3 config migration moves out.
    pub fn legacy_in_workspace(workspace_path: &SystemPath) -> Self {
        Self {
            path: workspace_path.join(super::DB_FILE),
        }
    }

    /// The main file's path. The sidecars are derived from it and are not
    /// part of any caller's business.
    pub fn path(&self) -> &SystemPath {
        &self.path
    }

    /// Whether the main file exists. A missing index is a normal state (a
    /// workspace that has never been opened), not an error.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// The sidecars that currently exist beside the main file, each paired
    /// with the suffix that named it.
    ///
    /// Empty for a cleanly closed index: SQLite checkpoints and removes the
    /// WAL pair when the last connection closes. A non-empty result means the
    /// index is open right now, or the process that held it died.
    ///
    /// The suffix is carried rather than re-derived from the path: [`move_to`]
    /// needs it to name the destination, and recovering it by comparing
    /// formatted strings has a failure mode ("no suffix") that maps a WAL file
    /// onto the destination's *main* path.
    ///
    /// [`move_to`]: IndexFile::move_to
    fn existing_sidecars(&self) -> Vec<(&'static str, SystemPath)> {
        SIDECAR_SUFFIXES
            .iter()
            .map(|suffix| (*suffix, self.path.with_name_suffix(suffix)))
            .filter(|(_, path)| path.exists())
            .collect()
    }

    /// Moves this index — main file and sidecars — to `dest`.
    ///
    /// Refuses rather than overwrites: an existing destination index, or any
    /// destination sidecar, aborts before anything moves. If a sidecar move
    /// fails partway, the main file is moved back, so a failure leaves the
    /// source index whole instead of split across two directories.
    ///
    /// A no-op only when there is genuinely nothing here — no main file *and*
    /// no sidecars. A stranded WAL with no index beside it (a cleanup that
    /// failed halfway) still moves: leaving it behind would let the next
    /// workspace of the same name adopt another database's transactions.
    ///
    /// The caller must have closed any [`NoteVault`](crate::NoteVault) holding
    /// this index first: Windows refuses to move a file with an open handle,
    /// and that failure surfaces here as an ordinary I/O error.
    pub fn move_to(&self, dest: &IndexFile) -> Result<(), SystemError> {
        let sidecars = self.existing_sidecars();
        if !self.exists() && sidecars.is_empty() {
            return Ok(());
        }
        if dest.exists() {
            return Err(SystemError::AlreadyExists {
                path: dest.path.to_string(),
            });
        }
        for suffix in SIDECAR_SUFFIXES {
            let occupied = dest.path.with_name_suffix(suffix);
            if occupied.exists() {
                return Err(SystemError::AlreadyExists {
                    path: occupied.to_string(),
                });
            }
        }

        let mut plan = Vec::new();
        if self.exists() {
            plan.push((self.path.clone(), dest.path.clone()));
        }
        for (suffix, source) in sidecars {
            plan.push((source, dest.path.with_name_suffix(suffix)));
        }
        move_all(&plan)
    }

    /// Deletes this index and its sidecars, returning each file that would not
    /// go along with the reason.
    ///
    /// A missing file is not an error — the point is that nothing is left
    /// behind. Neither is a file that will not delete a reason to stop: every
    /// remaining file is still attempted. Returning on the first failure left
    /// the deletable files on disk and reported only the one that blocked, so
    /// the usual Windows case — a handle still on the `-wal` — told the user
    /// about one file while silently keeping three.
    pub fn remove(&self) -> Vec<(SystemPath, SystemError)> {
        let mut stuck = Vec::new();
        for (_, sidecar) in self.existing_sidecars() {
            if let Err(e) = system::remove_file(sidecar.as_path()) {
                stuck.push((sidecar, e));
            }
        }
        if self.exists() {
            if let Err(e) = system::remove_file(self.path.as_path()) {
                stuck.push((self.path.clone(), e));
            }
        }
        stuck
    }
}

impl std::fmt::Display for IndexFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)
    }
}

/// Moves every pair in order, undoing the ones already done if one fails.
///
/// All-or-nothing is the point: an index whose WAL stayed behind has silently
/// lost its most recent transactions, and one whose main file moved without
/// the WAL is worse — it looks complete. Rollback is best-effort (a failing
/// undo leaves the original error, which is the one worth reporting).
fn move_all(pairs: &[(SystemPath, SystemPath)]) -> Result<(), SystemError> {
    let mut done = Vec::new();
    for (from, to) in pairs {
        match system::move_file(from.as_path(), to.as_path()) {
            Ok(()) => done.push((from, to)),
            Err(e) => {
                for (from, to) in done.iter().rev() {
                    let _ = system::move_file(to.as_path(), from.as_path());
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::sys;

    fn index_in(dir: &tempfile::TempDir, name: &str) -> IndexFile {
        IndexFile::in_dir(&sys(dir.path()), name)
    }

    fn write(path: &SystemPath, body: &str) {
        std::fs::write(path.as_path(), body).unwrap();
    }

    #[test]
    fn in_dir_names_the_file_after_the_workspace() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = index_in(&dir, "work");
        assert_eq!(
            index.path().as_path().file_name().unwrap(),
            "work.kimuncache"
        );
    }

    #[test]
    fn move_takes_the_sidecars_along() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = index_in(&dir, "old");
        let to = index_in(&dir, "new");
        write(from.path(), "index");
        write(&from.path().with_name_suffix("-wal"), "wal");
        write(&from.path().with_name_suffix("-shm"), "shm");

        from.move_to(&to).unwrap();

        assert!(!from.exists());
        assert!(!from.path().with_name_suffix("-wal").exists());
        assert!(!from.path().with_name_suffix("-shm").exists());
        assert_eq!(
            std::fs::read_to_string(to.path().as_path()).unwrap(),
            "index"
        );
        assert_eq!(
            std::fs::read_to_string(to.path().with_name_suffix("-wal").as_path()).unwrap(),
            "wal"
        );
        assert_eq!(
            std::fs::read_to_string(to.path().with_name_suffix("-shm").as_path()).unwrap(),
            "shm"
        );
    }

    #[test]
    fn move_of_a_missing_index_is_a_no_op() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = index_in(&dir, "absent");
        let to = index_in(&dir, "new");

        from.move_to(&to).unwrap();

        assert!(!to.exists());
    }

    /// A WAL with no index beside it — a `remove` that deleted the sidecars
    /// and then failed on the main file, or the inverse. Leaving it behind
    /// lets the next workspace of the same name adopt another database's
    /// transactions, so the move takes it even with nothing to lead it.
    #[test]
    fn move_takes_a_stranded_sidecar_with_no_main_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = index_in(&dir, "old");
        let to = index_in(&dir, "new");
        write(&from.path().with_name_suffix("-wal"), "orphan wal");

        from.move_to(&to).unwrap();

        assert!(
            !from.path().with_name_suffix("-wal").exists(),
            "the stale WAL must not stay under the old name"
        );
        assert_eq!(
            std::fs::read_to_string(to.path().with_name_suffix("-wal").as_path()).unwrap(),
            "orphan wal"
        );
        assert!(!to.exists(), "no main file was invented");
    }

    /// Every sidecar keeps its own suffix. Deriving it by comparing formatted
    /// strings had a "no match" case that named the destination's *main* path,
    /// which would rename the WAL over the index that had just landed there.
    #[test]
    fn each_sidecar_keeps_its_suffix_across_a_move() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = index_in(&dir, "old");
        let to = index_in(&dir, "new");
        write(from.path(), "index");
        for suffix in SIDECAR_SUFFIXES {
            write(&from.path().with_name_suffix(suffix), suffix);
        }

        from.move_to(&to).unwrap();

        assert_eq!(
            std::fs::read_to_string(to.path().as_path()).unwrap(),
            "index",
            "the index must not be overwritten by a sidecar"
        );
        for suffix in SIDECAR_SUFFIXES {
            assert_eq!(
                std::fs::read_to_string(to.path().with_name_suffix(suffix).as_path()).unwrap(),
                suffix
            );
        }
    }

    #[test]
    fn move_refuses_an_occupied_destination() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = index_in(&dir, "old");
        let to = index_in(&dir, "new");
        write(from.path(), "source");
        write(to.path(), "destination");

        let err = from.move_to(&to).unwrap_err();

        assert!(
            matches!(err, SystemError::AlreadyExists { .. }),
            "got {err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(to.path().as_path()).unwrap(),
            "destination",
            "an existing index must not be overwritten"
        );
        assert!(from.exists(), "source must be left alone");
    }

    /// A destination sidecar is as much of a collision as the index itself:
    /// moving on top of it would mix two indexes' WALs.
    #[test]
    fn move_refuses_an_occupied_destination_sidecar() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = index_in(&dir, "old");
        let to = index_in(&dir, "new");
        write(from.path(), "source");
        write(&to.path().with_name_suffix("-wal"), "stale wal");

        let err = from.move_to(&to).unwrap_err();

        assert!(
            matches!(err, SystemError::AlreadyExists { .. }),
            "got {err:?}"
        );
        assert!(from.exists(), "source must be left alone");
    }

    /// The rollback, exercised where it is actually reachable: the main file
    /// moves, a later file cannot, and everything goes back.
    ///
    /// Driving `move_all` directly rather than `move_to` is deliberate — a
    /// mid-sequence filesystem failure cannot be provoked from the outside
    /// without a race, and the behaviour worth pinning is "undo what already
    /// happened", not how the pairs were built.
    #[test]
    fn a_failed_move_puts_the_earlier_ones_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let main = sys(dir.path()).join("index.kimuncache");
        let wal = main.with_name_suffix("-wal");
        write(&main, "index");
        write(&wal, "wal");
        let moved_main = sys(dir.path()).join("moved.kimuncache");
        // Into a directory that does not exist: the rename fails, and it is
        // the second pair, so the first one has to be undone.
        let unreachable = sys(dir.path())
            .join("no-such-dir")
            .join("moved.kimuncache-wal");

        let result = move_all(&[
            (main.clone(), moved_main.clone()),
            (wal.clone(), unreachable),
        ]);

        assert!(result.is_err(), "the second move must fail");
        assert!(main.exists(), "index must be back where it started");
        assert!(wal.exists(), "WAL must be back where it started");
        assert!(!moved_main.exists(), "no half-moved index left behind");
        assert_eq!(std::fs::read_to_string(main.as_path()).unwrap(), "index");
    }

    #[test]
    fn remove_deletes_the_sidecars_too() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = index_in(&dir, "doomed");
        write(index.path(), "index");
        write(&index.path().with_name_suffix("-wal"), "wal");
        write(&index.path().with_name_suffix("-shm"), "shm");

        assert!(index.remove().is_empty());

        assert!(!index.exists());
        assert!(!index.path().with_name_suffix("-wal").exists());
        assert!(!index.path().with_name_suffix("-shm").exists());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn remove_of_a_missing_index_is_a_no_op() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(index_in(&dir, "absent").remove().is_empty());
    }

    /// The Windows case that matters: the handle is normally on the WAL, not
    /// on the index. Returning at the first failure left the `.kimuncache` and
    /// the `-shm` on disk untouched even though both would have deleted, and
    /// reported only the `-wal` — one file named out of three kept.
    ///
    /// A non-empty directory stands in for the lock, which cannot be provoked
    /// on demand; what is under test is that one failure does not stop the rest.
    #[test]
    fn remove_attempts_every_file_and_reports_each_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = index_in(&dir, "stuck");
        let wal = index.path().with_name_suffix("-wal");
        let shm = index.path().with_name_suffix("-shm");
        write(index.path(), "index");
        write(&shm, "shm");
        std::fs::create_dir(wal.as_path()).unwrap();
        std::fs::write(wal.as_path().join("occupied"), b"x").unwrap();

        let stuck = index.remove();

        assert!(!index.exists(), "the index itself was still deletable");
        assert!(!shm.exists(), "the -shm was still deletable");
        assert_eq!(stuck.len(), 1, "got {stuck:?}");
        assert_eq!(stuck[0].0, wal);
    }
}
