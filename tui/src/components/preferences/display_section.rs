use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use unicode_width::UnicodeWidthStr;

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::logic::message::AppTx;
use crate::logic::preferences::PreferencesModel;
use crate::settings::themes::Theme;

use super::{ClickMap, text_rect};

/// Number of selectable rows in this section.
const ROW_COUNT: usize = 4;
const ROW_LEADER_TIMEOUT: usize = 3;

/// The Display section: nerd fonts, update check, mouse capture (applies on
/// the next launch, the row says so) and the which-key delay. The values
/// live in the [`PreferencesModel`]; this holds the selection and the click
/// targets.
pub struct DisplaySection {
    list_state: ListState,
    clicks: ClickMap,
}

impl Default for DisplaySection {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplaySection {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            clicks: ClickMap::default(),
        }
    }

    /// Resolve a click: a row selects it, a checkbox toggles, `◀`/`▶` step
    /// the delay, clicking the selected row toggles it. Returns the key the
    /// click stands for, for the screen to run through its key path.
    pub fn handle_mouse(
        &mut self,
        m: &ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyEvent> {
        use ratatui::crossterm::event::KeyCode;
        let selected = self.list_state.selected();
        let activate = (selected != Some(ROW_LEADER_TIMEOUT)).then_some(KeyCode::Enter);
        self.clicks
            .resolve(m, selected, activate)
            .into_key(|row| self.list_state.select(Some(row)))
    }

    /// Toggle the currently selected row.
    fn toggle_selected(&self, model: &mut PreferencesModel) {
        match self.list_state.selected() {
            Some(0) => model.use_nerd_fonts = !model.use_nerd_fonts,
            Some(1) => model.update_check = !model.update_check,
            Some(2) => model.mouse = !model.mouse,
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let current = self.list_state.selected().unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(ROW_COUNT as isize);
        self.list_state.select(Some(next as usize));
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
        use ratatui::crossterm::event::KeyCode;
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.toggle_selected(model);
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
                model.step_leader_delay(false);
                EventState::Consumed
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.list_state.selected() == Some(ROW_LEADER_TIMEOUT) =>
            {
                model.step_leader_delay(true);
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
            .title("Display")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());

        use ratatui::crossterm::event::KeyCode;
        let checkbox = |on: bool| if on { "[x]" } else { "[ ]" };
        let fg = Style::default().fg(theme.fg.to_ratatui());
        let action = theme.action();
        let inner = block.inner(rect);
        // Controls per row, as (row, column, width, key) — mapped to screen
        // rects after rendering, once the list has settled its offset.
        let mut controls: Vec<(usize, u16, u16, KeyCode)> = Vec::new();
        let mut items = Vec::new();
        // Checkbox rows: label, then a clickable `[x]`.
        for (row, label, on) in [
            (0, "  Use Nerd Fonts  ", model.use_nerd_fonts),
            (1, "  Check for updates on startup  ", model.update_check),
            (2, "  Capture mouse (restart to apply)  ", model.mouse),
        ] {
            controls.push((row, label.width() as u16, 3, KeyCode::Enter));
            items.push(
                ListItem::new(Line::from(vec![
                    Span::raw(label),
                    Span::styled(checkbox(on), action),
                ]))
                .style(fg),
            );
        }
        // Stepper row: `◀` and `▶` step the delay.
        let label = "  Which-key Delay  ";
        let value = format!(" {}ms ", model.leader_timeout_ms);
        let left_col = label.width() as u16;
        let right_col = left_col + 1 + value.width() as u16;
        controls.push((ROW_LEADER_TIMEOUT, left_col, 1, KeyCode::Left));
        controls.push((ROW_LEADER_TIMEOUT, right_col, 1, KeyCode::Right));
        items.push(
            ListItem::new(Line::from(vec![
                Span::raw(label),
                Span::styled("◀", action),
                Span::raw(value),
                Span::styled("▶", action),
                Span::raw("  (←/→ to change)"),
            ]))
            .style(fg),
        );

        let list = List::new(items)
            .block(block)
            .style(theme.base_style())
            .highlight_style(
                Style::default()
                    .fg(theme.selection_fg.to_ratatui())
                    .bg(theme.selection_bg.to_ratatui()),
            );
        f.render_stateful_widget(list, rect, &mut self.list_state);

        // Rows map from the offset the list settled on while rendering.
        self.clicks.clear();
        let offset = self.list_state.offset();
        let row_rect = |row: usize| {
            let y = inner.y as usize + row.saturating_sub(offset);
            if row < offset || y >= inner.bottom() as usize {
                Rect::default()
            } else {
                Rect::new(inner.x, y as u16, inner.width, 1)
            }
        };
        for row in 0..ROW_COUNT {
            self.clicks.row(row_rect(row), row);
        }
        for (row, col, width, key) in controls {
            self.clicks
                .control(text_rect(row_rect(row), col, width), row, key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::InputEvent;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn model(nerd: bool, update: bool, mouse: bool, delay: u64) -> PreferencesModel {
        let mut m = PreferencesModel::new(&crate::settings::AppSettings::default());
        m.use_nerd_fonts = nerd;
        m.update_check = update;
        m.mouse = mouse;
        m.leader_timeout_ms = delay;
        m
    }

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
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 400);
        section.handle_input(&key(KeyCode::Enter), &mut m, &tx);
        assert!(!m.use_nerd_fonts);
        section.handle_input(&key(KeyCode::Enter), &mut m, &tx);
        assert!(m.use_nerd_fonts);
    }

    #[test]
    fn space_toggles_nerd_fonts() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(false, true, true, 400);
        section.handle_input(&key(KeyCode::Char(' ')), &mut m, &tx);
        assert!(m.use_nerd_fonts);
    }

    #[test]
    fn down_then_toggle_flips_update_check_only() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 400);
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Enter), &mut m, &tx);
        assert!(!m.update_check, "update_check should toggle off");
        assert!(m.use_nerd_fonts, "nerd fonts should be untouched");
    }

    #[test]
    fn down_twice_then_toggle_flips_mouse_only() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 400);
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Enter), &mut m, &tx);
        assert!(!m.mouse, "mouse should toggle off");
        assert!(m.use_nerd_fonts, "nerd fonts should be untouched");
        assert!(m.update_check, "update_check should be untouched");
    }

    #[test]
    fn down_three_times_then_right_increases_leader_timeout() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 400);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        }
        section.handle_input(&key(KeyCode::Right), &mut m, &tx);
        assert_eq!(m.leader_timeout_ms, 450);
    }

    #[test]
    fn left_decreases_leader_timeout() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 400);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        }
        section.handle_input(&key(KeyCode::Left), &mut m, &tx);
        assert_eq!(m.leader_timeout_ms, 350);
    }

    #[test]
    fn leader_timeout_clamps_at_zero() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 20);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        }
        section.handle_input(&key(KeyCode::Left), &mut m, &tx);
        assert_eq!(m.leader_timeout_ms, 0);
    }

    #[test]
    fn leader_timeout_clamps_at_max() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 1980);
        for _ in 0..3 {
            section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        }
        section.handle_input(&key(KeyCode::Right), &mut m, &tx);
        assert_eq!(m.leader_timeout_ms, 2000);
    }

    #[test]
    fn left_right_do_nothing_off_the_leader_timeout_row() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = DisplaySection::new();
        let mut m = model(true, true, true, 400);
        let state = section.handle_input(&key(KeyCode::Right), &mut m, &tx);
        assert_eq!(state, EventState::NotConsumed);
        assert_eq!(m.leader_timeout_ms, 400);
    }

    #[test]
    fn renders_leader_timeout_value() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = DisplaySection::new();
        let m = model(true, true, true, 400);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false, &m))
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
        let mut section = DisplaySection::new();
        let m = model(true, true, true, 400);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false, &m))
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
        let mut section = DisplaySection::new();
        let m = model(false, true, true, 400);
        let theme = Theme::gruvbox_dark();
        terminal
            .draw(|f| section.render(f, f.area(), &theme, false, &m))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let flat: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(
            flat.contains("[ ]"),
            "expected [ ] when nerd fonts disabled"
        );
    }
}
