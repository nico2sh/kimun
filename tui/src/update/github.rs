//! GitHub releases API access. Blocking (ureq) — call on `spawn_blocking`.

use serde::Deserialize;

use super::UpdateError;

const RELEASES_URL: &str =
    "https://api.github.com/repos/nico2sh/kimun/releases?per_page=100";
const TAG_PREFIX: &str = "kimun-notes-v";

#[derive(Debug, Clone, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// The newest stable app release and everything needed to download it.
#[derive(Debug, Clone)]
pub struct LatestRelease {
    /// Version without the `kimun-notes-v` prefix, e.g. `0.18.0`.
    pub version: String,
    /// Release notes (the GitHub release body), if any.
    pub notes: Option<String>,
    /// Release assets (raw binaries, archives, checksums).
    pub assets: Vec<Asset>,
}

impl LatestRelease {
    /// Find an asset by exact name.
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// Fetch the newest stable app release.
///
/// `/releases/latest` is deliberately **not** used: this repo interleaves
/// `kimun_core-v*` library releases with `kimun-notes-v*` app releases, and
/// "latest" is the newest of either — frequently a core release with no
/// binaries (see adr/0014). Instead the releases list (newest-first) is scanned
/// for the first `kimun-notes-v*` tag with no pre-release suffix.
pub fn latest_stable() -> Result<LatestRelease, UpdateError> {
    let body = super::http_get(RELEASES_URL)?.into_string()?;

    let releases: Vec<Release> = serde_json::from_str(&body)?;

    releases
        .into_iter()
        .find_map(|release| {
            let version = release.tag_name.strip_prefix(TAG_PREFIX)?;
            // Skip pre-releases (flagged, or a hyphenated semver suffix).
            if release.prerelease || version.contains('-') {
                return None;
            }
            Some(LatestRelease {
                version: version.to_string(),
                notes: release.body.filter(|b| !b.trim().is_empty()),
                assets: release.assets,
            })
        })
        .ok_or(UpdateError::NoRelease)
}
