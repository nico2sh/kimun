//! The **Input source** seam (CONTEXT.md § App shell), merged with the app
//! channel into the one `next()` the **App loop** awaits.
//!
//! Two kinds of event reach the loop. Terminal-originated ones — key, mouse,
//! paste, resize — come from the input source: crossterm's `EventStream` in
//! the app, any `Stream<Item = AppEvent>` in tests. App-originated ones —
//! autosave done, indexing done, a server answer — come from spawned tasks
//! through the app channel. The loop needs to tell them apart: app messages
//! are peeked without blocking and coalesced into one frame; a real input
//! event always forces a fresh await and its own draw. That is why the seam
//! is the input *stream* and not one merged stream handed to the loop.
//!
//! The input source ends only when the terminal is gone (or a script ran
//! out), and the loop treats its end as quit.

use std::io;
use std::pin::Pin;

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEventKind};
use futures::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;

use crate::app::ctrl_h::CtrlHPolicy;
use crate::components::events::{AppEvent, AppTx, InputEvent};

/// Terminal-originated events, already decoded. Boxed rather than generic so
/// the loop, the app and `main` carry no type parameter for it (the same line
/// ADR-0009 draws for the editor backend).
pub type InputSource = Pin<Box<dyn Stream<Item = AppEvent> + Send>>;

/// Owns the app channel and the input source. Exposes a single `next()`
/// await point for the loop.
pub struct EventHandler {
    tx: AppTx,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    input: InputSource,
}

impl EventHandler {
    /// The app's handler: crossterm reads the terminal. `ctrl_h` is the
    /// session's resolved Ctrl-H rule, applied as events are decoded — no
    /// `Default`, because guessing that rule is exactly what this type must
    /// not do.
    pub fn new(ctrl_h: CtrlHPolicy) -> Self {
        Self::from_input(crossterm_input(ctrl_h))
    }

    /// A handler over any input source — a `futures::stream::iter` of scripted
    /// events in tests, a replayed recording, anything that yields `AppEvent`.
    /// The stream is fused here, so an adapter need not be.
    pub fn from_input(input: impl Stream<Item = AppEvent> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            input: Box::pin(input.fuse()),
        }
    }

    /// Returns a cloned sender. Pass this to screens and components as `&AppTx`.
    pub fn app_sender(&self) -> AppTx {
        self.tx.clone()
    }

    /// Non-blocking peek of the app channel only. Input is never polled here:
    /// the loop coalesces queued app messages between blocking awaits, and a
    /// real input event must always get its own `next()` and its own draw.
    ///
    /// `Disconnected` is structurally unreachable: `self.tx` is owned by this
    /// handler and live across the `&mut self` borrow, so at least one sender
    /// always exists while `try_next` runs.
    pub fn try_next(&mut self) -> Option<AppEvent> {
        match self.rx.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                unreachable!(
                    "EventHandler::tx is owned by this struct and the `&mut self` borrow \
                     guarantees it outlives this call; channel cannot be Disconnected here"
                )
            }
        }
    }

    /// Wait for the next event. App messages first (`biased`), then input.
    /// An exhausted input source yields `Quit`, every time it is asked.
    pub async fn next(&mut self) -> AppEvent {
        tokio::select! {
            biased;
            Some(msg) = self.rx.recv() => msg,
            event = self.input.next() => event.unwrap_or(AppEvent::Quit),
        }
    }
}

/// The crossterm adapter: the terminal's event stream, decoded.
fn crossterm_input(ctrl_h: CtrlHPolicy) -> impl Stream<Item = AppEvent> + Send {
    EventStream::new().filter_map(move |event| {
        tracing::debug!("RAW EVENT: {:?}", event);
        futures::future::ready(decode(event, ctrl_h))
    })
}

/// What one crossterm event means to the loop. Key releases are dropped (the
/// kitty protocol reports them; the app acts on presses), a resize is a
/// redraw, focus and unknown events are nothing, and a read error is logged
/// and skipped rather than ending the source.
///
/// This is also where `ctrl_h` is applied. The rewrite belongs here, at the
/// one place every terminal key enters the app, rather than in the shortcut
/// tier or a backend: both of those would have to agree about it, and a
/// Backspace the editor sees as a chord is the same bug either way.
pub(crate) fn decode(event: io::Result<CrosstermEvent>, ctrl_h: CtrlHPolicy) -> Option<AppEvent> {
    match event {
        Ok(CrosstermEvent::Key(key)) if key.kind != KeyEventKind::Release => {
            Some(AppEvent::Input(InputEvent::Key(ctrl_h.apply(key))))
        }
        Ok(CrosstermEvent::Mouse(mouse)) => Some(AppEvent::Input(InputEvent::Mouse(mouse))),
        Ok(CrosstermEvent::Paste(text)) => Some(AppEvent::Input(InputEvent::Paste(text))),
        Ok(CrosstermEvent::Resize(_, _)) => Some(AppEvent::Redraw),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("terminal input error: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use ratatui::crossterm::event::{
        Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
    };

    fn key(code: KeyCode, kind: KeyEventKind) -> CrosstermEvent {
        CrosstermEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind,
            state: KeyEventState::NONE,
        })
    }

    /// The rule that changes nothing, for the tests that are not about it.
    const CHORD: CtrlHPolicy = CtrlHPolicy::Chord;

    #[test]
    fn decode_keeps_presses_and_drops_releases() {
        assert!(matches!(
            decode(Ok(key(KeyCode::Char('a'), KeyEventKind::Press)), CHORD),
            Some(AppEvent::Input(InputEvent::Key(k))) if k.code == KeyCode::Char('a')
        ));
        assert!(decode(Ok(key(KeyCode::Char('a'), KeyEventKind::Release)), CHORD).is_none());
    }

    #[test]
    fn decode_turns_a_resize_into_a_redraw_and_skips_errors() {
        assert!(matches!(
            decode(Ok(CrosstermEvent::Resize(80, 24)), CHORD),
            Some(AppEvent::Redraw)
        ));
        assert!(decode(Err(std::io::Error::other("hangup")), CHORD).is_none());
    }

    /// The reported bug: a terminal whose Backspace key sends `0x08` reaches
    /// crossterm as Ctrl-H, which the shortcut tier claims as a binding
    /// (`FocusSidebar`, by default) before the editor ever sees it. Under the
    /// Backspace rule the seam hands the loop a Backspace instead.
    #[test]
    fn decode_applies_the_ctrl_h_rule() {
        let ctrl_h = CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
        assert!(matches!(
            decode(Ok(ctrl_h.clone()), CtrlHPolicy::Backspace),
            Some(AppEvent::Input(InputEvent::Key(k)))
                if k.code == KeyCode::Backspace && k.modifiers == KeyModifiers::NONE
        ));
        assert!(matches!(
            decode(Ok(ctrl_h), CHORD),
            Some(AppEvent::Input(InputEvent::Key(k)))
                if k.code == KeyCode::Char('h') && k.modifiers == KeyModifiers::CONTROL
        ));
    }

    /// A release still drops under the rewrite: the rule decides *which* key
    /// an event is, never whether the loop hears about it.
    #[test]
    fn the_ctrl_h_rule_does_not_revive_releases() {
        let release = CrosstermEvent::Key(KeyEvent::new_with_kind(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        ));
        assert!(decode(Ok(release), CtrlHPolicy::Backspace).is_none());
    }

    #[tokio::test]
    async fn a_scripted_input_source_is_delivered_in_order() {
        let mut events = EventHandler::from_input(stream::iter([
            AppEvent::Redraw,
            AppEvent::Input(InputEvent::Paste("p".into())),
        ]));
        assert!(matches!(events.next().await, AppEvent::Redraw));
        assert!(matches!(
            events.next().await,
            AppEvent::Input(InputEvent::Paste(s)) if s == "p"
        ));
    }

    #[tokio::test]
    async fn app_messages_are_drained_before_input() {
        let mut events = EventHandler::from_input(stream::iter([AppEvent::Redraw]));
        events.app_sender().send(AppEvent::Quit).unwrap();
        // Biased: the queued app message wins even though input is ready.
        assert!(matches!(events.next().await, AppEvent::Quit));
        assert!(matches!(events.next().await, AppEvent::Redraw));
    }

    #[tokio::test]
    async fn an_exhausted_input_source_yields_quit() {
        let mut events = EventHandler::from_input(stream::empty());
        assert!(matches!(events.next().await, AppEvent::Quit));
        // …and keeps yielding it: the loop may ask once more on its way out.
        assert!(matches!(events.next().await, AppEvent::Quit));
    }

    #[test]
    fn try_next_only_peeks_the_app_channel() {
        let mut events = EventHandler::from_input(stream::iter([AppEvent::Redraw]));
        assert!(events.try_next().is_none());
        events.app_sender().send(AppEvent::Redraw).unwrap();
        assert!(matches!(events.try_next(), Some(AppEvent::Redraw)));
    }
}
