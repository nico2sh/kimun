use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::{SortFieldSetting, SortOrderSetting};
use crate::settings::themes::Theme;

pub struct SortingSection {
    pub default_sort_field: SortFieldSetting,
    pub default_sort_order: SortOrderSetting,
    pub journal_sort_field: SortFieldSetting,
    pub journal_sort_order: SortOrderSetting,
    list_state: ListState,
}

impl SortingSection {
    pub fn new(
        default_sort_field: SortFieldSetting,
        default_sort_order: SortOrderSetting,
        journal_sort_field: SortFieldSetting,
        journal_sort_order: SortOrderSetting,
    ) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            default_sort_field,
            default_sort_order,
            journal_sort_field,
            journal_sort_order,
            list_state,
        }
    }

    const ROW_COUNT: usize = 4;

    fn cycle_field(f: SortFieldSetting) -> SortFieldSetting {
        match f {
            SortFieldSetting::Name => SortFieldSetting::Title,
            SortFieldSetting::Title => SortFieldSetting::Name,
        }
    }

    fn cycle_order(o: SortOrderSetting) -> SortOrderSetting {
        match o {
            SortOrderSetting::Ascending => SortOrderSetting::Descending,
            SortOrderSetting::Descending => SortOrderSetting::Ascending,
        }
    }

    fn field_label(f: SortFieldSetting) -> &'static str {
        match f {
            SortFieldSetting::Name => "Name",
            SortFieldSetting::Title => "Title",
        }
    }

    fn order_label(o: SortOrderSetting) -> &'static str {
        match o {
            SortOrderSetting::Ascending => "Ascending",
            SortOrderSetting::Descending => "Descending",
        }
    }
}

impl Component for SortingSection {
    fn handle_input(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        let selected = self.list_state.selected().unwrap_or(0);
        match key.code {
            ratatui::crossterm::event::KeyCode::Up
            | ratatui::crossterm::event::KeyCode::Char('k') => {
                self.list_state
                    .select(Some((selected + Self::ROW_COUNT - 1) % Self::ROW_COUNT));
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Down
            | ratatui::crossterm::event::KeyCode::Char('j') => {
                self.list_state
                    .select(Some((selected + 1) % Self::ROW_COUNT));
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Enter
            | ratatui::crossterm::event::KeyCode::Char(' ') => {
                match selected {
                    0 => self.default_sort_field = Self::cycle_field(self.default_sort_field),
                    1 => self.default_sort_order = Self::cycle_order(self.default_sort_order),
                    2 => self.journal_sort_field = Self::cycle_field(self.journal_sort_field),
                    3 => self.journal_sort_order = Self::cycle_order(self.journal_sort_order),
                    _ => {}
                }
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);

        // Outer container
        let outer = Block::default()
            .title("Sorting")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.base_style());
        let inner = outer.inner(rect);
        f.render_widget(outer, rect);

        // Stack the two sub-groups vertically inside the outer block.
        // Each sub-block needs 2 content rows + 2 border rows = 4 rows.
        let halves = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Length(4)])
            .split(inner);

        let selected = self.list_state.selected().unwrap_or(0);
        let highlight = Style::default()
            .fg(theme.fg_selected.to_ratatui())
            .bg(theme.bg_selected.to_ratatui());

        // ── Default sub-block (rows 0–1) ───────────────────────────────────
        let default_items = vec![
            ListItem::new(format!(
                "  Sort field:  [{}]",
                Self::field_label(self.default_sort_field)
            ))
            .style(Style::default().fg(theme.fg.to_ratatui())),
            ListItem::new(format!(
                "  Sort order:  [{}]",
                Self::order_label(self.default_sort_order)
            ))
            .style(Style::default().fg(theme.fg.to_ratatui())),
        ];
        let mut default_state = ListState::default();
        default_state.select(if selected < 2 { Some(selected) } else { None });
        let default_block = Block::default()
            .title("Default")
            .borders(Borders::ALL)
            .border_style(theme.border_style(focused && selected < 2))
            .style(theme.base_style());
        f.render_stateful_widget(
            List::new(default_items)
                .block(default_block)
                .highlight_style(highlight),
            halves[0],
            &mut default_state,
        );

        // ── Journal sub-block (rows 2–3) ────────────────────────────────────
        let journal_items = vec![
            ListItem::new(format!(
                "  Sort field:  [{}]",
                Self::field_label(self.journal_sort_field)
            ))
            .style(Style::default().fg(theme.fg.to_ratatui())),
            ListItem::new(format!(
                "  Sort order:  [{}]",
                Self::order_label(self.journal_sort_order)
            ))
            .style(Style::default().fg(theme.fg.to_ratatui())),
        ];
        let mut journal_state = ListState::default();
        journal_state.select(if selected >= 2 { Some(selected - 2) } else { None });
        let journal_block = Block::default()
            .title("Journal")
            .borders(Borders::ALL)
            .border_style(theme.border_style(focused && selected >= 2))
            .style(theme.base_style());
        f.render_stateful_widget(
            List::new(journal_items)
                .block(journal_block)
                .highlight_style(highlight),
            halves[1],
            &mut journal_state,
        );
    }
}
