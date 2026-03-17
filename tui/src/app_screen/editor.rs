use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use kimun_core::nfs::VaultPath;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};

use crate::app_screen::AppScreen;
use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

enum Focus {
    Sidebar,
    Editor,
}

pub struct EditorScreen {
    vault: Arc<NoteVault>,
    settings: AppSettings,
    theme: Theme,
    editor: TextEditorComponent,
    sidebar: SidebarComponent,
    path: VaultPath,
    focus: Focus,
    sidebar_visible: bool,
    toggle_key: String,
}

impl EditorScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self {
        let kb = settings.key_bindings.clone();
        let theme = settings.get_theme();
        let toggle_key = kb
            .to_hashmap()
            .get(&ActionShortcuts::ToggleSidebar)
            .and_then(|v| v.first().cloned())
            .map(|c| c.to_string())
            .unwrap_or_else(|| "^B".to_string());
        Self {
            settings,
            theme,
            editor: TextEditorComponent::new(kb.clone()),
            sidebar: SidebarComponent::new(kb, vault.clone()),
            vault,
            path,
            focus: Focus::Editor,
            sidebar_visible: true,
            toggle_key,
        }
    }
}

impl EditorScreen {
    pub async fn open_path(&mut self, path: VaultPath, tx: &AppTx) {
        self.path = path.clone();
        let content = self.vault.get_note_text(&self.path).await.unwrap();
        self.editor.set_text(content);

        // Only load the sidebar on first open (when it has no entries yet).
        // Selecting a note while browsing should not reload the sidebar.
        if self.sidebar.is_empty() {
            let dir = if path.is_note() {
                path.get_parent_path().0
            } else {
                path
            };
            self.navigate_sidebar(dir, tx).await;
        }
    }

    pub async fn navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx) {
        let (options, rx) = VaultBrowseOptionsBuilder::new(&dir)
            .non_recursive()
            .full_validation()
            .build();

        let vault = self.vault.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            vault.browse_vault(options).await.ok();
            tx2.send(AppMessage::Redraw).ok();
        });

        self.sidebar.start_loading(rx, dir);
    }
}

impl EditorScreen {
    pub fn focus_editor(&mut self) {
        self.focus = Focus::Editor;
    }

    pub fn focus_sidebar(&mut self) {
        self.sidebar_visible = true;
        self.focus = Focus::Sidebar;
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible {
            self.focus_editor();
        }
    }
}

#[async_trait]
impl AppScreen for EditorScreen {
    async fn on_enter(&mut self, tx: &AppTx) {
        self.open_path(self.path.clone(), tx).await;
    }

    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        if let AppEvent::Key(key) = event {
            if let Some(combo) = key_event_to_combo(key) {
                if let Some(ActionShortcuts::ToggleSidebar) =
                    self.settings.key_bindings.get_action(&combo)
                {
                    self.toggle_sidebar();
                    return EventState::Consumed;
                }
            }
        }

        // Mouse events are routed to all components regardless of focus so that
        // clicking anywhere can transfer focus correctly.
        if matches!(event, AppEvent::Mouse(_)) {
            if self.sidebar.handle_event(event, tx).is_consumed() {
                return EventState::Consumed;
            }
            return self.editor.handle_event(event, tx);
        }

        match self.focus {
            Focus::Sidebar => self.sidebar.handle_event(event, tx),
            Focus::Editor => self.editor.handle_event(event, tx),
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let theme = &self.theme;

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        let header = Block::default()
            .title("Kimün")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .title_style(Style::default().fg(theme.accent.to_ratatui()));
        f.render_widget(header, rows[0]);

        let columns = if self.sidebar_visible {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(30), Constraint::Min(0)])
                .split(rows[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0)])
                .split(rows[1])
        };

        let editor_focused = matches!(self.focus, Focus::Editor);
        let sidebar_focused = matches!(self.focus, Focus::Sidebar);

        let editor_area = if self.sidebar_visible {
            self.sidebar.render(f, columns[0], theme, sidebar_focused);
            columns[1]
        } else {
            columns[0]
        };

        let editor_border_style = theme.border_style(editor_focused);
        let editor_block = Block::default()
            .title("Editor")
            .borders(Borders::ALL)
            .border_style(editor_border_style);
        let editor_inner = editor_block.inner(editor_area);
        f.render_widget(editor_block, editor_area);
        self.editor.render(f, editor_inner, theme, editor_focused);

        let focus_label = if editor_focused { "EDITOR" } else { "SIDEBAR" };
        let footer = Block::default()
            .title(format!("[{focus_label}]  Ctrl+Q: Quit  |  Tab: Sidebar→Editor  |  Shift+Tab: Editor→Sidebar  |  {}: Toggle sidebar", self.toggle_key))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .title_style(Style::default().fg(theme.fg_secondary.to_ratatui()));
        f.render_widget(footer, rows[2]);
    }

    async fn handle_app_message(&mut self, msg: AppMessage, tx: &AppTx) -> Option<AppMessage> {
        match msg {
            AppMessage::OpenPath(path) => {
                if path.is_note() {
                    self.open_path(path, tx).await;
                } else {
                    self.navigate_sidebar(path, tx).await;
                }
                None
            }
            AppMessage::FocusEditor => {
                self.focus_editor();
                None
            }
            AppMessage::FocusSidebar => {
                self.focus_sidebar();
                None
            }
            other => Some(other),
        }
    }
}
