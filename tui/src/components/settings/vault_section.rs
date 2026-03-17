use std::path::PathBuf;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::themes::Theme;

pub struct VaultSection {
    current_path: Option<PathBuf>,
}

impl VaultSection {
    pub fn new(current_path: Option<PathBuf>) -> Self {
        Self { current_path }
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.current_path = path;
    }
}

impl Component for VaultSection {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
        match key.code {
            ratatui::crossterm::event::KeyCode::Enter
            | ratatui::crossterm::event::KeyCode::Char('b') => {
                tx.send(AppMessage::OpenFileBrowser).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let path_str = self.current_path.as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(no vault set)".to_string());
        let text = format!("{}    [Enter: Browse]", path_str);
        let block = Block::default()
            .title("Vault Path")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());
        let para = Paragraph::new(text).block(block).style(theme.base_style());
        f.render_widget(para, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_no_vault_set_when_none() {
        let section = VaultSection::new(None);
        assert!(section.current_path.is_none());
    }

    #[test]
    fn renders_path_when_some() {
        let path = PathBuf::from("/Users/me/notes");
        let section = VaultSection::new(Some(path.clone()));
        assert_eq!(section.current_path.as_ref().unwrap(), &path);
    }

    #[test]
    fn set_path_updates_current() {
        let mut section = VaultSection::new(None);
        let path = PathBuf::from("/Users/me/notes");
        section.set_path(Some(path.clone()));
        assert_eq!(section.current_path.as_ref().unwrap(), &path);
    }

    #[test]
    fn enter_sends_open_file_browser() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = VaultSection::new(None);
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let result = section.handle_event(&key, &tx);
        assert!(matches!(result, crate::components::event_state::EventState::Consumed));
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppMessage::OpenFileBrowser));
    }

    #[test]
    fn b_key_sends_open_file_browser() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = VaultSection::new(None);
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let result = section.handle_event(&key, &tx);
        assert!(matches!(result, crate::components::event_state::EventState::Consumed));
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppMessage::OpenFileBrowser));
    }

    #[test]
    fn renders_no_vault_set_text() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = VaultSection::new(None);
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        terminal.draw(|f| {
            section.render(f, f.area(), &theme, false);
        }).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let flat: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("(no vault set)"), "Expected '(no vault set)' in rendered output");
    }

    #[test]
    fn renders_vault_path_text() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let path = PathBuf::from("/Users/me/notes");
        let mut section = VaultSection::new(Some(path));
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        terminal.draw(|f| {
            section.render(f, f.area(), &theme, false);
        }).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let flat: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("notes"), "Expected vault path in rendered output");
    }
}
