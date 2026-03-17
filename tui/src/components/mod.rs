pub mod app_message;
pub mod event_state;
pub mod events;
pub mod file_list;
pub mod sidebar;
pub mod text_editor;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::app_message::AppTx;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::themes::Theme;

pub trait Component {
    /// Handle an event. Send `AppMessage`s through `tx` for app-level effects.
    /// Returns whether this component consumed the event.
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        let _ = (event, tx);
        EventState::NotConsumed
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme);
}
