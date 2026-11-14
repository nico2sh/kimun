use tokio::task::JoinHandle;

use crate::logic::message::{AppMessage, AppTx, TickId};

pub struct AutosaveTimer {
    handle: Option<JoinHandle<()>>,
    /// The period of the running tick, in seconds.
    interval_secs: u64,
    /// What each tick names itself as ([`AppMessage::AutosaveOf`]).
    id: TickId,
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
            id: TickId::fresh(),
        }
    }

    /// What this timer's ticks name themselves as, the same across restarts.
    pub fn id(&self) -> TickId {
        self.id
    }

    /// Abort any running timer and start a new one that sends
    /// `AppMessage::AutosaveOf(self.id())` every `interval_secs` seconds
    /// (skipping the first tick).
    pub fn restart(&mut self, interval_secs: u64, tx: AppTx) {
        self.stop();
        self.interval_secs = interval_secs;
        let id = self.id;
        self.handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                if tx.send(AppMessage::AutosaveOf(id)).is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_tick_names_its_timer() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut timer = AutosaveTimer::new();
        assert_ne!(timer.id(), AutosaveTimer::new().id(), "one id per timer");
        timer.restart(1, tx);
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a tick within its period")
            .expect("the timer still runs");
        assert!(
            matches!(message, AppMessage::AutosaveOf(id) if id == timer.id()),
            "{message:?}"
        );
    }
}
