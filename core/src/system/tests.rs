use super::*;

/// A `/`-separated literal as the host writes it, so assertions read the same
/// on Windows (`C:\a\b`) as on unix (`/a/b`).
fn host_path(unix_style: &str) -> PathBuf {
    let trimmed = unix_style.trim_start_matches('/');
    if cfg!(windows) {
        PathBuf::from(format!("C:\\{}", trimmed.replace('/', "\\")))
    } else {
        PathBuf::from(format!("/{trimmed}"))
    }
}

/// The *relative* counterpart of [`host_path`], separators and all.
///
/// `PathBuf::from("a/b")` keeps its `/` on Windows, while everything
/// [`normalize`] builds comes back `\`-separated — and [`assert_same`]
/// compares the raw strings, so the two never match there.
fn rel_path(unix_style: &str) -> PathBuf {
    unix_style.split('/').collect()
}

/// Compares raw strings, not `Path`s: `PartialEq` for paths goes through
/// `Components`, which drops `.` itself and would pass whether or not
/// normalization happened.
fn assert_same(actual: impl AsRef<Path>, expected: impl AsRef<Path>) {
    assert_eq!(actual.as_ref().as_os_str(), expected.as_ref().as_os_str());
}

// ── normalize ──────────────────────────────────────────────────────────

#[test]
fn normalize_drops_cur_dir_components() {
    assert_same(normalize(&host_path("/a/./b")), host_path("/a/b"));
    // The case that breaks Windows: a trailing `.` on a verbatim path.
    assert_same(normalize(&host_path("/a/b").join(".")), host_path("/a/b"));
    assert_same(normalize(Path::new("./a/./b")), rel_path("a/b"));
}

#[test]
fn normalize_resolves_parent_dir_lexically() {
    assert_same(normalize(&host_path("/a/b/../c")), host_path("/a/c"));
    assert_same(normalize(&host_path("/a/b/../..")), host_path("/"));
    // `..` cannot climb above a root; the root's parent is the root.
    assert_same(normalize(&host_path("/../..")), host_path("/"));
}

#[test]
fn normalize_keeps_leading_parent_dirs_of_relative_paths() {
    // Nothing to cancel and no root to stop at, so these must survive:
    // dropping them would silently retarget the path at the cwd.
    assert_same(normalize(Path::new("../a")), rel_path("../a"));
    assert_same(normalize(Path::new("a/../../b")), rel_path("../b"));
}

/// A bare drive prefix is not a root. `C:..\a` is drive-relative — it still
/// has a parent to climb out of — so dropping the `..` retargets it at the
/// current directory on `C:`, the same silent rebase relative paths are
/// protected from above.
#[test]
#[cfg(windows)]
fn normalize_keeps_parent_dirs_of_drive_relative_paths() {
    assert_same(normalize(Path::new(r"C:..\a")), PathBuf::from(r"C:..\a"));
    assert_same(
        normalize(Path::new(r"C:a\..\..\b")),
        PathBuf::from(r"C:..\b"),
    );
    // With the root present there *is* nothing above to climb to.
    assert_same(normalize(Path::new(r"C:\..\a")), PathBuf::from(r"C:\a"));
}

#[test]
fn normalize_of_an_empty_result_is_cur_dir() {
    assert_same(normalize(Path::new(".")), PathBuf::from("."));
    assert_same(normalize(Path::new("a/..")), PathBuf::from("."));
    assert_same(normalize(Path::new("")), PathBuf::from("."));
}

// ── SystemPath ─────────────────────────────────────────────────────────

#[test]
fn try_absolute_normalizes() {
    let path = SystemPath::try_absolute(host_path("/a/./b/../c")).unwrap();
    assert_same(&path, host_path("/a/c"));
}

#[test]
fn try_absolute_rejects_relative_paths() {
    // Anchoring on the working directory is the failure this type exists to
    // prevent, so a relative path is an error rather than a silent rebase.
    let err = SystemPath::try_absolute("notes/vault").unwrap_err();
    assert!(
        matches!(err, SystemError::NotAbsolute { .. }),
        "got {err:?}"
    );
}

#[test]
fn resolve_rebases_relative_paths_on_the_given_base() {
    let base = SystemPath::try_absolute(host_path("/some/config/dir")).unwrap();
    let resolved = SystemPath::resolve("history", &base);
    assert_same(&resolved, host_path("/some/config/dir/history"));
}

/// The `cache_dir = "."` default must not leave a `.` in the result, whether
/// or not the target exists to canonicalize.
#[test]
fn resolve_strips_cur_dir() {
    let base = SystemPath::try_absolute(host_path("/some/config/dir")).unwrap();
    assert_same(
        SystemPath::resolve("./history", &base),
        host_path("/some/config/dir/history"),
    );
    assert_same(
        SystemPath::resolve(".", &base),
        host_path("/some/config/dir"),
    );

    let existing = tempfile::TempDir::new().unwrap();
    let canonical = SystemPath::try_absolute(existing.path().canonicalize().unwrap()).unwrap();
    assert_same(SystemPath::resolve(".", &canonical), canonical.as_path());
}

#[test]
fn resolve_leaves_absolute_paths_alone() {
    let base = SystemPath::try_absolute(host_path("/config")).unwrap();
    let input = host_path("/absolute/notes");
    assert_same(SystemPath::resolve(&input, &base), input);
}

#[test]
fn resolve_climbs_out_of_the_base_with_dotdot() {
    let base = tempfile::TempDir::new().unwrap();
    let sibling = base.path().join("sibling");
    let sub = base.path().join("sub");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::create_dir_all(&sub).unwrap();

    let resolved = SystemPath::resolve("../sibling", &SystemPath::try_absolute(&sub).unwrap());

    assert_same(&resolved, sibling.canonicalize().unwrap());
}

#[test]
fn resolve_canonicalizes_what_exists() {
    let base = tempfile::TempDir::new().unwrap();
    let base_path = SystemPath::try_absolute(base.path().canonicalize().unwrap()).unwrap();
    let notes = base.path().join("notes");
    std::fs::create_dir_all(&notes).unwrap();

    let resolved = SystemPath::resolve("notes", &base_path);
    assert_same(&resolved, notes.canonicalize().unwrap());
}

#[test]
#[cfg(unix)]
fn resolve_expands_tilde_to_home() {
    let home = PathBuf::from(std::env::var("HOME").expect("HOME must be set on unix"));
    let base = SystemPath::try_absolute(host_path("/irrelevant")).unwrap();
    let resolved = SystemPath::resolve("~/kimun-no-such-directory-2f8a1c/notes", &base);
    assert_same(&resolved, home.join("kimun-no-such-directory-2f8a1c/notes"));
}

/// The other tilde branch: a target that *does* exist gets canonicalized, so
/// the result is the real on-disk path rather than the literal `~/…` join.
///
/// The directory has to live under the real `$HOME`, not a tempdir: `~`
/// expands via the `HOME` env var, and env vars are process-global — pointing
/// `HOME` elsewhere would race every other test in this binary. Hence the
/// PID-keyed name and the drop guard (which a SIGKILL would outlive, leaving
/// one stray `kimun-test-tilde-*` directory behind).
#[test]
#[cfg(unix)]
fn resolve_canonicalizes_an_existing_tilde_target() {
    let home = PathBuf::from(std::env::var("HOME").expect("HOME must be set on unix"));
    let unique = format!("kimun-test-tilde-{}", std::process::id());
    let target = home.join(&unique).join("notes");
    std::fs::create_dir_all(&target).unwrap();

    struct RemoveOnDrop(PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = RemoveOnDrop(home.join(&unique));

    let base = SystemPath::try_absolute(host_path("/irrelevant")).unwrap();
    let resolved = SystemPath::resolve(format!("~/{unique}/notes"), &base);

    assert_same(&resolved, target.canonicalize().unwrap());
}

#[test]
#[cfg(unix)]
fn resolve_tilde_alone_is_home() {
    let home = PathBuf::from(std::env::var("HOME").expect("HOME must be set on unix"));
    let base = SystemPath::try_absolute(host_path("/irrelevant")).unwrap();

    let resolved = SystemPath::resolve("~", &base);

    // canonicalize may resolve symlinks, so compare canonical forms.
    assert_same(&resolved, home.canonicalize().unwrap_or(home.clone()));
}

#[test]
#[cfg(windows)]
fn resolve_tilde_uses_userprofile() {
    let home = std::env::var("USERPROFILE").expect("USERPROFILE must be set on Windows");
    let base = SystemPath::try_absolute(host_path("/irrelevant")).unwrap();

    let resolved = SystemPath::resolve("~/Documents/notes", &base);

    assert!(
        resolved.as_path().starts_with(&home),
        "expected a path under USERPROFILE={home}, got {resolved}"
    );
}

/// The home directory comes from the host's own variable — `HOME` on unix,
/// `USERPROFILE` on Windows.
#[test]
fn home_reads_the_platform_variable() {
    let expected = if cfg!(windows) {
        std::env::var("USERPROFILE")
    } else {
        std::env::var("HOME")
    }
    .expect("the host's home variable must be set");

    let home = home().expect("home directory should resolve");

    assert_same(&home, normalize(Path::new(&expected)));
}

#[test]
fn join_stays_normalized() {
    let dir = SystemPath::try_absolute(host_path("/a/b")).unwrap();
    assert_same(dir.join("c"), host_path("/a/b/c"));
    assert_same(dir.join(".."), host_path("/a"));
    assert_same(dir.join("./c"), host_path("/a/b/c"));
}

// ── layout ─────────────────────────────────────────────────────────────

/// The layout rule is computation, so both hosts' answers are checked from
/// whichever host runs the suite — the reason [`Host`] is a value.
#[test]
fn app_dir_follows_each_host_convention() {
    let home = SystemPath::try_absolute(host_path("/home/user")).unwrap();

    let unix = app_dir_under(&home, Host::Unix);
    let windows = app_dir_under(&home, Host::Windows);

    assert!(
        unix.as_path()
            .ends_with(Path::new(".config").join(APP_DIR_NAME)),
        "unix app dir should be ~/.config/<app>, got {unix}"
    );
    assert!(
        windows.as_path().ends_with(APP_DIR_NAME),
        "windows app dir should be <home>/<app>, got {windows}"
    );
    assert!(
        !windows.as_path().to_string_lossy().contains(".config"),
        "windows app dir must not use the XDG shape, got {windows}"
    );
}

/// One app directory, so config and logs cannot drift apart the way they did
/// when two modules each picked a location.
#[test]
fn log_dir_sits_under_the_app_dir() {
    let logs = log_dir();
    let parent = logs.parent().expect("log dir has a parent");
    assert!(logs.as_path().ends_with(LOG_DIR_NAME), "got {logs}");
    assert!(parent.as_path().ends_with(APP_DIR_NAME), "got {parent}");
}

#[test]
fn log_dir_is_absolute_even_without_a_home() {
    // Logging must never anchor on the working directory, home or not.
    assert!(log_dir().as_path().is_absolute());
}

/// The no-home branch, which the test above cannot reach (CI always has a
/// home) and which takes `$TMPDIR` exactly as the OS reports it — including
/// the two shapes a `SystemPath` must never hold.
#[test]
fn log_dir_in_temp_normalizes_a_dotted_tmpdir() {
    let logs = log_dir_in_temp(&host_path("/var/tmp/."), &host_path("/irrelevant"));

    assert_same(
        &logs,
        host_path("/var/tmp").join(APP_DIR_NAME).join(LOG_DIR_NAME),
    );
    assert!(
        !logs.as_path().to_string_lossy().contains(&format!(
            "{}.{}",
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        )),
        "a verbatim Windows path reads `.` as a directory named `.`, got {logs}"
    );
}

#[test]
fn log_dir_in_temp_anchors_a_relative_tmpdir_on_the_working_directory() {
    // `std::env::temp_dir` hands back `$TMPDIR` verbatim, so it can be
    // relative. Anywhere absolute beats a log path that moves with the cwd.
    let logs = log_dir_in_temp(Path::new("./tmp"), &host_path("/work"));

    assert!(logs.as_path().is_absolute(), "got {logs}");
    assert_same(
        &logs,
        host_path("/work/tmp").join(APP_DIR_NAME).join(LOG_DIR_NAME),
    );
}

/// Git Bash, MSYS2 and Cygwin all set `HOME` on Windows, to a unix-shaped path
/// with no drive prefix — set, but not absolute there. Stopping at the first
/// variable that merely exists meant `USERPROFILE` was never read and the app
/// could not resolve its own directory, so it would not start at all.
#[test]
fn home_falls_through_a_set_but_unusable_candidate() {
    let usable = host_path("/users/bob");

    // Relative on every host, which is what `/c/Users/bob` is on Windows.
    let relative = home_from([
        Some(rel_path("c/Users/bob").into_os_string()),
        Some(usable.clone().into_os_string()),
    ]);
    let empty = home_from([
        Some(std::ffi::OsString::new()),
        Some(usable.clone().into_os_string()),
    ]);
    let unset = home_from([None, Some(usable.clone().into_os_string())]);

    for (case, result) in [("relative", relative), ("empty", empty), ("unset", unset)] {
        assert_same(result.unwrap_or_else(|e| panic!("{case}: {e}")), &usable);
    }
}

/// Only when nothing is usable is there no home. `NoHome` rather than
/// `NotAbsolute`, because the answer the caller needs is "there isn't one",
/// not which of the candidates was malformed.
#[test]
fn home_with_no_usable_candidate_is_no_home() {
    let err = home_from([Some(rel_path("relative").into_os_string()), None]).unwrap_err();
    assert!(matches!(err, SystemError::NoHome), "got {err:?}");
}

/// The home directory when there is one, so the browser opens somewhere the
/// user recognizes rather than at a root.
#[test]
fn browse_root_is_an_absolute_directory() {
    let root = browse_root().expect("home or cwd must resolve");
    assert!(root.as_path().is_absolute(), "got {root}");
    assert!(root.is_dir(), "got {root}");
}

// ── operations ─────────────────────────────────────────────────────────

/// Both codes, checked from either platform.
#[test]
fn cross_device_errors_are_recognized_per_host() {
    let exdev = std::io::Error::from_raw_os_error(18);
    let not_same_device = std::io::Error::from_raw_os_error(17);

    assert!(is_cross_device_for(Host::Unix, &exdev));
    assert!(!is_cross_device_for(Host::Unix, &not_same_device));

    assert!(is_cross_device_for(Host::Windows, &not_same_device));
    assert!(!is_cross_device_for(Host::Windows, &exdev));

    // No OS error at all is not a cross-device failure.
    assert!(!is_cross_device_for(HOST, &std::io::Error::other("boom")));
    assert!(!is_cross_device_for(
        HOST,
        &std::io::Error::from_raw_os_error(2)
    ));
}

/// "Somebody still has this open" is a Windows-only condition, and only for
/// the two codes that actually mean it. Both answers checked from either host.
#[test]
fn open_file_errors_are_recognized_only_on_windows() {
    let sharing = std::io::Error::from_raw_os_error(32);
    let locked = std::io::Error::from_raw_os_error(33);
    let denied = std::io::Error::from_raw_os_error(5);

    assert!(is_locked_for(Host::Windows, &sharing));
    assert!(is_locked_for(Host::Windows, &locked));
    // A real permissions problem must be reported at once, not waited out.
    assert!(!is_locked_for(Host::Windows, &denied));
    assert!(!is_locked_for(
        Host::Windows,
        &std::io::Error::from_raw_os_error(2)
    ));
    assert!(!is_locked_for(
        Host::Windows,
        &std::io::Error::other("boom")
    ));

    // Unix renames and unlinks open files, so there is never anything to wait
    // for — including for the codes that mean something else entirely there.
    for err in [&sharing, &locked, &denied] {
        assert!(!is_locked_for(Host::Unix, err));
    }
}

/// A non-lock error is returned on the first attempt, and success is not
/// retried either — the loop must cost nothing when there is no lock.
#[test]
fn retry_while_locked_does_not_retry_what_it_should_not() {
    let path = host_path("/some/file");

    let mut attempts = 0;
    let result: std::io::Result<()> = retry_while_locked("move", &path, || {
        attempts += 1;
        Err(std::io::Error::from_raw_os_error(2))
    });
    assert!(result.is_err());
    assert_eq!(attempts, 1, "a plain failure must not be retried");

    let mut attempts = 0;
    let result: std::io::Result<()> = retry_while_locked("move", &path, || {
        attempts += 1;
        Ok(())
    });
    assert!(result.is_ok());
    assert_eq!(attempts, 1);
}

/// A lock that clears is waited out and the operation then succeeds — the
/// whole point of the retry, and the case a unix-only suite would otherwise
/// never execute, since `is_locked_for(Host::Unix, _)` is always false.
///
/// Drives the loop through `HOST`, so on unix this asserts the no-wait path
/// (first error returned immediately) and on Windows the retry path.
#[test]
fn retry_while_locked_waits_out_a_lock_that_clears() {
    let path = host_path("/some/file");
    let mut attempts = 0;

    let result: std::io::Result<()> = retry_while_locked("move", &path, || {
        attempts += 1;
        if attempts < 3 {
            Err(std::io::Error::from_raw_os_error(32)) // ERROR_SHARING_VIOLATION
        } else {
            Ok(())
        }
    });

    if HOST == Host::Windows {
        assert!(result.is_ok(), "a lock that clears must not fail the op");
        assert_eq!(attempts, 3);
    } else {
        // Nothing to wait for: unix renames and unlinks open files, so error
        // 32 is just an error and comes straight back.
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
}

#[test]
fn ensure_dir_creates_missing_parents_and_is_idempotent() {
    let dir = tempfile::TempDir::new().unwrap();
    let nested = SystemPath::try_absolute(dir.path()).unwrap().join("a/b/c");

    ensure_dir(&nested).unwrap();
    assert!(nested.is_dir());

    // Second call on an existing directory is a no-op, not an error: this is
    // what `ensure_app_dir` leans on at every startup.
    ensure_dir(&nested).unwrap();
    assert!(nested.is_dir());
}

/// A *file* where a directory was asked for is a failure, not an idempotent
/// success — reporting it here is the difference between "cannot create that"
/// at the prompt and an empty listing the user only understands next launch.
#[test]
fn a_file_where_a_directory_was_asked_for_is_an_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let occupied = dir.path().join("notes");
    std::fs::write(&occupied, b"not a directory").unwrap();

    let err = create_dir(&occupied).unwrap_err();
    assert!(matches!(err, SystemError::Io { .. }), "got {err:?}");

    let err = ensure_dir(&SystemPath::try_absolute(&occupied).unwrap()).unwrap_err();
    assert!(matches!(err, SystemError::Io { .. }), "got {err:?}");

    assert_eq!(std::fs::read(&occupied).unwrap(), b"not a directory");
}

#[test]
fn canonical_resolves_an_existing_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let nested = dir.path().join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();

    let resolved = SystemPath::canonical(dir.path().join("a").join(".").join("b")).unwrap();

    assert_same(&resolved, nested.canonicalize().unwrap());
}

#[test]
fn canonical_fails_for_a_path_that_is_not_there() {
    let dir = tempfile::TempDir::new().unwrap();
    let err = SystemPath::canonical(dir.path().join("nope")).unwrap_err();
    assert!(matches!(err, SystemError::Io { .. }), "got {err:?}");
}

#[test]
fn read_dir_lists_entries_as_absolute_paths() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("one.toml"), "").unwrap();
    std::fs::create_dir(dir.path().join("two")).unwrap();

    let mut names: Vec<String> = read_dir(&SystemPath::try_absolute(dir.path()).unwrap())
        .unwrap()
        .into_iter()
        .inspect(|p| assert!(p.as_path().is_absolute(), "{p} is not absolute"))
        .map(|p| p.as_path().file_name().unwrap().to_string_lossy().into())
        .collect();
    names.sort();

    assert_eq!(names, ["one.toml", "two"]);
}

#[test]
fn read_dir_reports_a_missing_directory() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = SystemPath::try_absolute(dir.path()).unwrap().join("nope");
    assert!(read_dir(&missing).is_err());
}

#[test]
fn remove_empty_dir_removes_only_an_empty_one() {
    let dir = tempfile::TempDir::new().unwrap();
    let empty = dir.path().join("empty");
    let occupied = dir.path().join("occupied");
    std::fs::create_dir(&empty).unwrap();
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("note.md"), "keep me").unwrap();

    remove_empty_dir(&empty).unwrap();
    assert!(!empty.exists());

    // A directory holding the user's files must survive an undo.
    assert!(remove_empty_dir(&occupied).is_err());
    assert!(occupied.join("note.md").exists());

    // Already gone is the desired state, not an error.
    remove_empty_dir(&empty).unwrap();
}

#[test]
fn make_executable_is_ok_on_every_host() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("kimun");
    std::fs::write(&file, b"#!/bin/sh\n").unwrap();

    make_executable(&file).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "got {:o}", mode & 0o777);
    }
}

#[test]
fn move_file_moves_within_a_volume() {
    let dir = tempfile::TempDir::new().unwrap();
    let from = dir.path().join("a.txt");
    let to = dir.path().join("sub").join("b.txt");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::write(&from, b"payload").unwrap();

    move_file(&from, &to).unwrap();

    assert!(!from.exists(), "source should be gone");
    assert_eq!(std::fs::read(&to).unwrap(), b"payload");
}

#[test]
fn move_file_reports_a_missing_source_rather_than_falling_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let err = move_file(&dir.path().join("nope.txt"), &dir.path().join("out.txt")).unwrap_err();
    assert!(matches!(err, SystemError::Io { .. }), "got {err:?}");
}

/// The cross-volume fallback is all-or-nothing, like the rename it stands in
/// for. Leaving the copy behind on a failed unlink put the file at *both*
/// paths while still reporting an error, and [`crate::IndexFile`]'s rollback
/// cannot reach it — the pair never counted as done. The next launch then found
/// an index at the destination and opened that half-populated copy.
///
/// Unix-only because the failure needs an unlink that fails while the copy
/// succeeds, and a directory without write permission is how that is arranged.
/// Windows reaches the same code through a held handle, which cannot be
/// provoked on demand.
#[cfg(unix)]
#[test]
fn a_failed_unlink_undoes_the_copy() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().unwrap();
    let source_dir = dir.path().join("source");
    std::fs::create_dir(&source_dir).unwrap();
    let from = source_dir.join("index.kimuncache");
    let to = dir.path().join("moved.kimuncache");
    std::fs::write(&from, b"index").unwrap();
    // Readable but not writable: the copy can still read the file, while its
    // directory entry cannot be removed.
    std::fs::set_permissions(&source_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    // Root ignores the mode bits, so there would be nothing to observe.
    let enforced = std::fs::File::create(source_dir.join("probe")).is_err();

    let result = copy_then_unlink(&from, &to);

    std::fs::set_permissions(&source_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    if !enforced {
        return;
    }
    assert!(result.is_err(), "the unlink must fail");
    assert!(from.exists(), "the source must be left whole");
    assert!(!to.exists(), "the copy must not survive a failed move");
}

/// The other half of that rollback: it may only remove a destination this call
/// created. `fs::copy` overwrites, so deleting a `to` that was already there
/// destroys a file the caller never asked to have touched — a worse outcome
/// than the leftover the rollback exists to prevent.
#[cfg(unix)]
#[test]
fn a_failed_unlink_leaves_a_destination_it_did_not_create() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().unwrap();
    let source_dir = dir.path().join("source");
    std::fs::create_dir(&source_dir).unwrap();
    let from = source_dir.join("index.kimuncache");
    let to = dir.path().join("occupied.kimuncache");
    std::fs::write(&from, b"index").unwrap();
    std::fs::write(&to, b"someone else's file").unwrap();
    std::fs::set_permissions(&source_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let enforced = std::fs::File::create(source_dir.join("probe")).is_err();

    let result = copy_then_unlink(&from, &to);

    std::fs::set_permissions(&source_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    if !enforced {
        return;
    }
    assert!(result.is_err(), "the unlink must fail");
    assert!(from.exists(), "the source must be left whole");
    assert!(
        to.exists(),
        "a destination this call did not create must survive"
    );
}

#[test]
fn replace_atomically_writes_and_leaves_no_temp_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("history").join("default.txt");

    replace_atomically(&target, b"first\n").unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "first\n");

    replace_atomically(&target, b"second\n").unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "second\n");

    let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n != "default.txt")
        .collect();
    assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
}

/// Two writers of the same file must not pick the same temp path: the second
/// `File::create` would truncate the first's half-written file, and whichever
/// renamed last would publish the interleaved result — the exact corruption
/// this recipe exists to prevent.
#[test]
fn concurrent_writers_of_one_file_get_distinct_temp_paths() {
    let target = Path::new("/config/kimun.toml");

    let first = temp_name_for(target);
    let second = temp_name_for(target);

    assert_ne!(first, second);
    assert_eq!(
        first.parent(),
        target.parent(),
        "the temp file must be a sibling, or the rename crosses a volume"
    );
    assert!(
        first
            .to_string_lossy()
            .contains(&std::process::id().to_string()),
        "another process's writer must not collide either, got {first:?}"
    );
}

/// The whole file is replaced, not merged into: a shorter payload must not
/// leave the tail of the previous one behind.
#[test]
fn replace_atomically_truncates_a_longer_previous_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("history.txt");

    replace_atomically(&target, b"a long first line\nand a second\n").unwrap();
    replace_atomically(&target, b"short\n").unwrap();

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "short\n");
}

#[test]
fn create_dir_returns_an_absolute_normalized_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let nested = dir.path().join("a").join(".").join("b");

    let created = create_dir(&nested).unwrap();

    assert!(created.as_path().is_absolute());
    assert!(created.exists());
    assert!(
        !created.as_path().to_string_lossy().contains("/./"),
        "got {created}"
    );
}

#[test]
fn exe_name_follows_the_host() {
    assert_eq!(exe_name_for(Host::Unix, "nvim"), "nvim");
    assert_eq!(exe_name_for(Host::Windows, "nvim"), "nvim.exe");
}
