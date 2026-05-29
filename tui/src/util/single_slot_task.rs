//! Single-slot cancellable tokio task.
//!
//! Encodes the "one task at a time; aborting an old one cancels its
//! spawned future" pattern that the autocomplete controller and the
//! editor's autosave both reinvent. Drop aborts — so the spawned
//! future cannot outlive the parent that owned the slot.
//!
//! ```ignore
//! let mut slot: SingleSlotTask<String> = SingleSlotTask::empty();
//! slot.spawn(async { "hello".to_string() });
//! // Spawning again aborts the prior task.
//! slot.spawn(async { "world".to_string() });
//! ```
//!
//! `JoinHandle` alone does NOT cancel its task on drop — it merely
//! detaches. This type holds an `AbortHandle` alongside and cancels
//! through it in `abort()` and `Drop`.

use std::future::Future;
use std::time::Duration;
use tokio::task::{AbortHandle, JoinError, JoinHandle};

pub struct SingleSlotTask<T> {
    handle: Option<JoinHandle<T>>,
    abort: Option<AbortHandle>,
}

impl<T> SingleSlotTask<T> {
    pub fn empty() -> Self {
        Self {
            handle: None,
            abort: None,
        }
    }

    /// True between `spawn` and the moment the task's future finishes
    /// (cancellation OR normal return). Cheap; no awaiting.
    pub fn is_in_flight(&self) -> bool {
        self.handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    /// Abort the in-flight task (if any) and clear the slot. No-op on
    /// an empty slot. `abort()` is asynchronous — the spawned future
    /// is cancelled at its next await point, so a result already
    /// sent on a channel may still arrive at its consumer. Consumers
    /// are responsible for filtering stale results (e.g. via revision
    /// match).
    pub fn abort(&mut self) {
        if let Some(h) = self.abort.take() {
            h.abort();
        }
        self.handle = None;
    }
}

impl<T: Send + 'static> SingleSlotTask<T> {
    /// Aborts any in-flight task, then spawns `fut`. Returns the new
    /// task's `AbortHandle` so callers that want to cancel from
    /// somewhere other than the slot's owner (e.g. an event handler
    /// holding a clone) can do so. Usually ignored.
    pub fn spawn<F>(&mut self, fut: F) -> AbortHandle
    where
        F: Future<Output = T> + Send + 'static,
    {
        if let Some(prev) = self.abort.take() {
            prev.abort();
        }
        let handle = tokio::spawn(fut);
        let abort_handle = handle.abort_handle();
        self.abort = Some(abort_handle.clone());
        self.handle = Some(handle);
        abort_handle
    }

    /// `Some(result)` if the task completed within the deadline.
    /// `None` if the deadline expired — the slot keeps the handle so
    /// the caller can decide whether to `abort()` explicitly. The
    /// returned `Result` is `JoinHandle`'s own result type, so a
    /// panic in the task surfaces as `Err(JoinError)`.
    pub async fn await_with_timeout(&mut self, dur: Duration) -> Option<Result<T, JoinError>> {
        let handle = self.handle.as_mut()?;
        match tokio::time::timeout(dur, handle).await {
            Ok(res) => {
                // Task completed — clear the slot.
                self.handle = None;
                self.abort = None;
                Some(res)
            }
            Err(_) => None,
        }
    }
}

impl<T> Drop for SingleSlotTask<T> {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    #[tokio::test]
    async fn empty_slot_reports_idle() {
        let slot: SingleSlotTask<()> = SingleSlotTask::empty();
        assert!(!slot.is_in_flight());
    }

    #[tokio::test]
    async fn spawn_runs_to_completion() {
        let mut slot: SingleSlotTask<u32> = SingleSlotTask::empty();
        slot.spawn(async { 42 });
        let out = slot.await_with_timeout(Duration::from_secs(1)).await;
        assert_eq!(out.expect("must complete").expect("no panic"), 42);
        assert!(!slot.is_in_flight());
    }

    #[tokio::test]
    async fn single_slot_task_spawn_aborts_previous() {
        let canary = Arc::new(AtomicBool::new(false));
        let mut slot: SingleSlotTask<()> = SingleSlotTask::empty();

        let canary_clone = canary.clone();
        slot.spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            canary_clone.store(true, Ordering::SeqCst);
        });

        // Replace with a fast task. The 60s sleeper must NEVER run to
        // completion, so the canary stays false.
        slot.spawn(async {});
        let _ = slot.await_with_timeout(Duration::from_secs(1)).await;

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !canary.load(Ordering::SeqCst),
            "previous task must have been aborted"
        );
    }

    #[tokio::test]
    async fn single_slot_task_drop_aborts_in_flight() {
        let canary = Arc::new(AtomicBool::new(false));
        {
            let mut slot: SingleSlotTask<()> = SingleSlotTask::empty();
            let canary_clone = canary.clone();
            slot.spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                canary_clone.store(true, Ordering::SeqCst);
            });
            // slot drops here.
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !canary.load(Ordering::SeqCst),
            "drop must abort the spawned task"
        );
    }

    #[tokio::test]
    async fn single_slot_task_timeout_returns_none_keeps_handle() {
        let mut slot: SingleSlotTask<()> = SingleSlotTask::empty();
        slot.spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        let out = slot.await_with_timeout(Duration::from_millis(50)).await;
        assert!(out.is_none(), "long task must time out");
        assert!(
            slot.is_in_flight(),
            "handle should survive a timeout so caller can decide to abort"
        );
        // Explicit cleanup.
        slot.abort();
        assert!(!slot.is_in_flight());
    }

    #[tokio::test]
    async fn explicit_abort_clears_slot() {
        let mut slot: SingleSlotTask<()> = SingleSlotTask::empty();
        let notify = Arc::new(Notify::new());
        let n = notify.clone();
        slot.spawn(async move {
            n.notified().await;
        });
        assert!(slot.is_in_flight());
        slot.abort();
        assert!(!slot.is_in_flight());
    }
}
