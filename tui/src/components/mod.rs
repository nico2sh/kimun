pub mod autocomplete;
pub mod autosave_timer;
pub mod backlinks_panel;
pub mod dialogs;
pub mod event_state;
pub mod events;
pub mod file_list;
pub mod footer_bar;
pub mod indexing;
pub mod note_browser;
pub mod overlay;
pub mod panel;
pub mod query_vars;
pub mod saved_search_breadcrumb;
pub mod saved_searches_modal;
pub mod search_list;
pub mod settings;
pub mod sidebar;
pub mod single_line_input;
pub mod text_editor;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::themes::Theme;

/// Centre a popup occupying `percent_x`% × `percent_y`% of `area`.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_height = (area.height as u32 * percent_y as u32 / 100) as u16;
    let popup_width = (area.width as u32 * percent_x as u32 / 100) as u16;
    Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let r = centered_rect(80, 75, area);
        assert_eq!(r.width, 80);
        assert_eq!(r.height, 30);
        assert_eq!(r.x, 10); // (100 - 80) / 2
        assert_eq!(r.y, 5); // (40 - 30) / 2
    }

    #[test]
    fn centered_rect_does_not_underflow() {
        // Very small area — must not panic.
        let area = Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        };
        let _ = centered_rect(80, 75, area);
    }
}
