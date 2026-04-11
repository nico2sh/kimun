use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::sidebar::SidebarComponent;
use crate::settings::SharedSettings;
use crate::settings::themes::Theme;

pub struct BrowseScreen {
    vault: Arc<NoteVault>,
    sidebar: SidebarComponent,
    theme: Theme,
    path: VaultPath,
}

impl BrowseScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: SharedSettings) -> Self {
        let s = settings.read().unwrap();
        let kb = s.key_bindings.clone();
        let theme = s.get_theme();
        let icons = s.icons();
        let sidebar = SidebarComponent::new(kb, vault.clone(), icons, &s);
        drop(s);
        Self {
            sidebar,
            vault,
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
            tx2.send(AppEvent::Redraw).ok();
        });
        self.sidebar.start_loading(rx, dir);
    }
}

#[async_trait]
impl AppScreen for BrowseScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Browse
    }

    async fn on_enter(&mut self, tx: &AppTx) {
        self.navigate_sidebar(self.path.clone(), tx).await;
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.sidebar.handle_input(event, tx)
    }

    fn render(&mut self, f: &mut Frame) {
        f.render_widget(
            Block::default().style(self.theme.base_style()),
            f.area(),
        );

        // Split into content area + one-line hint bar at the bottom.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(f.area());

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(60),
                Constraint::Min(0),
            ])
            .split(rows[0]);

        self.sidebar.render(f, cols[1], &self.theme, true);

        f.render_widget(
            Paragraph::new(
                " Type to filter  ·  Enter to open  ·  Type + Enter to create a new note",
            )
            .style(
                Style::default()
                    .fg(self.theme.fg_muted.to_ratatui())
                    .bg(self.theme.bg.to_ratatui()),
            ),
            rows[1],
        );
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        if let AppEvent::OpenPath(path) = &msg
            && !path.is_note() {
                let dir = path.clone();
                self.navigate_sidebar(dir, tx).await;
                return None;
            }
        Some(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use std::sync::RwLock;
    use ratatui::crossterm::event::KeyCode;
    use tokio::sync::mpsc::unbounded_channel;

    fn make_settings_with_defaults() -> SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    async fn make_vault() -> Arc<NoteVault> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let thread_id = std::thread::current().id();
        let dir = std::env::temp_dir().join(format!("kimun_browse_test_{nonce}_{thread_id:?}"));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(NoteVault::new(&dir).await.unwrap())
    }

    fn key_event(code: KeyCode) -> InputEvent {
        InputEvent::Key(ratatui::crossterm::event::KeyEvent {
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
    async fn esc_does_not_quit() {
        // Quit is now handled globally in main.rs via Ctrl+Q; BrowseScreen ignores Esc.
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        screen.handle_input(&key_event(KeyCode::Esc), &tx);
        assert!(
            rx.try_recv().is_err(),
            "Esc should not send any message from BrowseScreen"
        );
    }

    #[tokio::test]
    async fn handle_app_message_open_path_dir_is_consumed() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let dir = VaultPath::new("subdir");
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen
            .handle_app_message(AppEvent::OpenPath(dir.clone()), &tx)
            .await;
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
        let result = screen
            .handle_app_message(AppEvent::OpenPath(note.clone()), &tx)
            .await;
        assert!(result.is_some(), "OpenPath(note) should be forwarded");
        assert!(matches!(result.unwrap(), AppEvent::OpenPath(_)));
    }

    #[tokio::test]
    async fn handle_app_message_unrelated_is_forwarded() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.handle_app_message(AppEvent::FocusEditor, &tx).await;
        assert!(result.is_some(), "FocusEditor should be forwarded");
    }
}
