use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

pub struct QuickNoteModal {
    input: SingleLineInput,
    vault: Arc<NoteVault>,
    pub error: Option<String>,
}

impl QuickNoteModal {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self {
            input: SingleLineInput::new(),
            vault,
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        // Enter — possibly with Shift to open the new note after creating it.
        if let KeyCode::Enter = key.code {
            if self.input.value().trim().is_empty() {
                tx.send(AppEvent::CloseOverlay).ok();
            } else {
                self.submit(tx, key.modifiers.contains(KeyModifiers::SHIFT));
            }
            return EventState::Consumed;
        }
        match self.input.handle_key(&key) {
            InputOutcome::Cancel => {
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            InputOutcome::Changed => {
                self.error = None;
                EventState::Consumed
            }
            InputOutcome::Consumed | InputOutcome::Submit => EventState::Consumed,
            InputOutcome::NotConsumed => EventState::NotConsumed,
        }
    }

    fn submit(&self, tx: &AppTx, open_after: bool) {
        let text = self.input.value().to_string();
        let vault = Arc::clone(&self.vault);
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            match vault.quick_note(&text).await {
                Ok(details) => {
                    if open_after {
                        tx_clone.send(AppEvent::EntryCreated(details.path)).ok();
                    } else {
                        tx_clone.send(AppEvent::CloseOverlay).ok();
                    }
                }
                Err(e) => {
                    tx_clone.send(AppEvent::DialogError(e.to_string())).ok();
                }
            }
        });
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let height = if self.error.is_some() { 9 } else { 8 };
        let popup_area = super::fixed_centered_rect(62, height, rect);

        f.render_widget(Clear, popup_area);

        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let outer_block = Block::default()
            .title(" Quick Note ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused.to_ratatui()))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: input
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: hint line 1
                Constraint::Length(1), // 4: hint line 2
                Constraint::Length(1), // 5: error (optional)
                Constraint::Min(0),    // 6: remainder
            ])
            .split(inner);

        if self.input.is_empty() {
            // Placeholder text + caret in muted style.
            f.render_widget(
                Paragraph::new("  Type your thought...")
                    .style(Style::default().fg(fg_muted).bg(bg)),
                rows[1],
            );
            f.set_cursor_position((rows[1].x + 2, rows[1].y));
        } else {
            // 2-space indent matches the placeholder above.
            self.input
                .render(f, rows[1], Style::default().fg(fg).bg(bg), 2, true);
        }

        super::render_separator(f, rows[2], fg_muted, bg);

        f.render_widget(
            Paragraph::new("  [Enter] Save  [Shift+Enter] Save & Open")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );
        f.render_widget(
            Paragraph::new("  [Esc] Cancel").style(Style::default().fg(fg_muted).bg(bg)),
            rows[4],
        );

        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[5], msg, bg);
        }
    }
}
