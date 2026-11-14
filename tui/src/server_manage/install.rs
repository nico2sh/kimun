//! Put a verified server binary in place and make the running server use it, or put the old one
//! back (ADR-0059).
//!
//! The steps, in order: take the update lock; read the installed version and refuse anything that
//! is not newer; stage the new binary and run it to see that it reports the expected version;
//! swap it in ([`Swap`]); restart the service; wait until the new version answers steadily and the
//! service is still running. A failure before the swap changes nothing. A failure after it puts the
//! old binary back, restarts again, and checks that the old server answers.

use std::time::{Duration, Instant};

use super::lock::{LockError, UpdateLock};
use super::marker::ServerInstall;
use super::service::ServiceControl;
use super::swap::Swap;

/// What a server answers at its health address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// No answer: the server is down or not reachable.
    Down,
    /// It answered. `None` is a server that predates version reporting.
    Up(Option<String>),
}

/// Asks the server that runs at the configured URL how it is.
pub trait VersionProbe: Send + Sync {
    fn reply(&self) -> Reply;
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
    fn reply(&self) -> Reply {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(3))
            .build();
        let Ok(response) = agent
            .get(&self.url)
            .set("User-Agent", crate::update::USER_AGENT)
            .call()
        else {
            return Reply::Down;
        };
        let version = response
            .into_string()
            .ok()
            .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
            .and_then(|json| json.get("version")?.as_str().map(str::to_string));
        Reply::Up(version)
    }
}

/// How long to wait, how often to ask, and how steady an answer must be.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// For the new server to answer with the new version. The server loads its embedding model at
    /// start, so this is generous.
    pub wait_new: Duration,
    /// For the old server to answer again after a rollback.
    pub wait_old: Duration,
    /// Between two questions.
    pub interval: Duration,
    /// How many answers in a row must carry the new version before the update counts as done. A
    /// server that answers once and then crashes must not pass.
    pub steady: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            wait_new: Duration::from_secs(120),
            wait_old: Duration::from_secs(60),
            interval: Duration::from_secs(1),
            steady: 5,
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
    /// Another update of this install is running.
    Busy,
    /// The update lock could not be made.
    Lock(String),
    /// The installed binary does not report its version, so "newer" cannot be decided.
    InstalledVersionUnknown,
    /// A version is not a plain `X.Y.Z`, so "newer" cannot be decided.
    VersionUnreadable(String),
    /// The release is not newer than the installed binary. This is the version floor.
    NotNewer { installed: String, release: String },
    /// The new binary does not run, or does not report the version it should. Nothing changed.
    NewBinaryUnusable(String),
    /// The swap failed before the service was touched. The old binary is in place.
    Swap(String),
    /// The swap happened, but the server did not come up steadily with the new version.
    NotRunning { reason: String, rollback: Rollback },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(f, "another update of this server is already running"),
            Self::Lock(why) => write!(f, "could not start the update ({why}). Nothing was changed"),
            Self::InstalledVersionUnknown => write!(
                f,
                "the installed server does not report its version, so the app cannot tell if the release is newer. Nothing was changed"
            ),
            Self::VersionUnreadable(v) => write!(
                f,
                "the version {v:?} is not a plain X.Y.Z number, so the app cannot tell if it is newer. Nothing was changed"
            ),
            Self::NotNewer { installed, release } => write!(
                f,
                "version {release} is not newer than the installed {installed}. Nothing was changed"
            ),
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
            } => write!(
                f,
                "the new server did not start ({reason}), and the old server could not be restored ({why})"
            ),
        }
    }
}

impl std::error::Error for InstallError {}

/// Ask `probe` until `done` accepts `needed` replies in a row, or `timeout` passes.
fn wait_until(
    probe: &dyn VersionProbe,
    timeout: Duration,
    interval: Duration,
    needed: u32,
    done: impl Fn(&Reply) -> bool,
) -> bool {
    let start = Instant::now();
    let mut in_a_row = 0;
    loop {
        if done(&probe.reply()) {
            in_a_row += 1;
            if in_a_row >= needed {
                return true;
            }
        } else {
            in_a_row = 0;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(interval);
    }
}

/// The new version answers, `steady` times in a row, and the service still runs.
fn wait_for_new(restart: &Restart, new_version: &str) -> Result<(), String> {
    let t = &restart.timing;
    let steady = wait_until(
        restart.probe,
        t.wait_new,
        t.interval,
        t.steady.max(1),
        |r| matches!(r, Reply::Up(Some(v)) if v == new_version),
    );
    if !steady {
        return Err(format!(
            "it did not report version {new_version} steadily within {} seconds",
            t.wait_new.as_secs()
        ));
    }
    if !restart.control.is_active() {
        return Err("the service stopped after it answered".to_string());
    }
    Ok(())
}

/// Put the old binary back and restart. The old server must answer again: with its own version, or
/// with no version when it predates version reporting.
fn roll_back(swap: &Swap, old_version: &str, legacy: bool, restart: &Restart) -> Rollback {
    if let Err(e) = swap.roll_back() {
        return Rollback::Failed(e);
    }
    if let Err(e) = restart.control.restart() {
        return Rollback::Failed(format!("{e}. Restart the service yourself"));
    }
    let t = &restart.timing;
    let back = wait_until(restart.probe, t.wait_old, t.interval, 1, |r| {
        // An answer with no version counts only when the server answered that way before the update.
        (legacy && matches!(r, Reply::Up(None)))
            || matches!(r, Reply::Up(Some(v)) if v == old_version)
    });
    if back {
        Rollback::Restored
    } else {
        Rollback::Failed(
            "the old server did not answer after the restart. Look at the service log".to_string(),
        )
    }
}

/// Install `binary` (the server, version `new_version`) and restart the service.
pub fn install_and_restart(
    install: &ServerInstall,
    binary: &[u8],
    new_version: &str,
    restart: &Restart,
    progress: &dyn Fn(Step),
) -> Result<(), InstallError> {
    progress(Step::Installing);
    let _lock = UpdateLock::acquire(&install.install_dir).map_err(|e| match e {
        LockError::Busy => InstallError::Busy,
        LockError::Io(why) => InstallError::Lock(why),
    })?;
    let swap = Swap::new(install);

    // The installed binary decides what is "newer". A manifest, a probe, or a signed old release
    // cannot talk the app into a downgrade.
    let installed = swap
        .installed_version()
        .ok_or(InstallError::InstalledVersionUnknown)?;
    for v in [new_version, installed.as_str()] {
        if crate::update::parse_version(v).is_none() {
            return Err(InstallError::VersionUnreadable(v.to_string()));
        }
    }
    if !crate::update::is_newer(new_version, &installed) {
        return Err(InstallError::NotNewer {
            installed,
            release: new_version.to_string(),
        });
    }

    // A server that predates version reporting answers without a version. Only then may a rollback
    // accept an answer without a version.
    let legacy = matches!(restart.probe.reply(), Reply::Up(None));

    swap.stage(binary, new_version)
        .map_err(InstallError::NewBinaryUnusable)?;
    swap.commit().map_err(InstallError::Swap)?;

    progress(Step::Restarting);
    let started = restart.control.restart().and_then(|()| {
        progress(Step::WaitingForServer);
        wait_for_new(restart, new_version)
    });
    match started {
        Ok(()) => {
            swap.drop_backup();
            Ok(())
        }
        Err(reason) => {
            progress(Step::RollingBack);
            let rollback = roll_back(&swap, &installed, legacy, restart);
            Err(InstallError::NotRunning { reason, rollback })
        }
    }
}

// The fixtures run shell scripts as the server binary.
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::path::Path;
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
        kimun_core::system::make_executable(&install.binary()).unwrap();
        Fixture { _dir: dir, install }
    }

    fn contents(path: &Path) -> String {
        String::from_utf8(std::fs::read(path).unwrap()).unwrap()
    }

    #[derive(Default)]
    struct FakeService {
        restarts: Mutex<usize>,
        /// Restart numbers (1-based) that fail.
        fail_on: Mutex<Vec<usize>>,
        /// Whether the service runs when asked. Defaults to running.
        stopped: Mutex<bool>,
    }

    impl ServiceControl for FakeService {
        fn is_active(&self) -> bool {
            !*self.stopped.lock().unwrap()
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

    /// Answers by the number of restarts so far.
    struct FakeProbe<'a> {
        service: &'a FakeService,
        answers: Box<dyn Fn(usize) -> Reply + Send + Sync + 'a>,
    }

    impl VersionProbe for FakeProbe<'_> {
        fn reply(&self) -> Reply {
            (self.answers)(*self.service.restarts.lock().unwrap())
        }
    }

    fn up(v: &str) -> Reply {
        Reply::Up(Some(v.to_string()))
    }

    /// The old server before the first restart, the new one after it.
    fn healthy(service: &FakeService) -> FakeProbe<'_> {
        FakeProbe {
            service,
            answers: Box::new(|n| up(if n >= 1 { "2.0.0" } else { "1.0.0" })),
        }
    }

    fn fast() -> Timing {
        Timing {
            wait_new: Duration::from_millis(80),
            wait_old: Duration::from_millis(80),
            interval: Duration::from_millis(5),
            steady: 3,
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
        install_and_restart(&f.install, binary, "2.0.0", &restart, &|_| {})
    }

    fn files(f: &Fixture) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let (b, s, o) = Swap::new(&f.install).paths().to_owned_triple();
        (b, s, o)
    }

    trait Triple {
        fn to_owned_triple(self) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf);
    }
    impl Triple for (&Path, &Path, &Path) {
        fn to_owned_triple(self) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
            (
                self.0.to_path_buf(),
                self.1.to_path_buf(),
                self.2.to_path_buf(),
            )
        }
    }

    #[test]
    fn a_good_update_swaps_restarts_and_waits_for_a_steady_new_version() {
        let f = fixture();
        let service = FakeService::default();
        run(&f, &script("2.0.0"), &service, &healthy(&service)).unwrap();
        let (bin, staged, backup) = files(&f);
        assert!(contents(&bin).contains("2.0.0"));
        assert_eq!(*service.restarts.lock().unwrap(), 1);
        assert!(!backup.exists(), "the backup goes after success");
        assert!(!staged.exists());
        assert!(
            !f.install
                .install_dir
                .join(".kimun-server.update.lock")
                .exists(),
            "the lock is released"
        );
    }

    #[test]
    fn the_binary_is_never_missing_during_the_swap() {
        // The backup is a link to the old inode, and one rename replaces the real name.
        let f = fixture();
        let swap = Swap::new(&f.install);
        swap.stage(&script("2.0.0"), "2.0.0").unwrap();
        let (bin, _, backup) = files(&f);
        swap.commit().unwrap();
        assert!(bin.exists());
        assert!(contents(&bin).contains("2.0.0"));
        assert!(
            contents(&backup).contains("1.0.0"),
            "the old binary is kept"
        );
        swap.roll_back().unwrap();
        assert!(contents(&bin).contains("1.0.0"));
    }

    #[test]
    fn a_release_that_is_not_newer_than_the_installed_binary_is_refused() {
        // The floor: the installed binary decides, whatever the manifest or the probe say.
        for release in ["1.0.0", "0.9.0"] {
            let f = fixture();
            let service = FakeService::default();
            let probe = healthy(&service);
            let restart = Restart {
                control: &service,
                probe: &probe,
                timing: fast(),
            };
            let err = install_and_restart(&f.install, &script(release), release, &restart, &|_| {})
                .unwrap_err();
            assert!(matches!(err, InstallError::NotNewer { .. }), "{err}");
            assert!(contents(&f.install.binary()).contains("1.0.0"));
            assert_eq!(*service.restarts.lock().unwrap(), 0);
        }
    }

    #[test]
    fn an_installed_binary_that_does_not_report_a_version_blocks_the_update() {
        let f = fixture();
        std::fs::write(f.install.binary(), "#!/bin/sh\necho oops\n").unwrap();
        let service = FakeService::default();
        let err = run(&f, &script("2.0.0"), &service, &healthy(&service)).unwrap_err();
        assert_eq!(err, InstallError::InstalledVersionUnknown);
        assert_eq!(*service.restarts.lock().unwrap(), 0);
    }

    #[test]
    fn a_new_binary_that_reports_the_wrong_version_changes_nothing() {
        let f = fixture();
        let service = FakeService::default();
        let err = run(&f, &script("3.0.0"), &service, &healthy(&service)).unwrap_err();
        assert!(
            matches!(err, InstallError::NewBinaryUnusable(ref w) if w.contains("3.0.0")),
            "{err}"
        );
        assert!(contents(&f.install.binary()).contains("1.0.0"));
        assert_eq!(*service.restarts.lock().unwrap(), 0);
        assert!(!files(&f).1.exists());
    }

    #[test]
    fn a_new_binary_that_does_not_run_changes_nothing() {
        let f = fixture();
        let service = FakeService::default();
        let err = run(&f, b"this is not a program", &service, &healthy(&service)).unwrap_err();
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
            answers: Box::new(|_| up("1.0.0")),
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
        // The old server keeps answering: the case of a container on the same URL.
        let f = fixture();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| up("1.0.0")),
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
    fn a_server_that_answers_once_and_then_stops_is_not_a_success() {
        // Up for one probe after the restart, then silent. The old binary is still there to go back to.
        let f = fixture();
        let service = FakeService::default();
        let calls = Mutex::new(0u32);
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(move |n| {
                if n == 0 {
                    return up("1.0.0");
                }
                let mut c = calls.lock().unwrap();
                *c += 1;
                // The first answer after the restart is the new version, then it crashes. After the
                // rollback restart (restart number 2) the old server answers.
                match (n, *c) {
                    (1, 1) => up("2.0.0"),
                    (1, _) => Reply::Down,
                    _ => up("1.0.0"),
                }
            }),
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
        assert!(contents(&f.install.binary()).contains("1.0.0"));
    }

    #[test]
    fn a_service_that_stops_after_answering_is_rolled_back() {
        let f = fixture();
        let service = FakeService::default();
        *service.stopped.lock().unwrap() = true;
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|n| up(if n == 1 { "2.0.0" } else { "1.0.0" })),
        };
        let err = run(&f, &script("2.0.0"), &service, &probe).unwrap_err();
        assert!(
            matches!(err, InstallError::NotRunning { ref reason, .. } if reason.contains("service stopped")),
            "{err}"
        );
        assert!(contents(&f.install.binary()).contains("1.0.0"));
    }

    #[test]
    fn a_rollback_that_cannot_restart_says_so() {
        let f = fixture();
        let service = FakeService::default();
        *service.fail_on.lock().unwrap() = vec![1, 2];
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| Reply::Down),
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
            answers: Box::new(|_| Reply::Down),
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
    fn an_old_server_that_predates_version_reporting_counts_as_back_when_it_answers() {
        let f = fixture();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|_| Reply::Up(None)),
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
    }

    #[test]
    fn an_answer_without_a_version_is_not_the_old_server_unless_it_answered_so_before() {
        // Down before the update, "up with no version" after the rollback: some other process.
        let f = fixture();
        let service = FakeService::default();
        let probe = FakeProbe {
            service: &service,
            answers: Box::new(|n| if n == 0 { Reply::Down } else { Reply::Up(None) }),
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
    fn a_version_that_is_not_plain_is_named_not_called_old() {
        let f = fixture();
        std::fs::write(
            f.install.binary(),
            "#!/bin/sh\necho kimun-server 1.0.0-rc.1\n",
        )
        .unwrap();
        let service = FakeService::default();
        let err = run(&f, &script("2.0.0"), &service, &healthy(&service)).unwrap_err();
        assert_eq!(err, InstallError::VersionUnreadable("1.0.0-rc.1".into()));
    }

    #[test]
    fn a_stale_backup_from_an_earlier_update_is_replaced() {
        let f = fixture();
        std::fs::write(files(&f).2, "stale").unwrap();
        let service = FakeService::default();
        run(&f, &script("2.0.0"), &service, &healthy(&service)).unwrap();
        assert!(contents(&f.install.binary()).contains("2.0.0"));
    }

    #[test]
    fn a_second_update_while_one_runs_is_refused() {
        let f = fixture();
        let _held = UpdateLock::acquire(&f.install.install_dir).unwrap();
        let service = FakeService::default();
        let err = run(&f, &script("2.0.0"), &service, &healthy(&service)).unwrap_err();
        assert_eq!(err, InstallError::Busy);
        assert!(contents(&f.install.binary()).contains("1.0.0"));
        assert_eq!(*service.restarts.lock().unwrap(), 0);
    }

    #[test]
    fn a_staged_file_left_by_a_crash_is_never_installed() {
        let f = fixture();
        std::fs::write(files(&f).1, script("9.9.9")).unwrap();
        let service = FakeService::default();
        run(&f, &script("2.0.0"), &service, &healthy(&service)).unwrap();
        assert!(contents(&f.install.binary()).contains("2.0.0"));
    }
}
