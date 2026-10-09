use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use super::ModalShell;
use crate::components::clickable::list_index_at;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct WorkspaceSwitcherModal {
    workspaces: Vec<(String, bool)>, // (name, is_current)
    list_state: ListState,
    /// Dismiss-on-outside-press and hint-chip clicks (see [`ModalShell`]).
    shell: ModalShell,
    /// Where the rows were drawn in the last render.
    list_rect: Rect,
    hints: HintRow,
}

impl WorkspaceSwitcherModal {
    pub fn new(settings: &AppSettings) -> Self {
        let mut workspaces: Vec<(String, bool)> = Vec::new();
        if let Some(ref wc) = settings.workspace_config {
            let current = &wc.global.current_workspace;
            let mut names: Vec<&String> = wc.workspaces.keys().collect();
            names.sort();
            for name in names {
                workspaces.push((name.clone(), name == current));
            }
        }
        let mut list_state = ListState::default();
        if !workspaces.is_empty() {
            let current_idx = workspaces
                .iter()
                .position(|(_, is_cur)| *is_cur)
                .unwrap_or(0);
            list_state.select(Some(current_idx));
        }
        Self {
            workspaces,
            list_state,
            shell: ModalShell::default(),
            list_rect: Rect::default(),
            hints: HintRow::new(&[
                (KeyCode::Enter, "Enter", "Switch"),
                (KeyCode::Esc, "Esc", "Cancel"),
            ]),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_workspaces(workspaces: Vec<(String, bool)>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(workspaces.iter().position(|(_, cur)| *cur));
        Self {
            workspaces,
            list_state,
            shell: ModalShell::default(),
            list_rect: Rect::default(),
            hints: HintRow::new(&[
                (KeyCode::Enter, "Enter", "Switch"),
                (KeyCode::Esc, "Esc", "Cancel"),
            ]),
        }
    }

    /// Click a row to select it, click the selected row to switch; the wheel
    /// moves the selection; a press outside the popup cancels. Modal: every
    /// mouse event is consumed.
    pub fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        if let Some(key) = self.shell.pointer_key(m, &[&self.hints]) {
            return self.handle_key(key, tx);
        }
        match m.kind {
            MouseEventKind::ScrollUp => {
                self.handle_key(KeyEvent::from(KeyCode::Up), tx);
            }
            MouseEventKind::ScrollDown => {
                self.handle_key(KeyEvent::from(KeyCode::Down), tx);
            }
            _ => {
                let len = self.workspaces.len();
                if let Some(idx) = list_index_at(m, self.list_rect, self.list_state.offset(), len) {
                    if self.list_state.selected() == Some(idx) {
                        return self.handle_key(KeyEvent::from(KeyCode::Enter), tx);
                    }
                    self.list_state.select(Some(idx));
                }
            }
        }
        EventState::Consumed
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Up => {
                if !self.workspaces.is_empty() {
                    let cur = self.list_state.selected().unwrap_or(0);
                    let next = if cur == 0 {
                        self.workspaces.len() - 1
                    } else {
                        cur - 1
                    };
                    self.list_state.select(Some(next));
                }
                EventState::Consumed
            }
            KeyCode::Down => {
                if !self.workspaces.is_empty() {
                    let cur = self.list_state.selected().unwrap_or(0);
                    let next = (cur + 1) % self.workspaces.len();
                    self.list_state.select(Some(next));
                }
                EventState::Consumed
            }
            KeyCode::Enter => {
                if let Some(idx) = self.list_state.selected()
                    && let Some((name, is_current)) = self.workspaces.get(idx)
                    && !is_current
                {
                    tx.send(AppEvent::WorkspaceSwitched(name.clone())).ok();
                }
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            KeyCode::Esc => {
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let height = (self.workspaces.len() as u16 + 5).min(rect.height.saturating_sub(4));
        let width = 50u16.min(rect.width.saturating_sub(4));
        let popup = super::fixed_centered_rect(width, height, rect);
        self.shell.set(popup);

        let inner = modal_chrome(
            f,
            popup,
            theme,
            ModalSpec {
                title: Some(" Switch Workspace "),
                border: Some(Style::default().fg(theme.focus_border.to_ratatui())),
                ..Default::default()
            },
        );

        if self.workspaces.is_empty() {
            f.render_widget(
                Paragraph::new("  No workspaces configured.\n  Use Preferences to create one.")
                    .style(Style::default().fg(gray).bg(bg)),
                inner,
            );
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

        let items: Vec<ListItem> = self
            .workspaces
            .iter()
            .map(|(name, is_current)| {
                let marker = if *is_current { "\u{25CF} " } else { "  " };
                let style = if *is_current {
                    Style::default()
                        .fg(theme.accent.to_ratatui())
                        .bg(bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(fg).bg(bg)
                };
                ListItem::new(format!("{}{}", marker, name)).style(style)
            })
            .collect();

        let list = List::new(items)
            .style(Style::default().bg(bg))
            .highlight_style(Style::default().bg(theme.selection_bg.to_ratatui()));

        self.list_rect = rows[0];
        f.render_stateful_widget(list, rows[0], &mut self.list_state);

        self.hints
            .render(f, rows[1], Style::default().fg(gray).bg(bg), theme);
    }
}

impl_dialog!(WorkspaceSwitcherModal);
