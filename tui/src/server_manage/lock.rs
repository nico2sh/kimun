//! One update at a time per install folder.
//!
//! The staged file and the backup have fixed names, so two updates that run together would
//! overwrite each other's files. This lock is a file made with `create_new`, which only one
//! caller can win. It guards against another window, the terminal app, or a second app process.
//! A lock older than [`STALE`] belongs to a crashed update and is taken over.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use kimun_core::system;

const LOCK_FILE: &str = ".kimun-server.update.lock";
/// An update takes minutes at most. A lock older than this was left by a crash.
const STALE: Duration = Duration::from_secs(15 * 60);

/// Held while an update runs. Dropping it releases the lock, but only if the file is still ours.
#[derive(Debug)]
pub struct UpdateLock {
    path: PathBuf,
    token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// Another update holds the lock.
    Busy,
    /// The lock file could not be made.
    Io(String),
}

/// Text that no other holder writes: the process id and a counter.
fn new_token() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

impl UpdateLock {
    /// Take the lock for the install folder `dir`.
    pub fn acquire(dir: &Path) -> Result<Self, LockError> {
        Self::acquire_with_age_limit(dir, STALE)
    }

    fn acquire_with_age_limit(dir: &Path, stale: Duration) -> Result<Self, LockError> {
        let path = dir.join(LOCK_FILE);
        let token = new_token();
        for attempt in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(token.as_bytes())
                        .map_err(|e| LockError::Io(e.to_string()))?;
                    return Ok(Self { path, token });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt == 0 && is_stale(&path, stale) {
                        // Claim the stale file by renaming it away. A rename is atomic: of two
                        // callers that both saw the stale file, one renames it and the other finds
                        // nothing to rename. Only a caller that won may go on to create a new lock.
                        let claimed = dir.join(format!("{LOCK_FILE}.{token}.stale"));
                        if system::move_file(&path, &claimed).is_ok() {
                            let _ = system::remove_file(&claimed);
                        }
                        continue;
                    }
                    return Err(LockError::Busy);
                }
                Err(e) => return Err(LockError::Io(e.to_string())),
            }
        }
        Err(LockError::Busy)
    }
}

fn is_stale(path: &Path, limit: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age >= limit)
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        // A holder whose lock was taken over (it ran past the age limit) must not delete the new
        // owner's lock.
        if std::fs::read_to_string(&self.path).is_ok_and(|text| text == self.token) {
            let _ = system::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_holder_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let first = UpdateLock::acquire(dir.path()).unwrap();
        assert_eq!(
            UpdateLock::acquire(dir.path()).unwrap_err(),
            LockError::Busy
        );
        drop(first);
        assert!(UpdateLock::acquire(dir.path()).is_ok(), "released on drop");
    }

    #[test]
    fn a_lock_left_by_a_crash_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let first = UpdateLock::acquire(dir.path()).unwrap();
        std::mem::forget(first); // a crash: no drop, the file stays
        assert_eq!(
            UpdateLock::acquire(dir.path()).unwrap_err(),
            LockError::Busy
        );
        assert!(UpdateLock::acquire_with_age_limit(dir.path(), Duration::ZERO).is_ok());
    }

    #[test]
    fn a_holder_that_lost_its_lock_does_not_delete_the_new_owners() {
        let dir = tempfile::tempdir().unwrap();
        let first = UpdateLock::acquire(dir.path()).unwrap();
        // The first holder ran past the age limit, and a second took the lock over.
        let second = UpdateLock::acquire_with_age_limit(dir.path(), Duration::ZERO).unwrap();
        drop(first);
        assert_eq!(
            UpdateLock::acquire(dir.path()).unwrap_err(),
            LockError::Busy,
            "the second still holds it"
        );
        drop(second);
        assert!(UpdateLock::acquire(dir.path()).is_ok());
    }

    #[test]
    fn a_missing_folder_is_an_io_error() {
        assert!(matches!(
            UpdateLock::acquire(Path::new("/no/such/dir")),
            Err(LockError::Io(_))
        ));
    }
}
