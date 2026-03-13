use ratatui::crossterm::event::{KeyEvent, MouseEvent};

/// All events that flow through the system — both input events (from crossterm)
/// and app-level events emitted by components to communicate with each other.
#[derive(Debug, Clone)]
pub enum AppEvent {
    // ── Input events ────────────────────────────────────────────────────────
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    // ── Component-to-component events (add here as the app grows) ───────────
    // e.g. NoteSelected(VaultPath), SearchChanged(String), …
}
