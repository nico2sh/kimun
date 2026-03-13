pub mod editor;
pub mod settings;
pub mod start;

use async_trait::async_trait;
use ratatui::Frame;

use crate::components::app_message::AppTx;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;

#[async_trait]
pub trait AppScreen: Send {
    /// Called once when the screen mounts. Send `AppMessage`s through `tx` to
    /// trigger navigation (e.g. `StartScreen` checking whether a vault exists).
    async fn on_enter(&mut self, _tx: &AppTx) {}

    /// Handle an event. Send messages through `tx` for navigation or quit.
    /// Returns whether this screen consumed the event.
    fn handle_event(&mut self, event: AppEvent, tx: &AppTx) -> EventState;

    fn render(&mut self, f: &mut Frame);
}
