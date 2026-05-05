pub mod autosave_timer;
pub mod backlinks_panel;
pub mod dialog_manager;
pub mod dialogs;
pub mod event_state;
pub mod events;
pub mod file_list;
pub mod footer_bar;
pub mod indexing;
pub mod note_browser;
pub mod settings;
pub mod sidebar;
pub mod text_editor;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::themes::Theme;

pub trait Component {
    /// Handle an event. Send `AppEvent`s through `tx` for app-level effects.
    /// Returns whether this component consumed the event.
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let _ = (event, tx);
        EventState::NotConsumed
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool);

    /// Context-sensitive shortcut hints shown in the hints bar when this
    /// component is focused.  Each entry is `(key_display, label)`.
    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![]
    }
}
