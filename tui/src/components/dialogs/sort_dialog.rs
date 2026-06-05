use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, SortTarget};
use crate::components::file_list::{SortField, SortOrder};
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::settings::themes::Theme;

/// The selectable rows, in display order.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Field,
    Order,
    GroupDirs,
}

/// Modal that edits sort field / order (+ a sidebar-only "group directories"
/// toggle). Changes apply live: each toggle emits `AppEvent::SortChanged`
/// (`persist = false`). `s` (sidebar only) emits the same event with
/// `persist = true` (save as default); Enter/Esc emit `CloseOverlay`.
pub struct SortDialog {
    target: SortTarget,
    pub(crate) field: SortField,
    pub(crate) order: SortOrder,
    group_dirs: bool,
    rows: Vec<Row>,
    selected: usize,
}

impl SortDialog {
    pub fn new(target: SortTarget, field: SortField, order: SortOrder, group_dirs: bool) -> Self {
        let mut rows = vec![Row::Field, Row::Order];
        if target == SortTarget::Sidebar {
            rows.push(Row::GroupDirs);
        }
        Self {
            target,
            field,
            order,
            group_dirs,
            rows,
            selected: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Emit the current selection. `persist` requests saving it as the default
    /// (sidebar's `s` key); a plain toggle sends `persist = false` for live apply.
    fn emit(&self, tx: &AppTx, persist: bool) {
        tx.send(AppEvent::SortChanged {
            target: self.target,
            field: self.field,
            order: self.order,
            group_directories: self.group_dirs,
            persist,
        })
        .ok();
    }

    fn toggle_selected(&mut self, tx: &AppTx) {
        match self.rows[self.selected] {
            Row::Field => self.field = self.field.cycle(),
            Row::Order => self.order = self.order.toggle(),
            Row::GroupDirs => self.group_dirs = !self.group_dirs,
        }
        self.emit(tx, false);
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
                self.emit(tx, true);
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
        // rows + borders(2) + footer(1).
        let outer_height = self.rows.len() as u16 + 3;
        let popup = super::fixed_centered_rect(OUTER_WIDTH, outer_height, rect);
        let inner = modal_chrome(
            f,
            popup,
            theme,
            ModalSpec {
                title: Some(" Sort "),
                border: Some(Style::default().fg(theme.fg.to_ratatui())),
                ..Default::default()
            },
        );
        if inner.height < 2 {
            return;
        }

        // Split body (rows) from a fixed 1-line footer. `Min(1)` collapses the
        // body before the footer disappears, so the footer is never overlapped
        // on a short terminal (mirrors help_dialog).
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let body = chunks[0];
        let footer_area = chunks[1];

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();
        let fg_sel = theme.selection_fg.to_ratatui();
        let bg_sel = theme.selection_bg.to_ratatui();

        for (i, &row) in self.rows.iter().enumerate() {
            let y = body.y + i as u16;
            if y >= body.y + body.height {
                break;
            }
            let (label, value) = self.row_label(row);
            let selected = i == self.selected;
            let style = if selected {
                Style::default()
                    .fg(fg_sel)
                    .bg(bg_sel)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg).bg(bg)
            };
            let marker = if selected { ">" } else { " " };
            f.render_widget(
                Paragraph::new(format!(" {marker} {label:<20}{value}")).style(style),
                Rect {
                    x: body.x,
                    y,
                    width: body.width,
                    height: 1,
                },
            );
        }

        let footer = if self.target == SortTarget::Sidebar {
            "  [↑↓] Move  [Space] Toggle  [s] Save default  [Enter/Esc] Close"
        } else {
            "  [↑↓] Move  [Space] Toggle  [Enter/Esc] Close"
        };
        f.render_widget(
            Paragraph::new(footer).style(Style::default().fg(gray).bg(bg)),
            footer_area,
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
        SortDialog::new(
            SortTarget::Sidebar,
            SortField::Name,
            SortOrder::Ascending,
            false,
        )
    }

    #[test]
    fn space_toggles_field_and_emits_change() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Title);
        let evt = rx.try_recv().expect("a SortChanged event");
        match evt {
            AppEvent::SortChanged {
                target,
                field,
                order,
                group_directories,
                persist,
            } => {
                assert_eq!(target, SortTarget::Sidebar);
                assert_eq!(field, SortField::Title);
                assert_eq!(order, SortOrder::Ascending);
                assert!(!group_directories);
                assert!(!persist, "a plain toggle is not a save");
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
        let query = SortDialog::new(
            SortTarget::Query,
            SortField::Name,
            SortOrder::Ascending,
            false,
        );
        assert_eq!(query.row_count(), 2);
    }

    #[test]
    fn s_saves_default_for_sidebar_only() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('s')), &tx);
        assert!(
            matches!(
                rx.try_recv(),
                Ok(AppEvent::SortChanged { persist: true, .. })
            ),
            "s on the sidebar emits a persisting SortChanged"
        );

        let mut q = SortDialog::new(
            SortTarget::Query,
            SortField::Name,
            SortOrder::Ascending,
            false,
        );
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
