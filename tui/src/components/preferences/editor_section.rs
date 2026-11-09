use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::logic::message::AppTx;
use crate::logic::preferences::{PreferencesModel, backend_label};
use crate::settings::themes::Theme;

use super::{ClickMap, text_rect};

const ROW_AUTOSAVE: usize = 0;
const ROW_BACKEND: usize = 1;
const ROW_COUNT: usize = 2;

/// The Editor section: autosave interval and editor backend. The values
/// live in the [`PreferencesModel`] (ranges, steps and the backend cycle
/// with them); this holds the selection and the click targets.
pub struct EditorSection {
    selected_row: usize,
    clicks: ClickMap,
}

impl Default for EditorSection {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorSection {
    pub fn new() -> Self {
        Self {
            selected_row: ROW_AUTOSAVE,
            clicks: ClickMap::default(),
        }
    }

    /// Resolve a click: a setting's label or value selects it, `◀`/`▶`
    /// step it, re-clicking the backend cycles it. Returns the key the
    /// click stands for.
    pub fn handle_mouse(
        &mut self,
        m: &ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyEvent> {
        use ratatui::crossterm::event::KeyCode;
        let activate = (self.selected_row == ROW_BACKEND).then_some(KeyCode::Enter);
        self.clicks
            .resolve(m, Some(self.selected_row), activate)
            .into_key(|row| self.selected_row = row)
    }

    /// `  ◀  value  ▶` with both arrows as click targets for `row`.
    fn stepper(&mut self, r: Rect, row: usize, value: &str, theme: &Theme) -> Line<'static> {
        use ratatui::crossterm::event::KeyCode;
        let right_col = format!("  ◀  {value}  ").width() as u16;
        self.clicks.control(text_rect(r, 2, 1), row, KeyCode::Left);
        self.clicks
            .control(text_rect(r, right_col, 1), row, KeyCode::Right);
        Line::from(vec![
            Span::raw("  "),
            Span::styled("◀", theme.action()),
            Span::raw(format!("  {value}  ")),
            Span::styled("▶", theme.action()),
            Span::raw("   (←/→ to change)"),
        ])
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
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_row = (self.selected_row + ROW_COUNT - 1) % ROW_COUNT;
                EventState::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected_row = (self.selected_row + 1) % ROW_COUNT;
                EventState::Consumed
            }
            KeyCode::Left | KeyCode::Char('h') => {
                match self.selected_row {
                    ROW_AUTOSAVE => model.step_autosave(false),
                    _ => model.cycle_backend(false),
                }
                EventState::Consumed
            }
            KeyCode::Right | KeyCode::Char('l') => {
                match self.selected_row {
                    ROW_AUTOSAVE => model.step_autosave(true),
                    _ => model.cycle_backend(true),
                }
                EventState::Consumed
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.selected_row == ROW_BACKEND => {
                model.cycle_backend(true);
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
            .title("Editor")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // autosave label
                Constraint::Length(1), // autosave value
                Constraint::Length(1), // spacer
                Constraint::Length(1), // backend label
                Constraint::Length(1), // backend value
                Constraint::Length(1), // backend hint
                Constraint::Min(0),
            ])
            .split(inner);

        let value_style = |row: usize| {
            if focused && self.selected_row == row {
                ratatui::style::Style::default()
                    .fg(theme.accent.to_ratatui())
                    .bg(theme.bg.to_ratatui())
            } else {
                ratatui::style::Style::default()
                    .fg(theme.fg.to_ratatui())
                    .bg(theme.bg.to_ratatui())
            }
        };

        let autosave_style = value_style(ROW_AUTOSAVE);
        let backend_style = value_style(ROW_BACKEND);
        self.clicks.clear();
        for (label_row, value_row, row) in [(0, 1, ROW_AUTOSAVE), (3, 4, ROW_BACKEND)] {
            self.clicks.row(rows[label_row], row);
            self.clicks.row(rows[value_row], row);
        }

        let label = Paragraph::new("Autosave Interval").style(theme.base_style());
        f.render_widget(label, rows[0]);
        let autosave = self.stepper(
            rows[1],
            ROW_AUTOSAVE,
            &format!("{}s", model.autosave_interval_secs),
            theme,
        );
        f.render_widget(Paragraph::new(autosave).style(autosave_style), rows[1]);

        let label = Paragraph::new("Editor Backend").style(theme.base_style());
        f.render_widget(label, rows[3]);
        let backend = self.stepper(
            rows[4],
            ROW_BACKEND,
            backend_label(model.editor_backend),
            theme,
        );
        f.render_widget(Paragraph::new(backend).style(backend_style), rows[4]);
        let hint = Paragraph::new("  applies when a note is opened").style(
            ratatui::style::Style::default()
                .fg(theme.gray.to_ratatui())
                .bg(theme.bg.to_ratatui()),
        );
        f.render_widget(hint, rows[5]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::preferences::{AUTOSAVE_MAX_SECS, AUTOSAVE_MIN_SECS};
    use crate::settings::EditorBackendSetting;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn model(secs: u64, backend: EditorBackendSetting) -> PreferencesModel {
        let mut m = PreferencesModel::new(&crate::settings::AppSettings::default());
        m.autosave_interval_secs = secs;
        m.editor_backend = backend;
        m
    }

    #[test]
    fn right_increases_interval_by_step() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Right), &mut m, &tx);
        assert_eq!(m.autosave_interval_secs, 15);
    }

    #[test]
    fn left_decreases_interval_by_step() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Left), &mut m, &tx);
        assert_eq!(m.autosave_interval_secs, 5);
    }

    #[test]
    fn left_clamps_at_min() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(5, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Left), &mut m, &tx);
        assert_eq!(m.autosave_interval_secs, AUTOSAVE_MIN_SECS);
    }

    #[test]
    fn right_clamps_at_max() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(298, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Right), &mut m, &tx);
        assert_eq!(m.autosave_interval_secs, AUTOSAVE_MAX_SECS);
    }

    #[test]
    fn l_key_increases_interval() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Char('l')), &mut m, &tx);
        assert_eq!(m.autosave_interval_secs, 15);
    }

    #[test]
    fn h_key_decreases_interval() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Char('h')), &mut m, &tx);
        assert_eq!(m.autosave_interval_secs, 5);
    }

    #[test]
    fn down_selects_backend_row_and_right_cycles() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Right), &mut m, &tx);
        assert_eq!(m.editor_backend, EditorBackendSetting::Vim);
        // autosave untouched while on the backend row
        assert_eq!(m.autosave_interval_secs, 10);
    }

    #[test]
    fn backend_cycle_wraps_forward() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        for expected in [
            EditorBackendSetting::Vim,
            EditorBackendSetting::Nvim,
            EditorBackendSetting::Plain,
        ] {
            section.handle_input(&key(KeyCode::Right), &mut m, &tx);
            assert_eq!(m.editor_backend, expected);
        }
    }

    #[test]
    fn backend_cycle_reverses_with_left() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Left), &mut m, &tx);
        assert_eq!(m.editor_backend, EditorBackendSetting::Nvim);
    }

    #[test]
    fn enter_cycles_backend_only_on_its_row() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        // On the autosave row Enter is not consumed and changes nothing.
        let state = section.handle_input(&key(KeyCode::Enter), &mut m, &tx);
        assert_eq!(state, EventState::NotConsumed);
        assert_eq!(m.editor_backend, EditorBackendSetting::Plain);
        // On the backend row it cycles.
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Enter), &mut m, &tx);
        assert_eq!(m.editor_backend, EditorBackendSetting::Vim);
    }

    #[test]
    fn row_navigation_wraps() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new();
        let mut m = model(10, EditorBackendSetting::Plain);
        section.handle_input(&key(KeyCode::Up), &mut m, &tx); // wraps to backend row
        section.handle_input(&key(KeyCode::Char(' ')), &mut m, &tx);
        assert_eq!(m.editor_backend, EditorBackendSetting::Vim);
    }
}
