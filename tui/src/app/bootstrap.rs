//! Process bootstrap for the TUI: file logging and the panic hook. Both are
//! set up before the **Terminal session** is entered and outlive it.

use std::path::{Path, PathBuf};

use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

/// Initialises file logging under `log_dir`.
///
/// Returns `Some(guard)` on success; the caller must keep the guard alive for
/// the duration of the program. Returns `None` and prints a warning to stderr
/// on failure — startup is not aborted. Takes the directory as a parameter so
/// tests can call it with a controlled path.
pub fn init_logging(log_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    if let Err(e) = kimun_core::system::create_dir(log_dir) {
        eprintln!("kimun: could not create log directory: {e}");
        return None;
    }

    let log_path = log_dir.join("kimun.log");
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("kimun: could not open log file: {e}");
            return None;
        }
    };

    let (writer, guard) = tracing_appender::non_blocking(file);

    #[cfg(debug_assertions)]
    let file_level_filter = LevelFilter::DEBUG;
    #[cfg(not(debug_assertions))]
    let file_level_filter = LevelFilter::WARN;

    let file_layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync> =
        tracing_subscriber::fmt::layer()
            .compact()
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(file_level_filter)
            .boxed();

    // No stderr layer — writing to stderr corrupts the ratatui alternate
    // screen. Debug logs are captured in the log file at DEBUG level instead.
    let layers: Vec<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> = vec![file_layer];

    // try_init instead of init so tests can call this without panicking on the
    // global-subscriber-already-set error.
    let _ = tracing_subscriber::registry().with(layers).try_init();

    // Forward log:: crate events into the tracing pipeline.
    tracing_log::LogTracer::init().ok();

    Some(guard)
}

/// Installs a panic hook that leaves the terminal session (so the panic
/// message is readable), records the panic through tracing, appends it with a
/// backtrace to `log_path` directly (independent of the subscriber), then
/// defers to the default hook.
pub fn install_panic_hook(log_path: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        super::terminal::leave(&mut std::io::stderr());

        // Emit through tracing first (subscriber may still be active).
        tracing::error!("panic: {info}");

        // Direct fallback write — independent of the tracing subscriber.
        let parent = log_path.parent().unwrap_or(Path::new("."));
        let _ = kimun_core::system::create_dir(parent);
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let _ = writeln!(file, "[PANIC] {info}");
                let bt = std::backtrace::Backtrace::force_capture();
                let _ = writeln!(file, "{bt}");
            }
            Err(e) => {
                eprintln!("kimun: could not write panic to log: {e}");
            }
        }

        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::init_logging;

    #[test]
    fn init_logging_returns_none_on_bad_path() {
        // A directory nested under a regular *file* cannot be created on any
        // platform, so this reliably exercises the early return before
        // `try_init` — leaving the global subscriber singleton unset.
        //
        // A literal like `/nonexistent/readonly/path` does not do that: on
        // Windows it has no drive prefix, so it is merely rooted and resolves
        // against the current drive, where `create_dir_all` cheerfully creates
        // it (and litters the drive root) and `init_logging` returns `Some`.
        let tmp = tempfile::TempDir::new().unwrap();
        let not_a_dir = tmp.path().join("regular-file");
        std::fs::write(&not_a_dir, b"not a directory").unwrap();

        let result = init_logging(&not_a_dir.join("logs"));

        assert!(result.is_none());
    }
}
