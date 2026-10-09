use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, FileOp, OverlayData};
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

pub struct QuickNoteModal {
    input: SingleLineInput,
    vault: Arc<NoteVault>,
    pub error: Option<String>,
    save_hints: HintRow,
    cancel_hint: HintRow,
}

impl QuickNoteModal {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self {
            input: SingleLineInput::new(),
            vault,
            error: None,
            save_hints: HintRow::new(&[
                (KeyCode::Enter, "Enter", "Save"),
                (KeyCode::Enter, "Shift+Enter", "Save & Open"),
            ])
            .with_modifiers(1, KeyModifiers::SHIFT),
            cancel_hint: HintRow::new(&[(KeyCode::Esc, "Esc", "Cancel")]),
        }
    }

    /// A click on a hint chip runs its key. Modal: every mouse event is
    /// consumed.
    pub fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        if let Some(key) = self.save_hints.hit(m).or_else(|| self.cancel_hint.hit(m)) {
            self.handle_key(key, tx);
        }
        EventState::Consumed
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
                    // quick_note always materialises a fresh note — tell the
                    // sidebar so it refreshes if browsing that directory.
                    tx_clone
                        .send(AppEvent::FileOp(FileOp::Created(details.path.clone())))
                        .ok();
                    if open_after {
                        tx_clone.send(AppEvent::open(details.path)).ok();
                    } else {
                        tx_clone.send(AppEvent::CloseOverlay).ok();
                    }
                }
                Err(e) => {
                    tx_clone
                        .send(AppEvent::OverlayData(OverlayData::Error(e.to_string())))
                        .ok();
                }
            }
        });
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let height = if self.error.is_some() { 9 } else { 8 };
        let popup_area = super::fixed_centered_rect(62, height, rect);

        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let inner = modal_chrome(
            f,
            popup_area,
            theme,
            ModalSpec {
                title: Some(" Quick Note "),
                border: Some(Style::default().fg(theme.focus_border.to_ratatui())),
                ..Default::default()
            },
        );

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
                Paragraph::new("  Type your thought...").style(Style::default().fg(gray).bg(bg)),
                rows[1],
            );
            f.set_cursor_position((rows[1].x + 2, rows[1].y));
        } else {
            // 2-space indent matches the placeholder above.
            self.input
                .render(f, rows[1], Style::default().fg(fg).bg(bg), 2, true);
        }

        super::render_separator(f, rows[2], gray, bg);

        let hint_style = Style::default().fg(gray).bg(bg);
        self.save_hints.render(f, rows[3], hint_style, theme);
        self.cancel_hint.render(f, rows[4], hint_style, theme);

        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[5], msg, theme);
        }
    }
}

impl_dialog!(QuickNoteModal, error);
