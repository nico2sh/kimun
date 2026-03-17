use ratatui::Frame;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;

pub struct TextEditorComponent {
    text_area: TextArea<'static>,
    /// Tracks the rendered rect to map mouse click coordinates.
    rect: Rect,
}

impl TextEditorComponent {
    pub fn new() -> Self {
        Self {
            text_area: TextArea::default(),
            rect: Rect::default(),
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
                use ratatui::crossterm::event::KeyCode;
                use crate::components::app_message::AppMessage;
                // Shift+Tab: yield focus back to the sidebar.
                if key.code == KeyCode::BackTab {
                    tx.send(AppMessage::FocusSidebar).ok();
                    return EventState::Consumed;
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
