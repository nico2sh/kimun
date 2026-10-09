use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Paragraph, Wrap};

use crate::components::Component;
use crate::components::clickable::is_press_outside;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::server_client::sync::ServerUpdate;
use crate::settings::themes::Theme;

/// Informational dialog opened from the footer's `rag: server update` segment.
/// The TUI cannot tell how the server was installed, so it only points the
/// user at the method they used.
///
/// ```text
/// ┌─ Server Update ──────────────────────────────────────┐
/// │                                                      │
/// │  server 0.18.0 available                             │
/// │                                                      │
/// │  Update it the same way you installed it (install    │
/// │  script, Docker image, package manager, ...).        │
/// │                                                      │
/// │  [Esc] Close                                         │
/// └──────────────────────────────────────────────────────┘
/// ```
pub struct ServerUpdateDialog {
    headline: String,
    /// Outer popup rect from the last render; a press outside closes.
    popup_rect: Rect,
    close_hint: HintRow,
}

impl ServerUpdateDialog {
    pub fn new(update: &ServerUpdate) -> Self {
        Self {
            headline: update.summary(),
            popup_rect: Rect::default(),
            close_hint: HintRow::new(&[(KeyCode::Esc, "Esc", "Close")]),
        }
    }

    /// Modal: every mouse event is consumed; a press outside (or on the
    /// close hint) closes.
    pub fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        if is_press_outside(m, self.popup_rect) {
            return self.handle_key(KeyEvent::from(KeyCode::Esc), tx);
        }
        if let Some(key) = self.close_hint.hit(m) {
            return self.handle_key(key, tx);
        }
        EventState::Consumed
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        if key.code == KeyCode::Esc {
            tx.send(AppEvent::CloseOverlay).ok();
        }
        EventState::Consumed // swallow other keys while open
    }
}

impl Component for ServerUpdateDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let popup_area = super::fixed_centered_rect(58, 10, rect);
        self.popup_rect = popup_area;

        let inner = modal_chrome(
            f,
            popup_area,
            theme,
            ModalSpec {
                title: Some(" Server Update "),
                border: Some(Style::default().fg(theme.accent.to_ratatui())),
                ..Default::default()
            },
        );

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .horizontal_margin(2)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: headline
                Constraint::Length(1), // 2: spacer
                Constraint::Length(2), // 3: how to update
                Constraint::Length(1), // 4: spacer
                Constraint::Length(1), // 5: Esc hint
                Constraint::Min(0),
            ])
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();

        f.render_widget(
            Paragraph::new(self.headline.as_str()).style(
                Style::default()
                    .fg(theme.accent.to_ratatui())
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            rows[1],
        );
        f.render_widget(
            Paragraph::new(
                "Update it the same way you installed it (install script, Docker image, package manager, …).",
            )
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(fg).bg(bg)),
            rows[3],
        );
        self.close_hint
            .render(f, rows[5], Style::default().fg(gray).bg(bg), theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn esc_closes_and_other_keys_are_swallowed() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
        let mut d = ServerUpdateDialog::new(&ServerUpdate::Legacy);
        assert_eq!(
            d.handle_key(KeyEvent::from(KeyCode::Char('x')), &tx),
            EventState::Consumed
        );
        assert!(rx.try_recv().is_err());
        d.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
    }
}
