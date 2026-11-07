use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::logic::message::AppTx;
pub use crate::logic::preferences::DEFAULT_SERVER_URL_HINT;
use crate::logic::preferences::PreferencesModel;
use crate::settings::themes::Theme;

/// Preferences section for the optional Kimün server connection.
///
/// One combined address field (URL including port) mapped to
/// `GlobalConfig::kimun_server_url`. An empty field means the server
/// features (semantic search, Q&A) stay off — matching the `None`
/// semantics of the config value. Every edit commits to the model
/// ([`PreferencesModel::set_server_url`], which normalises it).
pub struct ServerSection {
    input: SingleLineInput,
}

impl ServerSection {
    /// The field, prefilled with the model's URL.
    pub fn new(model: &PreferencesModel) -> Self {
        let input = match &model.server_url {
            Some(url) => SingleLineInput::with_value(url.clone()),
            None => SingleLineInput::new(),
        };
        Self { input }
    }

    pub fn handle_input(
        &mut self,
        event: &InputEvent,
        model: &mut PreferencesModel,
        _tx: &AppTx,
    ) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match self.input.handle_key(key) {
            InputOutcome::Changed => {
                model.set_server_url(self.input.value());
                EventState::Consumed
            }
            InputOutcome::Consumed => EventState::Consumed,
            // Enter/Esc bubble up so the screen keeps its save/close flow.
            InputOutcome::Submit | InputOutcome::Cancel | InputOutcome::NotConsumed => {
                EventState::NotConsumed
            }
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
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

    fn model(url: Option<&str>) -> PreferencesModel {
        let mut m = PreferencesModel::new(&crate::settings::AppSettings::default());
        m.server_url = url.map(str::to_string);
        m
    }

    fn type_str(section: &mut ServerSection, m: &mut PreferencesModel, s: &str) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        for c in s.chars() {
            section.handle_input(&key(KeyCode::Char(c)), m, &tx);
        }
    }

    #[test]
    fn new_with_none_starts_empty() {
        let m = model(None);
        let section = ServerSection::new(&m);
        assert!(section.input.is_empty());
    }

    #[test]
    fn new_with_value_preserves_it() {
        let m = model(Some("http://myhost:9000"));
        let section = ServerSection::new(&m);
        assert_eq!(section.input.value(), "http://myhost:9000");
    }

    #[test]
    fn typing_commits_value_live() {
        let mut m = model(None);
        let mut section = ServerSection::new(&m);
        type_str(&mut section, &mut m, "http://myhost:9000");
        assert_eq!(m.server_url.as_deref(), Some("http://myhost:9000"));
    }

    #[test]
    fn clearing_field_commits_none() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut m = model(Some("x"));
        let mut section = ServerSection::new(&m);
        section.handle_input(&key(KeyCode::Backspace), &mut m, &tx);
        assert_eq!(m.server_url, None);
    }

    #[test]
    fn whitespace_only_commits_none() {
        let mut m = model(None);
        let mut section = ServerSection::new(&m);
        type_str(&mut section, &mut m, "   ");
        assert_eq!(m.server_url, None);
    }

    #[test]
    fn value_is_trimmed() {
        let mut m = model(None);
        let mut section = ServerSection::new(&m);
        type_str(&mut section, &mut m, " http://h:1 ");
        assert_eq!(m.server_url.as_deref(), Some("http://h:1"));
    }

    #[test]
    fn enter_and_esc_bubble_up() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut m = model(None);
        let mut section = ServerSection::new(&m);
        assert_eq!(
            section.handle_input(&key(KeyCode::Enter), &mut m, &tx),
            EventState::NotConsumed
        );
        assert_eq!(
            section.handle_input(&key(KeyCode::Esc), &mut m, &tx),
            EventState::NotConsumed
        );
    }

    #[test]
    fn renders_placeholder_hint_when_empty() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let m = model(None);
        let mut section = ServerSection::new(&m);
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
        let m = model(Some("http://myhost:9000"));
        let mut section = ServerSection::new(&m);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("http://myhost:9000"));
    }
}
