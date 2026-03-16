use std::any::Any;

use async_trait::async_trait;
use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::{Block, Borders};

use crate::app_screen::AppScreen;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;

pub struct SettingsScreen {}

impl SettingsScreen {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl AppScreen for SettingsScreen {
    fn handle_event(&mut self, event: AppEvent, tx: &AppTx) -> EventState {
        match event {
            AppEvent::Key(key) if key.code == KeyCode::Char('q') => {
                tx.send(AppMessage::Quit).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let block = Block::default().title("Settings").borders(Borders::ALL);
        f.render_widget(block, f.area());
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
