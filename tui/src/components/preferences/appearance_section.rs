use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::themes::Theme;

use super::ClickMap;

pub struct AppearanceSection {
    themes: Vec<Theme>,
    list_state: ListState,
    clicks: ClickMap,
}

impl AppearanceSection {
    pub fn new(themes: Vec<Theme>, active_name: &str) -> Self {
        let idx = themes
            .iter()
            .position(|t| t.name == active_name)
            .unwrap_or(0);
        let mut list_state = ListState::default();
        list_state.select(Some(idx));
        Self {
            themes,
            list_state,
            clicks: ClickMap::default(),
        }
    }

    /// Resolve a click: a theme row selects (and so previews) it; the wheel
    /// steps through themes. Returns the key a wheel notch stands for; a
    /// row click changes the selection directly (the screen syncs it).
    pub fn handle_mouse(
        &mut self,
        m: &ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyEvent> {
        // No controls and no re-click action: a row click only selects.
        self.clicks
            .resolve(m, self.list_state.selected(), None)
            .into_key(|row| self.list_state.select(Some(row)))
    }

    pub fn selected_theme_name(&self) -> &str {
        &self.selected_theme().name
    }

    /// The currently highlighted theme. Cheaper than resolving the name
    /// through `AppSettings::get_theme()`, which rebuilds the whole theme
    /// list (including custom-theme disk reads) on every call.
    pub fn selected_theme(&self) -> &Theme {
        debug_assert!(
            !self.themes.is_empty(),
            "AppearanceSection requires at least one theme"
        );
        let idx = self.list_state.selected().unwrap_or(0);
        &self.themes[idx]
    }
}

impl Component for AppearanceSection {
    fn handle_input(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
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
            .title("Appearance")
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
        // Rows are hit-tested inside the block actually drawn.
        let inner = block.inner(rect);
        let list = List::new(items)
            .block(block)
            .style(theme.base_style())
            .highlight_style(
                ratatui::style::Style::default()
                    .fg(theme.selection_fg.to_ratatui())
                    .bg(theme.selection_bg.to_ratatui()),
            );
        f.render_stateful_widget(list, rect, &mut self.list_state);
        // After rendering: the list may have scrolled to keep the selection
        // in view, and rows map from the offset it settled on.
        self.clicks.clear();
        self.clicks
            .list_rows(inner, self.list_state.offset(), self.themes.len());
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
        let section = AppearanceSection::new(make_themes(), "Gruvbox Light");
        assert_eq!(section.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn down_moves_selection() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = AppearanceSection::new(make_themes(), "Gruvbox Dark");
        let key = crate::components::events::InputEvent::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        section.handle_input(&key, &tx);
        assert_eq!(section.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = AppearanceSection::new(make_themes(), "Gruvbox Dark");
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false))
            .unwrap();
    }
}
