//! Update awareness for the server: is a newer stable `kimun_server-v*`
//! release out? Notify only — the server never replaces its own binary (it may
//! run in a container, under a service manager, or from a hand-placed binary;
//! each upgrades differently). The result surfaces on the web UI dashboard and
//! in `/health`, where the TUI shows it as a passive hint.
//!
//! The check runs in the background, at most once a day. Its
//! result lives in an [`UpdateCheck`] created once per process, so in-process
//! restarts (adr 0028) reuse it instead of re-querying GitHub.

use std::sync::{Arc, RwLock, Weak};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::server_state::AppState;

/// The version compiled into this binary.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const RELEASES_API: &str = "https://api.github.com/repos/nico2sh/kimun/releases?per_page=100";
const RELEASES_PAGE: &str = "https://github.com/nico2sh/kimun/releases";
const TAG_PREFIX: &str = "kimun_server-v";

/// How long a successful check result is reused.
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the background task wakes to see whether a check is due. Also the
/// retry delay after a failed check.
const POLL_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Set by the Docker image (`ENV KIMUN_SERVER_INSTALL=docker`) so the upgrade
/// hint says `docker pull` instead of re-running the install script.
const INSTALL_ENV: &str = "KIMUN_SERVER_INSTALL";

/// The shared, process-lifetime result of the latest-release check.
#[derive(Clone, Default)]
pub struct UpdateCheck {
    inner: Arc<RwLock<Checked>>,
}

#[derive(Default)]
struct Checked {
    at: Option<Instant>,
    latest: Option<String>,
}

impl UpdateCheck {
    pub fn new() -> Self {
        Self::default()
    }

    /// The newest stable release seen by the last successful check, if any.
    pub fn latest(&self) -> Option<String> {
        self.inner.read().ok()?.latest.clone()
    }

    /// The newest stable release, only when it is newer than this binary.
    pub fn available(&self) -> Option<String> {
        self.latest().filter(|v| is_newer(v, CURRENT_VERSION))
    }

    fn is_due(&self, now: Instant) -> bool {
        self.inner
            .read()
            .map(|c| {
                c.at.is_none_or(|at| now.duration_since(at) >= CHECK_INTERVAL)
            })
            .unwrap_or(false)
    }

    pub(crate) fn record(&self, latest: String, now: Instant) {
        if let Ok(mut c) = self.inner.write() {
            c.at = Some(now);
            c.latest = Some(latest);
        }
    }
}

/// Starts the background check for this serving iteration, unless disabled by
/// `[server] update_check = false`. Holds only a `Weak` so a restart that drops
/// the state ends the task (same pattern as the job sweep).
pub fn spawn_check(state: &Arc<AppState>) {
    if !state.config.server.update_check {
        return;
    }
    let weak: Weak<AppState> = Arc::downgrade(state);
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            interval.tick().await;
            let Some(state) = weak.upgrade() else { break };
            let check = state.update_check.clone();
            drop(state);
            if !check.is_due(Instant::now()) {
                continue;
            }
            match fetch_latest(&client).await {
                Ok(latest) => {
                    if is_newer(&latest, CURRENT_VERSION) {
                        tracing::info!(
                            "kimun-server {latest} is available (running {CURRENT_VERSION})"
                        );
                    }
                    check.record(latest, Instant::now());
                }
                Err(e) => tracing::debug!("update check failed: {e}"),
            }
        }
    });
}

/// Human-facing releases page.
pub fn releases_url() -> &'static str {
    RELEASES_PAGE
}

/// The command that upgrades this install: `docker pull` inside the image,
/// re-running the install script otherwise (it updates in place and restarts
/// the service it set up).
pub fn upgrade_hint() -> &'static str {
    if std::env::var(INSTALL_ENV).is_ok_and(|v| v == "docker") {
        "docker pull ghcr.io/nico2sh/kimun-server:latest, then recreate the container"
    } else {
        "curl -fsSL https://kimun.2co.dev/install-server.sh | sh"
    }
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
}

async fn fetch_latest(client: &reqwest::Client) -> anyhow::Result<String> {
    let releases: Vec<GhRelease> = client
        .get(RELEASES_API)
        .header(
            "User-Agent",
            concat!("kimun-server/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    latest_stable(&releases).ok_or_else(|| anyhow::anyhow!("no stable server release found"))
}

/// The first (newest) `kimun_server-v*` tag without a pre-release suffix.
/// `/releases/latest` is not usable: the server's releases are published with
/// `git_release_latest = false` (release-plz.toml), and pre-release status is
/// the tag hyphen, not GitHub's `prerelease` flag — same policy as the TUI
/// and `install-server.sh`.
fn latest_stable(releases: &[GhRelease]) -> Option<String> {
    releases.iter().find_map(|r| {
        let version = r.tag_name.strip_prefix(TAG_PREFIX)?;
        (!version.contains('-')).then(|| version.to_string())
    })
}

/// Is `candidate` strictly newer than `current`? Unparseable input compares as
/// not-newer (never nudge on garbage).
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
        }
    }

    #[test]
    fn latest_stable_skips_other_series_and_prereleases() {
        let releases = [
            rel("kimun-notes-v0.30.0"),
            rel("kimun_server-v0.6.0-beta.1"),
            rel("kimun_core-v0.20.0"),
            rel("kimun_server-v0.5.1"),
            rel("kimun_server-v0.5.0"),
        ];
        assert_eq!(latest_stable(&releases).as_deref(), Some("0.5.1"));
        assert_eq!(latest_stable(&[rel("kimun-notes-v1.0.0")]), None);
    }

    #[test]
    fn newer_versions_compare_correctly() {
        assert!(is_newer("0.5.0", "0.4.3"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.4.3", "0.4.3"));
        assert!(!is_newer("0.4.2", "0.4.3"));
        assert!(!is_newer("garbage", "0.4.3"));
        assert!(!is_newer("0.5.0-beta.1", "0.4.3"));
    }

    #[test]
    fn available_only_when_newer() {
        let check = UpdateCheck::new();
        assert_eq!(check.available(), None);
        assert!(check.is_due(Instant::now()));

        check.record("0.0.1".into(), Instant::now());
        assert_eq!(check.latest().as_deref(), Some("0.0.1"));
        assert_eq!(check.available(), None, "older than the running binary");
        assert!(!check.is_due(Instant::now()));

        check.record("999.0.0".into(), Instant::now());
        assert_eq!(check.available().as_deref(), Some("999.0.0"));
    }

    #[test]
    fn check_becomes_due_after_the_interval() {
        let check = UpdateCheck::new();
        let then = Instant::now();
        check.record("0.0.1".into(), then);
        assert!(!check.is_due(then + CHECK_INTERVAL - Duration::from_secs(1)));
        assert!(check.is_due(then + CHECK_INTERVAL));
    }
}
