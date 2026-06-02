use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, SortTarget};
use crate::components::file_list::{SortField, SortOrder};
use crate::settings::themes::Theme;

/// The selectable rows, in display order.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Field,
    Order,
    GroupDirs,
}

/// Modal that edits sort field / order (+ a sidebar-only "group directories"
/// toggle). Changes apply live: each toggle emits `AppEvent::SortChanged`.
/// `s` (sidebar only) emits `SortSaveDefault`; Enter/Esc emit `CloseOverlay`.
pub struct SortDialog {
    target: SortTarget,
    pub(crate) field: SortField,
    pub(crate) order: SortOrder,
    group_dirs: bool,
    rows: Vec<Row>,
    selected: usize,
}

impl SortDialog {
    pub fn new(
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_dirs: bool,
    ) -> Self {
        let mut rows = vec![Row::Field, Row::Order];
        if target == SortTarget::Sidebar {
            rows.push(Row::GroupDirs);
        }
        Self { target, field, order, group_dirs, rows, selected: 0 }
    }

    #[cfg(test)]
    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn emit_change(&self, tx: &AppTx) {
        tx.send(AppEvent::SortChanged {
            target: self.target,
            field: self.field,
            order: self.order,
            group_directories: self.group_dirs,
        })
        .ok();
    }

    fn toggle_selected(&mut self, tx: &AppTx) {
        match self.rows[self.selected] {
            Row::Field => self.field = self.field.cycle(),
            Row::Order => self.order = self.order.toggle(),
            Row::GroupDirs => self.group_dirs = !self.group_dirs,
        }
        self.emit_change(tx);
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.rows.len() - 1);
            }
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => {
                self.toggle_selected(tx);
            }
            KeyCode::Char('s') if self.target == SortTarget::Sidebar => {
                tx.send(AppEvent::SortSaveDefault {
                    target: self.target,
                    field: self.field,
                    order: self.order,
                    group_directories: self.group_dirs,
                })
                .ok();
            }
            KeyCode::Enter | KeyCode::Esc => {
                tx.send(AppEvent::CloseOverlay).ok();
            }
            _ => {}
        }
        EventState::Consumed
    }

    fn row_label(&self, row: Row) -> (String, String) {
        match row {
            Row::Field => (
                "Sort by".to_string(),
                match self.field {
                    SortField::Name => "Name".to_string(),
                    SortField::Title => "Title".to_string(),
                },
            ),
            Row::Order => (
                "Order".to_string(),
                match self.order {
                    SortOrder::Ascending => "Ascending \u{2191}".to_string(),
                    SortOrder::Descending => "Descending \u{2193}".to_string(),
                },
            ),
            Row::GroupDirs => (
                "Group directories".to_string(),
                if self.group_dirs { "On" } else { "Off" }.to_string(),
            ),
        }
    }
}

const OUTER_WIDTH: u16 = 44;

impl crate::components::Component for SortDialog {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        if let InputEvent::Key(key) = event {
            self.handle_key(*key, tx)
        } else {
            EventState::NotConsumed
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let outer_height = self.rows.len() as u16 + 4;
        let popup = super::fixed_centered_rect(OUTER_WIDTH, outer_height, rect);
        f.render_widget(Clear, popup);

        let block = Block::default()
            .title(" Sort ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.fg.to_ratatui()))
            .style(theme.panel_style());
        let inner = block.inner(popup);
        f.render_widget(block, popup);
        if inner.height < 2 {
            return;
        }

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let fg_sel = theme.fg_selected.to_ratatui();
        let bg_sel = theme.bg_selected.to_ratatui();

        for (i, &row) in self.rows.iter().enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }
            let (label, value) = self.row_label(row);
            let selected = i == self.selected;
            let style = if selected {
                Style::default().fg(fg_sel).bg(bg_sel).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg).bg(bg)
            };
            let marker = if selected { ">" } else { " " };
            f.render_widget(
                Paragraph::new(format!(" {marker} {label:<20}{value}")).style(style),
                Rect { x: inner.x, y, width: inner.width, height: 1 },
            );
        }

        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer = if self.target == SortTarget::Sidebar {
            "  [↑↓] Move  [Space] Toggle  [s] Save default  [Enter/Esc] Close"
        } else {
            "  [↑↓] Move  [Space] Toggle  [Enter/Esc] Close"
        };
        f.render_widget(
            Paragraph::new(footer).style(Style::default().fg(fg_muted).bg(bg)),
            Rect { x: inner.x, y: footer_y, width: inner.width, height: 1 },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::SortTarget;
    use crate::components::file_list::{SortField, SortOrder};
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn sidebar_dialog() -> SortDialog {
        SortDialog::new(SortTarget::Sidebar, SortField::Name, SortOrder::Ascending, false)
    }

    #[test]
    fn space_toggles_field_and_emits_change() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Title);
        let evt = rx.try_recv().expect("a SortChanged event");
        match evt {
            AppEvent::SortChanged { target, field, order, group_directories } => {
                assert_eq!(target, SortTarget::Sidebar);
                assert_eq!(field, SortField::Title);
                assert_eq!(order, SortOrder::Ascending);
                assert!(!group_directories);
            }
            other => panic!("expected SortChanged, got {other:?}"),
        }
    }

    #[test]
    fn down_then_space_toggles_order() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Down), &tx);
        assert!(rx.try_recv().is_err(), "navigation alone emits nothing");
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.order, SortOrder::Descending);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::SortChanged { .. })));
    }

    #[test]
    fn group_row_present_only_for_sidebar() {
        let sidebar = sidebar_dialog();
        assert_eq!(sidebar.row_count(), 3);
        let query = SortDialog::new(SortTarget::Query, SortField::Name, SortOrder::Ascending, false);
        assert_eq!(query.row_count(), 2);
    }

    #[test]
    fn s_saves_default_for_sidebar_only() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('s')), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::SortSaveDefault { .. })));

        let mut q = SortDialog::new(SortTarget::Query, SortField::Name, SortOrder::Ascending, false);
        let (tx2, mut rx2) = unbounded_channel();
        q.handle_key(key(KeyCode::Char('s')), &tx2);
        assert!(rx2.try_recv().is_err(), "query target has no save-default");
    }

    #[test]
    fn enter_and_esc_close_overlay() {
        for code in [KeyCode::Enter, KeyCode::Esc] {
            let mut d = sidebar_dialog();
            let (tx, mut rx) = unbounded_channel();
            d.handle_key(key(code), &tx);
            assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
        }
    }
}
