//! The **update flow**'s rules, shared by both frontends: what a frontend may
//! offer for a newer release (install it in place, point at the package
//! manager, or point at the releases page), the flow's state machine, the
//! words it reports in, and the seam to the release backend.
//!
//! The release machinery itself is [`crate::update`] (the GitHub query, the
//! throttle and its `update_state.toml`, the dismissal, the in-place binary
//! swap); nothing here re-implements it. Every [`UpdateBackend`] method is
//! **blocking**: a frontend runs it on tokio's blocking pool.
//!
//! **The flow** ([`UpdateMachine`]): Idle → Checking → Available → Installing
//! → Installed (a restart runs it) or Failed (retry). Skipping a version
//! (`dismiss`) leaves it Skipped. A startup check is silent unless it finds a
//! newer version that was not skipped, and says so once per session; a
//! manual check bypasses the throttle and always reports (asked for while a
//! throttled check runs, a forced one follows it and is the report).

use crate::update::{self, InstallChannel, UpdateStatus};

/// The product's name in the flow's messages.
pub const PRODUCT: &str = "Kimün";

/// Which frontend asks: the release ships a self-update binary per frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frontend {
    /// The terminal app (`kimun`), whose raw binary every release ships.
    Terminal,
    /// The desktop app (`kimun-gui`).
    Desktop,
}

/// Whether a release ships the desktop app's binary. `build.yml` uploads
/// `kimun-gui-<version>-<platform>` beside the terminal app's, and
/// [`crate::update::apply`] takes the asset of the app that is running
/// ([`update::Product`]): the desktop app only ever replaces itself with its
/// own. A kill switch: set `false` to make the desktop app offer the releases
/// page instead of installing (a release train without desktop assets).
pub const DESKTOP_BINARY_PUBLISHED: bool = true;

impl Frontend {
    /// The product whose release asset this frontend replaces itself with.
    pub fn product(self) -> update::Product {
        match self {
            Self::Terminal => update::Product::Terminal,
            Self::Desktop => update::Product::Desktop,
        }
    }

    /// Whether the release publishes this frontend's binary, so that it can
    /// replace itself.
    pub fn binary_published(self) -> bool {
        match self {
            Self::Terminal => true,
            Self::Desktop => DESKTOP_BINARY_PUBLISHED,
        }
    }
}

/// What a frontend offers for a newer release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOffer {
    /// Download, verify and replace the running binary.
    Install,
    /// A package manager owns the install (or it could not be told how it
    /// was installed): the user updates with it. `command` is the upgrade
    /// command when one is known.
    PackageManager { command: Option<&'static str> },
    /// Self-update is allowed but this frontend's binary is not published:
    /// the user downloads it from the releases page.
    Download,
}

/// What can be offered on `channel` for a frontend whose binary is
/// `binary_published` ([`Frontend::binary_published`]). The channel rules
/// are [`InstallChannel`]'s: anything not self-update eligible is the
/// package manager's.
pub fn offer(channel: InstallChannel, binary_published: bool) -> UpdateOffer {
    if channel == InstallChannel::Bundle {
        // Not a package manager's: a whole new download replaces the bundle.
        UpdateOffer::Download
    } else if !channel.self_update_eligible() {
        UpdateOffer::PackageManager {
            command: channel.upgrade_hint(),
        }
    } else if binary_published {
        UpdateOffer::Install
    } else {
        UpdateOffer::Download
    }
}

// ---- The words --------------------------------------------------------------

/// A newer version was found.
pub fn available_text(version: &str) -> String {
    format!("{PRODUCT} {version} is available")
}

/// A manual check found nothing newer than `current`.
pub fn up_to_date_text(current: &str) -> String {
    format!("{PRODUCT} is up to date ({current})")
}

/// A manual check failed.
pub fn check_failed_text(error: &str) -> String {
    format!("Couldn't check for updates: {error}")
}

/// The install finished; the new binary runs from the next start.
pub fn installed_text(version: &str) -> String {
    format!("Updated to {version}. Restart {PRODUCT} to use it.")
}

/// The install finished and the app starts again by itself (the desktop app;
/// the terminal app only asks, see [`installed_text`]).
pub fn installed_restarting_text(version: &str) -> String {
    format!("Updated to {version}. Restarting {PRODUCT}…")
}

/// The install failed (or could not start).
pub fn install_failed_text(error: &str) -> String {
    format!("Couldn't install the update: {error}")
}

/// The version line: running → newest.
pub fn versions_text(status: &UpdateStatus) -> String {
    format!("{PRODUCT} {} → {}", status.current, status.latest)
}

/// What a manual check says while it runs.
pub const CHECKING_TEXT: &str = "Checking for updates…";
/// What the install says while it runs.
pub const INSTALLING_TEXT: &str = "Downloading and installing the update…";
/// What a package-managed install says instead of offering to install.
pub const PACKAGE_MANAGER_TEXT: &str = "Update available. Update with your package manager.";
/// What an install whose binary is not published says instead.
pub const DOWNLOAD_TEXT: &str = "Update available. Download it from the releases page.";
/// The window's close while the install runs.
pub const CLOSE_REFUSED_TEXT: &str = "Wait until the update is installed, then close again.";

// ---- The state machine -----------------------------------------------------------

/// Where the update flow stands.
#[derive(Debug, Clone)]
pub enum UpdatePhase {
    /// Nothing known (no check yet, or nothing newer).
    Idle,
    /// A check runs; `manual` when it reports whatever it finds, `force`
    /// when it bypasses the throttle.
    Checking { manual: bool, force: bool },
    /// A newer version is known.
    Available(UpdateStatus),
    /// That version is being installed.
    Installing(UpdateStatus),
    /// `version` is installed; a restart runs it.
    Installed { version: String },
    /// The install failed; it can be tried again.
    Failed { status: UpdateStatus, error: String },
    /// The user skipped `version` (persisted by the backend's `dismiss`).
    Skipped(String),
}

/// Whether a check may start ([`UpdateMachine::begin_check`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStart {
    /// Run the check; `force` bypasses the throttle (a manual check).
    Run { force: bool },
    /// A check or an install runs: this one is coalesced into it.
    Busy,
    /// An update is already installed this session: nothing newer to find
    /// until a restart (the running version is still the old one).
    Installed,
}

/// What a check that landed should tell the user
/// ([`UpdateMachine::check_landed`]).
#[derive(Debug, Clone)]
pub enum CheckReport {
    /// A newer version: say it is available (with a way to the details).
    Available(UpdateStatus),
    /// A manual check found nothing newer: say so, with this text.
    UpToDate(String),
    /// A manual check failed: say so, with this text.
    Failed(String),
    /// Nothing to say (a startup check that found nothing, was offline, or
    /// found a version already announced or skipped).
    Silent,
    /// The check that landed was throttled and a manual check waits on it:
    /// its answer is dropped (a throttled "nothing known" is no "up to
    /// date"). Run a forced check now and give its result to
    /// [`UpdateMachine::check_landed`]; the flow is still checking.
    RunForced,
}

/// The update flow's state machine. See the module docs.
#[derive(Debug, Clone)]
pub struct UpdateMachine {
    phase: UpdatePhase,
    /// What the phase was before the running check: a failed check goes
    /// back to it.
    before_check: Option<UpdatePhase>,
    /// The version already announced this session: a startup check does not
    /// announce it again.
    announced: Option<String>,
    /// A manual check asked for while a throttled one runs: a forced check
    /// runs once that one lands ([`CheckReport::RunForced`]).
    forced_after: bool,
}

impl Default for UpdateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateMachine {
    pub fn new() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            before_check: None,
            announced: None,
            forced_after: false,
        }
    }

    pub fn phase(&self) -> &UpdatePhase {
        &self.phase
    }

    /// The newer version known now, while it can be acted on (offered,
    /// installing, or failed to install).
    pub fn known(&self) -> Option<&UpdateStatus> {
        match &self.phase {
            UpdatePhase::Available(status)
            | UpdatePhase::Installing(status)
            | UpdatePhase::Failed { status, .. } => Some(status),
            _ => None,
        }
    }

    /// The phase to display: while a check runs, the phase it replaced, so
    /// a known version (or a failure) does not vanish for the round trip.
    pub fn shown_phase(&self) -> &UpdatePhase {
        match (&self.phase, &self.before_check) {
            (UpdatePhase::Checking { .. }, Some(before)) => before,
            _ => &self.phase,
        }
    }

    /// [`Self::known`] over [`Self::shown_phase`].
    pub fn shown(&self) -> Option<&UpdateStatus> {
        match self.shown_phase() {
            UpdatePhase::Available(status)
            | UpdatePhase::Installing(status)
            | UpdatePhase::Failed { status, .. } => Some(status),
            _ => None,
        }
    }

    pub fn is_checking(&self) -> bool {
        matches!(self.phase, UpdatePhase::Checking { .. })
    }

    pub fn is_installing(&self) -> bool {
        matches!(self.phase, UpdatePhase::Installing(_))
    }

    pub fn is_installed(&self) -> bool {
        matches!(self.phase, UpdatePhase::Installed { .. })
    }

    /// A check is asked for (`manual`: by the user, so it bypasses the
    /// throttle and reports). One asked for while a check runs joins it: a
    /// manual ask makes the running check report, and when that check is
    /// throttled (a startup check) a forced one follows it, whose answer is
    /// the report ([`CheckReport::RunForced`]).
    pub fn begin_check(&mut self, manual: bool) -> CheckStart {
        match &mut self.phase {
            UpdatePhase::Checking {
                manual: running,
                force,
            } => {
                *running |= manual;
                if manual && !*force {
                    self.forced_after = true;
                }
                CheckStart::Busy
            }
            UpdatePhase::Installing(_) => CheckStart::Busy,
            UpdatePhase::Installed { .. } => CheckStart::Installed,
            _ => {
                let checking = UpdatePhase::Checking {
                    manual,
                    force: manual,
                };
                let before = std::mem::replace(&mut self.phase, checking);
                self.before_check = Some(before);
                self.forced_after = false;
                CheckStart::Run { force: manual }
            }
        }
    }

    /// The check's result. Ignored unless a check runs. A throttled check
    /// a manual one waits on is not reported: the forced check runs instead
    /// ([`CheckReport::RunForced`]), still checking.
    pub fn check_landed(&mut self, result: Result<Option<UpdateStatus>, String>) -> CheckReport {
        let UpdatePhase::Checking { manual, .. } = self.phase else {
            return CheckReport::Silent;
        };
        if std::mem::take(&mut self.forced_after) {
            self.phase = UpdatePhase::Checking {
                manual: true,
                force: true,
            };
            return CheckReport::RunForced;
        }
        let before = self.before_check.take().unwrap_or(UpdatePhase::Idle);
        match result {
            Ok(Some(status)) if status.update_available => {
                let version = status.latest.clone();
                if !manual && status.dismissed {
                    self.phase = UpdatePhase::Skipped(version);
                    return CheckReport::Silent;
                }
                self.phase = UpdatePhase::Available(status.clone());
                let repeat = self.announced.as_deref() == Some(version.as_str());
                if manual || !repeat {
                    self.announced = Some(version);
                    CheckReport::Available(status)
                } else {
                    CheckReport::Silent
                }
            }
            Ok(found) => {
                self.phase = UpdatePhase::Idle;
                if manual {
                    let current = found
                        .map(|s| s.current)
                        .unwrap_or_else(|| update::CURRENT_VERSION.to_string());
                    CheckReport::UpToDate(up_to_date_text(&current))
                } else {
                    CheckReport::Silent
                }
            }
            Err(error) => {
                self.phase = before;
                if manual {
                    CheckReport::Failed(check_failed_text(&error))
                } else {
                    CheckReport::Silent
                }
            }
        }
    }

    /// Skip the known version: the version to persist, or `None` when
    /// there is none to skip (or it is being installed).
    pub fn dismiss(&mut self) -> Option<String> {
        let version = match &self.phase {
            UpdatePhase::Available(status) | UpdatePhase::Failed { status, .. } => {
                status.latest.clone()
            }
            _ => return None,
        };
        self.phase = UpdatePhase::Skipped(version.clone());
        Some(version)
    }

    /// Start installing the known version, when `offer` allows it: the
    /// status to install, or `None` (nothing known, already installing,
    /// or not an [`UpdateOffer::Install`]).
    pub fn begin_install(&mut self, offer: &UpdateOffer) -> Option<UpdateStatus> {
        if *offer != UpdateOffer::Install {
            return None;
        }
        let status = match &self.phase {
            UpdatePhase::Available(status) | UpdatePhase::Failed { status, .. } => status.clone(),
            _ => return None,
        };
        self.phase = UpdatePhase::Installing(status.clone());
        Some(status)
    }

    /// The install could not start (the open note could not be saved
    /// first): back to offering it, with why.
    pub fn install_refused(&mut self, error: String) {
        if let UpdatePhase::Available(status) | UpdatePhase::Failed { status, .. } = &self.phase {
            self.phase = UpdatePhase::Failed {
                status: status.clone(),
                error,
            };
        }
    }

    /// The install's result: the version installed, or why it failed.
    /// Ignored unless an install runs.
    pub fn install_landed(&mut self, result: Result<String, String>) {
        let UpdatePhase::Installing(status) = &self.phase else {
            return;
        };
        self.phase = match result {
            Ok(version) => UpdatePhase::Installed { version },
            Err(error) => UpdatePhase::Failed {
                status: status.clone(),
                error,
            },
        };
    }

    /// `Later` after a failure: the version is offered again, clean. While
    /// a check runs, the phase it replaced (the one shown) is cleaned, so
    /// the check going back to it does not bring the failure back.
    pub fn later(&mut self) {
        let checking = self.is_checking();
        let shown = match self.before_check.as_mut() {
            Some(before) if checking => before,
            _ => &mut self.phase,
        };
        if let UpdatePhase::Failed { status, .. } = shown {
            *shown = UpdatePhase::Available(status.clone());
        }
    }
}

// ---- The backend seam ----------------------------------------------------------

/// Where a frontend gets releases from and installs them. Every method is
/// blocking (network, files): run it on the blocking pool.
pub trait UpdateBackend: Send + Sync {
    /// Check for a newer release; `force` bypasses the throttle
    /// ([`crate::update::check`]). `Ok(None)`: throttled with nothing known.
    fn check(&self, force: bool) -> Result<Option<UpdateStatus>, String>;
    /// Download and install the newest release in place of the running
    /// binary: the version installed.
    fn install(&self, status: &UpdateStatus) -> Result<String, String>;
    /// Persist that `version` was skipped.
    fn dismiss(&self, version: &str) -> Result<(), String>;
    /// The human releases page.
    fn releases_url(&self) -> String;
    /// Whether this frontend's binary is published ([`Frontend`]).
    fn binary_published(&self) -> bool;
    /// What to offer for `status`.
    fn offer(&self, status: &UpdateStatus) -> UpdateOffer {
        offer(status.channel, self.binary_published())
    }
}

/// The real backend: [`crate::update`] (GitHub, `update_state.toml` in
/// kimün's config directory).
#[derive(Debug, Clone, Copy)]
pub struct SystemUpdates {
    frontend: Frontend,
}

impl SystemUpdates {
    pub fn new(frontend: Frontend) -> Self {
        Self { frontend }
    }
}

/// An install error, in words. A release without the desktop app's asset
/// (cut before desktop builds were published) is not the user's problem to
/// decode: say what it is.
fn install_error(frontend: Frontend, error: &update::UpdateError) -> String {
    match (frontend, error) {
        (Frontend::Desktop, update::UpdateError::MissingAsset(name))
            if name.starts_with("kimun-gui-") =>
        {
            format!(
                "this release has no desktop build for this system ({name}); see the releases page"
            )
        }
        _ => error.to_string(),
    }
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    crate::settings::config_dir().map_err(|e| e.to_string())
}

impl UpdateBackend for SystemUpdates {
    fn check(&self, force: bool) -> Result<Option<UpdateStatus>, String> {
        update::check(&config_dir()?, force).map_err(|e| e.to_string())
    }

    fn install(&self, status: &UpdateStatus) -> Result<String, String> {
        // Never swap in a binary that is not this frontend's (see
        // `DESKTOP_BINARY_PUBLISHED`), whatever a caller decided.
        if offer(status.channel, self.binary_published()) != UpdateOffer::Install {
            return Err("this install can't update itself".to_string());
        }
        let latest = update::fetch_latest().map_err(|e| e.to_string())?;
        update::apply(&latest, self.frontend.product())
            .map_err(|e| install_error(self.frontend, &e))?;
        Ok(latest.version)
    }

    fn dismiss(&self, version: &str) -> Result<(), String> {
        update::dismiss(&config_dir()?, version).map_err(|e| e.to_string())
    }

    fn releases_url(&self) -> String {
        update::releases_url().to_string()
    }

    fn binary_published(&self) -> bool {
        self.frontend.binary_published()
    }
}

/// The scripted backend: tests and the `scripted-updates` feature only,
/// never the shipped build.
#[cfg(any(test, feature = "scripted-updates"))]
mod scripted {
    use super::*;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    /// A scripted backend: no network, no files. The tests' fake, and the
    /// manual gate's (`KIMUN_GUI_FAKE_UPDATE`, [`ScriptedUpdates::from_spec`]).
    /// A check can be held until [`Self::release`] to stand for a slow network.
    pub struct ScriptedUpdates {
        script: Mutex<Script>,
        /// Whether a check may land (`false`: held).
        open: Mutex<bool>,
        opened: Condvar,
    }

    #[derive(Debug, Clone)]
    struct Script {
        /// The newest release (`None`: nothing newer than the running version),
        /// or the check's error.
        latest: Result<Option<String>, String>,
        channel: InstallChannel,
        published: bool,
        install: Result<(), String>,
        install_delay: Duration,
        dismissed: Option<String>,
        /// Each check's `force`, in order.
        checks: Vec<bool>,
        installs: usize,
        /// A check without `force` answers "nothing known" (`Ok(None)`), as
        /// the real throttle does with a fresh state file and no version
        /// cached.
        throttled: bool,
    }

    /// The releases page the scripted backend names.
    pub const SCRIPTED_RELEASES_URL: &str = "https://example.invalid/kimun/releases";

    impl Default for ScriptedUpdates {
        fn default() -> Self {
            Self::up_to_date()
        }
    }

    impl ScriptedUpdates {
        /// Nothing newer than the running version.
        pub fn up_to_date() -> Self {
            Self {
                script: Mutex::new(Script {
                    latest: Ok(None),
                    channel: InstallChannel::Direct,
                    published: true,
                    install: Ok(()),
                    install_delay: Duration::ZERO,
                    dismissed: None,
                    checks: Vec::new(),
                    installs: 0,
                    throttled: false,
                }),
                open: Mutex::new(true),
                opened: Condvar::new(),
            }
        }

        /// `version` is the newest release, newer than the running one, on a
        /// self-update install whose binary is published.
        pub fn with_release(version: &str) -> Self {
            let this = Self::up_to_date();
            this.edit(|s| s.latest = Ok(Some(version.to_string())));
            this
        }

        /// Every check fails with `error` (offline).
        pub fn offline(error: &str) -> Self {
            let this = Self::up_to_date();
            this.edit(|s| s.latest = Err(error.to_string()));
            this
        }

        /// The manual gate's fake, from `KIMUN_GUI_FAKE_UPDATE`:
        /// `<version>` (installable), `<version>:fail` (the install fails),
        /// `<version>:managed` (a Homebrew install), `<version>:download` (the
        /// binary is not published), `offline`, or `current` (up to date).
        /// The install takes two seconds and touches nothing. `None` for an
        /// empty or unknown spec.
        pub fn from_spec(spec: &str) -> Option<Self> {
            let spec = spec.trim();
            match spec {
                "" => return None,
                "offline" => return Some(Self::offline("network unreachable (simulated)")),
                "current" => return Some(Self::up_to_date()),
                _ => {}
            }
            let (version, mode) = spec.split_once(':').unwrap_or((spec, ""));
            let valid = version.split('.').count() == 3
                && version.split('.').all(|p| p.parse::<u64>().is_ok());
            if !valid {
                return None;
            }
            let this = Self::with_release(version).install_delay(Duration::from_secs(2));
            match mode {
                "" => {}
                "fail" => {
                    this.edit(|s| s.install = Err("checksum mismatch (simulated)".to_string()))
                }
                "managed" => this.edit(|s| s.channel = InstallChannel::Brew),
                "download" => this.edit(|s| s.published = false),
                _ => return None,
            }
            Some(this)
        }

        fn edit(&self, f: impl FnOnce(&mut Script)) {
            f(&mut self.script.lock().unwrap_or_else(|e| e.into_inner()));
        }

        fn read<R>(&self, f: impl FnOnce(&Script) -> R) -> R {
            f(&self.script.lock().unwrap_or_else(|e| e.into_inner()))
        }

        /// The install channel the statuses report.
        pub fn channel(self, channel: InstallChannel) -> Self {
            self.edit(|s| s.channel = channel);
            self
        }

        /// Whether this frontend's binary is published.
        pub fn published(self, published: bool) -> Self {
            self.edit(|s| s.published = published);
            self
        }

        /// The install fails with `error` (until [`Self::set_install`]).
        pub fn install_fails(self, error: &str) -> Self {
            self.edit(|s| s.install = Err(error.to_string()));
            self
        }

        /// How long an install takes.
        pub fn install_delay(self, delay: Duration) -> Self {
            self.edit(|s| s.install_delay = delay);
            self
        }

        /// Checks without `force` are throttled with nothing cached: they
        /// answer `Ok(None)` (see `throttled` in the script).
        pub fn throttled(self) -> Self {
            self.edit(|s| s.throttled = true);
            self
        }

        /// Later checks fail with `error` (offline).
        pub fn go_offline(&self, error: &str) {
            self.edit(|s| s.latest = Err(error.to_string()));
        }

        /// What later installs do.
        pub fn set_install(&self, result: Result<(), String>) {
            self.edit(|s| s.install = result);
        }

        /// Hold every check until [`Self::release`].
        pub fn hold(&self) {
            *self.open.lock().unwrap_or_else(|e| e.into_inner()) = false;
        }

        /// Let held checks land.
        pub fn release(&self) {
            *self.open.lock().unwrap_or_else(|e| e.into_inner()) = true;
            self.opened.notify_all();
        }

        /// Each check's `force`, in order.
        pub fn checks(&self) -> Vec<bool> {
            self.read(|s| s.checks.clone())
        }

        /// How many installs ran.
        pub fn installs(&self) -> usize {
            self.read(|s| s.installs)
        }

        /// The version skipped, if any.
        pub fn dismissed(&self) -> Option<String> {
            self.read(|s| s.dismissed.clone())
        }
    }

    impl UpdateBackend for ScriptedUpdates {
        fn check(&self, force: bool) -> Result<Option<UpdateStatus>, String> {
            self.edit(|s| s.checks.push(force));
            let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            while !*open {
                open = self.opened.wait(open).unwrap_or_else(|e| e.into_inner());
            }
            drop(open);
            self.read(|s| {
                if s.throttled && !force {
                    return Ok(None);
                }
                let current = update::CURRENT_VERSION.to_string();
                Ok(Some(match s.latest.clone()? {
                    Some(latest) => UpdateStatus {
                        dismissed: s.dismissed.as_deref() == Some(latest.as_str()),
                        current,
                        latest,
                        channel: s.channel,
                        update_available: true,
                    },
                    None => UpdateStatus {
                        latest: current.clone(),
                        current,
                        channel: s.channel,
                        update_available: false,
                        dismissed: false,
                    },
                }))
            })
        }

        fn install(&self, status: &UpdateStatus) -> Result<String, String> {
            let (delay, result) = self.read(|s| (s.install_delay, s.install.clone()));
            self.edit(|s| s.installs += 1);
            std::thread::sleep(delay);
            result.map(|()| status.latest.clone())
        }

        fn dismiss(&self, version: &str) -> Result<(), String> {
            self.edit(|s| s.dismissed = Some(version.to_string()));
            Ok(())
        }

        fn releases_url(&self) -> String {
            SCRIPTED_RELEASES_URL.to_string()
        }

        fn binary_published(&self) -> bool {
            self.read(|s| s.published)
        }
    }
}
#[cfg(any(test, feature = "scripted-updates"))]
pub use scripted::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::UpdateState;
    use std::time::Duration;

    fn status(latest: &str, channel: InstallChannel, dismissed: bool) -> UpdateStatus {
        UpdateStatus {
            current: "0.1.0".into(),
            latest: latest.into(),
            channel,
            update_available: true,
            dismissed,
        }
    }

    fn up_to_date() -> UpdateStatus {
        UpdateStatus {
            current: "0.1.0".into(),
            latest: "0.1.0".into(),
            channel: InstallChannel::Direct,
            update_available: false,
            dismissed: false,
        }
    }

    #[test]
    fn the_offer_follows_the_channel_and_the_published_binary() {
        use InstallChannel as C;
        assert_eq!(offer(C::Script, true), UpdateOffer::Install);
        assert_eq!(offer(C::Direct, true), UpdateOffer::Install);
        assert_eq!(offer(C::Direct, false), UpdateOffer::Download);
        assert_eq!(
            offer(C::Brew, true),
            UpdateOffer::PackageManager {
                command: Some("brew upgrade kimun")
            }
        );
        assert_eq!(
            offer(C::Cargo, true),
            UpdateOffer::PackageManager {
                command: Some("cargo install kimun-notes")
            }
        );
        assert_eq!(
            offer(C::Unknown, true),
            UpdateOffer::PackageManager { command: None }
        );
        // A macOS bundle is replaced whole: the releases page, whatever ships.
        assert_eq!(offer(C::Bundle, true), UpdateOffer::Download);
        assert_eq!(offer(C::Bundle, false), UpdateOffer::Download);
        // Both apps' binaries ship.
        assert!(Frontend::Terminal.binary_published());
        assert_eq!(
            Frontend::Desktop.binary_published(),
            DESKTOP_BINARY_PUBLISHED
        );
    }

    #[test]
    fn a_startup_check_announces_a_new_version_once() {
        let mut m = UpdateMachine::new();
        assert_eq!(m.begin_check(false), CheckStart::Run { force: false });
        assert!(m.is_checking());
        let s = status("0.2.0", InstallChannel::Direct, false);
        assert!(matches!(
            m.check_landed(Ok(Some(s.clone()))),
            CheckReport::Available(found) if found.latest == "0.2.0"
        ));
        assert_eq!(m.known().map(|s| s.latest.as_str()), Some("0.2.0"));
        // Again in the same session: not repeated.
        m.begin_check(false);
        assert!(matches!(m.check_landed(Ok(Some(s))), CheckReport::Silent));
        // A manual check says it anyway.
        assert_eq!(m.begin_check(true), CheckStart::Run { force: true });
        let s = status("0.2.0", InstallChannel::Direct, false);
        assert!(matches!(
            m.check_landed(Ok(Some(s))),
            CheckReport::Available(_)
        ));
    }

    #[test]
    fn a_startup_check_is_silent_offline_up_to_date_or_skipped() {
        let mut m = UpdateMachine::new();
        m.begin_check(false);
        assert!(matches!(
            m.check_landed(Err("offline".into())),
            CheckReport::Silent
        ));
        assert!(matches!(m.phase(), UpdatePhase::Idle));
        m.begin_check(false);
        assert!(matches!(
            m.check_landed(Ok(Some(up_to_date()))),
            CheckReport::Silent
        ));
        m.begin_check(false);
        assert!(matches!(m.check_landed(Ok(None)), CheckReport::Silent));
        m.begin_check(false);
        let skipped = status("0.2.0", InstallChannel::Direct, true);
        assert!(matches!(
            m.check_landed(Ok(Some(skipped))),
            CheckReport::Silent
        ));
        assert!(matches!(m.phase(), UpdatePhase::Skipped(v) if v == "0.2.0"));
    }

    #[test]
    fn a_manual_check_always_reports() {
        let mut m = UpdateMachine::new();
        m.begin_check(true);
        assert!(matches!(
            m.check_landed(Ok(Some(up_to_date()))),
            CheckReport::UpToDate(text) if text == up_to_date_text("0.1.0")
        ));
        m.begin_check(true);
        assert!(matches!(
            m.check_landed(Err("offline".into())),
            CheckReport::Failed(text) if text == check_failed_text("offline")
        ));
        // Even a skipped version: the user asked.
        m.begin_check(true);
        let skipped = status("0.2.0", InstallChannel::Direct, true);
        assert!(matches!(
            m.check_landed(Ok(Some(skipped))),
            CheckReport::Available(_)
        ));
    }

    #[test]
    fn a_check_while_one_runs_is_coalesced_and_reports_if_manual() {
        let mut m = UpdateMachine::new();
        m.begin_check(false);
        assert_eq!(m.begin_check(false), CheckStart::Busy);
        assert_eq!(m.begin_check(true), CheckStart::Busy);
        // The throttled check's answer waits for the forced one's.
        assert!(matches!(
            m.check_landed(Err("offline".into())),
            CheckReport::RunForced
        ));
        assert!(matches!(
            m.check_landed(Err("offline".into())),
            CheckReport::Failed(_)
        ));
        // A result with no check running is dropped.
        assert!(matches!(
            m.check_landed(Ok(Some(up_to_date()))),
            CheckReport::Silent
        ));
    }

    /// A manual ask while a throttled (startup) check runs: that check's
    /// answer is not reported (a throttled `Ok(None)` is no "up to date"),
    /// a forced check runs after it and its answer is the report.
    #[test]
    fn a_manual_check_during_a_throttled_one_runs_a_forced_check_after_it() {
        let mut m = UpdateMachine::new();
        assert_eq!(m.begin_check(false), CheckStart::Run { force: false });
        assert_eq!(m.begin_check(true), CheckStart::Busy);
        assert!(matches!(m.check_landed(Ok(None)), CheckReport::RunForced));
        assert!(m.is_checking(), "the forced check runs now");
        assert!(matches!(
            m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Direct, false)))),
            CheckReport::Available(found) if found.latest == "0.2.0"
        ));
        assert!(!m.is_checking());
        // Its failure reports too, and what was known before stays.
        let mut m = UpdateMachine::new();
        m.begin_check(false);
        m.begin_check(true);
        assert!(matches!(
            m.check_landed(Ok(Some(up_to_date()))),
            CheckReport::RunForced
        ));
        assert!(matches!(
            m.check_landed(Err("offline".into())),
            CheckReport::Failed(text) if text == check_failed_text("offline")
        ));
        assert!(matches!(m.phase(), UpdatePhase::Idle));
    }

    /// Two manual asks during a throttled check: one forced check after
    /// it. A manual ask during a forced check joins it.
    #[test]
    fn manual_asks_coalesce_into_one_forced_check() {
        let mut m = UpdateMachine::new();
        m.begin_check(false);
        assert_eq!(m.begin_check(true), CheckStart::Busy);
        assert_eq!(m.begin_check(true), CheckStart::Busy);
        assert!(matches!(m.check_landed(Ok(None)), CheckReport::RunForced));
        assert_eq!(m.begin_check(true), CheckStart::Busy, "joins the forced");
        assert_eq!(m.begin_check(false), CheckStart::Busy);
        assert!(matches!(
            m.check_landed(Ok(Some(up_to_date()))),
            CheckReport::UpToDate(_)
        ));
        assert!(!m.is_checking(), "no third check");
        // A manual check alone is forced from the start: nothing follows.
        assert_eq!(m.begin_check(true), CheckStart::Run { force: true });
        assert_eq!(m.begin_check(true), CheckStart::Busy);
        assert!(matches!(
            m.check_landed(Ok(Some(up_to_date()))),
            CheckReport::UpToDate(_)
        ));
        assert!(!m.is_checking());
    }

    /// While a check runs, nothing can act on the version shown (the dialog
    /// says the check runs and disables Install and Skip): `dismiss` and
    /// `begin_install` refuse, `known` is none; once it lands they act.
    #[test]
    fn nothing_acts_while_a_check_runs() {
        let mut m = UpdateMachine::new();
        m.begin_check(true);
        m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Direct, false))));
        m.begin_check(true);
        assert!(m.is_checking());
        assert!(m.shown().is_some(), "still shown");
        assert!(m.known().is_none());
        assert_eq!(m.dismiss(), None);
        assert!(m.begin_install(&UpdateOffer::Install).is_none());
        assert!(m.is_checking(), "the check still runs");
        m.check_landed(Err("offline".into()));
        assert_eq!(m.dismiss().as_deref(), Some("0.2.0"));
    }

    /// `Later` on a failed install while a check runs: the failure goes
    /// (the version is offered clean), and a check that fails then goes
    /// back to that, not to the failure.
    #[test]
    fn later_on_a_failure_cleans_up_while_a_check_runs() {
        let mut m = UpdateMachine::new();
        m.begin_check(true);
        m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Direct, false))));
        m.begin_install(&UpdateOffer::Install);
        m.install_landed(Err("checksum mismatch".into()));
        m.begin_check(true);
        assert!(matches!(m.shown_phase(), UpdatePhase::Failed { .. }));
        m.later();
        assert!(matches!(m.shown_phase(), UpdatePhase::Available(_)));
        assert!(m.is_checking(), "the check still runs");
        m.check_landed(Err("offline".into()));
        assert!(matches!(m.phase(), UpdatePhase::Available(_)));
    }

    #[test]
    fn a_failed_check_keeps_what_was_known() {
        let mut m = UpdateMachine::new();
        m.begin_check(false);
        m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Direct, false))));
        m.begin_check(true);
        m.check_landed(Err("offline".into()));
        assert_eq!(m.known().map(|s| s.latest.as_str()), Some("0.2.0"));
    }

    #[test]
    fn a_running_check_keeps_showing_what_was_known() {
        let mut m = UpdateMachine::new();
        m.begin_check(false);
        m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Direct, false))));
        m.begin_check(true);
        assert!(m.is_checking());
        assert!(matches!(m.shown_phase(), UpdatePhase::Available(_)));
        assert_eq!(m.shown().map(|s| s.latest.as_str()), Some("0.2.0"));
        m.check_landed(Err("offline".into()));
        assert!(matches!(m.shown_phase(), UpdatePhase::Available(_)));
    }

    #[test]
    fn install_runs_only_when_offered_and_lands_installed_or_failed() {
        let mut m = UpdateMachine::new();
        assert!(m.begin_install(&UpdateOffer::Install).is_none());
        m.begin_check(false);
        m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Brew, false))));
        // A managed install has nothing to install.
        let managed = offer(InstallChannel::Brew, true);
        assert!(m.begin_install(&managed).is_none());
        assert!(m.begin_install(&UpdateOffer::Download).is_none());
        let s = m.begin_install(&UpdateOffer::Install).expect("installs");
        assert_eq!(s.latest, "0.2.0");
        assert!(m.is_installing());
        assert_eq!(m.begin_check(true), CheckStart::Busy);
        assert_eq!(m.dismiss(), None, "not while installing");
        m.install_landed(Err("checksum mismatch".into()));
        assert!(
            matches!(m.phase(), UpdatePhase::Failed { error, .. } if error == "checksum mismatch")
        );
        // Retry.
        assert!(m.begin_install(&UpdateOffer::Install).is_some());
        m.install_landed(Ok("0.2.0".into()));
        assert!(matches!(m.phase(), UpdatePhase::Installed { version } if version == "0.2.0"));
        assert!(m.known().is_none());
        assert_eq!(m.begin_check(true), CheckStart::Installed);
    }

    #[test]
    fn a_refused_install_is_a_failure_and_later_offers_it_again() {
        let mut m = UpdateMachine::new();
        m.begin_check(true);
        m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Direct, false))));
        m.install_refused("the note could not be saved".into());
        assert!(matches!(m.phase(), UpdatePhase::Failed { .. }));
        m.later();
        assert!(matches!(m.phase(), UpdatePhase::Available(_)));
    }

    #[test]
    fn dismiss_skips_the_known_version() {
        let mut m = UpdateMachine::new();
        assert_eq!(m.dismiss(), None);
        m.begin_check(true);
        m.check_landed(Ok(Some(status("0.2.0", InstallChannel::Direct, false))));
        assert_eq!(m.dismiss().as_deref(), Some("0.2.0"));
        assert!(matches!(m.phase(), UpdatePhase::Skipped(v) if v == "0.2.0"));
    }

    /// The real throttle (`update::check` without `force`) answers from
    /// `update_state.toml` while it is fresh: no network, the cached
    /// version, the dismissal applied.
    #[test]
    fn a_fresh_state_file_throttles_the_startup_check() {
        let dir = tempfile::tempdir().unwrap();
        let state = UpdateState {
            last_check: Some(chrono::Utc::now()),
            latest_version: Some("999.0.0".into()),
            dismissed_version: Some("999.0.0".into()),
        };
        state.save(dir.path()).unwrap();
        let status = update::check(dir.path(), false)
            .expect("no network needed")
            .expect("the cached version");
        assert_eq!(status.latest, "999.0.0");
        assert!(status.update_available);
        assert!(status.dismissed, "the skip persists");
        // Fresh with nothing cached: nothing known, still no network.
        let empty = tempfile::tempdir().unwrap();
        UpdateState {
            last_check: Some(chrono::Utc::now()),
            ..UpdateState::default()
        }
        .save(empty.path())
        .unwrap();
        assert!(update::check(empty.path(), false).unwrap().is_none());
    }

    #[test]
    fn dismiss_persists_in_the_state_file() {
        let dir = tempfile::tempdir().unwrap();
        update::dismiss(dir.path(), "999.0.0").unwrap();
        assert_eq!(
            UpdateState::load(dir.path()).dismissed_version.as_deref(),
            Some("999.0.0")
        );
    }

    #[test]
    fn the_scripted_backend_reads_its_spec() {
        assert!(ScriptedUpdates::from_spec("").is_none());
        assert!(ScriptedUpdates::from_spec("nonsense").is_none());
        assert!(ScriptedUpdates::from_spec("9.9.9:weird").is_none());
        let b = ScriptedUpdates::from_spec("9.9.9").unwrap();
        let s = b.check(false).unwrap().unwrap();
        assert!(s.update_available);
        assert_eq!(s.latest, "9.9.9");
        assert_eq!(b.offer(&s), UpdateOffer::Install);
        let b = ScriptedUpdates::from_spec("9.9.9:managed").unwrap();
        let s = b.check(true).unwrap().unwrap();
        assert!(matches!(b.offer(&s), UpdateOffer::PackageManager { .. }));
        let b = ScriptedUpdates::from_spec("9.9.9:download").unwrap();
        let s = b.check(true).unwrap().unwrap();
        assert_eq!(b.offer(&s), UpdateOffer::Download);
        assert!(
            ScriptedUpdates::from_spec("offline")
                .unwrap()
                .check(true)
                .is_err()
        );
        let current = ScriptedUpdates::from_spec("current").unwrap();
        assert!(!current.check(true).unwrap().unwrap().update_available);
        let fails = ScriptedUpdates::from_spec("1.2.3:fail")
            .unwrap()
            .install_delay(Duration::ZERO);
        let s = fails.check(true).unwrap().unwrap();
        assert!(fails.install(&s).is_err());
        assert_eq!(fails.checks(), [true]);
        assert_eq!(fails.installs(), 1);
        fails.dismiss("1.2.3").unwrap();
        assert!(fails.check(false).unwrap().unwrap().dismissed);
    }

    /// A desktop app in a macOS bundle is offered the download, and the real
    /// backend refuses to install over it before any network or file work.
    #[test]
    fn a_bundled_desktop_app_never_installs_in_place() {
        let real = SystemUpdates::new(Frontend::Desktop);
        let s = status("999.0.0", InstallChannel::Bundle, false);
        assert_eq!(real.offer(&s), UpdateOffer::Download);
        assert!(real.install(&s).is_err());
    }

    /// An install on a package manager's channel is refused too, for either app.
    #[test]
    fn a_managed_install_never_installs_in_place() {
        for frontend in [Frontend::Terminal, Frontend::Desktop] {
            let real = SystemUpdates::new(frontend);
            let s = status("999.0.0", InstallChannel::Brew, false);
            assert!(matches!(real.offer(&s), UpdateOffer::PackageManager { .. }));
            assert!(real.install(&s).is_err());
        }
    }

    #[test]
    fn a_release_without_a_desktop_build_says_so() {
        let missing = update::UpdateError::MissingAsset("kimun-gui-9.9.9-linux-x64".into());
        let said = install_error(Frontend::Desktop, &missing);
        assert!(said.contains("no desktop build"), "{said}");
        // The terminal app, and any other error, keep the plain wording.
        assert_eq!(
            install_error(Frontend::Terminal, &missing),
            missing.to_string()
        );
        let other = update::UpdateError::MissingAsset("checksums-sha256.txt".into());
        assert_eq!(install_error(Frontend::Desktop, &other), other.to_string());
    }

    #[test]
    fn each_frontend_updates_to_its_own_product() {
        assert_eq!(Frontend::Terminal.product(), update::Product::Terminal);
        assert_eq!(Frontend::Desktop.product(), update::Product::Desktop);
    }
}
