//! Replacing the desktop app's own `Kimun.app` bundle on macOS (ADR-0056).
//!
//! Frontend-specific host work that does not touch vault content, so it lives
//! here and not in core. The modules compile under `cfg(test)` on every
//! platform: they hold logic over a `signature::CommandRunner` seam, and the
//! tests run that logic against canned tool output.
//!
//! - `signature`: whether a staged app may replace the running one
//!   (valid Developer ID signature, same Team ID).
//! - `swap`: mount the release image, stage and check the new app, swap it
//!   in, and undo on failure.
//! - `system`: the real installer over those two, for the running app.
//! - [`backend`]: the desktop app's update backend with the bundle's install
//!   added to the shared one.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The app bundle the running executable lives in: the first ancestor ending in
/// `.app` when the path goes through `Contents/MacOS`. `None` for anything else
/// (a raw binary).
pub(crate) fn installed_bundle(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|dir| dir.extension() == Some(OsStr::new("app")))
        .filter(|app| exe.starts_with(app.join("Contents").join("MacOS")))
        .map(Path::to_path_buf)
}

#[cfg(any(target_os = "macos", test))]
pub(crate) mod signature;

#[cfg(any(target_os = "macos", test))]
pub(crate) mod swap;

pub(crate) mod backend;

#[cfg(any(target_os = "macos", test))]
pub(crate) mod system;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_is_found_from_the_executable_path() {
        assert_eq!(
            installed_bundle(Path::new(
                "/Users/me/Applications/Kimun.app/Contents/MacOS/kimun-gui"
            )),
            Some(PathBuf::from("/Users/me/Applications/Kimun.app"))
        );
        assert_eq!(
            installed_bundle(Path::new("/Users/me/.local/bin/kimun-gui")),
            None
        );
        // `.app` in a parent folder name is not the bundle for a raw binary.
        assert_eq!(
            installed_bundle(Path::new("/Users/me/x.app/bin/kimun-gui")),
            None
        );
    }
}
