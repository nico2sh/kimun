pub mod browse;
pub mod editor;
pub mod settings;
pub mod start;

use async_trait::async_trait;
use ratatui::Frame;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Start,
    Browse,
    Editor,
    Settings,
}

#[async_trait]
pub trait AppScreen: Send {
    /// Called once when the screen mounts. Send `AppEvent`s through `tx` to
    /// trigger navigation (e.g. `StartScreen` checking whether a vault exists).
    async fn on_enter(&mut self, _tx: &AppTx) {}

    /// Handle an input event. Send events through `tx` for navigation or quit.
    /// Returns whether this screen consumed the event.
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState;

    fn render(&mut self, f: &mut Frame);

    /// Handle an application-level event. Return `None` if the screen consumed
    /// the event, or `Some(event)` to pass it back to the main loop.
    /// The default implementation forwards everything (screen doesn't handle it).
    async fn handle_app_message(&mut self, msg: AppEvent, _tx: &AppTx) -> Option<AppEvent> {
        Some(msg)
    }

    fn get_kind(&self) -> ScreenKind;

    /// Called once just before the screen is removed from the app (quit or screen transition).
    /// Default implementation is a no-op.
    async fn on_exit(&mut self, _tx: &AppTx) {}
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use std::sync::{Arc, RwLock};

    use super::*;
    use crate::app_screen::settings::SettingsScreen;
    use crate::settings::AppSettings;

    fn shared_defaults() -> crate::settings::SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    #[tokio::test]
    async fn non_editor_screen_passes_focus_message_back() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut screen = SettingsScreen::new(shared_defaults());
        let result = screen.handle_app_message(AppEvent::FocusSidebar, &tx).await;
        assert!(
            result.is_some(),
            "SettingsScreen should not consume FocusSidebar"
        );
    }

    #[tokio::test]
    async fn non_editor_screen_passes_focus_editor_message_back() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut screen = SettingsScreen::new(shared_defaults());
        let result = screen.handle_app_message(AppEvent::FocusEditor, &tx).await;
        assert!(
            result.is_some(),
            "SettingsScreen should not consume FocusEditor"
        );
    }

    #[tokio::test]
    async fn on_exit_default_is_noop() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut screen = SettingsScreen::new(shared_defaults());
        screen.on_exit(&tx).await; // must compile and not panic
    }
}
