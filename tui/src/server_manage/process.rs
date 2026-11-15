//! Run a program and give up on it after a time limit.
//!
//! The update calls programs that can hang: the new server binary (`--version`), `systemctl`, and
//! `launchctl`. A hang must not hold the update, and the window's "Updating…" state, for ever.

use std::io;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// How long a service-manager command may take.
pub const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a server binary may take to print its version.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

/// Run `cmd` with no input and captured output. When it does not finish within `timeout`, the
/// program is killed and the error has kind [`io::ErrorKind::TimedOut`].
///
/// The output is read while the program runs, so a program that writes more than a pipe holds does
/// not block. The readers are bounded too: a program that leaves a child holding the pipe open
/// cannot hold this call.
pub fn run_bounded(cmd: &mut Command, timeout: Duration) -> io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let stdout = reader(child.stdout.take());
    let stderr = reader(child.stderr.take());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
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
    };
    // The program has exited. Its output is complete unless a grandchild keeps the pipe open.
    let drain = Duration::from_secs(2);
    Ok(Output {
        status,
        stdout: stdout.finish(drain),
        stderr: stderr.finish(drain),
    })
}

/// What a reader thread has read so far, and a signal for when it reached the end.
struct Reader {
    bytes: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl Reader {
    /// The output, after waiting up to `wait` for the end of the stream. When a grandchild holds the
    /// pipe open, what was read by then is returned.
    fn finish(self, wait: Duration) -> Vec<u8> {
        let _ = self.done.recv_timeout(wait);
        self.bytes.lock().map(|b| b.clone()).unwrap_or_default()
    }
}

/// Read `pipe` on a thread, keeping what it reads as it reads.
fn reader<R: io::Read + Send + 'static>(pipe: Option<R>) -> Reader {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, done) = mpsc::channel();
    let sink = Arc::clone(&bytes);
    std::thread::spawn(move || {
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if let Ok(mut all) = sink.lock() {
                    all.extend_from_slice(&chunk[..n]);
                }
            }
        }
        let _ = tx.send(());
    });
    Reader { bytes, done }
}

#[cfg(test)]
#[cfg(unix)]
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
    fn a_program_with_a_lot_of_output_does_not_block() {
        // More than a pipe holds: it must not be mistaken for a hang.
        let out = run_bounded(
            Command::new("sh").args(["-c", "head -c 300000 /dev/zero"]),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(out.stdout.len(), 300_000);
    }

    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hold_the_call() {
        let start = Instant::now();
        let out = run_bounded(
            Command::new("sh").args(["-c", "sleep 30 & echo done"]),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("done"));
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "it waited for the grandchild"
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
