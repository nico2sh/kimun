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

/// Run `job` on tokio's blocking pool, from the GPUI side: its result, or
/// the task's failure as text. The job starts when this is called, not when
/// the future is first polled, so the runtime must be entered at the call.
///
/// The blocking task is spawned from a runtime task: the runtime wakes it
/// itself, so the GPUI executor polling the returned future is not needed
/// for the job to make progress.
pub(crate) fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = Result<T, String>> + 'static {
    let handle = tokio::spawn(async move { tokio::task::spawn_blocking(job).await });
    async move {
        match handle.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) | Err(e) => Err(e.to_string()),
        }
    }
}
