use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use std::path::PathBuf;

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Creating,
    Renaming,
    ConfirmDelete,
}

pub struct WorkspacesSection {
    /// Sorted list of (name, path, is_current).
    entries: Vec<(String, PathBuf, bool)>,
    list_state: ListState,
    mode: Mode,
    input: String,
    input_cursor: usize,
    error: Option<String>,
}

impl WorkspacesSection {
    pub fn new(settings: &AppSettings) -> Self {
        let mut section = Self {
            entries: Vec::new(),
            list_state: ListState::default(),
            mode: Mode::Normal,
            input: String::new(),
            input_cursor: 0,
            error: None,
        };
        section.refresh(settings);
        section
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.list_state
            .selected()
            .and_then(|i| self.entries.get(i))
            .map(|(name, _, _)| name.as_str())
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.entries
            .iter()
            .find(|(_, _, is_current)| *is_current)
            .map(|(_, path, _)| path.clone())
    }

    pub fn refresh(&mut self, settings: &AppSettings) {
        self.entries.clear();
        if let Some(ref wc) = settings.workspace_config {
            let current = &wc.global.current_workspace;
            let mut names: Vec<&String> = wc.workspaces.keys().collect();
            names.sort();
            for name in names {
                if let Some(entry) = wc.workspaces.get(name) {
                    self.entries
                        .push((name.clone(), entry.path.clone(), name == current));
                }
            }
        }
        // Preserve selection or default to first
        let max = self.entries.len();
        if max == 0 {
            self.list_state.select(None);
        } else {
            let prev = self.list_state.selected().unwrap_or(0);
            self.list_state.select(Some(prev.min(max - 1)));
        }
    }

    pub fn reset_mode(&mut self) {
        self.mode = Mode::Normal;
        self.input.clear();
        self.input_cursor = 0;
        self.error = None;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
    }

    // ---- private helpers ----

    fn move_up(&mut self) {
        if !self.entries.is_empty() {
            let cur = self.list_state.selected().unwrap_or(0);
            let next = if cur == 0 {
                self.entries.len() - 1
            } else {
                cur - 1
            };
            self.list_state.select(Some(next));
        }
    }

    fn move_down(&mut self) {
        if !self.entries.is_empty() {
            let cur = self.list_state.selected().unwrap_or(0);
            let next = (cur + 1) % self.entries.len();
            self.list_state.select(Some(next));
        }
    }

    fn handle_normal(&mut self, code: KeyCode, tx: &AppTx) -> EventState {
        match code {
            KeyCode::Up => {
                self.move_up();
                EventState::Consumed
            }
            KeyCode::Down => {
                self.move_down();
                EventState::Consumed
            }
            KeyCode::Enter => {
                if let Some((name, _, is_current)) =
                    self.list_state.selected().and_then(|i| self.entries.get(i))
                    && !is_current
                {
                    tx.send(AppEvent::WorkspaceSwitched(name.clone())).ok();
                }
                EventState::Consumed
            }
            KeyCode::Char('n') => {
                self.mode = Mode::Creating;
                if self.entries.is_empty() {
                    self.input = "default".to_string();
                    self.input_cursor = self.input.len();
                } else {
                    self.input.clear();
                    self.input_cursor = 0;
                }
                self.error = None;
                EventState::Consumed
            }
            KeyCode::Char('r') => {
                if let Some(name) = self.selected_name().map(|s| s.to_string()) {
                    self.mode = Mode::Renaming;
                    self.input = name;
                    self.input_cursor = self.input.len();
                    self.error = None;
                }
                EventState::Consumed
            }
            KeyCode::Char('d') => {
                if self.list_state.selected().is_some() {
                    self.mode = Mode::ConfirmDelete;
                    self.error = None;
                }
                EventState::Consumed
            }
            KeyCode::Char('b') => {
                tx.send(AppEvent::OpenFileBrowser).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn handle_text_input(&mut self, code: KeyCode) -> EventState {
        match code {
            KeyCode::Esc => {
                self.reset_mode();
                EventState::Consumed
            }
            KeyCode::Enter => {
                // Caller (SettingsScreen) checks mode + input
                EventState::Consumed
            }
            KeyCode::Backspace => {
                if self.input_cursor > 0 {
                    let prev = self.input[..self.input_cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.input.drain(prev..self.input_cursor);
                    self.input_cursor = prev;
                }
                EventState::Consumed
            }
            KeyCode::Delete => {
                if self.input_cursor < self.input.len() {
                    let next = self.input[self.input_cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.input_cursor + i)
                        .unwrap_or(self.input.len());
                    self.input.drain(self.input_cursor..next);
                }
                EventState::Consumed
            }
            KeyCode::Left => {
                if self.input_cursor > 0 {
                    self.input_cursor = self.input[..self.input_cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
                EventState::Consumed
            }
            KeyCode::Right => {
                if self.input_cursor < self.input.len() {
                    self.input_cursor = self.input[self.input_cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.input_cursor + i)
                        .unwrap_or(self.input.len());
                }
                EventState::Consumed
            }
            KeyCode::Char(c) => {
                self.error = None;
                self.input.insert(self.input_cursor, c);
                self.input_cursor += c.len_utf8();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn handle_confirm_delete(&mut self, code: KeyCode) -> EventState {
        match code {
            KeyCode::Char('y') => {
                // Stays in ConfirmDelete; SettingsScreen reads mode + selected_name
                EventState::Consumed
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.reset_mode();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }
}

impl Component for WorkspacesSection {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match self.mode {
            Mode::Normal => self.handle_normal(key.code, tx),
            Mode::Creating | Mode::Renaming => {
                if key.code == KeyCode::Enter {
                    // For Creating: signal OpenFileBrowser so SettingsScreen can pick up
                    if self.mode == Mode::Creating && !self.input.trim().is_empty() {
                        tx.send(AppEvent::OpenFileBrowser).ok();
                    }
                    // For both modes the caller inspects mode() + input()
                    return EventState::Consumed;
                }
                self.handle_text_input(key.code)
            }
            Mode::ConfirmDelete => self.handle_confirm_delete(key.code),
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        // Reserve bottom lines: 1 for hint, 1 for error (if any), 2 for border
        let title = format!("Workspaces ({})", self.entries.len());
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());

        let inner = block.inner(rect);
        f.render_widget(block, rect);

        if inner.height < 2 {
            return;
        }

        // Split inner into: list area and bottom bar(s)
        let mut constraints = vec![Constraint::Min(0), Constraint::Length(1)];
        if self.error.is_some() {
            constraints.push(Constraint::Length(1));
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        // --- List ---
        if self.entries.is_empty() {
            f.render_widget(
                Paragraph::new("  No workspaces configured.")
                    .style(Style::default().fg(fg_muted).bg(bg)),
                rows[0],
            );
        } else {
            let items: Vec<ListItem> = self
                .entries
                .iter()
                .map(|(name, path, is_current)| {
                    let marker = if *is_current { "\u{25CF} " } else { "  " };
                    let line = format!("{}{}  {}", marker, name, path.to_string_lossy());
                    let style = if *is_current {
                        Style::default()
                            .fg(theme.accent.to_ratatui())
                            .bg(bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(fg).bg(bg)
                    };
                    ListItem::new(line).style(style)
                })
                .collect();

            let list = List::new(items)
                .style(Style::default().bg(bg))
                .highlight_style(Style::default().bg(theme.bg_selected.to_ratatui()));

            f.render_stateful_widget(list, rows[0], &mut self.list_state);
        }

        // --- Hint line ---
        let hint_idx = 1;
        let hint_text = match &self.mode {
            Mode::Normal => {
                " [Enter] Switch  [n] New  [r] Rename  [d] Delete  [b] Browse path".to_string()
            }
            Mode::Creating => {
                let visible_cursor = self.input[..self.input_cursor].chars().count();
                let display = format!(" Name: {}", self.input);
                // Set cursor position
                let cursor_x = rows[hint_idx].x + 7 + visible_cursor as u16;
                let cursor_y = rows[hint_idx].y;
                if cursor_x < rows[hint_idx].x + rows[hint_idx].width {
                    f.set_cursor_position((cursor_x, cursor_y));
                }
                display
            }
            Mode::Renaming => {
                let visible_cursor = self.input[..self.input_cursor].chars().count();
                let display = format!(" New name: {}", self.input);
                let cursor_x = rows[hint_idx].x + 11 + visible_cursor as u16;
                let cursor_y = rows[hint_idx].y;
                if cursor_x < rows[hint_idx].x + rows[hint_idx].width {
                    f.set_cursor_position((cursor_x, cursor_y));
                }
                display
            }
            Mode::ConfirmDelete => {
                let name = self.selected_name().unwrap_or("?");
                format!(" Delete workspace '{}'? [y] Yes  [n/Esc] No", name)
            }
        };
        f.render_widget(
            Paragraph::new(hint_text).style(Style::default().fg(fg_muted).bg(bg)),
            rows[hint_idx],
        );

        // --- Error line ---
        if let Some(ref err) = self.error {
            let err_idx = 2;
            if err_idx < rows.len() {
                f.render_widget(
                    Paragraph::new(format!(" {}", err))
                        .style(Style::default().fg(theme.accent.to_ratatui()).bg(bg)),
                    rows[err_idx],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::workspace_config::{GlobalConfig, WorkspaceConfig, WorkspaceEntry};
    use ratatui::crossterm::event::{KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::collections::HashMap;

    fn make_settings(workspaces: Vec<(&str, &str)>, current: &str) -> AppSettings {
        let mut ws_map = HashMap::new();
        for (name, path) in &workspaces {
            ws_map.insert(
                name.to_string(),
                WorkspaceEntry {
                    path: PathBuf::from(path),
                    last_paths: vec![],
                    created: chrono::Utc::now(),
                    quick_note_path: None,
                    inbox_path: None,
                    resolved_path: None,
                },
            );
        }
        let mut settings = AppSettings::default();
        settings.workspace_config = Some(WorkspaceConfig {
            global: GlobalConfig {
                current_workspace: current.to_string(),
            },
            workspaces: ws_map,
        });
        settings
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
    fn new_section_loads_workspaces() {
        let settings = make_settings(vec![("work", "/work"), ("personal", "/personal")], "work");
        let section = WorkspacesSection::new(&settings);
        assert_eq!(section.entries.len(), 2);
        // Sorted alphabetically
        assert_eq!(section.entries[0].0, "personal");
        assert_eq!(section.entries[1].0, "work");
    }

    #[test]
    fn current_path_returns_active_workspace() {
        let settings = make_settings(vec![("notes", "/my/notes")], "notes");
        let section = WorkspacesSection::new(&settings);
        assert_eq!(section.current_path(), Some(PathBuf::from("/my/notes")));
    }

    #[test]
    fn up_down_navigate() {
        let settings = make_settings(vec![("a", "/a"), ("b", "/b"), ("c", "/c")], "a");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.list_state.select(Some(0));

        section.handle_input(&key(KeyCode::Down), &tx);
        assert_eq!(section.list_state.selected(), Some(1));

        section.handle_input(&key(KeyCode::Down), &tx);
        assert_eq!(section.list_state.selected(), Some(2));

        // Wraps
        section.handle_input(&key(KeyCode::Down), &tx);
        assert_eq!(section.list_state.selected(), Some(0));

        section.handle_input(&key(KeyCode::Up), &tx);
        assert_eq!(section.list_state.selected(), Some(2));
    }

    #[test]
    fn enter_sends_workspace_switched() {
        let settings = make_settings(vec![("a", "/a"), ("b", "/b")], "a");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        // Select "b" (index 1)
        section.list_state.select(Some(1));
        section.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("should send event");
        assert!(matches!(msg, AppEvent::WorkspaceSwitched(name) if name == "b"));
    }

    #[test]
    fn enter_on_current_does_not_send_event() {
        let settings = make_settings(vec![("a", "/a"), ("b", "/b")], "a");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        // "a" is at index 0 and is current
        section.list_state.select(Some(0));
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn n_enters_creating_mode() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('n')), &tx);
        assert_eq!(*section.mode(), Mode::Creating);
    }

    #[test]
    fn creating_mode_collects_text_and_sends_file_browser() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('n')), &tx);

        section.handle_input(&key(KeyCode::Char('t')), &tx);
        section.handle_input(&key(KeyCode::Char('e')), &tx);
        section.handle_input(&key(KeyCode::Char('s')), &tx);
        section.handle_input(&key(KeyCode::Char('t')), &tx);
        assert_eq!(section.input(), "test");

        section.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("should send OpenFileBrowser");
        assert!(matches!(msg, AppEvent::OpenFileBrowser));
    }

    #[test]
    fn creating_empty_name_does_not_send_file_browser() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('n')), &tx);
        section.handle_input(&key(KeyCode::Enter), &tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn esc_cancels_creating() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('n')), &tx);
        section.handle_input(&key(KeyCode::Char('x')), &tx);
        section.handle_input(&key(KeyCode::Esc), &tx);
        assert_eq!(*section.mode(), Mode::Normal);
        assert!(section.input().is_empty());
    }

    #[test]
    fn r_enters_renaming_mode() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('r')), &tx);
        assert_eq!(*section.mode(), Mode::Renaming);
    }

    #[test]
    fn d_enters_confirm_delete() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('d')), &tx);
        assert_eq!(*section.mode(), Mode::ConfirmDelete);
    }

    #[test]
    fn confirm_delete_y_stays_in_mode_for_caller() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('d')), &tx);
        let result = section.handle_input(&key(KeyCode::Char('y')), &tx);
        assert!(result.is_consumed());
        // Still in ConfirmDelete so caller can act
        assert_eq!(*section.mode(), Mode::ConfirmDelete);
    }

    #[test]
    fn confirm_delete_n_cancels() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('d')), &tx);
        section.handle_input(&key(KeyCode::Char('n')), &tx);
        assert_eq!(*section.mode(), Mode::Normal);
    }

    #[test]
    fn b_sends_open_file_browser() {
        let settings = make_settings(vec![("a", "/a")], "a");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.handle_input(&key(KeyCode::Char('b')), &tx);
        let msg = rx.try_recv().expect("should send event");
        assert!(matches!(msg, AppEvent::OpenFileBrowser));
    }

    #[test]
    fn backspace_deletes_char() {
        let settings = make_settings(vec![], "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.mode = Mode::Creating;

        section.handle_input(&key(KeyCode::Char('a')), &tx);
        section.handle_input(&key(KeyCode::Char('b')), &tx);
        assert_eq!(section.input(), "ab");

        section.handle_input(&key(KeyCode::Backspace), &tx);
        assert_eq!(section.input(), "a");
    }

    #[test]
    fn text_input_cursor_movement() {
        let settings = make_settings(vec![], "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.mode = Mode::Creating;

        section.handle_input(&key(KeyCode::Char('a')), &tx);
        section.handle_input(&key(KeyCode::Char('b')), &tx);
        section.handle_input(&key(KeyCode::Char('c')), &tx);
        assert_eq!(section.input_cursor, 3);

        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.input_cursor, 2);

        section.handle_input(&key(KeyCode::Left), &tx);
        assert_eq!(section.input_cursor, 1);

        section.handle_input(&key(KeyCode::Right), &tx);
        assert_eq!(section.input_cursor, 2);
    }

    #[test]
    fn refresh_updates_entries() {
        let settings1 = make_settings(vec![("a", "/a")], "a");
        let mut section = WorkspacesSection::new(&settings1);
        assert_eq!(section.entries.len(), 1);

        let settings2 = make_settings(vec![("a", "/a"), ("b", "/b")], "a");
        section.refresh(&settings2);
        assert_eq!(section.entries.len(), 2);
    }

    #[test]
    fn reset_mode_clears_state() {
        let settings = make_settings(vec![], "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = WorkspacesSection::new(&settings);
        section.mode = Mode::Creating;
        section.handle_input(&key(KeyCode::Char('x')), &tx);
        section.set_error("oops".to_string());

        section.reset_mode();
        assert_eq!(*section.mode(), Mode::Normal);
        assert!(section.input().is_empty());
        assert_eq!(section.input_cursor, 0);
        assert!(section.error.is_none());
    }

    #[test]
    fn empty_settings_shows_no_workspaces() {
        let settings = AppSettings::default();
        let section = WorkspacesSection::new(&settings);
        assert!(section.entries.is_empty());
        assert!(section.current_path().is_none());
    }

    #[test]
    fn renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let settings = make_settings(vec![("notes", "/my/notes")], "notes");
        let mut section = WorkspacesSection::new(&settings);
        let theme = Theme::gruvbox_dark();
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                section.render(f, f.area(), &theme, true);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let flat: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("Workspaces (1)"));
        assert!(flat.contains("notes"));
    }
}
