use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::EditorBackendSetting;
use crate::settings::themes::Theme;

const MIN_AUTOSAVE_SECS: u64 = 5;
const MAX_AUTOSAVE_SECS: u64 = 300;
const STEP: u64 = 5;

const ROW_AUTOSAVE: usize = 0;
const ROW_BACKEND: usize = 1;
const ROW_COUNT: usize = 2;

pub struct EditorSection {
    pub autosave_interval_secs: u64,
    pub editor_backend: EditorBackendSetting,
    selected_row: usize,
}

impl EditorSection {
    pub fn new(autosave_interval_secs: u64, editor_backend: EditorBackendSetting) -> Self {
        Self {
            autosave_interval_secs,
            editor_backend,
            selected_row: ROW_AUTOSAVE,
        }
    }

    fn cycle_backend(b: EditorBackendSetting, forward: bool) -> EditorBackendSetting {
        use EditorBackendSetting::*;
        if forward {
            match b {
                Textarea => Vim,
                Vim => Nvim,
                Nvim => Textarea,
            }
        } else {
            match b {
                Textarea => Nvim,
                Vim => Textarea,
                Nvim => Vim,
            }
        }
    }

    fn backend_label(b: EditorBackendSetting) -> &'static str {
        match b {
            EditorBackendSetting::Textarea => "Textarea",
            EditorBackendSetting::Vim => "Vim (built-in)",
            EditorBackendSetting::Nvim => "Nvim (external)",
        }
    }

    fn adjust_autosave(&mut self, increase: bool) {
        self.autosave_interval_secs = if increase {
            (self.autosave_interval_secs + STEP).min(MAX_AUTOSAVE_SECS)
        } else {
            self.autosave_interval_secs
                .saturating_sub(STEP)
                .max(MIN_AUTOSAVE_SECS)
        };
    }
}

impl Component for EditorSection {
    fn handle_input(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
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
                    ROW_AUTOSAVE => self.adjust_autosave(false),
                    _ => self.editor_backend = Self::cycle_backend(self.editor_backend, false),
                }
                EventState::Consumed
            }
            KeyCode::Right | KeyCode::Char('l') => {
                match self.selected_row {
                    ROW_AUTOSAVE => self.adjust_autosave(true),
                    _ => self.editor_backend = Self::cycle_backend(self.editor_backend, true),
                }
                EventState::Consumed
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.selected_row == ROW_BACKEND => {
                self.editor_backend = Self::cycle_backend(self.editor_backend, true);
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
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

        let label = Paragraph::new("Autosave Interval").style(theme.base_style());
        f.render_widget(label, rows[0]);
        let autosave = format!("  ◀  {}s  ▶   (←/→ to change)", self.autosave_interval_secs);
        f.render_widget(
            Paragraph::new(autosave).style(value_style(ROW_AUTOSAVE)),
            rows[1],
        );

        let label = Paragraph::new("Editor Backend").style(theme.base_style());
        f.render_widget(label, rows[3]);
        let backend = format!(
            "  ◀  {}  ▶   (←/→ to change)",
            Self::backend_label(self.editor_backend)
        );
        f.render_widget(
            Paragraph::new(backend).style(value_style(ROW_BACKEND)),
            rows[4],
        );
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
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn section() -> EditorSection {
        EditorSection::new(10, EditorBackendSetting::Textarea)
    }

    #[test]
    fn right_increases_interval_by_step() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.autosave_interval_secs, 15);
    }

    #[test]
    fn left_decreases_interval_by_step() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.autosave_interval_secs, 5);
    }

    #[test]
    fn left_clamps_at_min() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(5, EditorBackendSetting::Textarea);
        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.autosave_interval_secs, MIN_AUTOSAVE_SECS);
    }

    #[test]
    fn right_clamps_at_max() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = EditorSection::new(298, EditorBackendSetting::Textarea);
        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.autosave_interval_secs, MAX_AUTOSAVE_SECS);
    }

    #[test]
    fn l_key_increases_interval() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Char('l')), &tx);
        assert_eq!(section.autosave_interval_secs, 15);
    }

    #[test]
    fn h_key_decreases_interval() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Char('h')), &tx);
        assert_eq!(section.autosave_interval_secs, 5);
    }

    #[test]
    fn down_selects_backend_row_and_right_cycles() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Down), &tx);
        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.editor_backend, EditorBackendSetting::Vim);
        // autosave untouched while on the backend row
        assert_eq!(section.autosave_interval_secs, 10);
    }

    #[test]
    fn backend_cycle_wraps_forward() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Down), &tx);
        for expected in [
            EditorBackendSetting::Vim,
            EditorBackendSetting::Nvim,
            EditorBackendSetting::Textarea,
        ] {
            section.handle_input(&key(KeyCode::Right), &tx);
            assert_eq!(section.editor_backend, expected);
        }
    }

    #[test]
    fn backend_cycle_reverses_with_left() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Down), &tx);
        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.editor_backend, EditorBackendSetting::Nvim);
    }

    #[test]
    fn enter_cycles_backend_only_on_its_row() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        // On the autosave row Enter is not consumed and changes nothing.
        let state = section.handle_input(&key(KeyCode::Enter), &tx);
        assert_eq!(state, EventState::NotConsumed);
        assert_eq!(section.editor_backend, EditorBackendSetting::Textarea);
        // On the backend row it cycles.
        section.handle_input(&key(KeyCode::Down), &tx);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert_eq!(section.editor_backend, EditorBackendSetting::Vim);
    }

    #[test]
    fn row_navigation_wraps() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = section();
        section.handle_input(&key(KeyCode::Up), &tx); // wraps to backend row
        section.handle_input(&key(KeyCode::Char(' ')), &tx);
        assert_eq!(section.editor_backend, EditorBackendSetting::Vim);
    }
}
