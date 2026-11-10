//! The release manifest [`ReleaseProvider`]: `https://kimun.2co.dev/latest.json`, built by
//! `.github/scripts/release_manifest.py` from the published releases.
//!
//! One small file instead of the GitHub releases list, which holds only the newest 100
//! releases across three tag series (the app's release can fall out of that window) and
//! allows 60 unauthenticated calls an hour per address. The manifest also carries each
//! file's sha256, so applying an update needs no second download for the checksums.
//!
//! The manifest is an optimisation, never a dependency: when it cannot be fetched or
//! trusted (network, a schema this build does not know, a malformed entry) the provider
//! says why in the log and answers from [`GitHubProvider`], exactly as before.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use super::UpdateError;
use super::github::GitHubProvider;
use super::provider::{Asset, LatestRelease, ReleaseProvider};

const MANIFEST_URL: &str = "https://kimun.2co.dev/latest.json";
/// The manifest layout this build understands.
const SCHEMA: u32 = 1;
/// A hung site must not delay the fallback for long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Just the layout number, read first: a future layout may not parse as today's.
#[derive(Deserialize)]
struct Head {
    schema: u32,
}

#[derive(Deserialize)]
struct Manifest {
    app: Option<Series>,
}

#[derive(Deserialize)]
struct Series {
    version: String,
    #[serde(default)]
    assets: BTreeMap<String, ManifestAsset>,
}

#[derive(Deserialize)]
struct ManifestAsset {
    url: String,
    sha256: String,
}

/// Why a manifest was not used. Only ever logged: the caller falls back.
#[derive(Debug, PartialEq, Eq)]
struct Unusable(String);

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The app's newest stable release as the manifest `body` describes it, or why it cannot be
/// trusted. Strict on purpose, because the alternative to an error is the GitHub API:
/// the version must be a plain stable `X.Y.Z`, each digest 64 hex digits, each URL https.
fn parse(body: &str) -> Result<LatestRelease, Unusable> {
    let head: Head =
        serde_json::from_str(body).map_err(|e| Unusable(format!("not JSON with a schema: {e}")))?;
    if head.schema != SCHEMA {
        return Err(Unusable(format!(
            "schema {} (this build reads {SCHEMA})",
            head.schema
        )));
    }
    let manifest: Manifest =
        serde_json::from_str(body).map_err(|e| Unusable(format!("malformed: {e}")))?;
    let series = manifest
        .app
        .ok_or_else(|| Unusable("no app release".to_string()))?;
    if super::parse_version(&series.version).is_none() {
        return Err(Unusable(format!(
            "version {:?} is not a stable X.Y.Z",
            series.version
        )));
    }
    let mut assets = Vec::with_capacity(series.assets.len());
    for (name, a) in series.assets {
        let digest_ok = a.sha256.len() == 64 && a.sha256.bytes().all(|b| b.is_ascii_hexdigit());
        if !digest_ok {
            return Err(Unusable(format!("{name}: sha256 is not 64 hex digits")));
        }
        if !a.url.starts_with("https://") {
            return Err(Unusable(format!("{name}: url is not https")));
        }
        assets.push(Asset {
            name,
            url: a.url,
            sha256: Some(a.sha256.to_ascii_lowercase()),
        });
    }
    Ok(LatestRelease {
        version: series.version,
        assets,
    })
}

/// The manifest's answer, else the fallback's. `fetch` yields the manifest body.
fn resolve(
    fetch: impl FnOnce() -> Result<String, UpdateError>,
    fallback: impl FnOnce() -> Result<LatestRelease, UpdateError>,
) -> Result<LatestRelease, UpdateError> {
    let why = match fetch() {
        Ok(body) => match parse(&body) {
            Ok(release) => return Ok(release),
            Err(unusable) => unusable.to_string(),
        },
        Err(e) => e.to_string(),
    };
    tracing::warn!("release manifest not used ({why}); using the GitHub API");
    fallback()
}

fn fetch_manifest() -> Result<String, UpdateError> {
    Ok(ureq::get(MANIFEST_URL)
        .set("User-Agent", super::USER_AGENT)
        .timeout(FETCH_TIMEOUT)
        .call()?
        .into_string()?)
}

/// Resolves releases from the release manifest, falling back to the GitHub API.
#[derive(Debug, Default, Clone, Copy)]
pub struct ManifestProvider;

impl ReleaseProvider for ManifestProvider {
    fn latest_stable(&self) -> Result<LatestRelease, UpdateError> {
        resolve(fetch_manifest, || GitHubProvider.latest_stable())
    }

    fn releases_url(&self) -> &'static str {
        GitHubProvider.releases_url()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest(version: &str, sha: &str, url: &str) -> String {
        format!(
            r#"{{
  "schema": 1,
  "app": {{
    "version": "{version}",
    "assets": {{
      "kimun-{version}-linux-x64": {{ "url": "{url}", "sha256": "{sha}" }},
      "kimun-gui-{version}-linux-x64": {{ "url": "{url}", "sha256": "{sha}" }}
    }}
  }},
  "server": {{ "version": "0.5.1", "assets": {{}} }}
}}"#
        )
    }

    fn github_answer() -> Result<LatestRelease, UpdateError> {
        Ok(LatestRelease {
            version: "9.9.9".to_string(),
            assets: vec![],
        })
    }

    #[test]
    fn the_app_series_becomes_a_release_with_its_digests() {
        let r = parse(&manifest("0.24.8", H, "https://dl.example/x")).unwrap();
        assert_eq!(r.version, "0.24.8");
        assert_eq!(r.assets.len(), 2);
        let a = r.asset("kimun-gui-0.24.8-linux-x64").unwrap();
        assert_eq!(a.url, "https://dl.example/x");
        assert_eq!(a.sha256.as_deref(), Some(H));
    }

    #[test]
    fn digests_are_lowercased() {
        let upper = H.to_ascii_uppercase();
        let r = parse(&manifest("0.24.8", &upper, "https://dl.example/x")).unwrap();
        assert_eq!(r.assets[0].sha256.as_deref(), Some(H));
    }

    #[test]
    fn an_unknown_schema_is_unusable_even_if_the_rest_would_not_parse() {
        let err = parse(r#"{"schema": 2, "app": "a different shape"}"#).unwrap_err();
        assert!(err.0.contains("schema 2"), "{err}");
    }

    #[test]
    fn a_manifest_with_no_app_or_no_schema_or_garbage_is_unusable() {
        assert!(
            parse(r#"{"schema": 1}"#)
                .unwrap_err()
                .0
                .contains("no app release")
        );
        assert!(parse(r#"{"app": {}}"#).is_err());
        assert!(parse("<html>captive portal</html>").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn only_plain_stable_versions_are_trusted() {
        for bad in ["0.25.0-beta.1", "0.25", "latest", ""] {
            assert!(
                parse(&manifest(bad, H, "https://dl.example/x")).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_bad_digest_or_a_non_https_url_makes_it_unusable() {
        assert!(
            parse(&manifest("0.24.8", "abc", "https://dl.example/x"))
                .unwrap_err()
                .0
                .contains("sha256")
        );
        assert!(parse(&manifest("0.24.8", &"g".repeat(64), "https://dl.example/x")).is_err());
        assert!(
            parse(&manifest("0.24.8", H, "http://dl.example/x"))
                .unwrap_err()
                .0
                .contains("https")
        );
        assert!(parse(&manifest("0.24.8", H, "file:///etc/passwd")).is_err());
    }

    #[test]
    fn a_usable_manifest_never_reaches_the_fallback() {
        let r = resolve(
            || Ok(manifest("0.24.8", H, "https://dl.example/x")),
            || panic!("the fallback must not run"),
        )
        .unwrap();
        assert_eq!(r.version, "0.24.8");
    }

    #[test]
    fn an_unreachable_or_unusable_manifest_falls_back() {
        let unreachable = resolve(
            || Err(UpdateError::Io(std::io::Error::other("offline"))),
            github_answer,
        );
        assert_eq!(unreachable.unwrap().version, "9.9.9");
        let unusable = resolve(|| Ok(r#"{"schema": 7}"#.to_string()), github_answer);
        assert_eq!(unusable.unwrap().version, "9.9.9");
    }

    #[test]
    fn the_fallbacks_error_is_the_error_when_both_fail() {
        let r = resolve(
            || Err(UpdateError::Io(std::io::Error::other("offline"))),
            || Err(UpdateError::NoRelease),
        );
        assert!(matches!(r, Err(UpdateError::NoRelease)));
    }
}
