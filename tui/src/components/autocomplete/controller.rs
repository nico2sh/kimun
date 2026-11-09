//! The TUI's adapter over the toolkit-free [`AutocompleteEngine`]: it maps
//! crossterm keys and clicks onto [`PopupCommand`](crate::logic::autocomplete::PopupCommand)s, remembers where the popup
//! rows were last drawn (what a click is hit-tested against), and draws the
//! popup. Every rule — trigger detection, query debounce, staleness, accept
//! edit — lives in [`crate::logic::autocomplete`].

use std::sync::Arc;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

use super::popup::{self, key_command, mouse_command};
use crate::logic::autocomplete::{
    AutocompleteEngine, AutocompleteHost, AutocompleteMode, AutocompleteState, HandleKeyOutcome,
    RedrawCallback, SuggestionSource, TriggerOptions,
};
use crate::settings::themes::Theme;

pub struct AutocompleteController {
    engine: AutocompleteEngine,
    /// Where the visible rows were drawn by the last render (inside the
    /// border). Empty until rendered, and whenever the popup could not draw.
    rows_rect: Rect,
}

impl AutocompleteController {
    pub fn new(suggestions: Arc<dyn SuggestionSource>, mode: AutocompleteMode) -> Self {
        Self {
            engine: AutocompleteEngine::new(suggestions, mode),
            rows_rect: Rect::default(),
        }
    }

    pub fn with_trigger_opts(mut self, opts: TriggerOptions) -> Self {
        self.engine = self.engine.with_trigger_opts(opts);
        self
    }

    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.engine = self.engine.with_debounce(debounce);
        self
    }

    pub fn set_redraw_callback(&mut self, cb: RedrawCallback) {
        self.engine.set_redraw_callback(cb);
    }

    pub fn is_open(&self) -> bool {
        self.engine.is_open()
    }

    pub fn state(&self) -> Option<&AutocompleteState> {
        self.engine.state()
    }

    /// See [`AutocompleteEngine::state_mut`]: the render path re-anchors on
    /// the freshly drawn caret.
    pub fn state_mut(&mut self) -> Option<&mut AutocompleteState> {
        self.engine.state_mut()
    }

    /// Where the popup rows were last drawn; empty when not on screen.
    pub fn rows_rect(&self) -> Rect {
        self.rows_rect
    }

    /// Forget the drawn rows: the popup was not drawn this frame, so nothing
    /// of it may be clickable.
    pub fn clear_rows_rect(&mut self) {
        self.rows_rect = Rect::default();
    }

    /// Draw the popup over `screen` at its current anchor and record where
    /// its rows landed.
    pub fn render(&mut self, frame: &mut Frame, screen: Rect, theme: &Theme) {
        self.rows_rect = match self.engine.state() {
            Some(state) => popup::render(frame, state, screen, theme),
            None => Rect::default(),
        };
    }

    pub fn close(&mut self) {
        self.engine.close();
    }

    pub fn poll_results(&mut self) {
        self.engine.poll_results();
    }

    pub fn sync<H: AutocompleteHost>(&mut self, host: &H) {
        self.engine.sync(host);
    }

    pub fn refresh_if_open<H: AutocompleteHost>(&mut self, host: &H) {
        self.engine.refresh_if_open(host);
    }

    /// Route a key through the popup when one is open. `NotHandled` for keys
    /// that are not the popup's (typing, system-modifier combos).
    pub fn handle_key<H: AutocompleteHost>(&mut self, key: KeyEvent, host: &H) -> HandleKeyOutcome {
        match key_command(key) {
            Some(command) => self.engine.handle_command(command, host),
            None => HandleKeyOutcome::NotHandled,
        }
    }

    /// Route a mouse event through the popup: a click on a suggestion accepts
    /// it, the wheel over it moves the highlight. `NotHandled` for anything
    /// that missed the popup, so the host's own mouse handling runs.
    pub fn handle_mouse<H: AutocompleteHost>(
        &mut self,
        m: &MouseEvent,
        host: &H,
    ) -> HandleKeyOutcome {
        match mouse_command(m, self.rows_rect) {
            Some(command) => self.engine.handle_command(command, host),
            None => HandleKeyOutcome::NotHandled,
        }
    }
}
