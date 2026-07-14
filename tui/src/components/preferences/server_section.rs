use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

/// Shown grayed out when the field is empty. Mirrors the server's built-in
/// bind default (`127.0.0.1:7573` in `kimun-server`'s config).
pub const DEFAULT_SERVER_URL_HINT: &str = "http://localhost:7573";

/// Preferences section for the optional Kimün server connection.
///
/// One combined address field (URL including port) mapped to
/// `GlobalConfig::kimun_server_url`. An empty field means the server
/// features (semantic search, Q&A) stay off — matching the `None`
/// semantics of the config value.
pub struct ServerSection {
    /// Committed value: `None` when the field is empty.
    pub server_url: Option<String>,
    input: SingleLineInput,
}

impl ServerSection {
    pub fn new(server_url: Option<String>) -> Self {
        let input = match &server_url {
            Some(url) => SingleLineInput::with_value(url.clone()),
            None => SingleLineInput::new(),
        };
        Self { server_url, input }
    }

    fn commit(&mut self) {
        let trimmed = self.input.value().trim();
        self.server_url = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
}

impl Component for ServerSection {
    fn handle_input(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match self.input.handle_key(key) {
            InputOutcome::Changed => {
                self.commit();
                EventState::Consumed
            }
            InputOutcome::Consumed => EventState::Consumed,
            // Enter/Esc bubble up so the screen keeps its save/close flow.
            InputOutcome::Submit | InputOutcome::Cancel | InputOutcome::NotConsumed => {
                EventState::NotConsumed
            }
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let block = Block::default()
            .title("Server")
            .borders(Borders::ALL)
            .border_style(theme.border_style(focused))
            .style(theme.base_style());
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // label
                Constraint::Length(1), // value input
                Constraint::Length(1), // spacer
                Constraint::Length(1), // hint
                Constraint::Min(0),
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new("Kimün Server Address").style(theme.base_style()),
            rows[0],
        );

        let gray = Style::default()
            .fg(theme.gray.to_ratatui())
            .bg(theme.bg.to_ratatui());
        if self.input.is_empty() {
            // Placeholder with the default address; caret stays at the start.
            f.render_widget(
                Paragraph::new(format!("  {}", DEFAULT_SERVER_URL_HINT)).style(gray),
                rows[1],
            );
            if focused {
                f.set_cursor_position(ratatui::layout::Position {
                    x: rows[1].x + 2,
                    y: rows[1].y,
                });
            }
        } else {
            let value_style = if focused {
                Style::default()
                    .fg(theme.accent.to_ratatui())
                    .bg(theme.bg.to_ratatui())
            } else {
                theme.base_style()
            };
            self.input.render(f, rows[1], value_style, 2, focused);
        }

        f.render_widget(
            Paragraph::new(format!(
                "  URL with port, e.g. {} — leave empty to disable",
                DEFAULT_SERVER_URL_HINT
            ))
            .style(gray),
            rows[3],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn type_str(section: &mut ServerSection, s: &str) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        for c in s.chars() {
            section.handle_input(&key(KeyCode::Char(c)), &tx);
        }
    }

    #[test]
    fn new_with_none_starts_empty() {
        let section = ServerSection::new(None);
        assert_eq!(section.server_url, None);
    }

    #[test]
    fn new_with_value_preserves_it() {
        let section = ServerSection::new(Some("http://myhost:9000".to_string()));
        assert_eq!(section.server_url.as_deref(), Some("http://myhost:9000"));
    }

    #[test]
    fn typing_commits_value_live() {
        let mut section = ServerSection::new(None);
        type_str(&mut section, "http://myhost:9000");
        assert_eq!(section.server_url.as_deref(), Some("http://myhost:9000"));
    }

    #[test]
    fn clearing_field_commits_none() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = ServerSection::new(Some("x".to_string()));
        section.handle_input(&key(KeyCode::Backspace), &tx);
        assert_eq!(section.server_url, None);
    }

    #[test]
    fn whitespace_only_commits_none() {
        let mut section = ServerSection::new(None);
        type_str(&mut section, "   ");
        assert_eq!(section.server_url, None);
    }

    #[test]
    fn value_is_trimmed() {
        let mut section = ServerSection::new(None);
        type_str(&mut section, " http://h:1 ");
        assert_eq!(section.server_url.as_deref(), Some("http://h:1"));
    }

    #[test]
    fn enter_and_esc_bubble_up() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = ServerSection::new(None);
        assert_eq!(
            section.handle_input(&key(KeyCode::Enter), &tx),
            EventState::NotConsumed
        );
        assert_eq!(
            section.handle_input(&key(KeyCode::Esc), &tx),
            EventState::NotConsumed
        );
    }

    #[test]
    fn renders_placeholder_hint_when_empty() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = ServerSection::new(None);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, true))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(
            flat.contains(DEFAULT_SERVER_URL_HINT),
            "expected default address hint when empty"
        );
    }

    #[test]
    fn renders_value_when_set() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = ServerSection::new(Some("http://myhost:9000".to_string()));
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("http://myhost:9000"));
    }
}
