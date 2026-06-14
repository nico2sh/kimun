//! Install-channel detection. Decides whether this binary may self-update or
//! must defer to a package manager. See adr/0013.
//!
//! Order of precedence:
//!   1. The install marker (`install.toml`) written by `install.sh` — deterministic.
//!   2. A heuristic on the canonicalised executable path.
//! Anything that cannot be classified fails safe to notify-only.

use std::env;
use std::path::Path;
use std::sync::OnceLock;

const MARKER_FILE: &str = "install.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallChannel {
    /// Installed via the official `install.sh`.
    Script,
    /// A manually downloaded release archive.
    Direct,
    /// Homebrew tap — package-manager owned, notify-only.
    Brew,
    /// `cargo install` — package-manager owned, notify-only.
    Cargo,
    /// Could not determine; treated as notify-only.
    Unknown,
}

impl InstallChannel {
    /// Whether kimün may replace its own binary on this channel.
    pub fn self_update_eligible(self) -> bool {
        matches!(self, Self::Script | Self::Direct)
    }

    /// The command a user should run to upgrade on a package-manager channel,
    /// or `None` where self-update applies (or the channel is unknown).
    pub fn upgrade_hint(self) -> Option<&'static str> {
        match self {
            Self::Brew => Some("brew upgrade kimun"),
            Self::Cargo => Some("cargo install kimun-notes"),
            _ => None,
        }
    }
}

#[derive(serde::Deserialize)]
struct InstallMarker {
    channel: String,
}

/// Detect how the running binary was installed. `config_dir` is kimün's config
/// directory (where `install.sh` writes the marker).
///
/// The result is cached for the process lifetime — the install location cannot
/// change while kimün runs, so the marker read and the `current_exe`
/// canonicalisation happen only once.
pub fn detect(config_dir: &Path) -> InstallChannel {
    static CACHE: OnceLock<InstallChannel> = OnceLock::new();
    *CACHE.get_or_init(|| {
        channel_from_marker(config_dir).unwrap_or_else(channel_from_exe_path)
    })
}

fn channel_from_marker(config_dir: &Path) -> Option<InstallChannel> {
    let raw = std::fs::read_to_string(config_dir.join(MARKER_FILE)).ok()?;
    let marker: InstallMarker = toml::from_str(&raw).ok()?;
    match marker.channel.as_str() {
        "script" => Some(InstallChannel::Script),
        "direct" => Some(InstallChannel::Direct),
        "brew" => Some(InstallChannel::Brew),
        "cargo" => Some(InstallChannel::Cargo),
        _ => None,
    }
}

fn channel_from_exe_path() -> InstallChannel {
    let exe = match env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        // No idea where we live — do not risk touching a managed binary.
        Err(_) => return InstallChannel::Unknown,
    };
    let path = exe.to_string_lossy();

    // Homebrew: an explicit prefix env var, or the Cellar layout the formula
    // installs into (current_exe is canonicalised, so brew's bin symlink is
    // already resolved into the Cellar path).
    if let Ok(prefix) = env::var("HOMEBREW_PREFIX") {
        if !prefix.is_empty() && path.starts_with(prefix.as_str()) {
            return InstallChannel::Brew;
        }
    }
    if path.contains("/Cellar/") || path.contains("/homebrew/") {
        return InstallChannel::Brew;
    }

    // cargo install: under CARGO_HOME/bin or ~/.cargo/bin.
    if let Ok(cargo_home) = env::var("CARGO_HOME") {
        if !cargo_home.is_empty() && exe.starts_with(&cargo_home) {
            return InstallChannel::Cargo;
        }
    }
    if let Ok(home) = crate::settings::get_home_dir() {
        if exe.starts_with(home.join(".cargo").join("bin")) {
            return InstallChannel::Cargo;
        }
    }

    // Otherwise the user placed this binary themselves. Only call it
    // self-update eligible if its directory is actually writable: a binary in a
    // root-owned/system location (e.g. /usr/bin, a distro package, the Nix
    // store) must stay notify-only and never be overwritten in place, even
    // though it is neither brew nor cargo.
    match exe.parent() {
        Some(dir) if dir_is_writable(dir) => InstallChannel::Direct,
        _ => InstallChannel::Unknown,
    }
}

/// Whether a probe file can be created in `dir` (i.e. the current user may
/// write there). Cleans up the probe. A best-effort check used only to gate
/// self-update eligibility; on any error it returns false (fail safe).
fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".kimun-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
