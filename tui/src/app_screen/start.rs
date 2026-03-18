use async_trait::async_trait;
use kimun_core::nfs::VaultPath;
use ratatui::widgets::{Block, Borders};

use crate::app_screen::{AppScreen, ScreenKind};
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

    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Start
    }

    fn handle_event(&mut self, _event: &AppEvent, _tx: &AppTx) -> EventState {
        EventState::NotConsumed
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let block = Block::default().title("Start app").borders(Borders::ALL);
        f.render_widget(block, f.area());
    }

}
