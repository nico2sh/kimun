//! Machine-managed update state (`update_state.toml`). Holds the throttle
//! timestamp, the last-known latest version, and any version the user chose to
//! skip. Kept strictly separate from `config.toml`, which is user-owned and
//! never written by an automated action.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

const STATE_FILE: &str = "update_state.toml";

const STATE_HEADER: &str = "\
# Kimün update state — auto-generated, do not edit.
# Tracks the last update check, the latest known release, and any version you
# chose to skip. Your settings live in config.toml, not here.
";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UpdateState {
    /// When the GitHub releases API was last queried.
    #[serde(default)]
    pub last_check: Option<DateTime<Utc>>,
    /// Latest stable version seen at the last check (without the tag prefix).
    #[serde(default)]
    pub latest_version: Option<String>,
    /// A version the user dismissed; suppresses the notification until a newer
    /// one ships.
    #[serde(default)]
    pub dismissed_version: Option<String>,
}

impl UpdateState {
    /// Load from `config_dir`, returning the default (empty) state if the file
    /// is absent or unreadable — a missing/corrupt cache must never be fatal.
    pub fn load(config_dir: &Path) -> Self {
        std::fs::read_to_string(config_dir.join(STATE_FILE))
            .ok()
            .and_then(|raw| toml::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Persist to `config_dir`, prefixed with the do-not-edit header.
    pub fn save(&self, config_dir: &Path) -> io::Result<()> {
        let body = toml::to_string(self).map_err(io::Error::other)?;
        std::fs::write(config_dir.join(STATE_FILE), format!("{STATE_HEADER}{body}"))
    }

    /// Whether the last check is older than `max_age` (or never happened).
    pub fn is_stale(&self, now: DateTime<Utc>, max_age: Duration) -> bool {
        match self.last_check {
            Some(checked) => now.signed_duration_since(checked) >= max_age,
            None => true,
        }
    }
}
