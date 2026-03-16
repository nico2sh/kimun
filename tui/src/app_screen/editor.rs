use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders};

use crate::app_screen::AppScreen;
use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::text_editor::TextEditorComponent;
use crate::settings::AppSettings;

pub struct EditorScreen {
    vault: Arc<NoteVault>,
    settings: AppSettings,
    editor: TextEditorComponent,
    path: VaultPath,
}

impl EditorScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self {
        Self {
            vault,
            settings,
            editor: TextEditorComponent::new(),
            path,
        }
    }
}

impl EditorScreen {
    pub async fn open_path(&mut self, path: VaultPath) {
        self.path = path;
        let content = self.vault.get_note_text(&self.path).await.unwrap();
        self.editor.set_text(content);
    }
}

#[async_trait]
impl AppScreen for EditorScreen {
    async fn on_enter(&mut self, _tx: &AppTx) {
        let path = self
            .settings
            .last_paths
            .last()
            .map_or_else(|| VaultPath::root(), |p| p.to_owned());
        self.open_path(path).await;
    }

    fn handle_event(&mut self, event: AppEvent, tx: &AppTx) -> EventState {
        match event {
            AppEvent::Key(key) if key.code == KeyCode::Esc => {
                tx.send(AppMessage::Quit).ok();
                EventState::Consumed
            }
            _ => self.editor.handle_event(&event, tx),
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let block = Block::default().title("Editor").borders(Borders::ALL);
        let inner = block.inner(f.area());
        f.render_widget(block, f.area());
        self.editor.render(f, inner);
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
