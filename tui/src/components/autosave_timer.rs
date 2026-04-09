use tokio::task::JoinHandle;

use crate::components::events::{AppEvent, AppTx};

pub struct AutosaveTimer {
    handle: Option<JoinHandle<()>>,
}

impl Default for AutosaveTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl AutosaveTimer {
    pub fn new() -> Self {
        Self { handle: None }
    }

    /// Abort any running timer and start a new one that sends `AppEvent::Autosave`
    /// every `interval_secs` seconds (skipping the first tick).
    pub fn restart(&mut self, interval_secs: u64, tx: AppTx) {
        self.stop();
        self.handle = Some(tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                if tx.send(AppEvent::Autosave).is_err() {
                    break;
                }
            }
        }));
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
