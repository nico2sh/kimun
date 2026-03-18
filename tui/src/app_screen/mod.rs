pub mod browse;
pub mod editor;
pub mod settings;
pub mod start;

use async_trait::async_trait;
use ratatui::Frame;

use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;

#[async_trait]
pub trait AppScreen: Send {
    /// Called once when the screen mounts. Send `AppMessage`s through `tx` to
    /// trigger navigation (e.g. `StartScreen` checking whether a vault exists).
    async fn on_enter(&mut self, _tx: &AppTx) {}

    /// Handle an event. Send messages through `tx` for navigation or quit.
    /// Returns whether this screen consumed the event.
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState;

    fn render(&mut self, f: &mut Frame);

    /// Handle an application-level message. Return `None` if the screen consumed
    /// the message, or `Some(msg)` to pass it back to the main loop.
    /// The default implementation forwards everything (screen doesn't handle it).
    async fn handle_app_message(&mut self, msg: AppMessage, _tx: &AppTx) -> Option<AppMessage> {
        Some(msg)
    }

    /// Called once just before the screen is removed from the app (quit or screen transition).
    /// Default implementation is a no-op.
    async fn on_exit(&mut self, _tx: &AppTx) {}
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;
    use crate::app_screen::settings::SettingsScreen;
    use crate::components::app_message::AppMessage;
    use crate::settings::AppSettings;

    #[tokio::test]
    async fn non_editor_screen_passes_focus_message_back() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = SettingsScreen::new(AppSettings::default());
        let result = screen.handle_app_message(AppMessage::FocusSidebar, &tx).await;
        assert!(result.is_some(), "SettingsScreen should not consume FocusSidebar");
    }

    #[tokio::test]
    async fn non_editor_screen_passes_focus_editor_message_back() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = SettingsScreen::new(AppSettings::default());
        let result = screen.handle_app_message(AppMessage::FocusEditor, &tx).await;
        assert!(result.is_some(), "SettingsScreen should not consume FocusEditor");
    }

    #[tokio::test]
    async fn on_exit_default_is_noop() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = SettingsScreen::new(AppSettings::default());
        screen.on_exit(&tx).await; // must compile and not panic
    }
}
