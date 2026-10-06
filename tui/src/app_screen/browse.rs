use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, FileOp, InputEvent};
use crate::components::sidebar::SidebarComponent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::SharedSettings;
use crate::settings::themes::Theme;

pub struct BrowseScreen {
    sidebar: SidebarComponent,
    theme: Theme,
    settings: SharedSettings,
    /// Last mouse position, for the hover highlight (`components::clickable`).
    pointer: Option<ratatui::layout::Position>,
}

impl BrowseScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: SharedSettings) -> Self {
        let s = settings.read().unwrap();
        let theme = s.get_theme();
        let mut sidebar = SidebarComponent::from_settings(vault, &s);
        drop(s);
        // The sidebar's `current_dir` is the single source of truth for the
        // browsed directory; seed it so `on_enter` opens at `path`.
        sidebar.set_current_dir(path);
        // Browse hosts no overlays, so it cannot open the sort dialog: the
        // label would be a click target that does nothing.
        sidebar.set_sort_chip(false);
        Self {
            sidebar,
            theme,
            settings,
            pointer: None,
        }
    }

    async fn navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx) {
        // The sidebar hosts a streamed `SearchList`; (re)building its engine for
        // `dir` runs `browse_vault_stream` inside the source and emits rows as they
        // arrive. `navigate` updates the sidebar's `current_dir`.
        self.sidebar.navigate(dir, tx);
    }
}

#[async_trait]
impl AppScreen for BrowseScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Browse
    }

    async fn on_enter(&mut self, tx: &AppTx) {
        self.navigate_sidebar(self.sidebar.current_dir().clone(), tx)
            .await;
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        if let InputEvent::Mouse(m) = event {
            self.pointer = Some(ratatui::layout::Position::new(m.column, m.row));
        }
        // Intercept the journal shortcut (Ctrl+J by default) so today's entry
        // can be opened straight from Browse; everything else feeds the sidebar.
        if let InputEvent::Key(key) = event
            && let Some(combo) = key_event_to_combo(key)
        {
            let action = {
                let s = self.settings.read().unwrap();
                s.key_bindings.get_action(&combo)
            };
            if action == Some(ActionShortcuts::NewJournal) {
                // App-level OpenJournal resolves today's entry and routes it
                // via OpenPath, switching to the editor (Browse does not open
                // notes itself).
                tx.send(AppEvent::OpenJournal).ok();
                return EventState::Consumed;
            }
        }
        self.sidebar.handle_input(event, tx)
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) {
        if let AppEvent::FileOp(FileOp::Created(path)) = msg {
            // A note was created somewhere; rebuild the listing only when we are
            // browsing its directory so the new note shows up.
            let (parent, _) = path.get_parent_path();
            self.sidebar.refresh_if_showing(&parent, tx);
        }
    }

    fn render(&mut self, f: &mut Frame) {
        // Each screen starts its frame's click targets empty.
        crate::components::clickable::clear();
        f.render_widget(Block::default().style(self.theme.base_style()), f.area());

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
                    .fg(self.theme.gray.to_ratatui())
                    .bg(self.theme.bg.to_ratatui()),
            ),
            rows[1],
        );
        crate::components::clickable::apply_hover(f.buffer_mut(), self.pointer, &self.theme);
    }

    async fn try_open_path(
        &mut self,
        path: VaultPath,
        _emphasis: Option<Vec<String>>,
        tx: &AppTx,
    ) -> Option<VaultPath> {
        if !path.is_note() {
            self.navigate_sidebar(path, tx).await;
            return None;
        }
        // Notes are not handled by Browse — bubble up so the main loop opens
        // the editor for them.
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::AppEvent;
    use crate::settings::AppSettings;
    use crate::test_support::{key_event, temp_vault};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::sync::RwLock;
    use tokio::sync::mpsc::unbounded_channel;

    fn make_settings_with_defaults() -> SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    async fn make_vault() -> Arc<NoteVault> {
        temp_vault("browse").await
    }

    /// Browse cannot open the sort dialog, so it must not draw a sort label
    /// that looks clickable.
    #[tokio::test]
    async fn browse_draws_no_sort_label() {
        let vault = make_vault().await;
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), make_settings_with_defaults());
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
        t.draw(|f| screen.render(f)).unwrap();
        let text: String = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Search"), "the search box is drawn");
        assert!(!text.contains("Name ↑"), "but no sort label");
    }

    #[tokio::test]
    async fn new_seeds_sidebar_dir() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let path = VaultPath::new("subdir");
        let screen = BrowseScreen::new(vault, path.clone(), settings);
        assert_eq!(screen.sidebar.current_dir(), &path);
    }

    #[tokio::test]
    async fn esc_does_not_quit() {
        // Quit is a global shortcut of the App loop (`app::run_app`); BrowseScreen ignores Esc.
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
    async fn ctrl_j_requests_journal() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let ctrl_j = InputEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let state = screen.handle_input(&ctrl_j, &tx);
        assert_eq!(state, EventState::Consumed, "Ctrl+J should be consumed");
        assert!(
            matches!(rx.try_recv(), Ok(AppEvent::OpenJournal)),
            "Ctrl+J should request the journal entry"
        );
    }

    #[tokio::test]
    async fn try_open_path_dir_is_consumed() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let dir = VaultPath::new("subdir");
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.try_open_path(dir.clone(), None, &tx).await;
        assert!(result.is_none(), "dir path should be consumed");
        assert_eq!(
            screen.sidebar.current_dir(),
            &dir,
            "sidebar dir should be updated"
        );
    }

    #[tokio::test]
    async fn try_open_path_note_is_forwarded() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let note = VaultPath::note_path_from("test.md");
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.try_open_path(note.clone(), None, &tx).await;
        assert_eq!(
            result,
            Some(note),
            "note path should bubble up unchanged for the editor"
        );
    }
}
