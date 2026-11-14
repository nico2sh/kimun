//! Managing a Kimün server that runs on this machine: the update from inside the app (ADR-0059).
//!
//! Everything about the server's own life is here and nowhere else: reading the install marker,
//! deciding whether the app may update the server, downloading and verifying a release, swapping
//! the binary, restarting the service, and rolling back. The windows show a button and some
//! progress text. They hold none of this logic.
//!
//! The app updates a server only when the install script put it on this machine and set up a
//! service for it. Every other server (remote, Docker, Cargo, installed before the marker, or
//! installed without a service) keeps the plain update notice.

mod install;
mod marker;
mod release;
mod service;
mod target;

use std::path::{Path, PathBuf};

pub use install::{
    HttpProbe, InstallError, Restart, Rollback, Step, Timing, VersionProbe, install_and_restart,
};
pub use marker::{MarkerProblem, ServerInstall, ServiceKind};
pub use release::{Fetch, Network, SIGNING_KEY, ServerRelease};
pub use service::{ServiceControl, SystemService, service_file};
pub use target::is_local_url;

/// Why the app does not offer to update the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The server update runs on Linux and macOS only.
    Platform,
    /// The configured URL does not point at this machine.
    NotLocal,
    /// The install marker does not allow it.
    Marker(MarkerProblem),
    /// The marker names a service manager that this system does not use.
    WrongServiceManager,
    /// The marker names a binary that is not there.
    BinaryMissing(PathBuf),
    /// The marker names a service file that is not there.
    ServiceFileMissing(PathBuf),
    /// The service is not running, so it is not the server that answers at the URL.
    ServiceNotRunning,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Platform => write!(f, "the app updates a server on Linux and macOS only"),
            Self::NotLocal => write!(f, "the server runs on another machine"),
            Self::Marker(p) => write!(f, "{p}"),
            Self::WrongServiceManager => write!(
                f,
                "the install marker names a service manager that this system does not use"
            ),
            Self::BinaryMissing(p) => write!(f, "{} is missing", p.display()),
            Self::ServiceFileMissing(p) => write!(f, "{} is missing", p.display()),
            Self::ServiceNotRunning => write!(f, "the server's service is not running"),
        }
    }
}

/// Whether the app may update the server at `server_url`. Reads the marker in `config_dir`, looks
/// at the binary and the service file under `home`, and asks the service manager made by
/// `control_for`. Blocking: it runs a process.
pub fn check_with(
    config_dir: &Path,
    home: &Path,
    server_url: &str,
    control_for: &dyn Fn(ServiceKind) -> Box<dyn ServiceControl>,
) -> Result<ServerInstall, Unavailable> {
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        return Err(Unavailable::Platform);
    }
    if !is_local_url(server_url) {
        return Err(Unavailable::NotLocal);
    }
    let install = marker::load(config_dir).map_err(Unavailable::Marker)?;
    let right_manager = match install.service {
        ServiceKind::Systemd => cfg!(target_os = "linux"),
        ServiceKind::Launchd => cfg!(target_os = "macos"),
    };
    if !right_manager {
        return Err(Unavailable::WrongServiceManager);
    }
    let binary = install.binary();
    if !binary.is_file() {
        return Err(Unavailable::BinaryMissing(binary));
    }
    let unit = service_file(install.service, home);
    if !unit.is_file() {
        return Err(Unavailable::ServiceFileMissing(unit));
    }
    if !control_for(install.service).is_active() {
        return Err(Unavailable::ServiceNotRunning);
    }
    Ok(install)
}

/// [`check_with`] on this machine's real service manager and home folder.
pub fn check(config_dir: &Path, server_url: &str) -> Result<ServerInstall, Unavailable> {
    let home = kimun_core::system::home().map_err(|_| Unavailable::Platform)?;
    check_with(config_dir, home.as_path(), server_url, &|kind| {
        Box::new(SystemService::new(kind))
    })
}

/// Async [`check`], on the blocking pool.
pub async fn check_now(
    config_dir: PathBuf,
    server_url: String,
) -> Result<ServerInstall, Unavailable> {
    tokio::task::spawn_blocking(move || check(&config_dir, &server_url))
        .await
        .unwrap_or(Err(Unavailable::Platform))
}

/// Why an update did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// The app may not update this server.
    Unavailable(Unavailable),
    /// The release could not be found, downloaded, or verified. The server was not touched.
    Release(String),
    /// The manifest names no newer release than the running server.
    AlreadyCurrent(String),
    /// The install or the restart failed. The text says what state the server is in.
    Install(InstallError),
    /// The update task itself failed.
    Task(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(u) => write!(f, "the app cannot update this server: {u}"),
            Self::Release(why) => write!(f, "{why}. The server was not changed"),
            Self::AlreadyCurrent(v) => write!(f, "the server already runs version {v}"),
            Self::Install(e) => write!(f, "{e}"),
            Self::Task(why) => write!(f, "the update task failed: {why}"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// What a window needs from server management: whether the app may update the server at a URL,
/// and the update itself. Both block. The window runs them on the blocking pool. The real one
/// is [`SystemServerUpdates`]. A test passes a fake, so no test touches a service or the network.
pub trait ServerUpdates: Send + Sync {
    /// Whether the app may update the server at `server_url`.
    fn available(&self, server_url: &str) -> Result<ServerInstall, Unavailable>;
    /// Update the server at `server_url`. Returns the new version.
    fn update(&self, server_url: &str, progress: &dyn Fn(Step)) -> Result<String, UpdateError>;
}

/// [`ServerUpdates`] on this machine.
pub struct SystemServerUpdates {
    config_dir: PathBuf,
}

impl SystemServerUpdates {
    /// For the marker in `config_dir`, the app's configuration folder.
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }
}

impl ServerUpdates for SystemServerUpdates {
    fn available(&self, server_url: &str) -> Result<ServerInstall, Unavailable> {
        check(&self.config_dir, server_url)
    }

    fn update(&self, server_url: &str, progress: &dyn Fn(Step)) -> Result<String, UpdateError> {
        update_blocking(&self.config_dir, server_url, progress)
    }
}

/// Everything an update needs from the outside, so the whole flow runs in a test.
pub struct Environment<'a> {
    pub fetch: &'a dyn Fetch,
    /// The release manifest's text.
    pub manifest: &'a dyn Fn() -> Result<String, String>,
    pub control: &'a dyn ServiceControl,
    pub probe: &'a dyn VersionProbe,
    pub timing: Timing,
    /// The key that signed the release.
    pub key: &'a str,
    /// The release platform, for example `linux-x64`.
    pub platform: &'a str,
}

/// Update the server that `install` describes to the release in the manifest.
///
/// `current_version` is what the server reports now. The caller has already called [`check`].
/// Returns the new version.
pub fn update_with(
    install: &ServerInstall,
    current_version: Option<&str>,
    env: &Environment,
    progress: &dyn Fn(Step),
) -> Result<String, UpdateError> {
    progress(Step::Downloading);
    let body = (env.manifest)().map_err(UpdateError::Release)?;
    let release = release::parse_manifest(&body, env.platform).map_err(UpdateError::Release)?;
    if let Some(current) = current_version
        && !crate::update::is_newer(&release.version, current)
    {
        return Err(UpdateError::AlreadyCurrent(current.to_string()));
    }
    let binary = release::fetch_verified_binary(env.fetch, env.key, &release)
        .map_err(UpdateError::Release)?;
    install_and_restart(
        install,
        &binary,
        &release.version,
        current_version,
        &Restart {
            control: env.control,
            probe: env.probe,
            timing: env.timing,
        },
        progress,
    )
    .map_err(UpdateError::Install)?;
    Ok(release.version)
}

/// Update the local server at `server_url`: check that the app may, then do it. Returns the new
/// version. Blocking: call it on the blocking pool, or use [`update`].
pub fn update_blocking(
    config_dir: &Path,
    server_url: &str,
    progress: &dyn Fn(Step),
) -> Result<String, UpdateError> {
    // Checked again here: the machine may have changed since the button was shown.
    let install = check(config_dir, server_url).map_err(UpdateError::Unavailable)?;
    let platform =
        crate::update::release_platform().ok_or(UpdateError::Unavailable(Unavailable::Platform))?;
    let probe = HttpProbe::new(server_url);
    let control = SystemService::new(install.service);
    let current = probe.version();
    let env = Environment {
        fetch: &Network,
        manifest: &|| {
            crate::update::fetch_manifest_body()
                .map_err(|e| format!("could not read the release manifest: {e}"))
        },
        control: &control,
        probe: &probe,
        timing: Timing::default(),
        key: SIGNING_KEY,
        platform,
    };
    update_with(&install, current.as_deref(), &env, progress)
}

/// Async [`update_blocking`]. `progress` is called from the blocking pool.
pub async fn update(
    config_dir: PathBuf,
    server_url: String,
    progress: impl Fn(Step) + Send + 'static,
) -> Result<String, UpdateError> {
    tokio::task::spawn_blocking(move || update_blocking(&config_dir, &server_url, &progress))
        .await
        .map_err(|e| UpdateError::Task(e.to_string()))?
}

#[cfg(test)]
mod tests;
