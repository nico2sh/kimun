use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::themes::Theme;

pub struct DisplaySection {
    pub use_nerd_fonts: bool,
    list_state: ListState,
}

impl DisplaySection {
    pub fn new(use_nerd_fonts: bool) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            use_nerd_fonts,
            list_state,
        }
    }
}

impl Component for DisplaySection {
    fn handle_input(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match key.code {
            ratatui::crossterm::event::KeyCode::Enter
            | ratatui::crossterm::event::KeyCode::Char(' ') => {
                self.use_nerd_fonts = !self.use_nerd_fonts;
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let block = Block::default()
            .title("Display")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());

        let check = if self.use_nerd_fonts { "[x]" } else { "[ ]" };
        let items = vec![ListItem::new(format!("  Use Nerd Fonts  {}", check))
            .style(Style::default().fg(theme.fg.to_ratatui()))];

        let list = List::new(items)
            .block(block)
            .style(theme.base_style())
            .highlight_style(
                Style::default()
                    .fg(theme.fg_selected.to_ratatui())
                    .bg(theme.bg_selected.to_ratatui()),
            );
        f.render_stateful_widget(list, rect, &mut self.list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::InputEvent;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn enter_toggles_nerd_fonts() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(!section.use_nerd_fonts);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(section.use_nerd_fonts);
    }

    #[test]
    fn space_toggles_nerd_fonts() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(false);
        section.handle_input(&key(KeyCode::Char(' ')), &tx);
        assert!(section.use_nerd_fonts);
    }

    #[test]
    fn renders_checked_when_enabled() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = DisplaySection::new(true);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("[x]"), "expected [x] when nerd fonts enabled");
    }

    #[test]
    fn renders_unchecked_when_disabled() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = DisplaySection::new(false);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("[ ]"), "expected [ ] when nerd fonts disabled");
    }
}
