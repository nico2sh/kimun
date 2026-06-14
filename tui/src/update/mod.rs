//! Update awareness and (where permitted) self-update.
//!
//! On launch the app asks GitHub whether a newer stable `kimun-notes-v*` exists
//! and surfaces the result; on self-update-eligible channels it can also swap
//! the binary in place. All network and filesystem work here is **blocking** —
//! callers run it on `tokio::task::spawn_blocking` so the TUI never stalls.
//!
//! Design: adr/0013 (channel restriction) and adr/0014 (hand-rolled mechanics).
//! User-owned config (`update_check`) lives in `config.toml`; machine-managed
//! state (throttle, last-known version, dismissals) lives in `update_state.toml`.

mod apply;
mod channel;
mod github;
mod platform;
mod state;

pub use channel::{InstallChannel, detect as detect_channel};
pub use github::LatestRelease;
pub use state::UpdateState;

use chrono::{Duration, Utc};
use std::path::Path;

/// The version compiled into this binary.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long a check result is reused before the next launch re-queries GitHub.
const CHECK_INTERVAL_HOURS: i64 = 24;

/// The outcome of an update check, ready to drive the UI.
#[derive(Debug, Clone)]
pub struct UpdateStatus {
    /// The running version.
    pub current: String,
    /// The newest stable version available.
    pub latest: String,
    /// How this binary was installed (decides notify vs self-update).
    pub channel: InstallChannel,
    /// Whether `latest` is newer than `current`.
    pub update_available: bool,
    /// Whether the user has dismissed this exact `latest` version.
    pub dismissed: bool,
}

impl UpdateStatus {
    /// Whether the footer/dialog should nudge the user: an update exists and was
    /// not dismissed.
    pub fn should_notify(&self) -> bool {
        self.update_available && !self.dismissed
    }
}

/// Check for an update.
///
/// When `force` is false the check is throttled: if the cached result is fresh
/// (< [`CHECK_INTERVAL_HOURS`]) no network call is made and the cached version
/// is reused. `force` (manual check / `kimun update`) always queries GitHub.
///
/// Returns `Ok(None)` only when throttled with no cached version yet. Blocking.
pub fn check(config_dir: &Path, force: bool) -> Result<Option<UpdateStatus>, UpdateError> {
    let mut st = UpdateState::load(config_dir);
    let now = Utc::now();

    let latest = if force || st.is_stale(now, Duration::hours(CHECK_INTERVAL_HOURS)) {
        let release = github::latest_stable()?;
        st.last_check = Some(now);
        st.latest_version = Some(release.version.clone());
        // Best-effort persist; a write failure must not fail the check.
        if let Err(e) = st.save(config_dir) {
            tracing::warn!("could not save update state: {e}");
        }
        release.version
    } else {
        match &st.latest_version {
            Some(v) => v.clone(),
            None => return Ok(None),
        }
    };

    Ok(Some(UpdateStatus {
        current: CURRENT_VERSION.to_string(),
        update_available: is_newer(&latest, CURRENT_VERSION),
        dismissed: st.dismissed_version.as_deref() == Some(latest.as_str()),
        channel: channel::detect(config_dir),
        latest,
    }))
}

/// Fetch the full latest release (with downloadable assets), needed before
/// [`apply`]. Blocking.
pub fn fetch_latest() -> Result<LatestRelease, UpdateError> {
    github::latest_stable()
}

/// Download, verify, and install `latest`, replacing the running binary.
///
/// The caller must gate on [`InstallChannel::self_update_eligible`] first.
/// Blocking.
pub fn apply(latest: &LatestRelease) -> Result<(), UpdateError> {
    apply::self_update(latest)
}

/// Record that the user dismissed `version`, suppressing the notification until
/// a newer release appears. Writes only `update_state.toml`.
pub fn dismiss(config_dir: &Path, version: &str) -> std::io::Result<()> {
    let mut st = UpdateState::load(config_dir);
    st.dismissed_version = Some(version.to_string());
    st.save(config_dir)
}

/// Compare two `X.Y.Z` versions: is `candidate` strictly newer than `current`?
/// Unparseable input compares as not-newer (fail safe — never nudge on garbage).
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

/// Parse a plain `X.Y.Z` version into a comparable tuple. Returns `None` for
/// anything with a pre-release/build suffix or non-numeric parts — release tags
/// considered here are always plain stable triples.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Anything that can go wrong during an update check or self-update.
#[derive(Debug)]
pub enum UpdateError {
    /// Network / HTTP failure talking to GitHub.
    Http(Box<ureq::Error>),
    /// Failed to read a response body.
    Io(std::io::Error),
    /// Failed to parse the releases JSON.
    Parse(serde_json::Error),
    /// No stable `kimun-notes-v*` release found.
    NoRelease,
    /// This target has no published binary to self-update to.
    UnsupportedPlatform,
    /// A required release asset (binary or checksums) was absent.
    MissingAsset(String),
    /// No checksum line for the binary in `checksums-sha256.txt`.
    NoChecksum(String),
    /// Downloaded binary failed checksum verification.
    ChecksumMismatch { expected: String, actual: String },
    /// The in-place binary swap failed.
    Replace(std::io::Error),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "network error: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Parse(e) => write!(f, "could not parse GitHub response: {e}"),
            Self::NoRelease => write!(f, "no stable release found"),
            Self::UnsupportedPlatform => {
                write!(f, "no self-update binary is published for this platform")
            }
            Self::MissingAsset(name) => write!(f, "release is missing asset: {name}"),
            Self::NoChecksum(name) => write!(f, "no checksum published for {name}"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "checksum mismatch (expected {expected}, got {actual})")
            }
            Self::Replace(e) => write!(f, "could not replace the running binary: {e}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<ureq::Error> for UpdateError {
    fn from(e: ureq::Error) -> Self {
        Self::Http(Box::new(e))
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for UpdateError {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions_compare_correctly() {
        assert!(is_newer("0.18.0", "0.17.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.17.1", "0.17.0"));
        assert!(!is_newer("0.17.0", "0.17.0"));
        assert!(!is_newer("0.16.0", "0.17.0"));
    }

    #[test]
    fn unparseable_versions_never_nudge() {
        assert!(!is_newer("garbage", "0.17.0"));
        assert!(!is_newer("0.18.0-beta.1", "0.17.0"));
        assert!(!is_newer("0.18", "0.17.0"));
    }
}
