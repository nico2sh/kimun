//! Put a verified server binary in place and make the running server use it, or put the old one
//! back (ADR-0059).
//!
//! The steps, in order: stage the new binary next to the old one and run it to see that it works
//! and reports the expected version; move the old binary aside; move the new one in; restart the
//! service; wait for the server to answer with the new version. A failure before the swap leaves
//! everything as it was. A failure after the swap puts the old binary back and restarts again.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kimun_core::system;

use super::marker::{BINARY, ServerInstall};
use super::service::ServiceControl;

/// Answers with the version of the server that is running at the configured URL.
pub trait VersionProbe: Send + Sync {
    /// The running server's version, or `None` when it does not answer.
    fn version(&self) -> Option<String>;
}

/// `GET {url}/health`, which carries the server's version.
pub struct HttpProbe {
    url: String,
}

impl HttpProbe {
    pub fn new(server_url: &str) -> Self {
        Self {
            url: format!("{}/health", server_url.trim().trim_end_matches('/')),
        }
    }
}

impl VersionProbe for HttpProbe {
    fn version(&self) -> Option<String> {
        let response = ureq::get(&self.url)
            .set("User-Agent", crate::update::USER_AGENT)
            .timeout(Duration::from_secs(3))
            .call()
            .ok()?;
        let body: serde_json::Value = serde_json::from_str(&response.into_string().ok()?).ok()?;
        body.get("version")?.as_str().map(str::to_string)
    }
}

/// How long to wait, and how often to ask.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// For the new server to answer with the new version. The server loads its embedding model at
    /// start, so this is generous.
    pub wait_new: Duration,
    /// For the old server to answer again after a rollback.
    pub wait_old: Duration,
    /// Between two questions.
    pub interval: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            wait_new: Duration::from_secs(120),
            wait_old: Duration::from_secs(60),
            interval: Duration::from_secs(1),
        }
    }
}

/// What the install needs to restart the server and see it come back.
pub struct Restart<'a> {
    pub control: &'a dyn ServiceControl,
    pub probe: &'a dyn VersionProbe,
    pub timing: Timing,
}

/// A step the caller can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Downloading,
    Installing,
    Restarting,
    WaitingForServer,
    RollingBack,
}

/// What a rollback achieved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rollback {
    /// The old binary is back and the old server answers again.
    Restored,
    /// Something in the rollback failed: the text says what, and what to do.
    Failed(String),
}

/// Why the install did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// The new binary does not run, or does not report the version it should. Nothing changed.
    NewBinaryUnusable(String),
    /// The swap failed before the service was touched. The old binary is in place.
    Swap(String),
    /// The swap happened, but the server did not come up with the new version.
    NotRunning { reason: String, rollback: Rollback },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewBinaryUnusable(why) => {
                write!(
                    f,
                    "the new server does not work on this machine ({why}). Nothing was changed"
                )
            }
            Self::Swap(why) => write!(
                f,
                "could not put the new server in place ({why}). Nothing was changed"
            ),
            Self::NotRunning {
                reason,
                rollback: Rollback::Restored,
            } => {
                write!(
                    f,
                    "the new server did not start ({reason}). The old server is back"
                )
            }
            Self::NotRunning {
                reason,
                rollback: Rollback::Failed(why),
            } => {
                write!(
                    f,
                    "the new server did not start ({reason}), and the old server could not be restored ({why})"
                )
            }
        }
    }
}

impl std::error::Error for InstallError {}

fn staged_path(dir: &Path) -> PathBuf {
    dir.join(format!(".{BINARY}.new"))
}

fn backup_path(dir: &Path) -> PathBuf {
    dir.join(format!("{BINARY}.old"))
}

/// Run `binary --version` and return the version it reports (`kimun-server X.Y.Z`).
fn reported_version(binary: &Path) -> Result<String, String> {
    // A just-written executable can be briefly busy when another thread forks at that moment.
    let mut last = String::new();
    for _ in 0..20 {
        match Command::new(binary)
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                return text
                    .split_whitespace()
                    .nth(1)
                    .map(str::to_string)
                    .ok_or_else(|| format!("it printed {:?} for --version", text.trim()));
            }
            Ok(out) => return Err(format!("--version failed ({})", out.status)),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                last = e.to_string();
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("it cannot run: {e}")),
        }
    }
    Err(format!("it cannot run: {last}"))
}

/// Ask `probe` until it reports `want`, or `timeout` passes.
fn wait_for(probe: &dyn VersionProbe, want: &str, timeout: Duration, interval: Duration) -> bool {
    let start = Instant::now();
    loop {
        if probe.version().as_deref() == Some(want) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(interval);
    }
}

/// Put the old binary back and restart. The old server must answer again, when its version is known.
fn roll_back(
    install: &ServerInstall,
    current_version: Option<&str>,
    restart: &Restart,
) -> Rollback {
    let (control, probe, timing) = (restart.control, restart.probe, &restart.timing);
    let bin = install.binary();
    let backup = backup_path(&install.install_dir);
    if let Err(e) = system::move_file(&backup, &bin) {
        return Rollback::Failed(format!(
            "could not put {} back: {e}. The old file is {}",
            bin.display(),
            backup.display()
        ));
    }
    if let Err(e) = control.restart() {
        return Rollback::Failed(format!("{e}. Restart the service yourself"));
    }
    match current_version {
        Some(old) if !wait_for(probe, old, timing.wait_old, timing.interval) => Rollback::Failed(
            "the old server did not answer after the restart. Look at the service log".to_string(),
        ),
        _ => Rollback::Restored,
    }
}

/// Install `binary` (the server, version `new_version`) and restart the service.
///
/// `current_version` is the version the server reports now, when it reports one. It lets a
/// rollback check that the old server is back.
pub fn install_and_restart(
    install: &ServerInstall,
    binary: &[u8],
    new_version: &str,
    current_version: Option<&str>,
    restart: &Restart,
    progress: &dyn Fn(Step),
) -> Result<(), InstallError> {
    let (control, probe, timing) = (restart.control, restart.probe, &restart.timing);
    progress(Step::Installing);
    let dir = &install.install_dir;
    let bin = install.binary();
    let staged = staged_path(dir);
    let backup = backup_path(dir);

    // 1. Stage the new binary and make sure it works. Nothing is changed yet.
    system::replace_atomically(&staged, binary).map_err(|e| InstallError::Swap(e.to_string()))?;
    let staged_check = system::make_executable(&staged)
        .map_err(|e| e.to_string())
        .and_then(|()| reported_version(&staged))
        .and_then(|v| {
            if v == new_version {
                Ok(())
            } else {
                Err(format!("it reports version {v}, not {new_version}"))
            }
        });
    if let Err(why) = staged_check {
        let _ = system::remove_file(&staged);
        return Err(InstallError::NewBinaryUnusable(why));
    }

    // 2. Swap: old aside, new in. A failed second move puts the old one back.
    let swap = system::remove_file(&backup)
        .and_then(|()| system::move_file(&bin, &backup))
        .and_then(|()| {
            system::move_file(&staged, &bin).inspect_err(|_| {
                let _ = system::move_file(&backup, &bin);
            })
        });
    if let Err(e) = swap {
        let _ = system::remove_file(&staged);
        return Err(InstallError::Swap(e.to_string()));
    }

    // 3. Restart and wait for the new version. Any failure from here puts the old server back.
    progress(Step::Restarting);
    let started = control.restart().and_then(|()| {
        progress(Step::WaitingForServer);
        if wait_for(probe, new_version, timing.wait_new, timing.interval) {
            Ok(())
        } else {
            Err(format!(
                "it did not report version {new_version} within {} seconds",
                timing.wait_new.as_secs()
            ))
        }
    });
    match started {
        Ok(()) => {
            let _ = system::remove_file(&backup);
            Ok(())
        }
        Err(reason) => {
            progress(Step::RollingBack);
            let rollback = roll_back(install, current_version, restart);
            Err(InstallError::NotRunning { reason, rollback })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::server_manage::marker::ServiceKind;

    fn script(version: &str) -> Vec<u8> {
        format!("#!/bin/sh\necho kimun-server {version}\n").into_bytes()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        install: ServerInstall,
    }

    /// An install folder holding an "old" server that reports 1.0.0.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let install = ServerInstall {
            install_dir: dir.path().to_path_buf(),
            service: ServiceKind::Systemd,
        };
        std::fs::write(install.binary(), script("1.0.0")).unwrap();
        system::make_executable(&install.binary()).unwrap();
        Fixture { _dir: dir, install }
    }

    fn contents(path: &Path) -> String {
        String::from_utf8(std::fs::read(path).unwrap()).unwrap()
    }

    #[derive(Default)]
    struct FakeService {
        restarts: Mutex<usize>,
        /// Restart number (1-based) that fails.
        fail_on: Mutex<Vec<usize>>,
    }

    impl ServiceControl for FakeService {
        fn is_active(&self) -> bool {
            true
        }
        fn restart(&self) -> Result<(), String> {
            let mut n = self.restarts.lock().unwrap();
            *n += 1;
            if self.fail_on.lock().unwrap().contains(&*n) {
                Err("systemctl said no".into())
            } else {
                Ok(())
            }
        }
    }

    /// Reports `after_restart` once the service was restarted `restarts_needed` times, and the old
    /// version before that.
    struct FakeProbe<'a> {
        service: &'a FakeService,
        answers: Box<dyn Fn(usize) -> Option<String> + Send + Sync + 'a>,
    }

    impl VersionProbe for FakeProbe<'_> {
        fn version(&self) -> Option<String> {
            (self.answers)(*self.service.restarts.lock().unwrap())
        }
    }

    fn fast() -> Timing {
        Timing {
            wait_new: Duration::from_millis(60),
            wait_old: Duration::from_millis(60),
            interval: Duration::from_millis(5),
        }
    }

    fn run(
        f: &Fixture,
        binary: &[u8],
        service: &FakeService,
        probe: &FakeProbe,
    ) -> Result<(), InstallError> {
        let restart = Restart {
            control: service,
            probe,
            timing: fast(),
        };
        install_and_restart(
            &f.install,
            binary,
            "2.0.0",
            Some("1.0.0"),
            &restart,
            &|_| {},
        )
    }

    #[test]
    fn a_good_update_swaps_restarts_and_waits_for_the_new_version() {
        let f = fixture();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|n| Some(if n >= 1 { "2.0.0" } else { "1.0.0" }.into())),
        };
        run(&f, &script("2.0.0"), &service, &probe).unwrap();
        assert!(contents(&f.install.binary()).contains("2.0.0"));
        assert_eq!(*service.restarts.lock().unwrap(), 1);
        assert!(
            !backup_path(&f.install.install_dir).exists(),
            "the backup goes after success"
        );
        assert!(!staged_path(&f.install.install_dir).exists());
    }

    #[test]
    fn a_new_binary_that_reports_the_wrong_version_changes_nothing() {
        let f = fixture();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| Some("1.0.0".into())),
        };
        let err = run(&f, &script("3.0.0"), &service, &probe).unwrap_err();
        assert!(
            matches!(err, InstallError::NewBinaryUnusable(ref w) if w.contains("3.0.0")),
            "{err}"
        );
        assert!(contents(&f.install.binary()).contains("1.0.0"));
        assert_eq!(*service.restarts.lock().unwrap(), 0);
        assert!(!staged_path(&f.install.install_dir).exists());
    }

    #[test]
    fn a_new_binary_that_does_not_run_changes_nothing() {
        let f = fixture();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| None),
        };
        let err = run(&f, b"this is not a program", &service, &probe).unwrap_err();
        assert!(matches!(err, InstallError::NewBinaryUnusable(_)), "{err}");
        assert!(contents(&f.install.binary()).contains("1.0.0"));
        assert_eq!(*service.restarts.lock().unwrap(), 0);
    }

    #[test]
    fn a_failed_restart_puts_the_old_server_back() {
        let f = fixture();
        let service = FakeService::default();
        *service.fail_on.lock().unwrap() = vec![1];
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| Some("1.0.0".into())),
        };
        let err = run(&f, &script("2.0.0"), &service, &probe).unwrap_err();
        assert!(
            matches!(
                err,
                InstallError::NotRunning {
                    rollback: Rollback::Restored,
                    ..
                }
            ),
            "{err}"
        );
        assert!(
            contents(&f.install.binary()).contains("1.0.0"),
            "the old binary is back"
        );
        assert_eq!(
            *service.restarts.lock().unwrap(),
            2,
            "restarted again after the rollback"
        );
    }

    #[test]
    fn a_server_that_never_reports_the_new_version_is_rolled_back() {
        let f = fixture();
        let service = FakeService::default();
        // The old server keeps answering: the case of a container on the same URL.
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| Some("1.0.0".into())),
        };
        let err = run(&f, &script("2.0.0"), &service, &probe).unwrap_err();
        assert!(
            matches!(
                err,
                InstallError::NotRunning {
                    rollback: Rollback::Restored,
                    ..
                }
            ),
            "{err}"
        );
        assert!(err.to_string().contains("The old server is back"));
        assert!(contents(&f.install.binary()).contains("1.0.0"));
    }

    #[test]
    fn a_rollback_that_cannot_restart_says_so() {
        let f = fixture();
        let service = FakeService::default();
        *service.fail_on.lock().unwrap() = vec![1, 2];
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| None),
        };
        let err = run(&f, &script("2.0.0"), &service, &probe).unwrap_err();
        match err {
            InstallError::NotRunning {
                rollback: Rollback::Failed(why),
                ..
            } => {
                assert!(why.contains("Restart the service yourself"), "{why}")
            }
            other => panic!("{other}"),
        }
        assert!(
            contents(&f.install.binary()).contains("1.0.0"),
            "the binary is back even so"
        );
    }

    #[test]
    fn a_rollback_whose_old_server_stays_silent_is_not_called_restored() {
        let f = fixture();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| None),
        };
        let err = run(&f, &script("2.0.0"), &service, &probe).unwrap_err();
        assert!(
            matches!(
                err,
                InstallError::NotRunning {
                    rollback: Rollback::Failed(_),
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_stale_backup_from_an_earlier_update_is_replaced() {
        let f = fixture();
        std::fs::write(backup_path(&f.install.install_dir), "stale").unwrap();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|n| Some(if n >= 1 { "2.0.0" } else { "1.0.0" }.into())),
        };
        run(&f, &script("2.0.0"), &service, &probe).unwrap();
        assert!(contents(&f.install.binary()).contains("2.0.0"));
    }
}
