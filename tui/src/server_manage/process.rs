//! Run a program and give up on it after a time limit.
//!
//! The update calls programs that can hang: the new server binary (`--version`), `systemctl`, and
//! `launchctl`. A hang must not hold the update, and the window's "Updating…" state, for ever.

use std::io;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long a service-manager command may take.
pub const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a server binary may take to print its version.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

/// Run `cmd` with no input and captured output. When it does not finish within `timeout`, the
/// program is killed and the error has kind [`io::ErrorKind::TimedOut`].
pub fn run_bounded(cmd: &mut Command, timeout: Duration) -> io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("did not finish within {} seconds", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_finishes_returns_its_output() {
        let out = run_bounded(Command::new("echo").arg("hi"), Duration::from_secs(5)).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn a_program_that_hangs_is_killed_and_reported() {
        let start = Instant::now();
        let err =
            run_bounded(Command::new("sleep").arg("30"), Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "it did not wait for the program"
        );
    }

    #[test]
    fn a_program_that_cannot_start_is_an_error() {
        assert!(
            run_bounded(
                &mut Command::new("/no/such/program"),
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
