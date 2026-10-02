use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::themes::Theme;

/// Number of selectable rows in this section.
const ROW_COUNT: usize = 4;
const ROW_LEADER_TIMEOUT: usize = 3;

const MAX_LEADER_TIMEOUT_MS: u64 = 2000;
const LEADER_TIMEOUT_STEP: u64 = 50;

pub struct DisplaySection {
    pub use_nerd_fonts: bool,
    /// Whether kimün checks GitHub for a newer release on startup.
    pub update_check: bool,
    /// Whether kimün captures the mouse for in-app use. Read only at startup, so
    /// toggling here applies on the next launch (the row says so).
    pub mouse: bool,
    /// Hesitation before the which-key overlay reveals itself. 0 shows it
    /// the instant the leader sequence arms.
    pub leader_timeout_ms: u64,
    list_state: ListState,
}

impl DisplaySection {
    pub fn new(
        use_nerd_fonts: bool,
        update_check: bool,
        mouse: bool,
        leader_timeout_ms: u64,
    ) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            use_nerd_fonts,
            update_check,
            mouse,
            leader_timeout_ms,
            list_state,
        }
    }

    /// Toggle the currently selected row.
    fn toggle_selected(&mut self) {
        match self.list_state.selected() {
            Some(0) => self.use_nerd_fonts = !self.use_nerd_fonts,
            Some(1) => self.update_check = !self.update_check,
            Some(2) => self.mouse = !self.mouse,
            _ => {}
        }
    }

    fn adjust_leader_timeout(&mut self, increase: bool) {
        self.leader_timeout_ms = if increase {
            (self.leader_timeout_ms + LEADER_TIMEOUT_STEP).min(MAX_LEADER_TIMEOUT_MS)
        } else {
            self.leader_timeout_ms.saturating_sub(LEADER_TIMEOUT_STEP)
        };
    }

    fn move_selection(&mut self, delta: isize) {
        let current = self.list_state.selected().unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(ROW_COUNT as isize);
        self.list_state.select(Some(next as usize));
    }
}

impl Component for DisplaySection {
    fn handle_input(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        use ratatui::crossterm::event::KeyCode;
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.toggle_selected();
                EventState::Consumed
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                EventState::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                EventState::Consumed
            }
            KeyCode::Left | KeyCode::Char('h')
                if self.list_state.selected() == Some(ROW_LEADER_TIMEOUT) =>
            {
                self.adjust_leader_timeout(false);
                EventState::Consumed
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.list_state.selected() == Some(ROW_LEADER_TIMEOUT) =>
            {
                self.adjust_leader_timeout(true);
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

        let checkbox = |on: bool| if on { "[x]" } else { "[ ]" };
        let fg = Style::default().fg(theme.fg.to_ratatui());
        let items = vec![
            ListItem::new(format!(
                "  Use Nerd Fonts  {}",
                checkbox(self.use_nerd_fonts)
            ))
            .style(fg),
            ListItem::new(format!(
                "  Check for updates on startup  {}",
                checkbox(self.update_check)
            ))
            .style(fg),
            ListItem::new(format!(
                "  Capture mouse (restart to apply)  {}",
                checkbox(self.mouse)
            ))
            .style(fg),
            ListItem::new(format!(
                "  Which-key Delay  ◀ {}ms ▶  (←/→ to change)",
                self.leader_timeout_ms
            ))
            .style(fg),
        ];

        let list = List::new(items)
            .block(block)
            .style(theme.base_style())
            .highlight_style(
                Style::default()
                    .fg(theme.selection_fg.to_ratatui())
                    .bg(theme.selection_bg.to_ratatui()),
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
        let mut section = DisplaySection::new(true, true, true, 400);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(!section.use_nerd_fonts);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(section.use_nerd_fonts);
    }

    #[test]
    fn space_toggles_nerd_fonts() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(false, true, true, 400);
        section.handle_input(&key(KeyCode::Char(' ')), &tx);
        assert!(section.use_nerd_fonts);
    }

    #[test]
    fn down_then_toggle_flips_update_check_only() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true, true, true, 400);
        section.handle_input(&key(KeyCode::Down), &tx);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(!section.update_check, "update_check should toggle off");
        assert!(section.use_nerd_fonts, "nerd fonts should be untouched");
    }

    #[test]
    fn down_twice_then_toggle_flips_mouse_only() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true, true, true, 400);
        section.handle_input(&key(KeyCode::Down), &tx);
        section.handle_input(&key(KeyCode::Down), &tx);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(!section.mouse, "mouse should toggle off");
        assert!(section.use_nerd_fonts, "nerd fonts should be untouched");
        assert!(section.update_check, "update_check should be untouched");
    }

    #[test]
    fn down_three_times_then_right_increases_leader_timeout() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true, true, true, 400);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &tx);
        }
        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.leader_timeout_ms, 450);
    }

    #[test]
    fn left_decreases_leader_timeout() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true, true, true, 400);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &tx);
        }
        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.leader_timeout_ms, 350);
    }

    #[test]
    fn leader_timeout_clamps_at_zero() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true, true, true, 20);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &tx);
        }
        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.leader_timeout_ms, 0);
    }

    #[test]
    fn leader_timeout_clamps_at_max() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true, true, true, 1980);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &tx);
        }
        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.leader_timeout_ms, 2000);
    }

    #[test]
    fn left_right_do_nothing_off_the_leader_timeout_row() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new(true, true, true, 400);
        let state = section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(state, EventState::NotConsumed);
        assert_eq!(section.leader_timeout_ms, 400);
    }

    #[test]
    fn renders_leader_timeout_value() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = DisplaySection::new(true, true, true, 400);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("400ms"), "expected the delay value: {flat}");
    }

    #[test]
    fn renders_checked_when_enabled() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = DisplaySection::new(true, true, true, 400);
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
        let mut section = DisplaySection::new(false, true, true, 400);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(
            flat.contains("[ ]"),
            "expected [ ] when nerd fonts disabled"
        );
    }
}
