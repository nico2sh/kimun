use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

pub struct CreateNoteDialog {
    pub path: VaultPath,
    pub vault: Arc<NoteVault>,
    /// Pre-formatted `"  {path}"` for zero-allocation rendering.
    pub filename: String,
    pub error: Option<String>,
}

impl CreateNoteDialog {
    pub fn new(path: VaultPath, vault: Arc<NoteVault>) -> Self {
        let filename = format!("  {}", path);
        Self { path, vault, filename, error: None }
    }

    /// Handle a raw [`KeyEvent`]. Returns [`EventState::Consumed`] for Enter and Esc.
    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Enter => {
                let path = self.path.clone();
                let vault = Arc::clone(&self.vault);
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    match vault.load_or_create_note(&path, None).await {
                        Ok(_) => {
                            tx_clone.send(AppEvent::OpenPath(path)).ok();
                        }
                        Err(e) => {
                            tx_clone.send(AppEvent::DialogError(e.to_string())).ok();
                        }
                    }
                });
                EventState::Consumed
            }
            KeyCode::Esc => {
                tx.send(AppEvent::CloseDialog).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }
}

impl Component for CreateNoteDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let height = if self.error.is_some() { 10 } else { 9 };
        let popup_area = super::fixed_centered_rect(52, height, rect);

        f.render_widget(Clear, popup_area);

        let fg_muted = theme.fg_muted.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let outer_block = Block::default()
            .title(" Create note? ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(fg_muted))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: path
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: body
                Constraint::Length(1), // 4: spacer
                Constraint::Length(1), // 5: hint
                Constraint::Length(1), // 6: error (optional)
                Constraint::Min(0),    // 7: remainder
            ])
            .split(inner);

        super::render_path_row(f, rows[1], &self.filename, fg, bg);
        super::render_separator(f, rows[2], fg_muted, bg);
        f.render_widget(
            Paragraph::new("  Note doesn't exist.")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );
        f.render_widget(
            Paragraph::new("  [Enter] Create   [Esc] Cancel")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[5],
        );
        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[6], msg, bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn new_does_not_panic() {
        let (_tx, _rx) = mpsc::unbounded_channel::<AppEvent>();
        let path = VaultPath::root();
        let _ = (path,);
    }

    #[test]
    fn esc_sends_close_dialog() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join("kimun_create_esc_test");
            std::fs::create_dir_all(&tmp).unwrap();

            let vault_result = NoteVault::new(tmp).await;
            let Ok(vault) = vault_result else { return };
            let vault = Arc::new(vault);

            let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog = CreateNoteDialog::new(VaultPath::root(), vault);

            let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::CloseDialog");
            assert!(matches!(event, AppEvent::CloseDialog));
        });
    }

    #[test]
    fn enter_returns_consumed() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join("kimun_create_enter_test");
            std::fs::create_dir_all(&tmp).unwrap();

            let vault_result = NoteVault::new(tmp).await;
            let Ok(vault) = vault_result else { return };
            let vault = Arc::new(vault);

            let (tx, _rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog = CreateNoteDialog::new(VaultPath::root(), vault);

            let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
        });
    }
}
