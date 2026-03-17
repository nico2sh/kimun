use ratatui::Frame;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;

pub struct TextEditorComponent {
    text_area: TextArea<'static>,
    /// Tracks the rendered rect to map mouse click coordinates.
    rect: Rect,
    key_bindings: KeyBindings,
}

impl TextEditorComponent {
    pub fn new(key_bindings: KeyBindings) -> Self {
        Self {
            text_area: TextArea::default(),
            rect: Rect::default(),
            key_bindings,
        }
    }

    pub fn lines(&self) -> &[String] {
        self.text_area.lines()
    }

    pub fn set_text(&mut self, text: String) {
        let lines = text.lines();
        self.text_area = TextArea::from(lines);
    }
}

impl Component for TextEditorComponent {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        match event {
            AppEvent::Key(key) => {
                // Check keybindings for navigation actions.
                if let Some(combo) = key_event_to_combo(key) {
                    if let Some(ActionShortcuts::FocusSidebar) = self.key_bindings.get_action(&combo) {
                        tx.send(AppMessage::FocusSidebar).ok();
                        return EventState::Consumed;
                    }
                }
                self.text_area.input(*key);
                EventState::Consumed
            }
            AppEvent::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::Down(_) => {
                        tx.send(AppMessage::FocusEditor).ok();
                        if mouse.row >= self.rect.y && mouse.column >= self.rect.x {
                            let row = mouse.row - self.rect.y;
                            let col = mouse.column - self.rect.x;
                            self.text_area.move_cursor(CursorMove::Jump(row, col));
                        }
                    }
                    _ => {
                        self.text_area.input(*mouse);
                    }
                }
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect) {
        self.rect = rect;
        f.render_widget(&self.text_area, rect);
    }
}
