//! Provider-agnostic release model and the [`ReleaseProvider`] trait.
//!
//! The self-updater talks to a release backend only through this trait. To use
//! a different source (GitLab, a self-hosted index, a mirror), implement
//! `ReleaseProvider` and swap the constructor in [`super::provider`]. Everything
//! downstream (status computation, download, checksum, swap) is backend-neutral
//! — it only consumes [`LatestRelease`]/[`Asset`] and their plain URLs.

use super::UpdateError;

/// A single downloadable release artifact (binary, archive, or checksums file).
#[derive(Debug, Clone)]
pub struct Asset {
    /// Asset filename, e.g. `kimun-0.18.0-linux-x64`.
    pub name: String,
    /// Direct download URL.
    pub url: String,
}

/// The newest stable app release and everything needed to download it.
#[derive(Debug, Clone)]
pub struct LatestRelease {
    /// Version without any tag prefix, e.g. `0.18.0`.
    pub version: String,
    /// Release notes, if the backend supplies them.
    pub notes: Option<String>,
    /// Downloadable assets (raw binaries, archives, checksums).
    pub assets: Vec<Asset>,
}

impl LatestRelease {
    /// Find an asset by exact name.
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// Source of release information for the self-updater. Implement this and swap
/// the constructor in [`super::provider`] to change backends without touching
/// any caller.
pub trait ReleaseProvider {
    /// The newest stable release for this app, with its downloadable assets.
    /// Blocking — callers run it on the blocking pool.
    fn latest_stable(&self) -> Result<LatestRelease, UpdateError>;

    /// Human-facing page listing releases, shown when self-update isn't
    /// available on the current install channel.
    fn releases_url(&self) -> &'static str;
}
