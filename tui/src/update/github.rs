//! GitHub releases [`ReleaseProvider`]. Blocking (ureq) — call on the blocking
//! pool. Maps GitHub's JSON onto the provider-agnostic [`LatestRelease`].

use serde::Deserialize;

use super::UpdateError;
use super::provider::{Asset, LatestRelease, ReleaseProvider};

const RELEASES_API: &str = "https://api.github.com/repos/nico2sh/kimun/releases?per_page=100";
const RELEASES_PAGE: &str = "https://github.com/nico2sh/kimun/releases";
const TAG_PREFIX: &str = "kimun-notes-v";

/// GitHub-side JSON shapes (private — never leak past this module).
#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Resolves releases from the GitHub REST API.
#[derive(Debug, Default, Clone, Copy)]
pub struct GitHubProvider;

impl ReleaseProvider for GitHubProvider {
    /// Fetch the newest stable app release.
    ///
    /// `/releases/latest` is deliberately **not** used: this repo interleaves
    /// `kimun_core-v*` library releases with `kimun-notes-v*` app releases, and
    /// "latest" is the newest of either — frequently a core release with no
    /// binaries (see adr/0014). Instead the releases list (newest-first) is
    /// scanned for the first `kimun-notes-v*` tag with no pre-release suffix.
    ///
    /// Pre-release status is determined by the **tag hyphen** (e.g.
    /// `…-v0.18.0-beta.1`), not GitHub's `prerelease` boolean: that flag is
    /// unreliable here — release-plz marks some plain stable releases (0.17.0,
    /// 0.18.0) as prerelease — and the tag convention is the project's actual
    /// policy, shared with `install.sh`.
    fn latest_stable(&self) -> Result<LatestRelease, UpdateError> {
        let body = super::http_get(RELEASES_API)?.into_string()?;
        let releases: Vec<GhRelease> = serde_json::from_str(&body)?;

        releases
            .into_iter()
            .find_map(|release| {
                let version = release.tag_name.strip_prefix(TAG_PREFIX)?;
                // Skip pre-releases: a hyphenated semver suffix on the tag.
                if version.contains('-') {
                    return None;
                }
                Some(LatestRelease {
                    version: version.to_string(),
                    assets: release
                        .assets
                        .into_iter()
                        .map(|a| Asset {
                            name: a.name,
                            url: a.browser_download_url,
                        })
                        .collect(),
                })
            })
            .ok_or(UpdateError::NoRelease)
    }

    fn releases_url(&self) -> &'static str {
        RELEASES_PAGE
    }
}
