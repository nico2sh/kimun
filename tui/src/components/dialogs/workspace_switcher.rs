use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct WorkspaceSwitcherModal {
    workspaces: Vec<(String, bool)>, // (name, is_current)
    list_state: ListState,
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
        }
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
                tx.send(AppEvent::CloseDialog).ok();
                EventState::Consumed
            }
            KeyCode::Esc => {
                tx.send(AppEvent::CloseDialog).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let height = (self.workspaces.len() as u16 + 5).min(rect.height.saturating_sub(4));
        let width = 50u16.min(rect.width.saturating_sub(4));
        let popup = super::fixed_centered_rect(width, height, rect);

        f.render_widget(Clear, popup);

        let outer = Block::default()
            .title(" Switch Workspace ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused.to_ratatui()))
            .style(theme.panel_style());
        let inner = outer.inner(popup);
        f.render_widget(outer, popup);

        if self.workspaces.is_empty() {
            f.render_widget(
                Paragraph::new("  No workspaces configured.\n  Use Settings to create one.")
                    .style(Style::default().fg(fg_muted).bg(bg)),
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
            .highlight_style(Style::default().bg(theme.bg_selected.to_ratatui()));

        f.render_stateful_widget(list, rows[0], &mut self.list_state);

        f.render_widget(
            Paragraph::new("  [Enter] Switch  [Esc] Cancel")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[1],
        );
    }
}
