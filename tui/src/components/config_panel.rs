//! The CFG **drawer view** as a component — the same shape as its sibling
//! views (Tags/Links/Outline), so `DrawerHost`'s dispatch stays uniform: one
//! arm per view, never inline view logic in the host (see adr/0023).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent};
use crate::components::panel::panel_block;
use crate::keys::leader::LeaderAction;
use crate::settings::themes::Theme;

/// What the CFG drawer view displays — resolved by the host screen when the
/// view opens (the panel itself holds no settings handle).
#[derive(Default, Clone)]
pub struct ConfigInfo {
    pub theme_name: String,
    pub leader_key: String,
    pub preferences_key: String,
    pub leader_timeout_ms: u64,
    pub config_path: String,
}

/// Read-only settings summary plus two launcher keys: `t`/Enter opens the
/// theme picker, `p` opens Preferences.
#[derive(Default)]
pub struct ConfigPanel {
    info: ConfigInfo,
}

impl ConfigPanel {
    /// Refresh what the view shows (called by the host when the view opens).
    pub fn set_info(&mut self, info: ConfigInfo) {
        self.info = info;
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![
            ("t/⏎".into(), "Theme picker".into()),
            ("p".into(), "Preferences".into()),
        ]
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        use ratatui::crossterm::event::KeyCode;
        if let InputEvent::Key(key) = event {
            match key.code {
                KeyCode::Char('t') | KeyCode::Enter => {
                    tx.send(AppEvent::ExecuteLeaderAction(LeaderAction::VaultTheme))
                        .ok();
                    return EventState::Consumed;
                }
                KeyCode::Char('p') => {
                    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenPreferences))
                        .ok();
                    return EventState::Consumed;
                }
                _ => {}
            }
        }
        EventState::NotConsumed
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let block = panel_block("Config", theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let info = &self.info;
        let label = Style::default().fg(theme.gray.to_ratatui());
        let value = Style::default().fg(theme.fg.to_ratatui());
        let keycap = Style::default().fg(theme.yellow.to_ratatui());
        let lines = vec![
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" theme    ", label),
                ratatui::text::Span::styled(info.theme_name.clone(), value),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" leader   ", label),
                ratatui::text::Span::styled(info.leader_key.clone(), value),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" prefs    ", label),
                ratatui::text::Span::styled(info.preferences_key.clone(), value),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" timeout  ", label),
                ratatui::text::Span::styled(
                    format!("{} ms (which-key reveal)", info.leader_timeout_ms),
                    value,
                ),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" config   ", label),
                ratatui::text::Span::styled(info.config_path.clone(), value),
            ]),
            ratatui::text::Line::default(),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" t ", keycap),
                ratatui::text::Span::styled("theme picker", label),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" p ", keycap),
                ratatui::text::Span::styled("preferences", label),
            ]),
        ];
        f.render_widget(Paragraph::new(lines), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn t_and_enter_open_the_theme_picker() {
        let (tx, mut rx) = unbounded_channel();
        let mut panel = ConfigPanel::default();
        for code in [KeyCode::Char('t'), KeyCode::Enter] {
            assert_eq!(panel.handle_input(&key(code), &tx), EventState::Consumed);
            assert!(matches!(
                rx.try_recv(),
                Ok(AppEvent::ExecuteLeaderAction(LeaderAction::VaultTheme))
            ));
        }
    }

    #[test]
    fn p_opens_preferences() {
        let (tx, mut rx) = unbounded_channel();
        let mut panel = ConfigPanel::default();
        assert_eq!(
            panel.handle_input(&key(KeyCode::Char('p')), &tx),
            EventState::Consumed
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(AppEvent::OpenScreen(ScreenEvent::OpenPreferences))
        ));
    }

    #[test]
    fn other_input_is_not_consumed() {
        let (tx, mut rx) = unbounded_channel();
        let mut panel = ConfigPanel::default();
        assert_eq!(
            panel.handle_input(&key(KeyCode::Char('x')), &tx),
            EventState::NotConsumed
        );
        assert_eq!(
            panel.handle_input(&InputEvent::Paste("hi".into()), &tx),
            EventState::NotConsumed
        );
        assert!(rx.try_recv().is_err(), "no event may be emitted");
    }
}
