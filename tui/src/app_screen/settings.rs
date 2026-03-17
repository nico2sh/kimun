use async_trait::async_trait;
use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::{Block, Borders};

use crate::app_screen::AppScreen;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsSection { Theme, Vault, Indexing }

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsFocus { Sidebar, Content }

pub struct SettingsScreen {
    pub settings: AppSettings,
    pub initial_settings: AppSettings,
    pub theme: Theme,
    section: SettingsSection,
    focus: SettingsFocus,
    pub pending_save_after_index: bool,
}

impl SettingsScreen {
    pub fn new(settings: AppSettings) -> Self {
        let theme = settings.get_theme();
        let initial_settings = settings.clone();
        Self {
            settings,
            initial_settings,
            theme,
            section: SettingsSection::Theme,
            focus: SettingsFocus::Sidebar,
            pending_save_after_index: false,
        }
    }
}

#[async_trait]
impl AppScreen for SettingsScreen {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        match event {
            AppEvent::Key(key) if key.code == KeyCode::Esc => {
                tx.send(AppMessage::CloseSettings).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let block = Block::default()
            .title("Settings")
            .borders(Borders::ALL);
        f.render_widget(block, f.area());
    }

    async fn handle_app_message(&mut self, msg: AppMessage, _tx: &AppTx) -> Option<AppMessage> {
        Some(msg)
    }
}
