//! Finding the terminal app (`kimun`) from the desktop app, and reading its
//! version. Read-only: nothing here installs, updates or changes it.
//!
//! Where it looks, in order: the directory the install marker names (where
//! `install.sh` puts `kimun` next to `kimun-gui`), then each directory of
//! `PATH`, then the places brew and cargo install to ([`fallback_dirs`]). A
//! desktop app started from a launcher has a minimal `PATH` that leaves those
//! out, so without the last step a brew or cargo install would show as "not
//! installed". The first `kimun` file found decides: if it does not run in
//! time or prints no version, the result is [`TerminalApp::VersionUnknown`],
//! never "not installed".
//!
//! Every function here is **blocking** (it starts a process): a frontend runs
//! it on tokio's blocking pool.

use std::ffi::OsStr;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The terminal app's file name on this platform.
const EXE_NAME: &str = if cfg!(windows) { "kimun.exe" } else { "kimun" };

/// How much of the program's output is read: a version line is a few bytes.
const OUTPUT_LIMIT: u64 = 4096;

/// What the desktop app found when it looked for the terminal app (`kimun`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalApp {
    /// No `kimun` in the install directory or on `PATH`.
    NotInstalled,
    /// `kimun` ran and reported `version`.
    Installed { version: String },
    /// A `kimun` file exists at `path`, but it did not run in time or did not
    /// print a version. Never reported as not installed.
    VersionUnknown { path: String },
}

/// How long `kimun --version` may run before it is stopped.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Runs `<path> --version`. A seam, so tests need no real binary.
pub(crate) trait VersionProbe {
    /// The program's standard output, or `None` when it could not start, did
    /// not finish within `timeout`, or failed.
    fn version_output(&self, path: &Path, timeout: Duration) -> Option<String>;
}

/// The real probe: starts the process.
pub(crate) struct ProcessProbe;

impl VersionProbe for ProcessProbe {
    fn version_output(&self, path: &Path, timeout: Duration) -> Option<String> {
        let mut command = Command::new(path);
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // A console program started by a window app opens a console window
        // unless told not to (CREATE_NO_WINDOW).
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().ok()?;
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                // Out of time, or the wait itself failed: stop the program and
                // reap it, so no process is left behind.
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        if !status.success() {
            return None;
        }
        let mut output = String::new();
        child
            .stdout
            .take()?
            .take(OUTPUT_LIMIT)
            .read_to_string(&mut output)
            .ok()?;
        Some(output)
    }
}

/// Look for the terminal app and read its version, with the real process
/// probe and the real `PATH`. `config_dir` holds the install marker.
pub fn detect_terminal_app(config_dir: &Path) -> TerminalApp {
    let home = kimun_core::system::home().ok();
    detect(
        config_dir,
        std::env::var_os("PATH").as_deref(),
        &fallback_dirs(home.as_ref().map(|h| h.as_path())),
        &ProcessProbe,
    )
}

/// The places a terminal app is installed to that a launcher's `PATH` may
/// leave out: Homebrew's and the system's `bin` (Unix), and the `bin`
/// directories of `cargo install` and of the usual per-user installs under
/// `home`.
fn fallback_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(unix)]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    if let Some(home) = home {
        dirs.push(home.join(".cargo").join("bin"));
        dirs.push(home.join(".local").join("bin"));
    }
    dirs
}

/// The directories to look in, in order: the install marker's, then `PATH`'s,
/// then `fallback`.
fn candidate_dirs(
    config_dir: &Path,
    path_var: Option<&OsStr>,
    fallback: &[PathBuf],
) -> Vec<PathBuf> {
    let marked = super::channel::marker_install_dir(config_dir);
    let on_path = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| !dir.as_os_str().is_empty());
    marked
        .into_iter()
        .chain(on_path)
        .chain(fallback.iter().cloned())
        .collect()
}

pub(crate) fn detect(
    config_dir: &Path,
    path_var: Option<&OsStr>,
    fallback: &[PathBuf],
    probe: &dyn VersionProbe,
) -> TerminalApp {
    for dir in candidate_dirs(config_dir, path_var, fallback) {
        let file = dir.join(EXE_NAME);
        if !file.is_file() {
            continue;
        }
        // The first file found decides: a later one is never a fallback for a
        // `kimun` that exists but does not answer.
        return match probe
            .version_output(&file, PROBE_TIMEOUT)
            .as_deref()
            .and_then(parse_version_output)
        {
            Some(version) => TerminalApp::Installed { version },
            None => TerminalApp::VersionUnknown {
                path: file.to_string_lossy().into_owned(),
            },
        };
    }
    TerminalApp::NotInstalled
}

/// The version in `kimun <version>` (the first line `kimun --version` prints).
/// A pre-release suffix is kept as printed. Anything else, including another
/// program's output, is `None`.
fn parse_version_output(output: &str) -> Option<String> {
    let mut words = output.split_whitespace();
    if words.next()? != "kimun" {
        return None;
    }
    let version = words.next()?;
    let release = version.split(['-', '+']).next()?;
    super::parse_version(release)?;
    Some(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;

    /// A probe that answers from a script and records what it was asked.
    struct FakeProbe {
        answer: Option<String>,
        asked: RefCell<Vec<PathBuf>>,
    }

    impl FakeProbe {
        fn printing(out: &str) -> Self {
            Self {
                answer: Some(out.to_string()),
                asked: RefCell::default(),
            }
        }
        fn failing() -> Self {
            Self {
                answer: None,
                asked: RefCell::default(),
            }
        }
    }

    impl VersionProbe for FakeProbe {
        fn version_output(&self, path: &Path, _timeout: Duration) -> Option<String> {
            self.asked.borrow_mut().push(path.to_path_buf());
            self.answer.clone()
        }
    }

    fn exe_name() -> &'static str {
        if cfg!(windows) { "kimun.exe" } else { "kimun" }
    }

    /// A directory holding a (fake) `kimun` file.
    fn dir_with_kimun() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(exe_name()), "").unwrap();
        dir
    }

    /// A config dir whose marker names `install_dir`.
    fn config_with_marker(install_dir: &Path) -> tempfile::TempDir {
        let cfg = tempfile::tempdir().unwrap();
        fs::write(
            cfg.path().join("install.toml"),
            format!(
                "channel = \"script\"\ninstall_dir = {:?}\n",
                install_dir.to_string_lossy()
            ),
        )
        .unwrap();
        cfg
    }

    fn path_of(dirs: &[&Path]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn nothing_found_is_not_installed() {
        let cfg = tempfile::tempdir().unwrap();
        let empty = tempfile::tempdir().unwrap();
        let probe = FakeProbe::printing("kimun 1.0.0");
        let got = detect(cfg.path(), Some(&path_of(&[empty.path()])), &[], &probe);
        assert_eq!(got, TerminalApp::NotInstalled);
        assert!(probe.asked.borrow().is_empty(), "no file, no process");
    }

    #[test]
    fn no_path_variable_and_no_marker_is_not_installed() {
        let cfg = tempfile::tempdir().unwrap();
        let got = detect(cfg.path(), None, &[], &FakeProbe::printing("kimun 1.0.0"));
        assert_eq!(got, TerminalApp::NotInstalled);
    }

    #[test]
    fn the_marker_directory_comes_before_path() {
        let marked = dir_with_kimun();
        let on_path = dir_with_kimun();
        let cfg = config_with_marker(marked.path());
        let probe = FakeProbe::printing("kimun 0.25.3\n");
        let got = detect(cfg.path(), Some(&path_of(&[on_path.path()])), &[], &probe);
        assert_eq!(
            got,
            TerminalApp::Installed {
                version: "0.25.3".into()
            }
        );
        assert_eq!(
            *probe.asked.borrow(),
            [marked.path().join(exe_name())],
            "only the marker's file is run"
        );
    }

    #[test]
    fn a_marker_directory_without_kimun_falls_through_to_path() {
        let marked_empty = tempfile::tempdir().unwrap();
        let on_path = dir_with_kimun();
        let cfg = config_with_marker(marked_empty.path());
        let probe = FakeProbe::printing("kimun 0.24.0");
        let got = detect(cfg.path(), Some(&path_of(&[on_path.path()])), &[], &probe);
        assert_eq!(
            got,
            TerminalApp::Installed {
                version: "0.24.0".into()
            }
        );
        assert_eq!(*probe.asked.borrow(), [on_path.path().join(exe_name())]);
    }

    #[test]
    fn a_marker_without_a_directory_is_ignored() {
        let cfg = tempfile::tempdir().unwrap();
        fs::write(cfg.path().join("install.toml"), "channel = \"brew\"\n").unwrap();
        let on_path = dir_with_kimun();
        let got = detect(
            cfg.path(),
            Some(&path_of(&[on_path.path()])),
            &[],
            &FakeProbe::printing("kimun 0.1.0"),
        );
        assert_eq!(
            got,
            TerminalApp::Installed {
                version: "0.1.0".into()
            }
        );
    }

    #[test]
    fn the_first_directory_on_path_wins() {
        let first = dir_with_kimun();
        let second = dir_with_kimun();
        let cfg = tempfile::tempdir().unwrap();
        let probe = FakeProbe::printing("kimun 0.2.0");
        detect(
            cfg.path(),
            Some(&path_of(&[first.path(), second.path()])),
            &[],
            &probe,
        );
        assert_eq!(*probe.asked.borrow(), [first.path().join(exe_name())]);
    }

    #[test]
    fn a_file_that_does_not_run_is_found_with_an_unknown_version() {
        let on_path = dir_with_kimun();
        let cfg = tempfile::tempdir().unwrap();
        let got = detect(
            cfg.path(),
            Some(&path_of(&[on_path.path()])),
            &[],
            &FakeProbe::failing(),
        );
        assert_eq!(
            got,
            TerminalApp::VersionUnknown {
                path: on_path
                    .path()
                    .join(exe_name())
                    .to_string_lossy()
                    .into_owned()
            }
        );
    }

    #[test]
    fn output_that_is_not_a_kimun_version_is_an_unknown_version() {
        let on_path = dir_with_kimun();
        let cfg = tempfile::tempdir().unwrap();
        for out in [
            "hello\n",
            "",
            "kimun-other 1.0.0\n",
            "kimun\n",
            "kimun abc\n",
        ] {
            let got = detect(
                cfg.path(),
                Some(&path_of(&[on_path.path()])),
                &[],
                &FakeProbe::printing(out),
            );
            assert!(
                matches!(got, TerminalApp::VersionUnknown { .. }),
                "{out:?} gave {got:?}"
            );
        }
    }

    #[test]
    fn a_pre_release_version_is_kept_as_printed() {
        let on_path = dir_with_kimun();
        let cfg = tempfile::tempdir().unwrap();
        let got = detect(
            cfg.path(),
            Some(&path_of(&[on_path.path()])),
            &[],
            &FakeProbe::printing("kimun 0.26.0-beta.1\n"),
        );
        assert_eq!(
            got,
            TerminalApp::Installed {
                version: "0.26.0-beta.1".into()
            }
        );
    }

    #[test]
    fn the_fallback_directories_are_looked_in_after_path() {
        // A desktop app started from a launcher has a minimal PATH: the places
        // brew and cargo install to are still searched.
        let cfg = tempfile::tempdir().unwrap();
        let empty_on_path = tempfile::tempdir().unwrap();
        let fallback = dir_with_kimun();
        let probe = FakeProbe::printing("kimun 0.25.3");
        let got = detect(
            cfg.path(),
            Some(&path_of(&[empty_on_path.path()])),
            &[fallback.path().to_path_buf()],
            &probe,
        );
        assert_eq!(
            got,
            TerminalApp::Installed {
                version: "0.25.3".into()
            }
        );
        assert_eq!(*probe.asked.borrow(), [fallback.path().join(exe_name())]);
    }

    #[test]
    fn path_comes_before_the_fallback_directories() {
        let cfg = tempfile::tempdir().unwrap();
        let on_path = dir_with_kimun();
        let fallback = dir_with_kimun();
        let probe = FakeProbe::printing("kimun 0.25.3");
        detect(
            cfg.path(),
            Some(&path_of(&[on_path.path()])),
            &[fallback.path().to_path_buf()],
            &probe,
        );
        assert_eq!(*probe.asked.borrow(), [on_path.path().join(exe_name())]);
    }

    #[test]
    fn the_fallback_list_names_the_usual_install_places() {
        let home = Path::new("/home/someone");
        let dirs = fallback_dirs(Some(home));
        assert!(dirs.contains(&home.join(".cargo").join("bin")));
        assert!(dirs.contains(&home.join(".local").join("bin")));
        #[cfg(unix)]
        {
            assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
            assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        }
        // Without a home directory, only the places that need none remain.
        assert!(fallback_dirs(None).iter().all(|d| d.is_absolute()));
        assert!(
            !fallback_dirs(None)
                .iter()
                .any(|d| d.ends_with(".cargo/bin"))
        );
    }

    #[cfg(unix)]
    mod process {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::time::Instant;

        fn script(body: &str) -> (tempfile::TempDir, PathBuf) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("kimun");
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            (dir, path)
        }

        #[test]
        fn the_real_probe_reads_what_the_program_prints() {
            let (_dir, path) = script("echo 'kimun 1.2.3'");
            assert_eq!(
                ProcessProbe
                    .version_output(&path, Duration::from_secs(5))
                    .as_deref(),
                Some("kimun 1.2.3\n")
            );
        }

        #[test]
        fn the_real_probe_stops_a_program_that_hangs() {
            let (_dir, path) = script("sleep 30");
            let started = Instant::now();
            let got = ProcessProbe.version_output(&path, Duration::from_millis(200));
            assert_eq!(got, None);
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the limit must hold"
            );
        }

        #[test]
        fn the_real_probe_gives_none_for_a_failing_program() {
            let (_dir, path) = script("echo 'kimun 1.2.3'; exit 3");
            assert_eq!(
                ProcessProbe.version_output(&path, Duration::from_secs(5)),
                None
            );
        }

        #[test]
        fn the_real_probe_gives_none_for_a_file_that_cannot_run() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("kimun");
            fs::write(&path, "not a program").unwrap();
            assert_eq!(
                ProcessProbe.version_output(&path, Duration::from_secs(5)),
                None
            );
        }
    }
}
