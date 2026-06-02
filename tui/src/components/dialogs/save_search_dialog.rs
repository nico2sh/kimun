use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

pub struct SaveSearchDialog {
    /// The query being saved (read-only context).
    pub query: String,
    /// User-supplied name for the saved search.
    name: SingleLineInput,
}

impl SaveSearchDialog {
    pub fn new(query: String) -> Self {
        Self {
            query,
            name: SingleLineInput::new(),
        }
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match self.name.handle_key(key) {
            InputOutcome::Submit => {
                let name = if self.name.value().trim().is_empty() {
                    self.query.clone()
                } else {
                    self.name.value().to_string()
                };
                tx.send(AppEvent::SaveSearchConfirmed {
                    name,
                    query: self.query.clone(),
                })
                .ok();
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            InputOutcome::Cancel => {
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            InputOutcome::Changed | InputOutcome::Consumed => EventState::Consumed,
            InputOutcome::NotConsumed => EventState::NotConsumed,
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let popup_area = super::fixed_centered_rect(62, 9, rect);

        f.render_widget(Clear, popup_area);

        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let outer_block = Block::default()
            .title(" Save search ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(fg_muted))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: query (read-only context)
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: name input
                Constraint::Length(1), // 4: spacer
                Constraint::Length(1), // 5: hint
                Constraint::Min(0),    // 6: remainder
            ])
            .split(inner);

        // Row 1: read-only query context in muted style.
        f.render_widget(
            Paragraph::new(format!("  Query: {}", self.query))
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[1],
        );

        super::render_separator(f, rows[2], fg_muted, bg);

        // Row 3: name input with a "Name: " prefix.
        let prefix = "  Name: ";
        let prefix_len = prefix.len() as u16;
        f.render_widget(
            Paragraph::new(prefix).style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );
        self.name
            .render(f, rows[3], Style::default().fg(fg).bg(bg), prefix_len, true);

        // Row 5: hints.
        f.render_widget(
            Paragraph::new("  [Enter] Save   [Esc] Cancel")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[5],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::{AppEvent, InputEvent};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn submit_emits_save_event_with_typed_name() {
        let mut d = SaveSearchDialog::new("<{note}".to_string());
        let (tx, mut rx) = unbounded_channel();
        for ch in ['l', 'i', 'n', 'k', 's'] {
            d.handle_input(&key(KeyCode::Char(ch)), &tx);
        }
        d.handle_input(&key(KeyCode::Enter), &tx);
        // Drain events; find the SaveSearchConfirmed.
        let mut found = None;
        while let Ok(e) = rx.try_recv() {
            if let AppEvent::SaveSearchConfirmed { name, query } = e {
                found = Some((name, query));
            }
        }
        let (name, query) = found.expect("SaveSearchConfirmed emitted");
        assert_eq!(name, "links");
        assert_eq!(query, "<{note}");
    }

    #[test]
    fn submit_empty_name_falls_back_to_query() {
        let mut d = SaveSearchDialog::new("#todo".to_string());
        let (tx, mut rx) = unbounded_channel();
        d.handle_input(&key(KeyCode::Enter), &tx);
        let mut found = None;
        while let Ok(e) = rx.try_recv() {
            if let AppEvent::SaveSearchConfirmed { name, query } = e {
                found = Some((name, query));
            }
        }
        let (name, query) = found.expect("emitted");
        assert_eq!(name, "#todo"); // empty → query used as name
        assert_eq!(query, "#todo");
    }
}
