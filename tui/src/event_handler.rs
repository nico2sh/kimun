use crossterm::event::{Event as CrosstermEvent, EventStream};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::components::app_message::{AppMessage, AppTx};

/// The unified event type for the main loop.
pub enum TuiEvent {
    /// A raw terminal event (key press, mouse click, resize, …).
    Crossterm(CrosstermEvent),
    /// An application-level message sent by a screen or component.
    App(AppMessage),
}

/// Owns both the app-message channel and the crossterm reader task, and
/// exposes a single `next()` await point for the main loop.
pub struct EventHandler {
    app_tx: AppTx,
    app_rx: mpsc::UnboundedReceiver<AppMessage>,
    crossterm_rx: mpsc::Receiver<CrosstermEvent>,
}

impl EventHandler {
    pub fn new() -> Self {
        let (app_tx, app_rx) = mpsc::unbounded_channel();
        let (crossterm_tx, crossterm_rx) = mpsc::channel(256);

        tokio::spawn(async move {
            let mut stream = EventStream::new();
            while let Some(Ok(event)) = stream.next().await {
                if crossterm_tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        Self { app_tx, app_rx, crossterm_rx }
    }

    /// Returns a cloned sender for app messages. Pass this to screens and
    /// components as `&AppTx` — the type is unchanged from before.
    pub fn app_sender(&self) -> AppTx {
        self.app_tx.clone()
    }

    /// Wait for the next event. App messages are checked before crossterm
    /// events (`biased`) so that a burst of queued messages drains quickly
    /// without interleaving unnecessary terminal reads.
    pub async fn next(&mut self) -> TuiEvent {
        tokio::select! {
            biased;
            Some(msg) = self.app_rx.recv() => TuiEvent::App(msg),
            Some(event) = self.crossterm_rx.recv() => TuiEvent::Crossterm(event),
        }
    }
}
