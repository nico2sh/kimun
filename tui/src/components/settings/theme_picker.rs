use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::AppTx;
use crate::components::events::InputEvent;
use crate::settings::themes::Theme;

pub struct ThemePicker {
    themes: Vec<Theme>,
    list_state: ListState,
}

impl ThemePicker {
    pub fn new(themes: Vec<Theme>, active_name: &str) -> Self {
        let idx = themes
            .iter()
            .position(|t| t.name == active_name)
            .unwrap_or(0);
        let mut list_state = ListState::default();
        list_state.select(Some(idx));
        Self { themes, list_state }
    }

    pub fn selected_theme_name(&self) -> &str {
        debug_assert!(
            !self.themes.is_empty(),
            "ThemePicker requires at least one theme"
        );
        let idx = self.list_state.selected().unwrap_or(0);
        &self.themes[idx].name
    }
}

impl Component for ThemePicker {
    fn handle_event(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        let count = self.themes.len();
        match key.code {
            ratatui::crossterm::event::KeyCode::Down
            | ratatui::crossterm::event::KeyCode::Char('j') => {
                let cur = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((cur + 1) % count));
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Up
            | ratatui::crossterm::event::KeyCode::Char('k') => {
                let cur = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((cur + count - 1) % count));
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let block = Block::default()
            .title("Theme")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());
        let items: Vec<ListItem> = self
            .themes
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let selected = self.list_state.selected() == Some(i);
                let prefix = if selected { "● " } else { "  " };
                ListItem::new(format!("{}{}", prefix, t.name))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .style(theme.base_style())
            .highlight_style(
                ratatui::style::Style::default()
                    .fg(theme.fg_selected.to_ratatui())
                    .bg(theme.bg_selected.to_ratatui()),
            );
        f.render_stateful_widget(list, rect, &mut self.list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_themes() -> Vec<Theme> {
        vec![
            Theme::gruvbox_dark(),
            Theme::gruvbox_light(),
            Theme::catppuccin_mocha(),
        ]
    }

    #[test]
    fn selected_theme_name_returns_initial() {
        let picker = ThemePicker::new(make_themes(), "Gruvbox Light");
        assert_eq!(picker.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn down_moves_selection() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = InputEvent::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn up_wraps_from_first_to_last() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = InputEvent::Key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Catppuccin Mocha");
    }

    #[test]
    fn down_wraps_from_last_to_first() {
        use crate::components::events::InputEvent;
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Catppuccin Mocha");
        let key = InputEvent::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Gruvbox Dark");
    }

    #[test]
    fn j_key_moves_selection_down() {
        use crate::components::events::InputEvent;
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = InputEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn k_key_wraps_from_first_to_last() {
        use crate::components::events::InputEvent;
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = InputEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Catppuccin Mocha");
    }

    #[test]
    fn renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| {
                picker.render(f, f.area(), &theme, false);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let flat: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(
            flat.contains("Gruvbox Dark"),
            "Expected theme name in rendered output"
        );
    }
}
