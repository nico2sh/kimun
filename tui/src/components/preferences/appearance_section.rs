use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::logic::message::AppTx;
use crate::logic::preferences::PreferencesModel;
use crate::settings::themes::Theme;

use super::ClickMap;

/// The Appearance section: the theme list. The themes and the selection
/// live in the [`PreferencesModel`] (`themes`, `theme_idx`); this holds the
/// list's scroll state and the click targets.
#[derive(Default)]
pub struct AppearanceSection {
    list_state: ListState,
    clicks: ClickMap,
}

impl AppearanceSection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a click: a theme row selects (and so previews) it; the wheel
    /// steps through themes. Returns the key a wheel notch stands for; a
    /// row click changes the model's selection directly (the screen syncs
    /// it).
    pub fn handle_mouse(
        &mut self,
        m: &ratatui::crossterm::event::MouseEvent,
        model: &mut PreferencesModel,
    ) -> Option<ratatui::crossterm::event::KeyEvent> {
        // No controls and no re-click action: a row click only selects.
        self.clicks
            .resolve(m, Some(model.theme_idx), None)
            .into_key(|row| {
                model.select_theme(row);
            })
    }

    pub fn handle_input(
        &mut self,
        event: &InputEvent,
        model: &mut PreferencesModel,
        _tx: &AppTx,
    ) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match key.code {
            ratatui::crossterm::event::KeyCode::Down
            | ratatui::crossterm::event::KeyCode::Char('j') => {
                model.move_theme(1);
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Up
            | ratatui::crossterm::event::KeyCode::Char('k') => {
                model.move_theme(-1);
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    pub fn render(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        theme: &Theme,
        focused: bool,
        model: &PreferencesModel,
    ) {
        let border_style = theme.border_style(focused);
        let block = Block::default()
            .title("Appearance")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());
        self.list_state.select(Some(model.theme_idx));
        let items: Vec<ListItem> = model
            .themes
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let selected = model.theme_idx == i;
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
            .list_rows(inner, self.list_state.offset(), model.themes.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(active: &str) -> PreferencesModel {
        let mut m = PreferencesModel::new(&crate::settings::AppSettings::default());
        m.themes = vec![
            Theme::gruvbox_dark(),
            Theme::gruvbox_light(),
            Theme::catppuccin_mocha(),
        ];
        m.theme_idx = m.themes.iter().position(|t| t.name == active).unwrap();
        m
    }

    #[test]
    fn selected_theme_name_returns_initial() {
        let m = model("Gruvbox Light");
        assert_eq!(m.selected_theme().name, "Gruvbox Light");
    }

    #[test]
    fn down_moves_selection() {
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = AppearanceSection::new();
        let mut m = model("Gruvbox Dark");
        let key = crate::components::events::InputEvent::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        section.handle_input(&key, &mut m, &tx);
        assert_eq!(m.selected_theme().name, "Gruvbox Light");
        assert_eq!(m.theme_name, "Gruvbox Light");
    }

    #[test]
    fn renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = AppearanceSection::new();
        let m = model("Gruvbox Dark");
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false, &m))
            .unwrap();
    }
}
