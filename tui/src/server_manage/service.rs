//! The service that runs the server: systemd on Linux, launchd on macOS.
//!
//! The app restarts the server only through the service manager. The app did not start that
//! process and cannot find it in a reliable way. [`ServiceControl`] is the seam: the real one
//! runs `systemctl` or `launchctl`, a test passes a fake.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::marker::ServiceKind;

/// The systemd unit's name. Must match `install-server.sh`.
const UNIT: &str = "kimun-server.service";
/// The launchd label. Must match `install-server.sh`.
const LAUNCHD_LABEL: &str = "dev.2co.kimun-server";

/// Ask about, and restart, the server's service.
pub trait ServiceControl: Send + Sync {
    /// Whether the service is running now.
    fn is_active(&self) -> bool;
    /// Restart the service. Returns the reason when it fails.
    fn restart(&self) -> Result<(), String>;
}

/// The service file the script wrote, under `home`.
pub fn service_file(kind: ServiceKind, home: &Path) -> PathBuf {
    match kind {
        ServiceKind::Systemd => home.join(".config/systemd/user").join(UNIT),
        ServiceKind::Launchd => home
            .join("Library/LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist")),
    }
}

/// The real service manager of this machine.
#[derive(Debug, Clone, Copy)]
pub struct SystemService {
    kind: ServiceKind,
}

impl SystemService {
    pub fn new(kind: ServiceKind) -> Self {
        Self { kind }
    }
}

fn run(cmd: &mut Command) -> Result<std::process::Output, String> {
    cmd.output()
        .map_err(|e| format!("could not run {:?}: {e}", cmd.get_program()))
}

fn failure(what: &str, out: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        format!("{what} failed ({})", out.status)
    } else {
        format!("{what} failed: {stderr}")
    }
}

/// The user's numeric id, which `launchctl` needs in a domain name.
fn user_id() -> Result<String, String> {
    let out = run(Command::new("id").arg("-u"))?;
    if !out.status.success() {
        return Err(failure("id -u", &out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl ServiceControl for SystemService {
    fn is_active(&self) -> bool {
        match self.kind {
            ServiceKind::Systemd => {
                run(Command::new("systemctl").args(["--user", "is-active", "--quiet", UNIT]))
                    .is_ok_and(|o| o.status.success())
            }
            // `launchctl list <label>` prints a "PID" entry only while the job runs.
            ServiceKind::Launchd => run(Command::new("launchctl").args(["list", LAUNCHD_LABEL]))
                .is_ok_and(|o| {
                    o.status.success() && String::from_utf8_lossy(&o.stdout).contains("\"PID\"")
                }),
        }
    }

    fn restart(&self) -> Result<(), String> {
        let out = match self.kind {
            ServiceKind::Systemd => {
                run(Command::new("systemctl").args(["--user", "restart", UNIT]))?
            }
            ServiceKind::Launchd => {
                let target = format!("gui/{}/{LAUNCHD_LABEL}", user_id()?);
                run(Command::new("launchctl").args(["kickstart", "-k", &target]))?
            }
        };
        if out.status.success() {
            Ok(())
        } else {
            Err(failure("restarting the service", &out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_files_are_where_the_script_writes_them() {
        let home = Path::new("/home/u");
        assert_eq!(
            service_file(ServiceKind::Systemd, home),
            PathBuf::from("/home/u/.config/systemd/user/kimun-server.service")
        );
        assert_eq!(
            service_file(ServiceKind::Launchd, home),
            PathBuf::from("/home/u/Library/LaunchAgents/dev.2co.kimun-server.plist")
        );
    }
}
