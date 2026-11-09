//! Install-channel detection. Decides whether this binary may self-update or
//! must defer to a package manager. Detection that is ambiguous or fails is
//! treated as notify-only: the conservative default never risks corrupting a
//! managed install.
//!
//! Order of precedence:
//!   1. Running from inside a macOS app bundle (`Foo.app/Contents/MacOS/`):
//!      always [`InstallChannel::Bundle`], whatever the marker says. Replacing
//!      the executable inside a signed bundle breaks its seal, and the config
//!      dir (and its marker) is shared with a terminal install elsewhere.
//!   2. The install marker (`install.toml`) written by `install.sh` — deterministic.
//!   3. A heuristic on the canonicalised executable path.
//!
//! Anything that cannot be classified fails safe to notify-only.

use std::env;
use std::path::{Path, PathBuf};
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
    /// The desktop app run from inside its macOS `.app` bundle: replaced as a
    /// whole by a new download (or a cask), never executable-by-executable.
    Bundle,
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
    /// Directory `install.sh` put the binary in. The marker file is shared by
    /// every kimün app (and outlives the install), so it only describes a
    /// binary that actually lives there.
    install_dir: Option<String>,
}

/// The running executable, as invoked and with symlinks resolved. Invariant
/// for the process, so resolved once.
struct ExePaths {
    raw: PathBuf,
    canonical: Option<PathBuf>,
}

fn exe_paths() -> Option<&'static ExePaths> {
    static EXE: OnceLock<Option<ExePaths>> = OnceLock::new();
    EXE.get_or_init(|| {
        let raw = env::current_exe().ok()?;
        let canonical = kimun_core::SystemPath::canonical(&raw)
            .map(|p| p.into_path_buf())
            .ok();
        Some(ExePaths { raw, canonical })
    })
    .as_ref()
}

/// Detect how the running binary was installed. `config_dir` is kimün's config
/// directory (where `install.sh` writes the marker).
///
/// The marker (cheap file read, depends on `config_dir`) is consulted first.
/// The fallback path heuristic — `current_exe` canonicalisation plus a
/// filesystem writability probe — is the expensive part and is invariant for
/// the process, so only *it* is cached. Caching keyed on the result of an
/// argument would let the first caller's `config_dir` win forever.
pub fn detect(config_dir: &Path) -> InstallChannel {
    let exe = exe_paths();
    // Check the path as invoked *and* resolved: a symlink pointing into a
    // bundle must still count as the bundle.
    let in_bundle = exe.is_some_and(|e| {
        is_app_bundle_path(&e.raw.to_string_lossy())
            || e.canonical
                .as_deref()
                .is_some_and(|c| is_app_bundle_path(&c.to_string_lossy()))
    });
    if in_bundle {
        return InstallChannel::Bundle;
    }
    let exe_dir = exe
        .and_then(|e| e.canonical.as_deref())
        .and_then(Path::parent);
    if let Some(channel) = channel_from_marker(config_dir, exe_dir) {
        return channel;
    }
    static EXE_CHANNEL: OnceLock<InstallChannel> = OnceLock::new();
    *EXE_CHANNEL.get_or_init(channel_from_exe_path)
}

/// Whether `path` is an executable inside a macOS app bundle.
fn is_app_bundle_path(path: &str) -> bool {
    path.contains(".app/Contents/MacOS/")
}

/// The channel the marker declares, if it exists and describes the binary
/// running from `exe_dir`. A marker naming a different `install_dir` belongs to
/// another install (e.g. `install.sh --gui` after a brew-installed `kimun`),
/// so it is ignored rather than trusted.
fn channel_from_marker(config_dir: &Path, exe_dir: Option<&Path>) -> Option<InstallChannel> {
    let raw = std::fs::read_to_string(config_dir.join(MARKER_FILE)).ok()?;
    let marker: InstallMarker = toml::from_str(&raw).ok()?;
    if let Some(dir) = marker.install_dir.as_deref().filter(|d| !d.is_empty()) {
        let marked = kimun_core::SystemPath::canonical(Path::new(dir))
            .map(|p| p.into_path_buf())
            .unwrap_or_else(|_| PathBuf::from(dir));
        if exe_dir != Some(marked.as_path()) {
            return None;
        }
    }
    match marker.channel.as_str() {
        "script" => Some(InstallChannel::Script),
        "direct" => Some(InstallChannel::Direct),
        "brew" => Some(InstallChannel::Brew),
        "cargo" => Some(InstallChannel::Cargo),
        "bundle" => Some(InstallChannel::Bundle),
        _ => None,
    }
}

fn channel_from_exe_path() -> InstallChannel {
    // No idea where we live — do not risk touching a managed binary.
    let Some(exe) = exe_paths().and_then(|e| e.canonical.clone()) else {
        return InstallChannel::Unknown;
    };
    let path = exe.to_string_lossy();

    // Homebrew: an explicit prefix env var, or the Cellar layout the formula
    // installs into (current_exe is canonicalised, so brew's bin symlink is
    // already resolved into the Cellar path).
    if let Ok(prefix) = env::var("HOMEBREW_PREFIX")
        && !prefix.is_empty()
        && path.starts_with(prefix.as_str())
    {
        return InstallChannel::Brew;
    }
    if path.contains("/Cellar/") || path.contains("/homebrew/") {
        return InstallChannel::Brew;
    }

    // cargo install: under CARGO_HOME/bin or ~/.cargo/bin.
    if let Ok(cargo_home) = env::var("CARGO_HOME")
        && !cargo_home.is_empty()
        && exe.starts_with(&cargo_home)
    {
        return InstallChannel::Cargo;
    }
    if let Ok(home) = kimun_core::system::home()
        && exe.starts_with(home.join(".cargo").join("bin"))
    {
        return InstallChannel::Cargo;
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
            let _ = kimun_core::system::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_inside_an_app_bundle_is_the_bundle_channel() {
        assert!(is_app_bundle_path(
            "/Applications/Kimün.app/Contents/MacOS/kimun-gui"
        ));
        assert!(is_app_bundle_path(
            "/Users/me/Apps/Kimun.app/Contents/MacOS/kimun-gui"
        ));
        assert!(!is_app_bundle_path("/Users/me/.local/bin/kimun-gui"));
        assert!(!is_app_bundle_path("/opt/Kimun.app/bin/kimun"));
    }

    #[test]
    fn a_bundle_is_never_self_update_eligible() {
        assert!(!InstallChannel::Bundle.self_update_eligible());
        assert_eq!(InstallChannel::Bundle.upgrade_hint(), None);
    }

    #[test]
    fn the_marker_may_name_the_bundle_channel() {
        let dir = std::env::temp_dir().join(format!("kimun-channel-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MARKER_FILE), "channel = \"bundle\"\n").unwrap();
        assert_eq!(
            channel_from_marker(&dir, None),
            Some(InstallChannel::Bundle)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_marker_for_another_install_dir_is_ignored() {
        let dir = std::env::temp_dir().join(format!("kimun-channel-dir-{}", std::process::id()));
        let here = dir.join("here");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(
            dir.join(MARKER_FILE),
            format!(
                "channel = \"script\"\ninstall_dir = \"{}\"\n",
                here.display()
            ),
        )
        .unwrap();
        let here_canon = kimun_core::SystemPath::canonical(&here)
            .unwrap()
            .into_path_buf();
        let else_canon = kimun_core::SystemPath::canonical(&elsewhere)
            .unwrap()
            .into_path_buf();
        assert_eq!(
            channel_from_marker(&dir, Some(&here_canon)),
            Some(InstallChannel::Script)
        );
        assert_eq!(channel_from_marker(&dir, Some(&else_canon)), None);
        assert_eq!(channel_from_marker(&dir, None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
