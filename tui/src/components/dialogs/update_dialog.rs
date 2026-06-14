use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::settings::themes::Theme;
use crate::update::UpdateStatus;

const RELEASES_URL: &str = "https://github.com/nico2sh/kimun/releases";

/// Dialog shown when a newer release is available. On self-update-eligible
/// channels it offers an in-place update; otherwise it shows the package
/// manager's upgrade command. Either way the user can skip the version.
///
/// ```text
/// ┌─ Update Available ───────────────────────────────────┐
/// │                                                      │
/// │  kimün 0.17.0  →  0.18.0                             │
/// │                                                      │
/// │  [U] Update now      [S] Skip this version           │
/// │                                                      │
/// │  Release notes: https://github.com/nico2sh/kimun/... │
/// │  [Esc] Close                                          │
/// └──────────────────────────────────────────────────────┘
/// ```
pub struct UpdateAvailableDialog {
    current: String,
    latest: String,
    /// Whether this channel can self-update in place.
    eligible: bool,
    /// Upgrade command for package-manager channels (e.g. `brew upgrade kimun`).
    upgrade_hint: Option<String>,
}

impl UpdateAvailableDialog {
    pub fn new(status: &UpdateStatus) -> Self {
        Self {
            current: status.current.clone(),
            latest: status.latest.clone(),
            eligible: status.channel.self_update_eligible(),
            upgrade_hint: status.channel.upgrade_hint().map(str::to_string),
        }
    }

    pub fn handle_key(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        tx: &AppTx,
    ) -> EventState {
        match key.code {
            KeyCode::Char('u') | KeyCode::Char('U') if self.eligible => {
                tx.send(AppEvent::ApplyUpdate).ok();
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                tx.send(AppEvent::DismissUpdate(self.latest.clone())).ok();
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            KeyCode::Esc => {
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            _ => EventState::Consumed, // swallow other keys while open
        }
    }
}

impl Component for UpdateAvailableDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let popup_area = super::fixed_centered_rect(58, 11, rect);

        let inner = modal_chrome(
            f,
            popup_area,
            theme,
            ModalSpec {
                title: Some(" Update Available "),
                border: Some(Style::default().fg(theme.accent.to_ratatui())),
                ..Default::default()
            },
        );

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: version line
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: action row
                Constraint::Length(1), // 4: spacer
                Constraint::Length(1), // 5: release notes / hint
                Constraint::Length(1), // 6: Esc hint
                Constraint::Min(0),    // 7: remainder
            ])
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();
        let key_fg = theme.selection_fg.to_ratatui();
        let accent = theme.accent.to_ratatui();

        // Row 1: version transition.
        f.render_widget(
            Paragraph::new(format!("  kimün {}  →  {}", self.current, self.latest)).style(
                Style::default()
                    .fg(accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            rows[1],
        );

        // Row 2: separator.
        super::render_separator(f, rows[2], gray, bg);

        // Row 3: actions.
        let key_style = Style::default()
            .fg(key_fg)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        let label_style = Style::default().fg(fg).bg(bg);
        if self.eligible {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(24), Constraint::Min(1)])
                .split(rows[3]);
            render_action(f, cols[0], "  [U]", " Update now", key_style, label_style);
            render_action(f, cols[1], "[S]", " Skip this version", key_style, label_style);
        } else {
            // Package-manager channel: show the upgrade command instead.
            let hint = self
                .upgrade_hint
                .clone()
                .unwrap_or_else(|| "Download the latest release manually.".to_string());
            f.render_widget(
                Paragraph::new(format!("  Run: {hint}")).style(label_style),
                rows[3],
            );
            render_action(
                f,
                rows[4],
                "  [S]",
                " Skip this version",
                key_style,
                label_style,
            );
        }

        // Row 5: release notes URL.
        f.render_widget(
            Paragraph::new(format!("  Release notes: {RELEASES_URL}"))
                .style(Style::default().fg(gray).bg(bg)),
            rows[5],
        );

        // Row 6: close hint.
        f.render_widget(
            Paragraph::new("  [Esc] Close").style(Style::default().fg(gray).bg(bg)),
            rows[6],
        );
    }
}

fn render_action(
    f: &mut Frame,
    area: Rect,
    key: &str,
    label: &str,
    key_style: Style,
    label_style: Style,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(key.len() as u16),
            Constraint::Min(1),
        ])
        .split(area);
    f.render_widget(Paragraph::new(key.to_string()).style(key_style), chunks[0]);
    f.render_widget(Paragraph::new(label.to_string()).style(label_style), chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::InstallChannel;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
    use tokio::sync::mpsc;

    fn status(channel: InstallChannel) -> UpdateStatus {
        UpdateStatus {
            current: "0.17.0".into(),
            latest: "0.18.0".into(),
            channel,
            update_available: true,
            dismissed: false,
        }
    }

    #[test]
    fn skip_sends_dismiss_and_close() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
        let mut d = UpdateAvailableDialog::new(&status(InstallChannel::Direct));
        let state = d.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE), &tx);
        assert_eq!(state, EventState::Consumed);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::DismissUpdate(v)) if v == "0.18.0"));
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
    }

    #[test]
    fn update_now_only_on_eligible_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
        // Eligible: 'u' applies.
        let mut d = UpdateAvailableDialog::new(&status(InstallChannel::Script));
        d.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::ApplyUpdate)));

        // Not eligible: 'u' is swallowed, no apply.
        let (tx2, mut rx2) = mpsc::unbounded_channel::<AppEvent>();
        let mut d2 = UpdateAvailableDialog::new(&status(InstallChannel::Brew));
        let state = d2.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE), &tx2);
        assert_eq!(state, EventState::Consumed);
        assert!(rx2.try_recv().is_err());
    }
}
