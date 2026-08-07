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
    assert_same(normalize(Path::new("./a/./b")), PathBuf::from("a/b"));
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
    assert_same(normalize(Path::new("../a")), PathBuf::from("../a"));
    assert_same(normalize(Path::new("a/../../b")), PathBuf::from("../b"));
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
