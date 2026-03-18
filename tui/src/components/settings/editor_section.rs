use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::app_message::AppTx;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::themes::Theme;

const MIN_AUTOSAVE_SECS: u64 = 5;
const MAX_AUTOSAVE_SECS: u64 = 300;
const STEP: u64 = 5;

pub struct EditorSection {
    pub autosave_interval_secs: u64,
}

impl EditorSection {
    pub fn new(autosave_interval_secs: u64) -> Self {
        Self { autosave_interval_secs }
    }
}

impl Component for EditorSection {
    fn handle_event(&mut self, event: &AppEvent, _tx: &AppTx) -> EventState {
        let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
        match key.code {
            ratatui::crossterm::event::KeyCode::Left
            | ratatui::crossterm::event::KeyCode::Char('h') => {
                self.autosave_interval_secs =
                    self.autosave_interval_secs.saturating_sub(STEP).max(MIN_AUTOSAVE_SECS);
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Right
            | ratatui::crossterm::event::KeyCode::Char('l') => {
                self.autosave_interval_secs =
                    (self.autosave_interval_secs + STEP).min(MAX_AUTOSAVE_SECS);
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let block = Block::default()
            .title("Editor")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        let label = Paragraph::new("Autosave Interval")
            .style(theme.base_style());
        f.render_widget(label, rows[0]);

        let value = format!(
            "  ◀  {}s  ▶   (←/→ to change)",
            self.autosave_interval_secs
        );
        let value_style = if focused {
            ratatui::style::Style::default()
                .fg(theme.accent.to_ratatui())
                .bg(theme.bg.to_ratatui())
        } else {
            ratatui::style::Style::default()
                .fg(theme.fg.to_ratatui())
                .bg(theme.bg.to_ratatui())
        };
        f.render_widget(Paragraph::new(value).style(value_style), rows[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn right_increases_interval_by_step() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(10);
        section.handle_event(&key(KeyCode::Right), &tx);
        assert_eq!(section.autosave_interval_secs, 15);
    }

    #[test]
    fn left_decreases_interval_by_step() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(10);
        section.handle_event(&key(KeyCode::Left), &tx);
        assert_eq!(section.autosave_interval_secs, 5);
    }

    #[test]
    fn left_clamps_at_min() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(5);
        section.handle_event(&key(KeyCode::Left), &tx);
        assert_eq!(section.autosave_interval_secs, MIN_AUTOSAVE_SECS);
    }

    #[test]
    fn right_clamps_at_max() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(298);
        section.handle_event(&key(KeyCode::Right), &tx);
        assert_eq!(section.autosave_interval_secs, MAX_AUTOSAVE_SECS);
    }

    #[test]
    fn l_key_increases_interval() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(10);
        section.handle_event(&key(KeyCode::Char('l')), &tx);
        assert_eq!(section.autosave_interval_secs, 15);
    }

    #[test]
    fn h_key_decreases_interval() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(10);
        section.handle_event(&key(KeyCode::Char('h')), &tx);
        assert_eq!(section.autosave_interval_secs, 5);
    }
}
