use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::components::Component;
use crate::components::clickable::is_press_outside;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::settings::themes::Theme;

/// Popup width, border included.
const WIDTH: u16 = 58;
/// Horizontal margin between the popup border and its text.
const MARGIN: u16 = 2;

/// Read-only message dialog: an accent headline, a wrapped body, and
/// `[Esc] Close`. Esc or a click outside/on the hint closes; everything else
/// is swallowed. Height follows the wrapped body.
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
pub struct InfoDialog {
    title: String,
    headline: String,
    body: String,
    /// Outer popup rect from the last render; a press outside closes.
    popup_rect: Rect,
    close_hint: HintRow,
}

impl InfoDialog {
    pub fn new(
        title: impl Into<String>,
        headline: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            title: format!(" {} ", title.into()),
            headline: headline.into(),
            body: body.into(),
            popup_rect: Rect::default(),
            close_hint: HintRow::new(&[(KeyCode::Esc, "Esc", "Close")]),
        }
    }

    /// Rows the body takes once wrapped to the popup's text width.
    fn body_rows(&self) -> u16 {
        let text_w = (WIDTH - 2 - 2 * MARGIN) as usize;
        self.body
            .lines()
            .map(|l| l.width().div_ceil(text_w).max(1))
            .sum::<usize>() as u16
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

impl Component for InfoDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let body_rows = self.body_rows();
        // border(2) + spacer, headline, spacer, body, spacer, hint
        let popup_area = super::fixed_centered_rect(WIDTH, 2 + 5 + body_rows, rect);
        self.popup_rect = popup_area;

        let inner = modal_chrome(
            f,
            popup_area,
            theme,
            ModalSpec {
                title: Some(self.title.as_str()),
                border: Some(Style::default().fg(theme.accent.to_ratatui())),
                ..Default::default()
            },
        );

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .horizontal_margin(MARGIN)
            .constraints([
                Constraint::Length(1),         // spacer
                Constraint::Length(1),         // headline
                Constraint::Length(1),         // spacer
                Constraint::Length(body_rows), // body
                Constraint::Length(1),         // spacer
                Constraint::Length(1),         // Esc hint
                Constraint::Min(0),
            ])
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
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
            Paragraph::new(self.body.as_str())
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(theme.fg.to_ratatui()).bg(bg)),
            rows[3],
        );
        self.close_hint.render(
            f,
            rows[5],
            Style::default().fg(theme.gray.to_ratatui()).bg(bg),
            theme,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn esc_closes_and_other_keys_are_swallowed() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
        let mut d = InfoDialog::new("T", "head", "body");
        assert_eq!(
            d.handle_key(KeyEvent::from(KeyCode::Char('x')), &tx),
            EventState::Consumed
        );
        assert!(rx.try_recv().is_err());
        d.handle_key(KeyEvent::from(KeyCode::Esc), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
    }

    #[test]
    fn height_follows_wrapped_body() {
        let short = InfoDialog::new("T", "h", "one line");
        let long = InfoDialog::new("T", "h", "x".repeat(200));
        let multi = InfoDialog::new("T", "h", "a\nb");
        assert_eq!(short.body_rows(), 1);
        assert_eq!(long.body_rows(), 4); // 200 / 50
        assert_eq!(multi.body_rows(), 2);
    }
}
