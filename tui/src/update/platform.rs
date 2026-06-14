//! Compile-time release-platform identity, matching the asset names produced by
//! `build.yml` (see adr/0014). Returns `None` on a target kimün does not
//! publish binaries for, which forces notify-only with no self-update path.

/// The release platform string for this build (`linux-x64`, `macos-x64`,
/// `macos-arm64`, `windows-x64`), or `None` if kimün publishes no binary for
/// this target.
pub fn platform() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos-x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-arm64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else {
        None
    }
}

/// Name of the raw binary release asset for `version` on this platform, e.g.
/// `kimun-0.18.0-linux-x64` (or `…-windows-x64.exe`). `None` on unsupported
/// platforms. Must match what `build.yml` uploads.
pub fn binary_asset_name(version: &str) -> Option<String> {
    let platform = platform()?;
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
    Some(format!("kimun-{version}-{platform}{ext}"))
}
