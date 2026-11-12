//! Compile-time release-platform identity, matching the asset names produced by
//! `build.yml`. Returns `None` on a target kimün does not
//! publish binaries for, which forces notify-only with no self-update path.

/// The release platform string for this build (`linux-x64`, `linux-arm64`,
/// `macos-x64`, `macos-arm64`, `windows-x64`), or `None` if kimün publishes no binary for
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

/// Which of kimün's two apps is updating itself. Each has its own raw-binary
/// asset on every release, and must only ever replace itself with its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// The terminal app, `kimun`.
    Terminal,
    /// The desktop app, `kimun-gui`.
    Desktop,
}

impl Product {
    /// The binary's (and the asset's) name stem.
    pub fn stem(self) -> &'static str {
        match self {
            Self::Terminal => "kimun",
            Self::Desktop => "kimun-gui",
        }
    }
}

/// Name of the raw binary release asset of `product` for `version` on this
/// platform, e.g. `kimun-0.18.0-linux-x64` or `kimun-gui-0.18.0-linux-x64` (or
/// `…-windows-x64.exe`). `None` on unsupported platforms. Must match what
/// `build.yml` uploads (and `verify-release-assets.sh` checks).
pub fn binary_asset_name(product: Product, version: &str) -> Option<String> {
    let platform = platform()?;
    let ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    Some(format!("{}-{version}-{platform}{ext}", product.stem()))
}

/// Name of the desktop app's signed macOS `.dmg` release asset for `version`
/// on this platform, e.g. `kimun-gui-0.18.0-macos-arm64.dmg`. `None` on
/// unsupported platforms. Must match what `build.yml` uploads (and
/// `verify-release-assets.sh` checks).
pub fn bundle_asset_name(version: &str) -> Option<String> {
    let platform = platform()?;
    Some(format!(
        "{}-{version}-{platform}.dmg",
        Product::Desktop.stem()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_app_has_its_own_asset_name() {
        let Some(platform) = platform() else { return };
        let ext = if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        };
        assert_eq!(
            binary_asset_name(Product::Terminal, "1.2.3").as_deref(),
            Some(format!("kimun-1.2.3-{platform}{ext}").as_str())
        );
        assert_eq!(
            binary_asset_name(Product::Desktop, "1.2.3").as_deref(),
            Some(format!("kimun-gui-1.2.3-{platform}{ext}").as_str())
        );
    }

    #[test]
    fn the_bundle_asset_is_the_desktop_dmg() {
        let Some(platform) = platform() else { return };
        assert_eq!(
            bundle_asset_name("1.2.3").as_deref(),
            Some(format!("kimun-gui-1.2.3-{platform}.dmg").as_str())
        );
    }

    #[test]
    fn the_desktop_name_is_never_the_terminals() {
        // The terminal app's asset name is a prefix of the desktop's glob
        // (`kimun-*`), never equal to it: an exact-name lookup cannot mix them up.
        assert_ne!(
            binary_asset_name(Product::Terminal, "9.9.9"),
            binary_asset_name(Product::Desktop, "9.9.9")
        );
    }
}
