//! The eligibility rules and the whole update flow, with a fake network, service and server.
#![cfg(test)]
// The fixtures run shell scripts as the server binary.
#![cfg(unix)]

use std::sync::Mutex;
use std::time::Duration;

use super::*;
use release::tests_support::{make_archive, sign_for_test};

struct Service {
    active: bool,
    restarts: Mutex<usize>,
}

impl ServiceControl for Service {
    fn is_active(&self) -> bool {
        self.active
    }
    fn restart(&self) -> Result<(), String> {
        *self.restarts.lock().unwrap() += 1;
        Ok(())
    }
}

fn active() -> Box<dyn ServiceControl> {
    Box::new(Service {
        active: true,
        restarts: Mutex::new(0),
    })
}

fn stopped() -> Box<dyn ServiceControl> {
    Box::new(Service {
        active: false,
        restarts: Mutex::new(0),
    })
}

/// A machine as `install-server.sh --service` leaves it.
struct Machine {
    _root: tempfile::TempDir,
    config: PathBuf,
    home: PathBuf,
    bin_dir: PathBuf,
}

fn machine() -> Machine {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().to_path_buf();
    let config = home.join(".config/kimun");
    let bin_dir = home.join(".local/bin");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::write(
        bin_dir.join("kimun-server"),
        "#!/bin/sh\necho kimun-server 1.0.0\n",
    )
    .unwrap();
    kimun_core::system::make_executable(&bin_dir.join("kimun-server")).unwrap();
    let unit = service_file(ServiceKind::Systemd, &home);
    std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
    std::fs::write(&unit, "[Unit]\n").unwrap();
    write_marker(&config, &bin_dir, "systemd");
    Machine {
        _root: root,
        config,
        home,
        bin_dir,
    }
}

fn write_marker(config: &Path, bin_dir: &Path, service: &str) {
    std::fs::write(
        config.join("server-install.toml"),
        format!(
            "installer = \"install-server.sh\"\ninstall_dir = \"{}\"\nservice = \"{service}\"\n",
            bin_dir.display()
        ),
    )
    .unwrap();
}

const LOCAL: &str = "http://127.0.0.1:7573";

fn check_on(
    m: &Machine,
    url: &str,
    control: fn() -> Box<dyn ServiceControl>,
) -> Result<ServerInstall, Unavailable> {
    check_with(&m.config, &m.home, url, &|_| control())
}

#[cfg(target_os = "linux")]
mod eligibility {
    use super::*;

    #[test]
    fn a_script_install_with_a_running_service_on_this_machine_may_be_updated() {
        let m = machine();
        let install = check_on(&m, LOCAL, active).unwrap();
        assert_eq!(install.install_dir, m.bin_dir);
        assert_eq!(install.service, ServiceKind::Systemd);
    }

    #[test]
    fn a_remote_server_may_not() {
        let m = machine();
        assert_eq!(
            check_on(&m, "http://192.168.1.5:7573", active),
            Err(Unavailable::NotLocal)
        );
    }

    #[test]
    fn no_marker_means_no_update() {
        let m = machine();
        std::fs::remove_file(m.config.join("server-install.toml")).unwrap();
        assert_eq!(
            check_on(&m, LOCAL, active),
            Err(Unavailable::Marker(MarkerProblem::Missing))
        );
    }

    #[test]
    fn a_marker_with_no_service_means_no_update() {
        let m = machine();
        write_marker(&m.config, &m.bin_dir, "none");
        assert_eq!(
            check_on(&m, LOCAL, active),
            Err(Unavailable::Marker(MarkerProblem::NoService))
        );
    }

    #[test]
    fn a_launchd_marker_on_linux_is_refused() {
        let m = machine();
        write_marker(&m.config, &m.bin_dir, "launchd");
        assert_eq!(
            check_on(&m, LOCAL, active),
            Err(Unavailable::WrongServiceManager)
        );
    }

    #[test]
    fn a_stale_marker_is_refused() {
        let m = machine();
        std::fs::remove_file(m.bin_dir.join("kimun-server")).unwrap();
        assert!(matches!(
            check_on(&m, LOCAL, active),
            Err(Unavailable::BinaryMissing(_))
        ));
        std::fs::write(m.bin_dir.join("kimun-server"), "x").unwrap();
        std::fs::remove_file(service_file(ServiceKind::Systemd, &m.home)).unwrap();
        assert!(matches!(
            check_on(&m, LOCAL, active),
            Err(Unavailable::ServiceFileMissing(_))
        ));
    }

    #[test]
    fn a_service_that_is_not_running_is_not_the_server_that_answers() {
        let m = machine();
        assert_eq!(
            check_on(&m, LOCAL, stopped),
            Err(Unavailable::ServiceNotRunning)
        );
    }
}

/// A fake release server: the files by URL.
struct Files(Vec<(String, Vec<u8>)>);

impl Fetch for Files {
    fn get(&self, url: &str, _max: u64) -> Result<Vec<u8>, String> {
        self.0
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| format!("404 {url}"))
    }
}

struct Probe<'a> {
    service: &'a Service,
    new: &'a str,
}

impl VersionProbe for Probe<'_> {
    fn reply(&self) -> Reply {
        Reply::Up(Some(
            if *self.service.restarts.lock().unwrap() >= 1 {
                self.new
            } else {
                "1.0.0"
            }
            .to_string(),
        ))
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

/// A signed release 2.0.0 for `linux-x64` as the manifest would name it.
struct Release {
    files: Files,
    manifest: String,
    key: String,
}

fn release(binary_version: &str, tamper_archive: bool, sign_other_text: bool) -> Release {
    let script = format!("#!/bin/sh\necho kimun-server {binary_version}\n");
    let mut archive = make_archive(&[("./kimun-server", script.as_bytes())]);
    let name = "kimun-server-2.0.0-linux-x64.tar.gz";
    let digest = release::sha256_hex(&archive);
    let checksums = format!("{digest}  {name}\n");
    let signed_text = if sign_other_text {
        "something else\n".to_string()
    } else {
        checksums.clone()
    };
    let (key, sig) = sign_for_test(signed_text.as_bytes());
    if tamper_archive {
        archive.push(0);
    }
    let manifest = format!(
        r#"{{"schema": 1, "server": {{"version": "2.0.0",
  "checksums_url": "https://dl.test/sums", "signature_url": "https://dl.test/sig",
  "assets": {{"{name}": {{"url": "https://dl.test/a.tgz", "sha256": "{digest}"}}}}}}}}"#
    );
    Release {
        files: Files(vec![
            ("https://dl.test/a.tgz".into(), archive),
            ("https://dl.test/sums".into(), checksums.into_bytes()),
            ("https://dl.test/sig".into(), sig.into_bytes()),
        ]),
        manifest,
        key,
    }
}

fn run(
    m: &Machine,
    r: &Release,
    current: Option<&str>,
    service: &Service,
) -> Result<String, UpdateError> {
    let install = ServerInstall {
        install_dir: m.bin_dir.clone(),
        service: ServiceKind::Systemd,
    };
    let probe = Probe {
        service,
        new: "2.0.0",
    };
    let manifest = || Ok(r.manifest.clone());
    let env = Environment {
        fetch: &r.files,
        manifest: &manifest,
        control: service,
        probe: &probe,
        timing: fast(),
        key: &r.key,
        platform: "linux-x64",
    };
    update_with(&install, current, &env, &|_| {})
}

fn bin_text(m: &Machine) -> String {
    std::fs::read_to_string(m.bin_dir.join("kimun-server")).unwrap()
}

#[test]
fn a_signed_release_is_installed_and_the_server_restarted() {
    let m = machine();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    let version = run(&m, &release("2.0.0", false, false), Some("1.0.0"), &service).unwrap();
    assert_eq!(version, "2.0.0");
    assert!(bin_text(&m).contains("2.0.0"));
    assert_eq!(*service.restarts.lock().unwrap(), 1);
}

#[test]
fn a_download_that_does_not_match_the_signed_checksum_changes_nothing() {
    let m = machine();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    let err = run(&m, &release("2.0.0", true, false), Some("1.0.0"), &service).unwrap_err();
    assert!(
        matches!(err, UpdateError::Release(ref w) if w.contains("signed checksum")),
        "{err}"
    );
    assert!(bin_text(&m).contains("1.0.0"));
    assert_eq!(*service.restarts.lock().unwrap(), 0);
}

#[test]
fn a_checksums_file_that_the_key_did_not_sign_changes_nothing() {
    let m = machine();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    let err = run(&m, &release("2.0.0", false, true), Some("1.0.0"), &service).unwrap_err();
    assert!(
        matches!(err, UpdateError::Release(ref w) if w.contains("signature")),
        "{err}"
    );
    assert!(bin_text(&m).contains("1.0.0"));
    assert_eq!(*service.restarts.lock().unwrap(), 0);
}

#[test]
fn a_signature_from_another_key_changes_nothing() {
    let m = machine();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    let mut r = release("2.0.0", false, false);
    r.key = SIGNING_KEY.to_string();
    let err = run(&m, &r, Some("1.0.0"), &service).unwrap_err();
    assert!(matches!(err, UpdateError::Release(_)), "{err}");
    assert!(bin_text(&m).contains("1.0.0"));
}

#[test]
fn a_release_that_is_not_newer_is_not_installed() {
    let m = machine();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    let err = run(&m, &release("2.0.0", false, false), Some("2.0.0"), &service).unwrap_err();
    assert_eq!(err, UpdateError::AlreadyCurrent("2.0.0".into()));
    assert_eq!(*service.restarts.lock().unwrap(), 0);
}

#[test]
fn a_binary_that_reports_another_version_than_the_manifest_changes_nothing() {
    let m = machine();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    let err = run(&m, &release("9.9.9", false, false), Some("1.0.0"), &service).unwrap_err();
    assert!(
        matches!(
            err,
            UpdateError::Install(InstallError::NewBinaryUnusable(_))
        ),
        "{err}"
    );
    assert!(bin_text(&m).contains("1.0.0"));
}

#[test]
fn a_server_that_predates_version_reporting_can_still_be_updated() {
    let m = machine();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    assert!(run(&m, &release("2.0.0", false, false), None, &service).is_ok());
}

#[test]
fn the_version_floor_is_the_installed_binary_not_the_probe() {
    // The probe cannot say what runs (None), and the installed binary already is the release.
    let m = machine();
    std::fs::write(
        m.bin_dir.join("kimun-server"),
        "#!/bin/sh\necho kimun-server 2.0.0\n",
    )
    .unwrap();
    let service = Service {
        active: true,
        restarts: Mutex::new(0),
    };
    let err = run(&m, &release("2.0.0", false, false), None, &service).unwrap_err();
    assert!(
        matches!(err, UpdateError::Install(InstallError::NotNewer { .. })),
        "{err}"
    );
    assert_eq!(*service.restarts.lock().unwrap(), 0);
}

#[cfg(target_os = "linux")]
mod service_port {
    use super::*;

    #[test]
    fn a_url_on_another_port_than_the_service_is_not_this_server() {
        let m = machine();
        std::fs::write(m.config.join("server.toml"), "[server]\nport = 7600\n").unwrap();
        assert_eq!(
            check_on(&m, LOCAL, active),
            Err(Unavailable::NotThisServer {
                url_port: Some(7573),
                service_port: 7600
            })
        );
        assert!(check_on(&m, "http://127.0.0.1:7600", active).is_ok());
    }

    #[test]
    fn with_no_server_toml_the_default_port_is_the_service_port() {
        let m = machine();
        assert!(check_on(&m, "http://localhost:7573", active).is_ok());
        assert!(matches!(
            check_on(&m, "http://localhost", active),
            Err(Unavailable::NotThisServer {
                url_port: Some(80),
                service_port: 7573
            })
        ));
    }

    #[test]
    fn a_server_toml_that_cannot_be_read_falls_back_to_the_default_port() {
        let m = machine();
        std::fs::write(m.config.join("server.toml"), "this is = not [toml").unwrap();
        assert!(check_on(&m, LOCAL, active).is_ok());
    }
}
