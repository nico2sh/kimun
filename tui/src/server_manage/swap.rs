//! The files of an update: stage the new binary, swap it in without ever leaving the install
//! without a binary, and put the old one back.
//!
//! The old binary is never moved away. A hard link keeps it under the backup name, and one rename
//! puts the new binary over the real name. A rename is atomic, so a crash at any point leaves a
//! working binary under the real name: the old one before the rename, the new one after it.

use std::path::{Path, PathBuf};

use kimun_core::system;

use super::marker::{BINARY, ServerInstall};
use super::process::{VERSION_TIMEOUT, run_bounded};

/// The paths of one update.
#[derive(Debug, Clone)]
pub struct Swap {
    bin: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
}

impl Swap {
    pub fn new(install: &ServerInstall) -> Self {
        let dir = &install.install_dir;
        Self {
            bin: install.binary(),
            staged: dir.join(format!(".{BINARY}.new")),
            backup: dir.join(format!("{BINARY}.old")),
        }
    }

    /// The version that the installed binary reports. This, and not a network probe, tells what is
    /// installed: it is the same answer the install script reads.
    pub fn installed_version(&self) -> Option<String> {
        reported_version(&self.bin).ok()
    }

    /// Write `binary` next to the installed one and check that it runs and reports `expect`.
    /// Nothing that is installed changes. On a failure the staged file is removed.
    pub fn stage(&self, binary: &[u8], expect: &str) -> Result<(), String> {
        // A file left by an earlier crash is replaced, never trusted.
        system::remove_file(&self.staged).map_err(|e| e.to_string())?;
        let checked = system::replace_atomically(&self.staged, binary)
            .map_err(|e| e.to_string())
            .and_then(|()| system::make_executable(&self.staged).map_err(|e| e.to_string()))
            .and_then(|()| reported_version(&self.staged))
            .and_then(|v| {
                if v == expect {
                    Ok(())
                } else {
                    Err(format!("it reports version {v}, not {expect}"))
                }
            });
        if checked.is_err() {
            let _ = system::remove_file(&self.staged);
        }
        checked
    }

    /// Keep the old binary as the backup, then put the staged one in its place. The installed
    /// name always exists. A failure leaves the old binary in place.
    pub fn commit(&self) -> Result<(), String> {
        system::remove_file(&self.backup).map_err(|e| e.to_string())?;
        self.link_backup()?;
        if let Err(e) = system::move_file(&self.staged, &self.bin) {
            let _ = system::remove_file(&self.backup);
            let _ = system::remove_file(&self.staged);
            return Err(e.to_string());
        }
        Ok(())
    }

    /// The backup, by hard link, or by copy where the file system has no hard links.
    fn link_backup(&self) -> Result<(), String> {
        if std::fs::hard_link(&self.bin, &self.backup).is_ok() {
            return Ok(());
        }
        let bytes =
            std::fs::read(&self.bin).map_err(|e| format!("could not read the old server: {e}"))?;
        system::replace_atomically(&self.backup, &bytes).map_err(|e| e.to_string())?;
        system::make_executable(&self.backup).map_err(|e| e.to_string())
    }

    /// Put the old binary back over the installed name.
    pub fn roll_back(&self) -> Result<(), String> {
        system::move_file(&self.backup, &self.bin).map_err(|e| {
            format!(
                "could not put {} back: {e}. The old file is {}",
                self.bin.display(),
                self.backup.display()
            )
        })
    }

    /// The update succeeded: the backup is not needed.
    pub fn drop_backup(&self) {
        let _ = system::remove_file(&self.backup);
    }

    #[cfg(test)]
    pub fn paths(&self) -> (&Path, &Path, &Path) {
        (&self.bin, &self.staged, &self.backup)
    }
}

/// Run `binary --version` and return the version it reports (`kimun-server X.Y.Z`).
pub fn reported_version(binary: &Path) -> Result<String, String> {
    // A just-written executable can be briefly busy when another thread forks at that moment.
    let mut last = String::new();
    for _ in 0..20 {
        match run_bounded(
            std::process::Command::new(binary).arg("--version"),
            VERSION_TIMEOUT,
        ) {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                return text
                    .split_whitespace()
                    .nth(1)
                    .map(str::to_string)
                    .ok_or_else(|| format!("it printed {:?} for --version", text.trim()));
            }
            Ok(out) => return Err(format!("--version failed ({})", out.status)),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                last = e.to_string();
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(format!("it cannot run: {e}")),
        }
    }
    Err(format!("it cannot run: {last}"))
}
