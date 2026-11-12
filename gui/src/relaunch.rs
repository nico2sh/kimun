//! Starting the app again after an update replaced it.
//!
//! The new process must not run next to this one: both would open the same
//! vault index. So the relaunch is a small detached helper that waits for this
//! process to exit and then starts the executable. The app asks for it at the
//! moment it quits, never before ([`Relauncher::relaunch_after_exit`]).

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

/// Starts the app again once this process has ended.
pub(crate) trait Relauncher: Send + Sync {
    /// Spawn the detached helper. It returns as soon as the helper runs; the
    /// caller then ends the app.
    fn relaunch_after_exit(&self) -> Result<(), String>;
}

/// The real relauncher: the executable and arguments this process started
/// with, read once at construction. Reading them later is wrong on Linux,
/// where the update renames a new file over the running one and
/// `current_exe` then reports `... (deleted)`.
#[cfg_attr(test, allow(dead_code))] // tests use the recorder
pub(crate) struct SystemRelauncher {
    exe: Option<PathBuf>,
    args: Vec<OsString>,
}

#[cfg_attr(test, allow(dead_code))]
impl SystemRelauncher {
    pub(crate) fn new() -> Self {
        Self {
            exe: std::env::current_exe().ok(),
            args: std::env::args_os().skip(1).collect(),
        }
    }
}

impl Relauncher for SystemRelauncher {
    fn relaunch_after_exit(&self) -> Result<(), String> {
        let exe = self
            .exe
            .as_deref()
            .ok_or("the running program's path is unknown")?;
        let mut command = helper_command(std::process::id(), exe, &self.args);
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        detach(&mut command);
        command
            .spawn()
            .map(drop)
            .map_err(|e| format!("could not start the relaunch helper: {e}"))
    }
}

/// The helper: wait for `pid` to end, then start `exe` with `args`.
#[cfg(unix)]
fn helper_command(pid: u32, exe: &std::path::Path, args: &[OsString]) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; shift; exec "$@""#)
        .arg("kimun-relaunch")
        .arg(pid.to_string())
        .arg(exe)
        .args(args);
    command
}

/// The helper: wait for `pid` to end, then start `exe` with `args`.
#[cfg(windows)]
fn helper_command(pid: u32, exe: &std::path::Path, args: &[OsString]) -> Command {
    let mut command = Command::new("powershell");
    command
        .arg("-NoProfile")
        .arg("-Command")
        .arg(
            "param($p, $exe) Wait-Process -Id $p -ErrorAction SilentlyContinue; \
             Start-Process -FilePath $exe -ArgumentList $args",
        )
        .arg(pid.to_string())
        .arg(exe)
        .args(args);
    command
}

/// Keep the helper alive after this process ends.
#[cfg(unix)]
#[cfg_attr(test, allow(dead_code))]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    // Its own process group: a signal sent to ours does not reach it.
    command.process_group(0);
}

/// Keep the helper alive after this process ends.
#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
}

#[cfg(test)]
pub(crate) use recording::RecordingRelauncher;

#[cfg(test)]
mod recording {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::Relauncher;

    /// Counts the relaunches asked for and starts nothing.
    #[derive(Default)]
    pub(crate) struct RecordingRelauncher {
        calls: AtomicUsize,
    }

    impl RecordingRelauncher {
        pub(crate) fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Relauncher for RecordingRelauncher {
        fn relaunch_after_exit(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The helper waits for the pid, then runs the program with its arguments.
    #[test]
    fn the_helper_waits_for_the_pid_then_runs_the_program() {
        let command = helper_command(
            4242,
            std::path::Path::new("/opt/kimun/kimun-gui"),
            &[OsString::from("--flag")],
        );
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(command.get_program(), "/bin/sh");
        assert_eq!(args[0], "-c");
        assert!(args[1].contains(r#"kill -0 "$1""#), "waits on the pid");
        assert!(args[1].ends_with(r#"exec "$@""#), "then runs the program");
        assert_eq!(
            &args[2..],
            ["kimun-relaunch", "4242", "/opt/kimun/kimun-gui", "--flag"]
        );
    }

    /// End to end on a real shell: the helper starts the program only after
    /// the awaited process has ended.
    #[test]
    fn the_program_starts_only_after_the_awaited_process_ends() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let mut waited = Command::new("sleep").arg("1").spawn().unwrap();
        let mut command = helper_command(
            waited.id(),
            std::path::Path::new("/usr/bin/touch"),
            &[marker.clone().into_os_string()],
        );
        let mut helper = command.spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!marker.exists(), "the awaited process still runs");
        waited.wait().unwrap();
        helper.wait().unwrap();
        assert!(marker.exists(), "the program started after it ended");
    }
}
