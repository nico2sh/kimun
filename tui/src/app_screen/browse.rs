use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::app_screen::AppScreen;
use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::sidebar::SidebarComponent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct BrowseScreen {
    vault: Arc<NoteVault>,
    sidebar: SidebarComponent,
    settings: AppSettings,
    theme: Theme,
    path: VaultPath,
}

impl BrowseScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self {
        let kb = settings.key_bindings.clone();
        let theme = settings.get_theme();
        Self {
            sidebar: SidebarComponent::new(kb, vault.clone()),
            vault,
            settings,
            theme,
            path,
        }
    }

    async fn navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx) {
        let (options, rx) = VaultBrowseOptionsBuilder::new(&dir)
            .non_recursive()
            .full_validation()
            .build();
        self.path = dir.clone();
        let vault = self.vault.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            vault.browse_vault(options).await.ok();
            tx2.send(AppMessage::Redraw).ok();
        });
        self.sidebar.start_loading(rx, dir);
    }
}

#[async_trait]
impl AppScreen for BrowseScreen {
    async fn on_enter(&mut self, tx: &AppTx) {
        self.navigate_sidebar(self.path.clone(), tx).await;
    }

    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        if let AppEvent::Key(key) = event {
            if let Some(combo) = key_event_to_combo(key) {
                if self.settings.key_bindings.get_action(&combo) == Some(ActionShortcuts::OpenSettings) {
                    tx.send(AppMessage::OpenSettings).ok();
                    return EventState::Consumed;
                }
            }
            if key.code == KeyCode::Esc {
                tx.send(AppMessage::Quit).ok();
                return EventState::Consumed;
            }
        }
        self.sidebar.handle_event(event, tx)
    }

    fn render(&mut self, f: &mut Frame) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(60), Constraint::Min(0)])
            .split(f.area());
        self.sidebar.render(f, cols[1], &self.theme, true);
    }

    async fn handle_app_message(&mut self, msg: AppMessage, tx: &AppTx) -> Option<AppMessage> {
        if let AppMessage::OpenPath(path) = &msg {
            if !path.is_note() {
                let dir = path.clone();
                self.navigate_sidebar(dir, tx).await;
                return None;
            }
        }
        Some(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn make_settings_with_defaults() -> AppSettings {
        AppSettings::default()
    }

    async fn make_vault() -> Arc<NoteVault> {
        let dir = std::env::temp_dir().join("kimun_browse_test_vault");
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(NoteVault::new(&dir).await.unwrap())
    }

    fn key_event(code: KeyCode) -> AppEvent {
        AppEvent::Key(ratatui::crossterm::event::KeyEvent {
            code,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            kind: ratatui::crossterm::event::KeyEventKind::Press,
            state: ratatui::crossterm::event::KeyEventState::NONE,
        })
    }

    #[tokio::test]
    async fn new_stores_path() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let path = VaultPath::root();
        let screen = BrowseScreen::new(vault, path.clone(), settings);
        assert_eq!(screen.path, path);
    }

    #[tokio::test]
    async fn esc_sends_quit() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        screen.handle_event(&key_event(KeyCode::Esc), &tx);
        let msg = rx.try_recv().expect("should have sent a message");
        assert!(matches!(msg, AppMessage::Quit));
    }

    #[tokio::test]
    async fn settings_keybinding_sends_open_settings() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        #[cfg(target_os = "macos")]
        let mods = ratatui::crossterm::event::KeyModifiers::SUPER;
        #[cfg(not(target_os = "macos"))]
        let mods = ratatui::crossterm::event::KeyModifiers::CONTROL;

        let event = AppEvent::Key(ratatui::crossterm::event::KeyEvent {
            code: ratatui::crossterm::event::KeyCode::Char(','),
            modifiers: mods,
            kind: ratatui::crossterm::event::KeyEventKind::Press,
            state: ratatui::crossterm::event::KeyEventState::NONE,
        });

        let (tx, mut rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        screen.handle_event(&event, &tx);
        let msg = rx.try_recv().expect("should have sent a message");
        assert!(matches!(msg, AppMessage::OpenSettings));
    }

    #[tokio::test]
    async fn handle_app_message_open_path_dir_is_consumed() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let dir = VaultPath::new("subdir");
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.handle_app_message(AppMessage::OpenPath(dir.clone()), &tx).await;
        assert!(result.is_none(), "OpenPath(dir) should be consumed");
        assert_eq!(screen.path, dir, "path should be updated");
    }

    #[tokio::test]
    async fn handle_app_message_open_path_note_is_forwarded() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let note = VaultPath::note_path_from("test.md");
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.handle_app_message(AppMessage::OpenPath(note.clone()), &tx).await;
        assert!(result.is_some(), "OpenPath(note) should be forwarded");
        assert!(matches!(result.unwrap(), AppMessage::OpenPath(_)));
    }

    #[tokio::test]
    async fn handle_app_message_unrelated_is_forwarded() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.handle_app_message(AppMessage::FocusEditor, &tx).await;
        assert!(result.is_some(), "FocusEditor should be forwarded");
    }
}
