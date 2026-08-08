//! Host-scoped paths and file operations: the machine kimün runs on.
//!
//! Sibling of [`nfs`](crate::nfs), and the split between them is the whole
//! point. `nfs` is **vault-scoped** — it addresses notes with
//! [`VaultPath`](crate::nfs::VaultPath) inside one workspace directory. This
//! module is **host-scoped**: the home directory, the app's own directories,
//! and the file operations that carry OS-specific knowledge (verbatim paths,
//! cross-volume moves, atomic replace). Everything else in the workspace goes
//! through `nfs`.
//!
//! It exists because that knowledge used to be spread across four modules in
//! two crates — each holding a fragment, none holding the rule — and three
//! Windows bugs in one week landed in three different fragments.
//!
//! Two ideas carry it:
//!
//! - [`SystemPath`] is a path that is absolute and normalized. Not by
//!   convention or by comment: the type cannot be constructed otherwise, so
//!   "did anybody resolve this?" stops being a question callers can get wrong.
//! - [`Host`] makes the platform a *value* where the rule is pure policy
//!   (directory layout, error classification, executable naming), so both
//!   branches compile and are tested on every platform. Rules that need the
//!   OS itself — `std::path` parses per target — stay behind `cfg` and are
//!   verified on that OS's CI leg.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Which host's rules apply. A value, not only a `cfg`, so policy that is
/// pure computation can be exercised for both hosts from any of them.
///
/// Two variants, not one per OS: macOS follows the unix rules everywhere this
/// module cares about, and the places it genuinely differs (its data
/// directory) are not this type's business.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// Linux, macOS and the BSDs.
    Unix,
    /// Windows.
    Windows,
}

/// The host this binary was built for.
pub const HOST: Host = if cfg!(windows) {
    Host::Windows
} else {
    Host::Unix
};

/// The app's directory name, suffixed in debug builds so a development build
/// never reads or writes a real installation's config, index or logs.
const APP_DIR_NAME: &str = if cfg!(debug_assertions) {
    "kimun_debug"
} else {
    "kimun"
};

/// Where [`log_dir`] sits inside the app directory.
const LOG_DIR_NAME: &str = "logs";

/// Failures of host path resolution and host file operations.
#[derive(Debug, thiserror::Error)]
pub enum SystemError {
    /// A path that must be absolute was not, and this module refuses to make
    /// it absolute against the working directory — that is how an index ends
    /// up wherever the binary happened to be started.
    #[error("path is not absolute: {path}")]
    NotAbsolute {
        /// The offending path.
        path: String,
    },
    /// The home directory could not be determined (`HOME`/`USERPROFILE` unset).
    #[error("cannot determine the home directory")]
    NoHome,
    /// The destination of a move is occupied, and this module never
    /// overwrites: the caller decides what to do with the file already there.
    #[error("already exists: {path}")]
    AlreadyExists {
        /// The occupied path.
        path: String,
    },
    /// A file could not be renamed or deleted because something still holds it
    /// open. Windows only — unix does neither operation on the handle.
    ///
    /// Separate from [`SystemError::Io`] because the OS message ("the process
    /// cannot access the file because it is being used by another process")
    /// never says *which* process, which leaves both the user and a bug report
    /// with nothing to act on. `holders` is what the Restart Manager reports,
    /// or `"unknown"` when it will not say.
    #[error("could not {action} {path}: held open by {holders}")]
    Locked {
        /// What was being attempted, e.g. `"move"`.
        action: &'static str,
        /// The path that is held open.
        path: String,
        /// Processes holding it, as `"name (pid N)"`, comma-separated.
        holders: String,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// A filesystem call failed.
    #[error("could not {action} {path}: {source}")]
    Io {
        /// What was being attempted, e.g. `"create directory"`.
        action: &'static str,
        /// The path it was attempted on.
        path: String,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
}

impl SystemError {
    fn io(action: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            action,
            path: path_to_string(path),
            source,
        }
    }

    /// [`SystemError::Locked`] when the failure is "somebody has it open",
    /// [`SystemError::Io`] otherwise.
    ///
    /// For the operations that a held handle can block — rename and delete —
    /// so the one failure a user can actually do something about says who to
    /// close. Naming the holders costs a Restart Manager round trip, which is
    /// why it happens here, on the failure path, rather than on every call.
    fn locked_or_io(action: &'static str, path: &Path, source: std::io::Error) -> Self {
        if is_locked_for(HOST, &source) {
            Self::Locked {
                action,
                path: path_to_string(path),
                holders: describe_holders(path),
                source,
            }
        } else {
            Self::io(action, path, source)
        }
    }
}

/// Converts an OS path to a `String`, losslessly when the path is valid UTF-8
/// and lossily (replacing invalid sequences) otherwise, so it never fails.
pub fn path_to_string<P: AsRef<Path>>(path: P) -> String {
    path.as_ref()
        .to_path_buf()
        .into_os_string()
        .into_string()
        .unwrap_or_else(|os_string| os_string.to_string_lossy().into())
}

/// An OS path kimün has made usable on this machine: **absolute** and
/// **normalized** (no `.` or `..` components left).
///
/// Normalization is not cosmetic. A canonicalized Windows path is the verbatim
/// form `\\?\C:\…`, and Win32 reads every component of a verbatim path
/// literally — `\\?\C:\kimun\.` names a file called `.`, which no
/// `create_dir_all` and no SQLite open can use. Comparing two [`Path`]s hides
/// it (`PartialEq` goes through `Components`, which drops `.`), so only the
/// real filesystem call notices, on one platform, at runtime.
///
/// The invariant lives in the type because the alternative was tried: a
/// `PathBuf` that callers were told to resolve first, with a silent fallback
/// when they didn't.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SystemPath(PathBuf);

impl SystemPath {
    /// Accepts an already-absolute path, normalizing it.
    ///
    /// Rejects a relative path rather than resolving it against the process's
    /// working directory: that would make the result depend on where the
    /// binary was launched, which is the failure this type exists to prevent.
    /// Callers holding a relative path want [`SystemPath::resolve`], which
    /// makes them name the base.
    pub fn try_absolute<P: AsRef<Path>>(path: P) -> Result<Self, SystemError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(SystemError::NotAbsolute {
                path: path_to_string(path),
            });
        }
        Ok(Self(normalize(path)))
    }

    /// Resolves a path that may be relative, may start with `~`, and may not
    /// exist yet, against `base`.
    ///
    /// `~` expands to the home directory (left as-is when there is none),
    /// relative paths resolve against `base`, `.`/`..` are removed, and the
    /// result is canonicalized when it already exists on disk — the canonical
    /// form is what later `starts_with` comparisons and the index's stored
    /// paths must agree with.
    pub fn resolve<P: AsRef<Path>>(path: P, base: &SystemPath) -> Self {
        let path = path.as_ref();
        let text = path.to_string_lossy();
        let expanded = if text.starts_with("~/") || text == "~" {
            match home() {
                Ok(home) => home.0.join(text.strip_prefix("~/").unwrap_or("")),
                Err(_) => path.to_path_buf(),
            }
        } else {
            path.to_path_buf()
        };
        let absolute = if expanded.is_relative() {
            base.0.join(expanded)
        } else {
            expanded
        };
        let absolute = normalize(&absolute);
        Self(absolute.canonicalize().unwrap_or(absolute))
    }

    /// Resolves an existing path to its canonical form — symlinks followed,
    /// `.`/`..` gone, absolute.
    ///
    /// Fails when the path does not exist: canonicalization is the OS
    /// answering "what is this really", and there is no answer for something
    /// that is not there. Callers resolving a path that may not exist yet want
    /// [`SystemPath::resolve`].
    pub fn canonical<P: AsRef<Path>>(path: P) -> Result<Self, SystemError> {
        let path = path.as_ref();
        let canonical = path
            .canonicalize()
            .map_err(|e| SystemError::io("resolve", path, e))?;
        Self::try_absolute(canonical)
    }

    /// This path with `segment` appended, staying normalized (so a `..`
    /// segment cancels rather than accumulating).
    pub fn join<S: AsRef<Path>>(&self, segment: S) -> Self {
        Self(normalize(&self.0.join(segment)))
    }

    /// This path with `suffix` appended to its file name — `index.db` plus
    /// `-wal` is `index.db-wal`, not `index.db/-wal` and not `index-wal.db`.
    /// How SQLite names the files beside a database.
    pub fn with_name_suffix(&self, suffix: &str) -> Self {
        let mut name = self.0.as_os_str().to_os_string();
        name.push(suffix);
        Self(PathBuf::from(name))
    }

    /// The containing directory, or `None` at the root.
    pub fn parent(&self) -> Option<Self> {
        self.0.parent().map(|p| Self(p.to_path_buf()))
    }

    /// Borrows the underlying OS path, for the standard library and for
    /// callers that take `AsRef<Path>`.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Consumes this path, yielding the [`PathBuf`].
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    /// Whether something exists at this path.
    pub fn exists(&self) -> bool {
        self.0.exists()
    }

    /// Whether this path names an existing directory.
    pub fn is_dir(&self) -> bool {
        self.0.is_dir()
    }
}

impl AsRef<Path> for SystemPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for SystemPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

/// Removes `.` components and resolves `..` lexically, without touching the
/// filesystem.
///
/// Purely lexical, so `a/../b` and a symlinked `a` can disagree with the
/// filesystem — callers that need the on-disk truth canonicalize afterwards;
/// this only makes the path nameable first. `..` at the root is dropped (the
/// root's parent is the root); in a relative path with nothing left to cancel
/// it is kept.
///
/// Not parameterized by [`Host`]: `std::path` parses separators and prefixes
/// for the *build target*, so running this with `Host::Windows` on Linux would
/// only mislead — `C:\a\.\b` is a single component there. It is verified
/// against real Windows paths on the Windows CI leg.
fn normalize(path: &Path) -> PathBuf {
    // "Rooted" means there is a root above which `..` cannot climb — a
    // `RootDir`, on its own or behind a Windows prefix. A bare prefix is *not*
    // one: `C:..\a` is drive-relative, so its `..` still has a directory to
    // climb out of and must be kept, exactly as in a relative path.
    let mut components = path.components();
    let rooted = match components.next() {
        Some(Component::RootDir) => true,
        Some(Component::Prefix(_)) => components.next() == Some(Component::RootDir),
        _ => false,
    };
    let mut out = PathBuf::new();
    // Named components `..` is allowed to cancel. Anything else already in
    // `out` (a prefix, a root, a kept `..`) must survive a `pop`.
    let mut cancellable = 0usize;
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if cancellable > 0 {
                    out.pop();
                    cancellable -= 1;
                } else if !rooted {
                    out.push("..");
                }
            }
            Component::Normal(name) => {
                out.push(name);
                cancellable += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

// ---------------------------------------------------------------------------
// Layout — where kimün keeps its own files
// ---------------------------------------------------------------------------

/// The user's home directory (`HOME`, then `USERPROFILE`).
pub fn home() -> Result<SystemPath, SystemError> {
    home_from([std::env::var_os("HOME"), std::env::var_os("USERPROFILE")])
}

/// The first candidate that is a usable home directory.
///
/// Falls through on a candidate that is *set but unusable*, not only on one
/// that is unset. `HOME` is set on Windows under Git Bash, MSYS2 and Cygwin —
/// to a unix-shaped path (`/c/Users/bob`) that carries no drive prefix and so
/// is not absolute there. Taking the first variable that merely *exists* meant
/// that path went to [`SystemPath::try_absolute`], failed, and returned an
/// error with `USERPROFILE` sitting right there unread: [`app_dir`] could not
/// resolve, so `AppSettings::load` failed and the app would not start at all.
/// An empty string falls through for the same reason.
///
/// Takes the candidates as an argument because the failure is unreachable on
/// the CI runner, whose `HOME` is always absolute.
fn home_from<const N: usize>(
    candidates: [Option<std::ffi::OsString>; N],
) -> Result<SystemPath, SystemError> {
    candidates
        .into_iter()
        .flatten()
        .filter(|raw| !raw.is_empty())
        .find_map(|raw| SystemPath::try_absolute(PathBuf::from(raw)).ok())
        .ok_or(SystemError::NoHome)
}

/// kimün's own directory under `home`, by `host`'s convention.
///
/// Takes both as parameters so the layout rule is testable for either host
/// from either host — the whole reason [`Host`] is a value.
pub fn app_dir_under(home: &SystemPath, host: Host) -> SystemPath {
    match host {
        // `~/.config/<app>`, the XDG-shaped location this app has always used
        // on unix — including macOS, where moving it to
        // `~/Library/Application Support` would strand every existing config.
        Host::Unix => home.join(".config").join(APP_DIR_NAME),
        // `%USERPROFILE%\<app>`.
        Host::Windows => home.join(APP_DIR_NAME),
    }
}

/// kimün's directory on this machine — config, per-workspace index caches and
/// history live here, and [`log_dir`] is under it.
///
/// One directory, one rule. It used to be two: the config directory and the
/// log directory disagreed on macOS (`~/.config/kimun` against
/// `~/Library/Application Support/kimun`) because two modules each answered
/// the question once.
pub fn app_dir() -> Result<SystemPath, SystemError> {
    Ok(app_dir_under(&home()?, HOST))
}

/// Where to open a directory browser when the caller has no better candidate:
/// the home directory, or the working directory when there is none.
///
/// Here rather than at the call site because the obvious literal is wrong on
/// one host: `Path::new("/")` is *not* absolute on Windows — it carries no
/// drive prefix — so [`SystemPath`] refuses it and the browser opens on an
/// empty listing with nothing to navigate to.
pub fn browse_root() -> Result<SystemPath, SystemError> {
    match home() {
        Ok(home) => Ok(home),
        Err(_) => {
            let cwd = std::env::current_dir()
                .map_err(|e| SystemError::io("resolve", Path::new("."), e))?;
            SystemPath::try_absolute(cwd)
        }
    }
}

/// [`app_dir`], created if absent.
pub fn ensure_app_dir() -> Result<SystemPath, SystemError> {
    let dir = app_dir()?;
    ensure_dir(&dir)?;
    Ok(dir)
}

/// Where the log file is written.
///
/// Infallible: logging must not be what stops the app from starting. Without a
/// home directory it falls back to the temp directory — absolute and
/// process-independent, unlike a relative path, which would scatter logs
/// wherever the binary was launched.
pub fn log_dir() -> SystemPath {
    match app_dir() {
        Ok(dir) => dir.join(LOG_DIR_NAME),
        Err(_) => log_dir_in_temp(
            &std::env::temp_dir(),
            &std::env::current_dir().unwrap_or_default(),
        ),
    }
}

/// kimün's log directory under `temp`, anchored to `cwd` if `temp` is relative.
///
/// Split out from [`log_dir`] and parameterized because the branch is
/// unreachable in a test run (CI always has a home) and the input is hostile:
/// `std::env::temp_dir` hands back `$TMPDIR` **verbatim** on unix, so it can be
/// relative (`TMPDIR=./tmp`) or carry `.` components (`TMPDIR=/var/tmp/.`) —
/// exactly the two things [`SystemPath`] must never hold. Building one straight
/// from the join bypassed both checks.
///
/// The result can only fail to be absolute if `$TMPDIR` is relative *and* the
/// process has no working directory, at which point logging is not the problem.
fn log_dir_in_temp(temp: &Path, cwd: &Path) -> SystemPath {
    let candidate = temp.join(APP_DIR_NAME).join(LOG_DIR_NAME);
    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        cwd.join(candidate)
    };
    SystemPath(normalize(&absolute))
}

// ---------------------------------------------------------------------------
// Operations — the ones that carry OS-specific knowledge
// ---------------------------------------------------------------------------

/// Creates `dir` and any missing parents. Succeeds if it already exists as a
/// directory; a *file* sitting at `dir` is an error, not a success — the caller
/// asked for somewhere to put things.
///
/// The guard is `is_dir`, not `exists`: skipping the call for anything that
/// exists reports success for a regular file, and the failure only surfaces
/// later, somewhere else, as "not a directory".
pub fn ensure_dir(dir: &SystemPath) -> Result<(), SystemError> {
    if !dir.is_dir() {
        std::fs::create_dir_all(dir)
            .map_err(|e| SystemError::io("create directory", dir.as_path(), e))?;
    }
    Ok(())
}

/// Creates the directory at `path` if absent and returns it as a
/// [`SystemPath`] — canonicalized, since it now exists.
///
/// The entry point for a directory the user named (a workspace, a cache
/// directory): it may be relative to the working directory *at the moment the
/// user typed it*, which is the one place resolving against the cwd is what
/// the user meant.
///
/// A file already at `path` is an error, for the reason given on
/// [`ensure_dir`]: `canonical` succeeds on a regular file, so an `exists` guard
/// would hand the caller a "directory" it cannot list.
pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<SystemPath, SystemError> {
    let path = path.as_ref();
    if !path.is_dir() {
        std::fs::create_dir_all(path).map_err(|e| SystemError::io("create directory", path, e))?;
    }
    SystemPath::canonical(path)
}

/// Whether a failed rename or delete means "somebody still holds this file
/// open" — a condition that passes on its own, unlike every other failure.
///
/// Only Windows has one: unix renames and unlinks a file whatever handles are
/// open on it, so there is nothing there to wait for. Windows refuses, and the
/// handle routinely outlives the code that owned it — closing a SQLite pool
/// returns before the OS has finished releasing the database file, and the
/// search indexer and antivirus both open files they find, unasked. That is
/// what made `workspace rename` fail on roughly one CI run in two with
/// `ERROR_SHARING_VIOLATION` on an index that had already been closed.
///
/// `ERROR_ACCESS_DENIED` is deliberately *not* here: it is also what a genuine
/// permissions problem returns, and waiting a second before reporting one
/// helps nobody. A file that is merely open reports 32 or 33.
///
/// Selected by `host` rather than by `cfg`, like [`is_cross_device_for`], so
/// both answers are exercised from whichever platform runs the suite.
pub fn is_locked_for(host: Host, err: &std::io::Error) -> bool {
    match host {
        Host::Unix => false,
        Host::Windows => matches!(
            err.raw_os_error(),
            Some(32) | // ERROR_SHARING_VIOLATION
            Some(33) //  ERROR_LOCK_VIOLATION
        ),
    }
}

/// How long to wait, in total, for another handle to let go of a file.
///
/// Measured, after three wrong guesses. Raising the ceiling to 60s to find the
/// shape of the wait produced the answer that matters: a lock either clears
/// within a second or two, or it does not clear at all — one CI run held an
/// index for the full sixty seconds and was still holding it at the end.
///
/// So a long ceiling buys nothing. It cannot rescue the second case, and the
/// first is over almost immediately; all it does is make a doomed
/// `workspace rename` hang for a minute before failing. Three seconds covers
/// every wait yet observed to succeed (the longest was ~2.7s) and gives up
/// promptly on the ones that never will.
///
/// The permanent case is a separate, unsolved bug — something on Windows keeps
/// a handle on a closed SQLite index. It is not a descriptor kimün holds: the
/// pool close is synchronous (sqlx drops the connection before acking
/// shutdown), and probing `/proc/self/fd` after `close()` across 25 rounds
/// found nothing open. [`retry_while_locked`] logs the waits so the next
/// investigation starts from data.
const LOCK_RETRY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// The gap between attempts, doubling from a millisecond up to this ceiling —
/// so a lock that clears immediately costs a millisecond, and a long one is
/// not polled thousands of times.
const LOCK_RETRY_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_millis(500);

/// A wait longer than this is worth a log line: below it the retry is doing its
/// job invisibly, above it something held the file long enough to be worth
/// knowing about.
const LOCK_RETRY_LOG_THRESHOLD: std::time::Duration = std::time::Duration::from_millis(500);

/// Runs `op`, retrying while it fails only because something still holds the
/// file open (see [`is_locked_for`]). Any other error, and the error from the
/// attempt that runs out of time, are returned as-is.
///
/// On unix `is_locked_for` is always false, so this is one call and no sleep —
/// the loop costs nothing on the platform that does not need it.
///
/// `what` and `path` name the operation in the log line; they are not used
/// otherwise.
fn retry_while_locked<T>(
    what: &str,
    path: &Path,
    mut op: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let started = std::time::Instant::now();
    let mut backoff = std::time::Duration::from_millis(1);
    loop {
        let result = op();
        let Err(error) = &result else {
            let waited = started.elapsed();
            if waited >= LOCK_RETRY_LOG_THRESHOLD {
                log::warn!(
                    "waited {waited:?} for another handle to release {} before {what} succeeded",
                    path.display()
                );
            }
            return result;
        };
        if !is_locked_for(HOST, error) || started.elapsed() >= LOCK_RETRY_TIMEOUT {
            if is_locked_for(HOST, error) {
                log::warn!(
                    "gave up after {:?} waiting to {what} {}; held by: {}",
                    started.elapsed(),
                    path.display(),
                    describe_holders(path)
                );
            }
            return result;
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(LOCK_RETRY_MAX_BACKOFF);
    }
}

/// The processes holding `path` open, as `"name (pid)"`, joined for a log line.
///
/// Windows says only "the process cannot access the file because it is being
/// used by another process" and never says which — so a user whose workspace
/// rename fails has nothing to act on, and neither does a bug report. This is
/// the same API the shell's "file in use" dialog uses.
///
/// Empty on unix, which does not block a rename on an open handle and so never
/// asks the question.
fn describe_holders(path: &Path) -> String {
    let holders = holders_of(path);
    if holders.is_empty() {
        "unknown".to_string()
    } else {
        holders.join(", ")
    }
}

/// Best-effort: any failure of the query itself yields no names rather than an
/// error, since this only ever runs to explain a failure that already happened.
#[cfg(not(windows))]
fn holders_of(_path: &Path) -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn holders_of(path: &Path) -> Vec<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
        RM_PROCESS_INFO,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut session: u32 = 0;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    // SAFETY: `key` is the size the API documents, and `session` is written
    // only on success, which is the only path that goes on to use it.
    if unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) } != ERROR_SUCCESS {
        return Vec::new();
    }

    let mut names = Vec::new();
    // SAFETY: one NUL-terminated path, counted as one; the pointer outlives
    // the call. The session is ended below on every path.
    let registered = unsafe {
        RmRegisterResources(
            session,
            1,
            &wide.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        )
    };
    if registered == ERROR_SUCCESS {
        // Ask for the count first: `RmGetList` reports how many entries it
        // wanted via `needed` when the buffer is too small.
        let mut needed: u32 = 0;
        let mut count: u32 = 0;
        let mut reason: u32 = 0;
        // SAFETY: a zero-length buffer is the documented way to size the call.
        let sized = unsafe {
            RmGetList(
                session,
                &mut needed,
                &mut count,
                std::ptr::null_mut(),
                &mut reason,
            )
        };
        if (sized == ERROR_MORE_DATA || sized == ERROR_SUCCESS) && needed > 0 {
            let mut infos: Vec<RM_PROCESS_INFO> =
                vec![unsafe { std::mem::zeroed() }; needed as usize];
            count = needed;
            // SAFETY: `infos` holds `count` entries, which is what is passed.
            let got = unsafe {
                RmGetList(
                    session,
                    &mut needed,
                    &mut count,
                    infos.as_mut_ptr(),
                    &mut reason,
                )
            };
            if got == ERROR_SUCCESS {
                for info in infos.iter().take(count as usize) {
                    let name = String::from_utf16_lossy(&info.strAppName);
                    let name = name.trim_end_matches('\0');
                    names.push(format!("{name} (pid {})", info.Process.dwProcessId));
                }
            }
        }
    }

    // SAFETY: `session` came from a successful `RmStartSession`.
    unsafe { RmEndSession(session) };
    names
}

/// Moves a file, including across volumes.
///
/// A rename cannot cross a filesystem, and the two hosts say so with different
/// codes; when that is the failure, this falls back to copy + unlink. Every
/// other failure is reported as-is — a cross-volume fallback must not paper
/// over a permissions error.
///
/// A destination still held open by another handle is waited out rather than
/// reported; see [`is_locked_for`] for why that is a Windows-only wait.
///
/// The fallback is all-or-nothing like the rename it stands in for: if the
/// unlink fails, the copy is undone. Leaving it meant a failed move reported an
/// error while the file existed at *both* paths, which the caller cannot undo —
/// [`crate::IndexFile`]'s rollback only walks the pairs that succeeded, so a
/// v2 → v3 migration whose source index was still held open left an index at
/// the new path too, and the next launch found it and opened the half-populated
/// copy.
pub fn move_file(from: &Path, to: &Path) -> Result<(), SystemError> {
    match retry_while_locked("move", from, || std::fs::rename(from, to)) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device_for(HOST, &e) => copy_then_unlink(from, to),
        Err(e) => Err(SystemError::locked_or_io("move", from, e)),
    }
}

/// [`move_file`]'s cross-volume fallback: copy, then unlink the source.
///
/// Split out because the branch is unreachable in a test run — provoking a real
/// `EXDEV` needs two filesystems — while the half-done state it can leave is
/// the part worth pinning.
fn copy_then_unlink(from: &Path, to: &Path) -> Result<(), SystemError> {
    std::fs::copy(from, to).map_err(|e| SystemError::io("copy", from, e))?;
    if let Err(e) = remove_file(from) {
        // Best-effort, and the plain call rather than `remove_file`: `to` was
        // created moments ago by the copy above, so the lock wait that helps a
        // caller's own file would only stall here.
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

/// Whether a failed rename means "source and destination are on different
/// volumes" — the one failure [`move_file`]'s copy + unlink fallback handles.
///
/// Both codes, selected by `host` rather than by `cfg`, so each is exercised
/// from any platform: unix reports `EXDEV`, Windows reports
/// `ERROR_NOT_SAME_DEVICE`. Matching only the unix errno once left the
/// fallback dead on Windows, where a vault on `D:` with its cache under `C:`
/// hit exactly this path.
pub fn is_cross_device_for(host: Host, err: &std::io::Error) -> bool {
    let expected = match host {
        Host::Unix => 18,    // EXDEV
        Host::Windows => 17, // ERROR_NOT_SAME_DEVICE
    };
    err.raw_os_error() == Some(expected)
}

/// Distinguishes the temp files of two writers inside one process. Paired with
/// the pid it makes [`temp_name_for`]'s result unique across the machine.
static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The sibling temp file [`replace_atomically`] writes before renaming.
///
/// Unique per writer, not derived from the target name alone. Two kimün
/// instances saving preferences at the same moment would otherwise pick the
/// same temp path: the second `File::create` truncates the first's half-written
/// file, and whichever renames last publishes it. That is the exact failure
/// this recipe exists to prevent, so the name carries the pid and a counter.
///
/// A sibling, so the rename that follows never crosses a volume.
fn temp_name_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    PathBuf::from(name)
}

/// Replaces the file at `path` with `contents`, atomically.
///
/// Writes a sibling temp file, flushes it to disk, then renames over the
/// target: a crash mid-write leaves the previous contents intact rather than a
/// truncated file. Missing parent directories are created.
pub fn replace_atomically(path: &Path, contents: &[u8]) -> Result<(), SystemError> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        if !parent.is_dir() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SystemError::io("create directory", parent, e))?;
        }
    }
    let tmp = temp_name_for(path);
    let write = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return Err(SystemError::io("write", &tmp, e));
    }
    // Same wait as `move_file`: the file being replaced is the config the app
    // rewrites on every preference change, and a scanner holding it briefly
    // must not surface as "could not save your settings".
    retry_while_locked("replace", path, || std::fs::rename(&tmp, path))
        .map_err(|e| SystemError::locked_or_io("replace", path, e))?;
    sync_dir(parent.unwrap_or(Path::new(".")));
    Ok(())
}

/// Flushes a directory's own entries to disk, so a rename into it survives a
/// crash.
///
/// `sync_all` on the temp file makes its *contents* durable; the rename that
/// publishes them only touches a directory entry, which on ext4/xfs can still
/// be in the page cache when the power goes. Without this, a write that
/// returned successfully can silently roll back.
///
/// Best-effort and unix-only: the data is already renamed, so failing the call
/// here would report a write that did happen as a failure, and Windows has no
/// directory handle to sync (`NTFS` orders the metadata itself).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    match std::fs::File::open(dir).and_then(|d| d.sync_all()) {
        Ok(()) => {}
        Err(e) => log::debug!("could not flush directory {}: {e}", dir.display()),
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Deletes a file. A path that is already gone is not an error — callers
/// reach for this to make something absent, and it is.
///
/// A file another handle still holds open is waited out rather than reported,
/// for the reason on [`is_locked_for`]: `workspace remove`'s delete is
/// best-effort, so a transient lock there does not fail the command — it
/// silently leaves the index behind forever.
pub fn remove_file(path: &Path) -> Result<(), SystemError> {
    match retry_while_locked("remove", path, || std::fs::remove_file(path)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SystemError::locked_or_io("remove", path, e)),
    }
}

/// Removes an empty directory. A directory that is already gone is not an
/// error; one that still holds files is (this undoes a directory that was
/// just created, and must never take a user's files with it).
pub fn remove_empty_dir(path: &Path) -> Result<(), SystemError> {
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SystemError::io("remove directory", path, e)),
    }
}

/// The entries directly inside `dir`, as [`SystemPath`]s, in the order the
/// filesystem reports them.
///
/// Returning `SystemPath`s is the point: a directory read is where raw OS
/// paths enter the program, and every one of them is absolute here because
/// `dir` is.
pub fn read_dir(dir: &SystemPath) -> Result<Vec<SystemPath>, SystemError> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| SystemError::io("read directory", dir.as_path(), e))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| SystemError::io("read directory", dir.as_path(), e))?;
        out.push(SystemPath(normalize(&entry.path())));
    }
    Ok(out)
}

/// Marks a file executable.
///
/// A unix permission bit and nothing at all on Windows, where executability
/// comes from the extension. Kept here rather than beside its one caller so
/// the `cfg` split lives with the other host rules.
pub fn make_executable(path: &Path) -> Result<(), SystemError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .map_err(|e| SystemError::io("read permissions of", path, e))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
            .map_err(|e| SystemError::io("set permissions on", path, e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// The name an executable goes by on `host` — `nvim` against `nvim.exe`.
pub fn exe_name_for(host: Host, stem: &str) -> String {
    match host {
        Host::Unix => stem.to_string(),
        Host::Windows => format!("{stem}.exe"),
    }
}

/// A [`SystemPath`] for a path a test has already made absolute (a `TempDir`,
/// a host literal). Panics rather than returning a `Result` because a test
/// that hands over a relative path is a broken test, not a failure case.
#[cfg(test)]
pub(crate) fn sys<P: AsRef<Path>>(path: P) -> SystemPath {
    SystemPath::try_absolute(&path).unwrap_or_else(|e| panic!("test path must be absolute: {e}"))
}

#[cfg(test)]
mod tests;
