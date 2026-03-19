use std::time::Duration;

use ratatui::layout::Rect;

use crate::components::events::{AppEvent, AppTx};

pub enum IndexingProgressState {
    Running {
        work: tokio::task::JoinHandle<()>,
        ticker: tokio::task::JoinHandle<()>,
    },
    Done(Duration),
    Failed(String),
}

impl Drop for IndexingProgressState {
    fn drop(&mut self) {
        if let Self::Running { work, ticker } = self {
            work.abort();
            ticker.abort();
        }
    }
}

pub fn spawn_running(work: tokio::task::JoinHandle<()>, tx: &AppTx) -> IndexingProgressState {
    let tx2 = tx.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if tx2.send(AppEvent::Redraw).is_err() {
                break;
            }
        }
    });
    IndexingProgressState::Running { work, ticker }
}

pub fn fixed_centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    let x = r.x + (r.width.saturating_sub(width)) / 2;
    let y = r.y + (r.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(r.width),
        height: height.min(r.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[tokio::test]
    async fn drop_aborts_running_tasks() {
        let completed = Arc::new(AtomicBool::new(false));
        let completed2 = completed.clone();

        let work = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            completed2.store(true, Ordering::SeqCst);
        });
        let ticker = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });

        let state = IndexingProgressState::Running { work, ticker };
        drop(state);

        // Yield several times: abort() is cooperative, the task needs at least one
        // poll after cancellation is posted before it is marked finished.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        assert!(
            !completed.load(Ordering::SeqCst),
            "work task should be aborted, not completed"
        );
    }
}
