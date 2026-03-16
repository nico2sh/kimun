use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use kimun_core::nfs::VaultPath;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders};

use crate::app_screen::AppScreen;
use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::settings::AppSettings;

enum Focus {
    Sidebar,
    Editor,
}

pub struct EditorScreen {
    vault: Arc<NoteVault>,
    settings: AppSettings,
    editor: TextEditorComponent,
    sidebar: SidebarComponent,
    path: VaultPath,
    focus: Focus,
}

impl EditorScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self {
        Self {
            vault,
            settings,
            editor: TextEditorComponent::new(),
            sidebar: SidebarComponent::new(),
            path,
            focus: Focus::Editor,
        }
    }
}

impl EditorScreen {
    pub async fn open_path(&mut self, path: VaultPath, tx: AppTx) {
        self.path = path.clone();
        let content = self.vault.get_note_text(&self.path).await.unwrap();
        self.editor.set_text(content);

        let dir = if path.is_note() {
            path.get_parent_path().0
        } else {
            path
        };

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

    pub async fn navigate_sidebar(&mut self, dir: VaultPath, tx: AppTx) {
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

#[async_trait]
impl AppScreen for EditorScreen {
    async fn on_enter(&mut self, tx: &AppTx) {
        let path = self
            .settings
            .last_paths
            .last()
            .map_or_else(|| VaultPath::root(), |p| p.to_owned());
        self.open_path(path, tx.clone()).await;
    }

    fn handle_event(&mut self, event: AppEvent, tx: &AppTx) -> EventState {
        match &event {
            AppEvent::Key(key) if key.code == KeyCode::Esc => {
                tx.send(AppMessage::Quit).ok();
                return EventState::Consumed;
            }
            AppEvent::Key(key) if key.code == KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Editor,
                    Focus::Editor => Focus::Sidebar,
                };
                self.sidebar.focused = matches!(self.focus, Focus::Sidebar);
                return EventState::Consumed;
            }
            _ => {}
        }

        match self.focus {
            Focus::Sidebar => self.sidebar.handle_event(&event, tx),
            Focus::Editor => self.editor.handle_event(&event, tx),
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        let header = Block::default().title("Kimün").borders(Borders::ALL);
        f.render_widget(header, rows[0]);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(0)])
            .split(rows[1]);

        self.sidebar.render(f, columns[0]);

        let editor_block = Block::default().title("Editor").borders(Borders::ALL);
        let editor_inner = editor_block.inner(columns[1]);
        f.render_widget(editor_block, columns[1]);
        self.editor.render(f, editor_inner);

        let footer = Block::default()
            .title("ESC: Quit  |  Tab: Switch focus")
            .borders(Borders::ALL);
        f.render_widget(footer, rows[2]);
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
