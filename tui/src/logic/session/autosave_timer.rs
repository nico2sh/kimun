use tokio::task::JoinHandle;

use crate::logic::message::{AppMessage, AppTx};

pub struct AutosaveTimer {
    handle: Option<JoinHandle<()>>,
    /// The period of the running tick, in seconds.
    interval_secs: u64,
}

impl Default for AutosaveTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl AutosaveTimer {
    pub fn new() -> Self {
        Self {
            handle: None,
            interval_secs: 0,
        }
    }

    /// Abort any running timer and start a new one that sends `AppMessage::Autosave`
    /// every `interval_secs` seconds (skipping the first tick).
    pub fn restart(&mut self, interval_secs: u64, tx: AppTx) {
        self.stop();
        self.interval_secs = interval_secs;
        self.handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                if tx.send(AppMessage::Autosave).is_err() {
                    break;
                }
            }
        }));
    }

    /// Whether a tick task is running.
    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// The period of the running tick, `None` while none runs.
    pub fn interval_secs(&self) -> Option<u64> {
        self.is_running().then_some(self.interval_secs)
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl Drop for AutosaveTimer {
    fn drop(&mut self) {
        self.stop();
    }
}
