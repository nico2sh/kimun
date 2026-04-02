use crossterm::event::Event as CrosstermEvent;
use crossterm::event::{EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc;

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

    /// Wait for the next event. App messages are drained first (`biased`), then
    /// crossterm input is read directly from the stream.
    pub async fn next(&mut self) -> AppEvent {
        loop {
            tokio::select! {
                biased;
                Some(msg) = self.rx.recv() => return msg,
                Some(Ok(event)) = self.crossterm_stream.next() => {
                    log::debug!("RAW EVENT: {:?}", event);
                    match event {
                        CrosstermEvent::Key(key) if key.kind != KeyEventKind::Release => {
                            return AppEvent::Input(InputEvent::Key(key));
                        }
                        CrosstermEvent::Mouse(mouse) => return AppEvent::Input(InputEvent::Mouse(mouse)),
                        CrosstermEvent::Resize(_, _) => return AppEvent::Redraw,
                        _ => continue,
                    }
                }
            }
        }
    }
}
