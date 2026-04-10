use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

pub struct QuickNoteModal {
    input: String,
    cursor: usize,
    vault: Arc<NoteVault>,
    pub error: Option<String>,
}

impl QuickNoteModal {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            vault,
            error: None,
        }
    }

    /// Number of visible characters before the cursor (for display positioning).
    fn cursor_char_offset(&self) -> usize {
        self.input[..self.cursor].chars().count()
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match (key.modifiers, key.code) {
            (m, KeyCode::Enter) if m.contains(KeyModifiers::SHIFT) => {
                if self.input.trim().is_empty() {
                    tx.send(AppEvent::CloseDialog).ok();
                } else {
                    self.submit(tx, true);
                }
                EventState::Consumed
            }
            (_, KeyCode::Enter) => {
                if self.input.trim().is_empty() {
                    tx.send(AppEvent::CloseDialog).ok();
                } else {
                    self.submit(tx, false);
                }
                EventState::Consumed
            }
            (_, KeyCode::Esc) => {
                tx.send(AppEvent::CloseDialog).ok();
                EventState::Consumed
            }
            (_, KeyCode::Backspace) => {
                self.error = None;
                if self.cursor > 0 {
                    // Move cursor back to the previous char boundary
                    let prev = self.input[..self.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.input.drain(prev..self.cursor);
                    self.cursor = prev;
                }
                EventState::Consumed
            }
            (_, KeyCode::Delete) => {
                if self.cursor < self.input.len() {
                    // Find the end of the current char
                    let next = self.input[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.input.len());
                    self.input.drain(self.cursor..next);
                }
                EventState::Consumed
            }
            (_, KeyCode::Left) => {
                if self.cursor > 0 {
                    self.cursor = self.input[..self.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
                EventState::Consumed
            }
            (_, KeyCode::Right) => {
                if self.cursor < self.input.len() {
                    self.cursor = self.input[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.input.len());
                }
                EventState::Consumed
            }
            (_, KeyCode::Home) => {
                self.cursor = 0;
                EventState::Consumed
            }
            (_, KeyCode::End) => {
                self.cursor = self.input.len();
                EventState::Consumed
            }
            (_, KeyCode::Char(c)) => {
                self.error = None;
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn submit(&self, tx: &AppTx, open_after: bool) {
        let text = self.input.clone();
        let vault = Arc::clone(&self.vault);
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            match vault.quick_note(&text).await {
                Ok(details) => {
                    if open_after {
                        tx_clone.send(AppEvent::EntryCreated(details.path)).ok();
                    } else {
                        tx_clone.send(AppEvent::CloseDialog).ok();
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

        let display_text = if self.input.is_empty() {
            "  Type your thought...".to_string()
        } else {
            format!("  {}", self.input)
        };
        let input_style = if self.input.is_empty() {
            Style::default().fg(fg_muted).bg(bg)
        } else {
            Style::default().fg(fg).bg(bg)
        };
        f.render_widget(Paragraph::new(display_text).style(input_style), rows[1]);

        // Place cursor (use char count for display, not byte offset)
        let cursor_x = rows[1].x + 2 + self.cursor_char_offset() as u16;
        let cursor_y = rows[1].y;
        f.set_cursor_position((cursor_x, cursor_y));

        super::render_separator(f, rows[2], fg_muted, bg);

        f.render_widget(
            Paragraph::new("  [Enter] Save  [Shift+Enter] Save & Open")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );
        f.render_widget(
            Paragraph::new("  [Esc] Cancel")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[4],
        );

        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[5], msg, bg);
        }
    }
}
