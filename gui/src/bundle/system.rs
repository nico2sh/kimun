//! The real bundle installer (ADR-0056): finds the running app's bundle, decides
//! whether it can replace itself, and installs the newest release over it.
//!
//! Compiled under `cfg(test)` on every platform so its glue is type-checked
//! off macOS; only macOS builds call it ([`detect`]).

// Only the macOS installer calls this; the unit tests build it without the app.
#![cfg_attr(test, allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use kimun_notes::update;

use super::backend::BundleInstaller;
use super::installed_bundle;
use super::signature::SystemRunner;
use super::swap::BundleSwap;

/// The bundle the running executable lives in, when the app can replace it:
/// there is one, and the folder holding it can be written to (the swap moves
/// the app aside and a new one in). Worked out once per process, on the first
/// window: the probe writes a file, and every window asks.
pub(crate) fn detect() -> Option<Arc<dyn BundleInstaller>> {
    static REPLACEABLE: OnceLock<Option<PathBuf>> = OnceLock::new();
    let bundle = once(&REPLACEABLE, || {
        let exe = std::env::current_exe().ok()?;
        replaceable_bundle(&exe)
    })?;
    Some(Arc::new(SystemBundleInstaller { bundle }))
}

/// `cell`'s value, computed by `make` the first time only.
fn once<T: Clone>(cell: &OnceLock<T>, make: impl FnOnce() -> T) -> T {
    cell.get_or_init(make).clone()
}

/// The bundle for `exe` when its folder is writable.
fn replaceable_bundle(exe: &Path) -> Option<PathBuf> {
    let bundle = installed_bundle(exe)?;
    folder_writable(bundle.parent()?).then_some(bundle)
}

/// Whether a file can be created in `dir`: the only honest test, since
/// permission bits do not say (ACLs, read-only volumes).
fn folder_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".kimun-write-test-{}", std::process::id()));
    let created = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    created
}

/// The release to install is the one the user confirmed. A newer one may have
/// come out since the check: installing it unasked would also leave the
/// offered, skipped and dismissed records pointing at another version.
fn confirm_version(confirmed: &str, newest: &str) -> Result<(), String> {
    if confirmed == newest {
        Ok(())
    } else {
        Err(format!(
            "version {confirmed} was announced, but {newest} is the newest now; check for updates again"
        ))
    }
}

struct SystemBundleInstaller {
    bundle: PathBuf,
}

impl BundleInstaller for SystemBundleInstaller {
    fn install(&self, version: &str) -> Result<String, String> {
        let latest = update::fetch_latest().map_err(|e| e.to_string())?;
        confirm_version(version, &latest.version)?;
        let name = update::bundle_asset_name(&latest.version)
            .ok_or("this system has no desktop app to update to")?;
        // A release cut without Apple signing has no `.dmg`: say what to do
        // rather than a bare missing-asset error.
        if latest.asset(&name).is_none() {
            return Err(format!(
                "this release has no macOS app image ({name}); see the releases page"
            ));
        }
        let image = update::download_asset(&latest, &name).map_err(|e| e.to_string())?;
        BundleSwap::new(&SystemRunner, self.bundle.clone())
            .install_image(&image, &std::env::temp_dir())
            .map_err(|e| e.to_string())?;
        Ok(latest.version)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn only_the_confirmed_version_is_installed() {
        assert_eq!(confirm_version("1.2.0", "1.2.0"), Ok(()));
        let err = confirm_version("1.2.0", "1.3.0").unwrap_err();
        assert!(err.contains("1.2.0") && err.contains("1.3.0"), "{err}");
        assert!(err.contains("check for updates"), "{err}");
    }

    #[test]
    fn the_answer_is_worked_out_once_however_often_it_is_asked() {
        let cell = OnceLock::new();
        let asked = std::cell::Cell::new(0);
        let probe = || {
            asked.set(asked.get() + 1);
            Some(PathBuf::from("/apps/Kimun.app"))
        };
        assert_eq!(once(&cell, probe), Some(PathBuf::from("/apps/Kimun.app")));
        assert_eq!(once(&cell, probe), Some(PathBuf::from("/apps/Kimun.app")));
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn a_writable_folder_is_writable_and_the_probe_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(folder_writable(dir.path()));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_folder_is_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!folder_writable(&dir.path().join("nope")));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_is_not_writable() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let mut perms = fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o555);
        fs::set_permissions(dir.path(), perms.clone()).unwrap();
        let writable = folder_writable(dir.path());
        perms.set_mode(0o755);
        fs::set_permissions(dir.path(), perms).unwrap();
        // Running as root writes anywhere: only assert when the mode held.
        if !is_root() {
            assert!(!writable);
        }
    }

    #[cfg(unix)]
    fn is_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
    }

    #[test]
    fn only_a_bundle_in_a_writable_folder_is_replaceable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir
            .path()
            .join("Kimun.app")
            .join("Contents")
            .join("MacOS")
            .join("kimun-gui");
        assert_eq!(replaceable_bundle(&exe), Some(dir.path().join("Kimun.app")));
        // A raw binary is not a bundle, writable folder or not.
        assert_eq!(replaceable_bundle(&dir.path().join("kimun-gui")), None);
    }
}
