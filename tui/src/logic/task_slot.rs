//! [`TaskSlot`]: the one background task a UI owns and replaces (the kimün
//! server's sync loop, in the TUI and the GUI alike). Tokio only, no UI crate.

use tokio::task::JoinHandle;

/// At most one long-running tokio task, owned: starting another aborts the
/// one before it, and dropping the slot aborts the task it holds. One per
/// vault per process for the kimün server's sync loop: two would replace
/// each other's index observer.
#[derive(Default)]
pub struct TaskSlot(Option<JoinHandle<()>>);

impl TaskSlot {
    /// Hold `task` (none: hold nothing), aborting the task held before.
    pub fn replace(&mut self, task: Option<JoinHandle<()>>) {
        self.stop();
        self.0 = task;
    }

    /// Abort the task held, if any.
    pub fn stop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }

    /// Whether a task is held.
    pub fn is_running(&self) -> bool {
        self.0.is_some()
    }
}

impl Drop for TaskSlot {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::TaskSlot;

    /// One task at a time: a second replaces (aborts) the first, and the
    /// slot going away aborts the second.
    #[test]
    fn a_slot_holds_one_task_and_aborts_on_replace_and_drop() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = rt.enter();
        let first = tokio::spawn(std::future::pending::<()>());
        let first_abort = first.abort_handle();
        let second = tokio::spawn(std::future::pending::<()>());
        let second_abort = second.abort_handle();
        let mut slot = TaskSlot::default();
        assert!(!slot.is_running());
        slot.replace(Some(first));
        assert!(slot.is_running());
        slot.replace(Some(second));
        rt.block_on(tokio::task::yield_now());
        assert!(first_abort.is_finished(), "the first task is aborted");
        assert!(!second_abort.is_finished(), "the second one runs");
        drop(slot);
        rt.block_on(tokio::task::yield_now());
        assert!(second_abort.is_finished(), "dropping the slot aborts it");
        // Replacing with nothing stops the task held.
        let third = tokio::spawn(std::future::pending::<()>());
        let third_abort = third.abort_handle();
        let mut slot = TaskSlot::default();
        slot.replace(Some(third));
        slot.replace(None);
        rt.block_on(tokio::task::yield_now());
        assert!(third_abort.is_finished());
        assert!(!slot.is_running());
    }
}
