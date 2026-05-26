use crossterm::event::Event as CrosstermEvent;
use crossterm::event::{EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;

use crate::components::events::{AppEvent, AppTx, InputEvent};

/// Owns the app-message channel and the crossterm stream.
/// Exposes a single `next()` await point for the main loop.
///
/// Crossterm events are read directly from the stream — no relay task or extra
/// channel allocation per keypress. App-level messages go through a channel
/// because they originate from spawned async tasks (autosave, indexing, etc.).
/// The `biased` select drains queued app messages before reading more input,
/// which keeps message bursts responsive.
pub struct EventHandler {
    tx: AppTx,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    crossterm_stream: EventStream,
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHandler {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            crossterm_stream: EventStream::new(),
        }
    }

    /// Returns a cloned sender. Pass this to screens and components as `&AppTx`.
    pub fn app_sender(&self) -> AppTx {
        self.tx.clone()
    }

    /// Non-blocking peek of the app-message channel. Returns `None` when no
    /// app message is immediately pending. Crossterm input is NOT polled here
    /// — those events come through the stream and are only delivered via the
    /// blocking `next()` await point.
    ///
    /// The main loop uses this to coalesce queued events (e.g. multiple
    /// `Redraw` messages that pile up while a long-running async task was
    /// firing them) between blocking awaits.
    ///
    /// `Disconnected` is treated as a programming error: the run loop owns
    /// an `AppTx`, so every sender clone reaching zero means the loop's
    /// own copy was dropped — impossible under the current architecture.
    /// Loudly panicking here flags any future refactor that accidentally
    /// hands the loop's `AppTx` to a screen instead of cloning, rather
    /// than silently spinning on a dead channel.
    pub fn try_next(&mut self) -> Option<AppEvent> {
        match self.rx.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                panic!("AppEvent channel disconnected — the run loop's AppTx was dropped")
            }
        }
    }

    /// Wait for the next event. App messages are drained first (`biased`), then
    /// crossterm input is read directly from the stream.
    pub async fn next(&mut self) -> AppEvent {
        loop {
            tokio::select! {
                biased;
                Some(msg) = self.rx.recv() => return msg,
                Some(Ok(event)) = self.crossterm_stream.next() => {
                    tracing::debug!("RAW EVENT: {:?}", event);
                    match event {
                        CrosstermEvent::Key(key) if key.kind != KeyEventKind::Release => {
                            return AppEvent::Input(InputEvent::Key(key));
                        }
                        CrosstermEvent::Mouse(mouse) => return AppEvent::Input(InputEvent::Mouse(mouse)),
                        CrosstermEvent::Paste(s) => return AppEvent::Input(InputEvent::Paste(s)),
                        CrosstermEvent::Resize(_, _) => return AppEvent::Redraw,
                        _ => continue,
                    }
                }
            }
        }
    }
}
