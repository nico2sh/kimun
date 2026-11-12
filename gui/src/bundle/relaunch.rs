//! Starting the app again after an update replaced it.
//!
//! The new process must not run next to this one: both would open the same
//! vault index. So the relaunch is a small detached helper that waits for this
//! process to exit and then starts the executable. The app asks for it at the
//! moment it quits, never before ([`Relauncher::relaunch_after_exit`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
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
        let (program, args) = launch_target(exe, &self.args);
        let mut command = helper_command(std::process::id(), &program, &args);
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

/// What the helper starts. An app inside a macOS bundle is opened through
/// `open`, like a double click, so the system activates it properly; running the
/// inner executable directly would not. Anything else runs as it started.
fn launch_target(exe: &Path, args: &[OsString]) -> (PathBuf, Vec<OsString>) {
    match super::installed_bundle(exe) {
        Some(bundle) => {
            let mut open_args = vec![bundle.into_os_string()];
            if !args.is_empty() {
                open_args.push("--args".into());
                open_args.extend(args.iter().cloned());
            }
            (PathBuf::from("/usr/bin/open"), open_args)
        }
        None => (exe.to_path_buf(), args.to_vec()),
    }
}

/// The helper: wait for `pid` to end, then start `exe` with `args`.
#[cfg(unix)]
fn helper_command(pid: u32, exe: &Path, args: &[OsString]) -> Command {
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
fn helper_command(pid: u32, exe: &Path, args: &[OsString]) -> Command {
    windows_helper(pid, exe, args)
}

/// The Windows helper's script. `powershell -Command` does not bind trailing
/// arguments to a `param()` block, so the values come from the environment
/// (set by [`windows_helper`]) and no path is ever quoted into the script.
/// `Start-Process -ArgumentList` rejects an empty list, hence the `if`.
#[cfg(any(windows, test))]
const WINDOWS_SCRIPT: &str = "Wait-Process -Id ([int]$env:KIMUN_RELAUNCH_PID) -ErrorAction SilentlyContinue; \
$a = @($env:KIMUN_RELAUNCH_ARGS -split \"`n\" | Where-Object { $_ }); \
if ($a.Count -gt 0) { Start-Process -FilePath $env:KIMUN_RELAUNCH_EXE -ArgumentList $a } \
else { Start-Process -FilePath $env:KIMUN_RELAUNCH_EXE }";

/// The Windows helper, built on every platform under test.
#[cfg(any(windows, test))]
fn windows_helper(pid: u32, exe: &Path, args: &[OsString]) -> Command {
    let joined = args
        .iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    let mut command = Command::new("powershell");
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(WINDOWS_SCRIPT)
        .env("KIMUN_RELAUNCH_PID", pid.to_string())
        .env("KIMUN_RELAUNCH_EXE", exe)
        .env("KIMUN_RELAUNCH_ARGS", joined);
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

    /// A bundled app is opened through `open`, with its arguments after
    /// `--args`; a plain binary runs as it started.
    #[test]
    fn a_bundled_app_is_opened_and_a_plain_binary_is_run() {
        let exe = Path::new("/Users/me/Applications/Kimun.app/Contents/MacOS/kimun-gui");
        assert_eq!(
            launch_target(exe, &[]),
            (
                PathBuf::from("/usr/bin/open"),
                vec![OsString::from("/Users/me/Applications/Kimun.app")]
            )
        );
        assert_eq!(
            launch_target(exe, &[OsString::from("--flag")]).1,
            [
                OsString::from("/Users/me/Applications/Kimun.app"),
                OsString::from("--args"),
                OsString::from("--flag")
            ]
        );
        let plain = Path::new("/home/me/.local/bin/kimun-gui");
        assert_eq!(
            launch_target(plain, &[OsString::from("--flag")]),
            (plain.to_path_buf(), vec![OsString::from("--flag")])
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

/// The Windows helper is built on every platform under test, so its command is
/// checked here even though only Windows runs it.
#[cfg(test)]
mod windows_tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::Path;

    use super::*;

    fn envs(command: &Command) -> HashMap<String, String> {
        command
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect()
    }

    /// `powershell -Command` does not bind trailing arguments to a `param()`
    /// block: they would be glued onto the script text. The values travel in
    /// the environment instead, so no quoting of paths can go wrong.
    #[test]
    fn the_values_travel_in_the_environment_not_as_trailing_arguments() {
        let command = windows_helper(
            4242,
            Path::new(r"C:\Program Files\Kimun\kimun-gui.exe"),
            &[OsString::from("--flag"), OsString::from("two words")],
        );
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(command.get_program(), "powershell");
        assert_eq!(
            args.len(),
            4,
            "only the fixed switches and the script: {args:?}"
        );
        assert_eq!(&args[..3], ["-NoProfile", "-NonInteractive", "-Command"]);
        assert!(!args[3].contains("param("), "no param() block");
        assert!(args[3].contains("Wait-Process -Id ([int]$env:KIMUN_RELAUNCH_PID)"));
        assert!(args[3].contains("Start-Process -FilePath $env:KIMUN_RELAUNCH_EXE"));
        let envs = envs(&command);
        assert_eq!(envs["KIMUN_RELAUNCH_PID"], "4242");
        assert_eq!(
            envs["KIMUN_RELAUNCH_EXE"],
            r"C:\Program Files\Kimun\kimun-gui.exe"
        );
        assert_eq!(envs["KIMUN_RELAUNCH_ARGS"], "--flag\ntwo words");
    }

    /// `Start-Process -ArgumentList @()` is an error in PowerShell: no
    /// arguments means the switch is left out.
    #[test]
    fn no_arguments_leave_the_argument_list_out() {
        let command = windows_helper(1, Path::new(r"C:\k.exe"), &[]);
        let script = command
            .get_args()
            .last()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(script.contains("if ($a.Count -gt 0)"));
        assert_eq!(envs(&command)["KIMUN_RELAUNCH_ARGS"], "");
    }
}
