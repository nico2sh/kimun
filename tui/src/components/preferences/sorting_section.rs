use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use unicode_width::UnicodeWidthStr;

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::logic::message::AppTx;
use crate::logic::preferences::{PreferencesModel, SortSlot};
use crate::settings::themes::Theme;

use super::{ClickMap, text_rect};

/// The Sorting section: default and journal sort field/order. The values
/// live in the [`PreferencesModel`] (cycling with them, see
/// [`PreferencesModel::cycle_sort`]); rows are [`SortSlot::ALL`] in order.
pub struct SortingSection {
    list_state: ListState,
    clicks: ClickMap,
}

impl Default for SortingSection {
    fn default() -> Self {
        Self::new()
    }
}

impl SortingSection {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            list_state,
            clicks: ClickMap::default(),
        }
    }

    /// Resolve a click: a row selects it, its `[value]` (or a re-click on
    /// the selected row) cycles it. Returns the key the click stands for.
    pub fn handle_mouse(
        &mut self,
        m: &ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyEvent> {
        use ratatui::crossterm::event::KeyCode;
        self.clicks
            .resolve(m, self.list_state.selected(), Some(KeyCode::Enter))
            .into_key(|row| self.list_state.select(Some(row)))
    }

    /// `  Sort field:  [Name]` with the `[value]` as a click target for
    /// `row`, on `line` of `inner` (the drawn block's inner area).
    fn value_row(
        &mut self,
        inner: Rect,
        line: usize,
        row: usize,
        label: &'static str,
        value: &str,
        theme: &Theme,
    ) -> ListItem<'static> {
        let r = if (line as u16) < inner.height {
            Rect::new(inner.x, inner.y + line as u16, inner.width, 1)
        } else {
            Rect::default()
        };
        let chip = format!("[{value}]");
        self.clicks.row(r, row);
        self.clicks.control(
            text_rect(r, label.width() as u16, chip.width() as u16),
            row,
            ratatui::crossterm::event::KeyCode::Enter,
        );
        ListItem::new(Line::from(vec![
            Span::raw(label),
            Span::styled(chip, theme.action()),
        ]))
        .style(Style::default().fg(theme.fg.to_ratatui()))
    }

    const ROW_COUNT: usize = SortSlot::ALL.len();

    pub fn handle_input(
        &mut self,
        event: &InputEvent,
        model: &mut PreferencesModel,
        _tx: &AppTx,
    ) -> EventState {
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
                if let Some(slot) = SortSlot::ALL.get(selected) {
                    model.cycle_sort(*slot);
                }
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
            .fg(theme.selection_fg.to_ratatui())
            .bg(theme.selection_bg.to_ratatui());

        // ── Default sub-block (rows 0–1) ───────────────────────────────────
        self.clicks.clear();
        let default_block = Block::default()
            .title("Default")
            .borders(Borders::ALL)
            .border_style(theme.border_style(focused && selected < 2))
            .style(theme.base_style());
        let default_inner = default_block.inner(halves[0]);
        let default_items = vec![
            self.value_row(
                default_inner,
                0,
                0,
                "  Sort field:  ",
                model.sort_label(SortSlot::DefaultField),
                theme,
            ),
            self.value_row(
                default_inner,
                1,
                1,
                "  Sort order:  ",
                model.sort_label(SortSlot::DefaultOrder),
                theme,
            ),
        ];
        let mut default_state = ListState::default();
        default_state.select(if selected < 2 { Some(selected) } else { None });
        f.render_stateful_widget(
            List::new(default_items)
                .block(default_block)
                .highlight_style(highlight),
            halves[0],
            &mut default_state,
        );

        // ── Journal sub-block (rows 2–3) ────────────────────────────────────
        let journal_block = Block::default()
            .title("Journal")
            .borders(Borders::ALL)
            .border_style(theme.border_style(focused && selected >= 2))
            .style(theme.base_style());
        let journal_inner = journal_block.inner(halves[1]);
        let journal_items = vec![
            self.value_row(
                journal_inner,
                0,
                2,
                "  Sort field:  ",
                model.sort_label(SortSlot::JournalField),
                theme,
            ),
            self.value_row(
                journal_inner,
                1,
                3,
                "  Sort order:  ",
                model.sort_label(SortSlot::JournalOrder),
                theme,
            ),
        ];
        let mut journal_state = ListState::default();
        journal_state.select(if selected >= 2 {
            Some(selected - 2)
        } else {
            None
        });
        f.render_stateful_widget(
            List::new(journal_items)
                .block(journal_block)
                .highlight_style(highlight),
            halves[1],
            &mut journal_state,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::from(code))
    }

    #[test]
    fn enter_cycles_the_selected_slot_only() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = SortingSection::new();
        let base = PreferencesModel::new(&AppSettings::default());
        let mut m = base.clone();
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Down), &mut m, &tx);
        section.handle_input(&key(KeyCode::Enter), &mut m, &tx);
        assert_ne!(m.journal_sort_field, base.journal_sort_field);
        assert_eq!(m.default_sort_field, base.default_sort_field);
        assert_eq!(m.default_sort_order, base.default_sort_order);
        assert_eq!(m.journal_sort_order, base.journal_sort_order);
        // Up wraps to the last slot.
        let mut section = SortingSection::new();
        let mut m = base.clone();
        section.handle_input(&key(KeyCode::Up), &mut m, &tx);
        section.handle_input(&key(KeyCode::Char(' ')), &mut m, &tx);
        assert_ne!(m.journal_sort_order, base.journal_sort_order);
    }
}
