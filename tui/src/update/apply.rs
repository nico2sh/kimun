//! Self-update: download the raw binary, verify its checksum, and swap the
//! running executable in place. Blocking (ureq) — call on `spawn_blocking`.
//! See adr/0014.

use std::io::Read;

use sha2::{Digest, Sha256};

use super::provider::LatestRelease;
use super::{UpdateError, platform};

const CHECKSUMS_ASSET: &str = "checksums-sha256.txt";

/// Download the raw binary for `latest`, verify it against
/// `checksums-sha256.txt`, and replace the currently running executable.
///
/// The caller must ensure self-update is permitted for the install channel; the
/// channel gate lives in [`super::InstallChannel`], not here.
pub fn self_update(latest: &LatestRelease) -> Result<(), UpdateError> {
    let asset_name =
        platform::binary_asset_name(&latest.version).ok_or(UpdateError::UnsupportedPlatform)?;

    let binary_asset = latest
        .asset(&asset_name)
        .ok_or_else(|| UpdateError::MissingAsset(asset_name.clone()))?;
    let checksums_asset = latest
        .asset(CHECKSUMS_ASSET)
        .ok_or_else(|| UpdateError::MissingAsset(CHECKSUMS_ASSET.to_string()))?;

    // Resolve the expected digest before downloading the (larger) binary.
    let checksums_bytes = download_bytes(&checksums_asset.url)?;
    let checksums = String::from_utf8_lossy(&checksums_bytes);
    let expected = checksum_for(&checksums, &asset_name)
        .ok_or_else(|| UpdateError::NoChecksum(asset_name.clone()))?;

    let bytes = download_bytes(&binary_asset.url)?;
    let actual = hex_sha256(&bytes);
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(UpdateError::ChecksumMismatch { expected, actual });
    }

    // Stage the verified binary next to the target so the swap is same-volume,
    // then let self-replace perform the cross-platform live-exe replacement.
    let exe = std::env::current_exe()?;
    let dir = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
    let staged = dir.join(format!(".{asset_name}.new"));
    std::fs::write(&staged, &bytes)?;
    set_executable(&staged)?;

    let result = self_replace::self_replace(&staged).map_err(UpdateError::Replace);
    // Clean up the staging file regardless of outcome.
    let _ = std::fs::remove_file(&staged);
    result
}

/// Upper bound on any single download, guarding against an unbounded read from
/// a rogue redirect or misconfigured asset. A truncated body simply fails the
/// later checksum comparison and is rejected.
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

fn download_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    let mut buf = Vec::new();
    super::http_get(url)?
        .into_reader()
        .take(MAX_DOWNLOAD_BYTES)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Look up the digest for `asset_name` in a `sha256sum`-format file
/// (`<hex>  <filename>` per line).
fn checksum_for(checksums: &str, asset_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let (digest, name) = line.split_once(char::is_whitespace)?;
        (name.trim() == asset_name).then(|| digest.trim().to_string())
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(unix)]
fn set_executable(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_executable(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
