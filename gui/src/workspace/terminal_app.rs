//! The **terminal app** seen from the window: Preferences > Display shows
//! whether `kimun` is installed and which version it is, read-only. The window
//! never installs or updates it (`CONTEXT.md`, GUI).
//!
//! The lookup is [`kimun_notes::update::detect_terminal_app`]: the install
//! marker's directory, then `PATH`, then `kimun --version` with a two-second
//! limit. It starts a process, so it runs on tokio's blocking pool, never on
//! the UI thread, **each time Preferences open** (so a terminal app updated in
//! a shell shows its new version the next time). While it runs the row says
//! "checking…". The words are `logic::update`'s ([`terminal_app_text`]).
//!
//! [`terminal_app_text`]: kimun_notes::logic::update::terminal_app_text

use std::sync::Arc;

use gpui_kit::{Context, Task};
use kimun_notes::update::TerminalApp;

use super::Workspace;
use crate::tasks::blocking;

/// What Preferences shows about the terminal app: no row, the lookup under
/// way, or what it found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum TerminalAppRow {
    #[default]
    Hidden,
    Checking,
    Found(TerminalApp),
}

/// Looks for the terminal app. Blocking: it runs on the blocking pool.
pub(crate) trait TerminalAppLookup: Send + Sync {
    fn find(&self) -> TerminalApp;
}

/// The real lookup, over the config directory that holds the install marker.
/// Not built under `cfg(test)`: a test never starts a process on the machine.
#[cfg(not(test))]
struct SystemLookup {
    config_dir: std::path::PathBuf,
}

#[cfg(not(test))]
impl TerminalAppLookup for SystemLookup {
    fn find(&self) -> TerminalApp {
        kimun_notes::update::detect_terminal_app(&self.config_dir)
    }
}

/// The lookup a window gets: the real one, or none under `cfg(test)` (a test
/// never starts a process on the machine) or without a config directory.
pub(crate) fn default_lookup() -> Option<Arc<dyn TerminalAppLookup>> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        let config_dir = kimun_notes::settings::config_dir().ok()?;
        Some(Arc::new(SystemLookup { config_dir }))
    }
}

/// The window's terminal app row: how it is looked up, what Preferences shows
/// now, and the lookup under way.
pub(crate) struct TerminalAppHost {
    lookup: Option<Arc<dyn TerminalAppLookup>>,
    row: TerminalAppRow,
    task: Option<Task<()>>,
}

impl TerminalAppHost {
    pub(crate) fn new(lookup: Option<Arc<dyn TerminalAppLookup>>) -> Self {
        Self {
            lookup,
            row: TerminalAppRow::Hidden,
            task: None,
        }
    }
}

impl Workspace {
    /// Preferences opened: look for the terminal app, off the UI thread. A
    /// lookup still running from an earlier open is dropped. With no lookup
    /// (see [`default_lookup`]) the row stays hidden. Needs the tokio runtime
    /// entered.
    pub(crate) fn start_terminal_app_lookup(&mut self, cx: &mut Context<Self>) {
        let Some(lookup) = self.terminal_app.lookup.clone() else {
            self.terminal_app.row = TerminalAppRow::Hidden;
            self.push_terminal_app_row(cx);
            return;
        };
        self.terminal_app.row = TerminalAppRow::Checking;
        self.push_terminal_app_row(cx);
        let job = blocking(move || lookup.find());
        self.terminal_app.task = Some(cx.spawn(async move |this, cx| {
            let found = job.await;
            this.update(cx, |w, cx| w.terminal_app_landed(found, cx))
                .ok();
        }));
    }

    fn terminal_app_landed(&mut self, found: Result<TerminalApp, String>, cx: &mut Context<Self>) {
        self.terminal_app.task = None;
        self.terminal_app.row = match found {
            Ok(app) => TerminalAppRow::Found(app),
            // The lookup itself failed (its task panicked): say nothing rather
            // than claim the terminal app is missing.
            Err(e) => {
                tracing::debug!("the terminal app lookup failed: {e}");
                TerminalAppRow::Hidden
            }
        };
        self.push_terminal_app_row(cx);
    }

    /// Preferences closed: no lookup is left running, and the row is gone.
    pub(super) fn stop_terminal_app_lookup(&mut self) {
        self.terminal_app.task = None;
        self.terminal_app.row = TerminalAppRow::Hidden;
    }

    /// Tell an open Preferences what to show about the terminal app.
    fn push_terminal_app_row(&mut self, cx: &mut Context<Self>) {
        let row = self.terminal_app.row.clone();
        if let Some(view) = self.preferences.clone() {
            view.update(cx, |view, cx| view.set_terminal_app(row, cx));
        }
    }

    /// Replace the terminal app lookup (tests).
    #[cfg(test)]
    pub(crate) fn set_terminal_app_lookup(&mut self, lookup: Arc<dyn TerminalAppLookup>) {
        self.terminal_app.lookup = Some(lookup);
    }
}
