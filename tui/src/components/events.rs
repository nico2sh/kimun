//! Terminal input, the one part of the app's event vocabulary that names the
//! terminal. Everything else moved to [`crate::logic::message`] (ADR-0049 §4).

use ratatui::crossterm::event::{KeyEvent, MouseEvent};

#[derive(Debug, Clone)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Bracketed-paste payload from the terminal. On macOS this is what
    /// Cmd+V delivers, since the terminal intercepts Cmd combos before they
    /// reach the TUI. The string may be empty when the clipboard holds only
    /// non-text content (e.g. an image).
    Paste(String),
}
