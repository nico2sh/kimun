//! The server install marker: `server-install.toml`, written by `install-server.sh` (ADR-0059).
//!
//! It is the only proof the app accepts that the script installed the Kimün server on this
//! machine. The app never guesses from a path or a unit file. The marker names the folder
//! holding the binary and the kind of service that restarts it.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The marker's file name, in the config folder next to `server.toml`.
pub const MARKER_FILE: &str = "server-install.toml";

/// The server binary's name, in the install folder.
pub const BINARY: &str = "kimun-server";

/// The service manager that restarts the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// `systemctl --user`, on Linux.
    Systemd,
    /// `launchctl`, on macOS.
    Launchd,
}

/// What a valid marker says about this machine's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInstall {
    /// The folder that holds the `kimun-server` binary.
    pub install_dir: PathBuf,
    /// The service that runs the binary.
    pub service: ServiceKind,
}

impl ServerInstall {
    /// Where the binary is.
    pub fn binary(&self) -> PathBuf {
        self.install_dir.join(BINARY)
    }
}

#[derive(Deserialize)]
struct Raw {
    installer: Option<String>,
    install_dir: Option<String>,
    service: Option<String>,
}

/// Why the marker does not allow an update. Shown to the user as the reason for "no button".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerProblem {
    /// No marker: Docker, Cargo, a remote server, or an install from before the marker.
    Missing,
    /// The marker is there but is not a marker this build can use.
    Unreadable(String),
    /// The script installed the server but set up no service, so nothing can restart it.
    NoService,
}

impl std::fmt::Display for MarkerProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "the install script did not install this server"),
            Self::Unreadable(why) => write!(f, "the install marker is not usable: {why}"),
            Self::NoService => write!(f, "the server has no service that can restart it"),
        }
    }
}

/// Read and check the marker in `config_dir`. This checks only the marker's own text. The
/// binary and the service file are checked by [`super::check`], because they change over time.
pub fn load(config_dir: &Path) -> Result<ServerInstall, MarkerProblem> {
    let raw = match std::fs::read_to_string(config_dir.join(MARKER_FILE)) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(MarkerProblem::Missing),
        Err(e) => return Err(MarkerProblem::Unreadable(e.to_string())),
    };
    parse(&raw)
}

fn parse(raw: &str) -> Result<ServerInstall, MarkerProblem> {
    let marker: Raw = toml::from_str(raw).map_err(|e| MarkerProblem::Unreadable(e.to_string()))?;
    if marker.installer.as_deref() != Some("install-server.sh") {
        return Err(MarkerProblem::Unreadable(
            "not written by install-server.sh".into(),
        ));
    }
    let dir = marker
        .install_dir
        .filter(|d| !d.is_empty())
        .ok_or_else(|| MarkerProblem::Unreadable("no install_dir".into()))?;
    let install_dir = PathBuf::from(dir);
    if !install_dir.is_absolute() {
        return Err(MarkerProblem::Unreadable(
            "install_dir is not an absolute path".into(),
        ));
    }
    let service = match marker.service.as_deref() {
        Some("systemd") => ServiceKind::Systemd,
        Some("launchd") => ServiceKind::Launchd,
        Some("none") => return Err(MarkerProblem::NoService),
        Some(other) => {
            return Err(MarkerProblem::Unreadable(format!(
                "unknown service {other:?}"
            )));
        }
        None => return Err(MarkerProblem::Unreadable("no service".into())),
    };
    Ok(ServerInstall {
        install_dir,
        service,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(service: &str) -> String {
        format!(
            "installer = \"install-server.sh\"\ninstall_dir = \"/home/u/.local/bin\"\nservice = \"{service}\"\n"
        )
    }

    #[test]
    fn a_systemd_marker_names_the_folder_and_the_service() {
        let m = parse(&text("systemd")).unwrap();
        assert_eq!(m.install_dir, PathBuf::from("/home/u/.local/bin"));
        assert_eq!(m.service, ServiceKind::Systemd);
        assert_eq!(m.binary(), PathBuf::from("/home/u/.local/bin/kimun-server"));
        assert_eq!(
            parse(&text("launchd")).unwrap().service,
            ServiceKind::Launchd
        );
    }

    #[test]
    fn a_marker_with_no_service_says_so() {
        assert_eq!(parse(&text("none")), Err(MarkerProblem::NoService));
    }

    #[test]
    fn a_missing_file_is_missing_not_broken() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Err(MarkerProblem::Missing));
    }

    #[test]
    fn a_marker_this_build_cannot_trust_is_unreadable() {
        for bad in [
            "",
            "garbage = [",
            "installer = \"someone-else\"\ninstall_dir = \"/x\"\nservice = \"systemd\"\n",
            "installer = \"install-server.sh\"\nservice = \"systemd\"\n",
            "installer = \"install-server.sh\"\ninstall_dir = \"relative/bin\"\nservice = \"systemd\"\n",
            "installer = \"install-server.sh\"\ninstall_dir = \"/x\"\nservice = \"sysv\"\n",
            "installer = \"install-server.sh\"\ninstall_dir = \"/x\"\n",
        ] {
            assert!(
                matches!(parse(bad), Err(MarkerProblem::Unreadable(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_file_the_script_writes_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MARKER_FILE), text("systemd")).unwrap();
        assert!(load(dir.path()).is_ok());
    }
}
