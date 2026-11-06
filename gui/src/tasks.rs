//! Tokio tasks seen from the GPUI side: the vault's reads and writes run on
//! tokio, off the UI thread, and their results come back here.

use tokio::task::JoinHandle;

/// A tokio task's result with its error as text: the task's own failure,
/// or the task panicking or being cancelled (the
/// [`JoinError`](tokio::task::JoinError)'s text).
pub(crate) async fn joined<T, E: ToString>(task: JoinHandle<Result<T, E>>) -> Result<T, String> {
    match task.await {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}
