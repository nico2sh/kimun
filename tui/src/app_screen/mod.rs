pub mod browse;
pub mod doc_meta;
pub mod editor;
pub mod onboarding;
pub mod overlay_host;
pub mod panel_set;
pub mod preferences;
pub mod start;

use async_trait::async_trait;
use ratatui::Frame;

use kimun_core::nfs::VaultPath;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Start,
    Browse,
    Editor,
    Onboarding,
    Preferences,
}

// `?Send`: screens own a `ratatui-textarea` `TextArea`, whose `Block` field is
// non-`Send` as of `ratatui-widgets 0.3.1` (its shadow effect holds an
// `Arc<dyn CellEffect>`). The screen is only ever driven on the main `block_on`
// future, never spawned, so dropping the `Send` bound is sound.
#[async_trait(?Send)]
pub trait AppScreen {
    /// Called once when the screen mounts. Send `AppEvent`s through `tx` to
    /// trigger navigation (e.g. `StartScreen` checking whether a vault exists).
    async fn on_enter(&mut self, _tx: &AppTx) {}

    /// Handle an input event. Send events through `tx` for navigation or quit.
    /// Returns whether this screen consumed the event.
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState;

    fn render(&mut self, f: &mut Frame);

    /// Handle an application-level event owned by this screen. Events the
    /// screen does not recognize are silently ignored. The default
    /// implementation handles nothing.
    async fn handle_app_message(&mut self, _msg: AppEvent, _tx: &AppTx) {}

    /// Try to open `path` within this screen (e.g. load a note into the
    /// buffer, or navigate a sidebar). Return `Some(path)` if the screen
    /// does not handle it, in which case the main loop switches to an
    /// appropriate screen. Default: not handled.
    /// `emphasis` carries the originating query's needles when the open came
    /// from a query result; only the editor screen uses it (spec §5.1) — it
    /// is dropped when the open reroutes to a screen switch.
    async fn try_open_path(
        &mut self,
        path: VaultPath,
        _emphasis: Option<Vec<String>>,
        _tx: &AppTx,
    ) -> Option<VaultPath> {
        Some(path)
    }

    /// Try to open the attachment at `path` within this screen (the editor
    /// screen shows it in its read-only attachment view; see ADR-0017). Return
    /// `Some(path)` if the screen does not handle it, in which case the main
    /// loop routes it to the editor screen. Default: not handled.
    async fn try_open_attachment(&mut self, path: VaultPath, _tx: &AppTx) -> Option<VaultPath> {
        Some(path)
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
    use crate::app_screen::preferences::PreferencesScreen;
    use crate::settings::AppSettings;

    fn shared_defaults() -> crate::settings::SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    #[tokio::test]
    async fn on_exit_default_is_noop() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut screen = PreferencesScreen::new(shared_defaults());
        screen.on_exit(&tx).await; // must compile and not panic
    }
}
