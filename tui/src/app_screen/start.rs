use std::any::Any;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::{Block, Borders};

use crate::app_screen::AppScreen;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::AppSettings;

pub struct StartScreen {
    settings: AppSettings,
}

impl StartScreen {
    pub fn new(settings: AppSettings) -> Self {
        Self { settings }
    }
}

#[async_trait]
impl AppScreen for StartScreen {
    async fn on_enter(&mut self, tx: &AppTx) {
        let path = self
            .settings
            .last_paths
            .last()
            .map_or_else(|| VaultPath::root(), |p| p.to_owned());
        tx.send(AppMessage::OpenPath(path)).ok();
    }

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
        let block = Block::default().title("Start app").borders(Borders::ALL);
        f.render_widget(block, f.area());
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
