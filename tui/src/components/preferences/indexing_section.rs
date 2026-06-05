use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::settings::themes::Theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndexAction {
    Fast,
    Full,
}

pub struct IndexingSection {
    pub selected: IndexAction,
    vault_available: bool,
}

impl IndexingSection {
    pub fn new(vault_available: bool) -> Self {
        Self {
            selected: IndexAction::Fast,
            vault_available,
        }
    }

    pub fn set_vault_available(&mut self, available: bool) {
        self.vault_available = available;
    }
}

impl Component for IndexingSection {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        if !self.vault_available {
            return EventState::NotConsumed;
        }
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match key.code {
            ratatui::crossterm::event::KeyCode::Right
            | ratatui::crossterm::event::KeyCode::Char('l') => {
                self.selected = IndexAction::Full;
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Left
            | ratatui::crossterm::event::KeyCode::Char('h') => {
                self.selected = IndexAction::Fast;
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Enter => {
                let msg = match self.selected {
                    IndexAction::Fast => AppEvent::TriggerFastReindex,
                    IndexAction::Full => AppEvent::TriggerFullReindex,
                };
                tx.send(msg).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let block = Block::default()
            .title("Reindex")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let fast_label = if self.selected == IndexAction::Fast {
            "[ Fast Reindex ]"
        } else {
            "  Fast Reindex  "
        };
        let full_label = if self.selected == IndexAction::Full {
            "[ Full Reindex ]"
        } else {
            "  Full Reindex  "
        };
        let dim = if self.vault_available {
            Style::default()
                .fg(theme.fg.to_ratatui())
                .bg(theme.bg.to_ratatui())
        } else {
            Style::default()
                .fg(theme.fg.to_ratatui())
                .bg(theme.bg.to_ratatui())
                .add_modifier(Modifier::DIM)
        };

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);
        f.render_widget(Paragraph::new(fast_label).style(dim), cols[0]);
        f.render_widget(Paragraph::new(full_label).style(dim), cols[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::AppEvent;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn not_consumed_when_vault_unavailable() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(false);
        let enter_result = section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(matches!(
            enter_result,
            crate::components::event_state::EventState::NotConsumed
        ));
        let right_result = section.handle_input(&key(KeyCode::Right), &tx);
        assert!(matches!(
            right_result,
            crate::components::event_state::EventState::NotConsumed
        ));
        let left_result = section.handle_input(&key(KeyCode::Left), &tx);
        assert!(matches!(
            left_result,
            crate::components::event_state::EventState::NotConsumed
        ));
        let l_result = section.handle_input(&key(KeyCode::Char('l')), &tx);
        assert!(matches!(
            l_result,
            crate::components::event_state::EventState::NotConsumed
        ));
        let h_result = section.handle_input(&key(KeyCode::Char('h')), &tx);
        assert!(matches!(
            h_result,
            crate::components::event_state::EventState::NotConsumed
        ));
        assert!(
            rx.try_recv().is_err(),
            "No messages should be sent when vault_available == false"
        );
    }

    #[test]
    fn set_vault_available_enables_keys() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(false);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(
            rx.try_recv().is_err(),
            "Enter should be blocked when unavailable"
        );
        section.set_vault_available(true);
        section.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx
            .try_recv()
            .expect("Enter should send message after enabling");
        assert!(matches!(msg, AppEvent::TriggerFastReindex));
    }

    #[test]
    fn right_cycles_fast_to_full() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        assert_eq!(section.selected, IndexAction::Fast);
        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.selected, IndexAction::Full);
    }

    #[test]
    fn left_cycles_full_to_fast() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_input(&key(KeyCode::Right), &tx);
        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.selected, IndexAction::Fast);
    }

    #[test]
    fn enter_on_fast_sends_trigger_fast_reindex() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppEvent::TriggerFastReindex));
    }

    #[test]
    fn enter_on_full_sends_trigger_full_reindex() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_input(&key(KeyCode::Right), &tx);
        assert!(rx.try_recv().is_err(), "Right should not send any message");
        section.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppEvent::TriggerFullReindex));
    }

    #[test]
    fn right_is_idempotent_when_already_full() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_input(&key(KeyCode::Right), &tx);
        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.selected, IndexAction::Full);
    }
}
