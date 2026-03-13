use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::{Block, Borders};
use ratatui_textarea::TextArea;

use crate::app_screen::AppScreen;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::AppSettings;

pub struct EditorScreen {
    vault: Arc<NoteVault>,
    settings: AppSettings,
    text_area: TextArea,
}

impl EditorScreen {
    pub fn new(vault: Arc<NoteVault>, settings: AppSettings) -> Self {
        let text_area = TextArea::default();
        Self {
            vault,
            settings,
            text_area,
        }
    }
}

#[async_trait]
impl AppScreen for EditorScreen {
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
        let block = Block::default().title("Editor").borders(Borders::ALL);
        f.render_widget(block, f.area());
    }
}
