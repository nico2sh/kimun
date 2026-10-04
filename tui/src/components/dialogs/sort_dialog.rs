use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, SortTarget};
use crate::components::file_list::{SortField, SortOrder};
use crate::components::key_picker::{KeyPicker, PickerOutcome};
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::components::sortable::SortState;
use crate::settings::themes::Theme;

/// The selectable rows, in display order.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Field,
    Order,
    Key,
    GroupDirs,
}

/// Modal that edits one list's [`SortState`]: field / order, plus a "group
/// directories" toggle for lists that have one. Changes apply live: each
/// toggle emits `AppEvent::SortChanged` (`persist = false`). `s` (sidebar
/// only) emits the same event with `persist = true` (save as default);
/// Enter/Esc emit `CloseOverlay`.
pub struct SortDialog {
    target: SortTarget,
    pub(crate) field: SortField,
    pub(crate) order: SortOrder,
    /// `Some` drives the "Group directories" row.
    group_dirs: Option<bool>,
    /// The Field cycle reaches Property (query-backed lists only).
    allows_property: bool,
    rows: Vec<Row>,
    selected: usize,
    picker: KeyPicker,
    /// Screen rect of each row from the last render, for clicks.
    row_rects: Vec<Rect>,
}

impl SortDialog {
    /// A dialog for `target`, opened on its current `state`.
    /// `allows_property` comes from the list's `SortableList::allows_property`.
    pub fn new(target: SortTarget, state: SortState, allows_property: bool) -> Self {
        let SortState {
            field,
            order,
            group_dirs,
        } = state;
        let key = match &field {
            SortField::Property(k) => k.clone(),
            _ => String::new(),
        };
        let mut d = Self {
            target,
            field,
            order,
            group_dirs,
            allows_property,
            rows: Vec::new(),
            selected: 0,
            picker: KeyPicker::new(&key),
            row_rects: Vec::new(),
        };
        d.rebuild_rows();
        d
    }

    /// Rows depend on the field: the Key row only exists for a property sort.
    fn rebuild_rows(&mut self) {
        let mut rows = vec![Row::Field, Row::Order];
        if matches!(self.field, SortField::Property(_)) {
            rows.push(Row::Key);
        }
        if self.group_dirs.is_some() {
            rows.push(Row::GroupDirs);
        }
        self.rows = rows;
        self.selected = self.selected.min(self.rows.len() - 1);
    }

    pub(crate) fn set_keys(&mut self, keys: Vec<String>) {
        self.picker.set_keys(keys);
    }

    #[cfg(test)]
    pub(crate) fn key_value(&self) -> &str {
        self.picker.value()
    }

    #[cfg(test)]
    pub(crate) fn row_origin(&self, i: usize) -> Option<(u16, u16)> {
        self.row_rects.get(i).map(|r| (r.x, r.y))
    }

    #[cfg(test)]
    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Emit the current selection. `persist` requests saving it as the default
    /// (sidebar's `s` key); a plain toggle sends `persist = false` for live apply.
    fn emit(&self, tx: &AppTx, persist: bool) {
        // A property sort with no key yet is not a usable order.
        if matches!(&self.field, SortField::Property(k) if k.trim().is_empty()) {
            return;
        }
        tx.send(AppEvent::SortChanged {
            target: self.target,
            state: SortState {
                field: self.field.clone(),
                order: self.order,
                group_dirs: self.group_dirs,
            },
            persist,
        })
        .ok();
    }

    fn toggle_selected(&mut self, tx: &AppTx) {
        match self.rows[self.selected] {
            Row::Field => {
                self.field = self.field.cycle(self.allows_property);
                if matches!(self.field, SortField::Property(_)) && !self.picker.value().is_empty() {
                    self.field = SortField::Property(self.picker.value().to_string());
                }
                self.rebuild_rows();
            }
            Row::Order => self.order = self.order.toggle(),
            Row::Key => {}
            Row::GroupDirs => self.group_dirs = self.group_dirs.map(|g| !g),
        }
        self.emit(tx, false);
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        if self.rows[self.selected] == Row::Key {
            let list_open = self.picker.is_list_open();
            let passthrough =
                !list_open && matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::Esc);
            if !passthrough {
                match self.picker.handle_key(&key) {
                    PickerOutcome::Accepted(k) => {
                        self.field = SortField::Property(k);
                        self.emit(tx, false);
                    }
                    PickerOutcome::Submit => {
                        let typed = self.picker.value().trim().to_string();
                        if !typed.is_empty() && self.field != SortField::Property(typed.clone()) {
                            self.field = SortField::Property(typed);
                            self.emit(tx, false);
                        } else {
                            tx.send(AppEvent::CloseOverlay).ok();
                        }
                    }
                    PickerOutcome::Cancel => {
                        tx.send(AppEvent::CloseOverlay).ok();
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }
        }
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

    pub fn handle_mouse(&mut self, ev: &MouseEvent, tx: &AppTx) -> EventState {
        if !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
            return EventState::Consumed;
        }
        let key_row = self.rows.iter().position(|r| *r == Row::Key);
        if key_row.is_some() {
            match self.picker.handle_click(ev.column, ev.row) {
                PickerOutcome::Accepted(k) => {
                    self.field = SortField::Property(k);
                    self.emit(tx, false);
                    return EventState::Consumed;
                }
                PickerOutcome::Consumed => {
                    if let Some(i) = key_row {
                        self.selected = i;
                    }
                    return EventState::Consumed;
                }
                _ => {}
            }
        }
        let pos = Position {
            x: ev.column,
            y: ev.row,
        };
        if let Some(i) = self.row_rects.iter().position(|r| r.contains(pos)) {
            self.selected = i;
            if self.rows[i] != Row::Key {
                self.toggle_selected(tx);
            }
        }
        EventState::Consumed
    }

    fn row_label(&self, row: Row) -> (String, String) {
        match row {
            Row::Field => (
                "Sort by".to_string(),
                match &self.field {
                    SortField::Name => "Name".to_string(),
                    SortField::Title => "Title".to_string(),
                    SortField::Property(_) => "Property".to_string(),
                },
            ),
            Row::Key => ("Key".to_string(), String::new()),
            Row::Order => (
                "Order".to_string(),
                match self.order {
                    SortOrder::Ascending => "Ascending \u{2191}".to_string(),
                    SortOrder::Descending => "Descending \u{2193}".to_string(),
                },
            ),
            Row::GroupDirs => (
                "Group directories".to_string(),
                if self.group_dirs == Some(true) {
                    "On"
                } else {
                    "Off"
                }
                .to_string(),
            ),
        }
    }
}

const OUTER_WIDTH: u16 = 44;

impl crate::components::Component for SortDialog {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => self.handle_key(*key, tx),
            InputEvent::Mouse(m) => self.handle_mouse(m, tx),
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        // rows + borders(2) + footer(1).
        let list_open = self.rows[self.selected] == Row::Key && self.picker.is_list_open();
        let outer_height = self.rows.len() as u16 + 3 + if list_open { 5 } else { 0 };
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

        self.row_rects.clear();
        let mut key_field = None;
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
            let row_rect = Rect {
                x: body.x,
                y,
                width: body.width,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(format!(" {marker} {label:<20}{value}")).style(style),
                row_rect,
            );
            self.row_rects.push(row_rect);
            if row == Row::Key {
                let off = 3 + 20;
                key_field = Some((
                    Rect {
                        x: row_rect.x + off.min(row_rect.width),
                        y,
                        width: row_rect.width.saturating_sub(off),
                        height: 1,
                    },
                    selected,
                ));
            }
        }

        let key_selected = self.rows[self.selected] == Row::Key;
        let footer = if key_selected {
            "  type a key · [Enter] apply · [Esc] close"
        } else if self.target == SortTarget::Sidebar {
            "  [↑↓] Move  [Space] Toggle  [s] Save default  [Enter/Esc] Close"
        } else {
            "  [↑↓] Move  [Space] Toggle  [Enter/Esc] Close  · click a row"
        };
        f.render_widget(
            Paragraph::new(footer).style(Style::default().fg(gray).bg(bg)),
            footer_area,
        );

        // Last, so the suggestion list draws over the rows and footer below.
        if let Some((field, focused)) = key_field {
            self.picker.render(f, field, inner, theme, focused);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::SortTarget;
    use crate::components::file_list::{SortField, SortOrder};
    use crate::components::sortable::SortState;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn state(field: SortField, group_dirs: Option<bool>) -> SortState {
        SortState {
            field,
            order: SortOrder::Ascending,
            group_dirs,
        }
    }

    fn sidebar_dialog() -> SortDialog {
        SortDialog::new(
            SortTarget::Sidebar,
            state(SortField::Name, Some(false)),
            false,
        )
    }

    #[test]
    fn empty_property_field_emits_nothing() {
        let mut d = SortDialog::new(SortTarget::Query, state(SortField::Name, None), true);
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx); // Name -> Title
        assert!(rx.try_recv().is_ok());
        d.handle_key(key(KeyCode::Char(' ')), &tx); // Title -> Property("")
        assert_eq!(d.field, SortField::Property(String::new()));
        assert!(rx.try_recv().is_err(), "no event for an empty key");
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
                state,
                persist,
            } => {
                assert_eq!(target, SortTarget::Sidebar);
                assert_eq!(state.field, SortField::Title);
                assert_eq!(state.order, SortOrder::Ascending);
                assert_eq!(state.group_dirs, Some(false));
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
    fn group_row_present_only_with_group_dirs() {
        let sidebar = sidebar_dialog();
        assert_eq!(sidebar.row_count(), 3);
        let query = SortDialog::new(SortTarget::Query, state(SortField::Name, None), true);
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

        let mut q = SortDialog::new(SortTarget::Query, state(SortField::Name, None), true);
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

    use crate::components::events::InputEvent;
    use crate::settings::themes::Theme;
    use ratatui::{Terminal, backend::TestBackend};

    fn query_dialog(field: SortField) -> SortDialog {
        SortDialog::new(SortTarget::Query, state(field, None), true)
    }

    fn draw(d: &mut SortDialog) {
        use crate::components::Component;
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| d.render(f, f.area(), &theme, true)).unwrap();
    }

    fn mouse(col: u16, row: u16) -> ratatui::crossterm::event::MouseEvent {
        match crate::test_support::mouse_down_at(col, row) {
            InputEvent::Mouse(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn query_cycle_reaches_property_and_waits_for_a_key() {
        let mut d = query_dialog(SortField::Title);
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Property(String::new()));
        assert!(rx.try_recv().is_err(), "no emit before a key is chosen");
        assert_eq!(d.row_count(), 3, "Key row appears");
    }

    #[test]
    fn accepting_a_key_emits_property_sort() {
        let mut d = query_dialog(SortField::Property(String::new()));
        d.set_keys(vec!["due".into(), "status".into()]);
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Down), &tx); // Order
        d.handle_key(key(KeyCode::Down), &tx); // Key
        for c in "du".chars() {
            d.handle_key(key(KeyCode::Char(c)), &tx);
        }
        d.handle_key(key(KeyCode::Enter), &tx); // accept "due" from the list
        match rx.try_recv() {
            Ok(AppEvent::SortChanged { state, .. }) => {
                assert_eq!(state.field, SortField::Property("due".into()))
            }
            other => panic!("expected SortChanged, got {other:?}"),
        }
    }

    #[test]
    fn opening_with_a_property_sort_preselects_it() {
        let d = query_dialog(SortField::Property("due".into()));
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.key_value(), "due");
    }

    #[test]
    fn sidebar_never_cycles_to_property() {
        let mut d = sidebar_dialog();
        let (tx, _rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx); // Name -> Title
        d.handle_key(key(KeyCode::Char(' ')), &tx); // Title -> Name
        assert_eq!(d.field, SortField::Name);
    }

    #[test]
    fn click_on_a_row_selects_and_toggles_it() {
        let mut d = sidebar_dialog();
        draw(&mut d);
        let (tx, mut rx) = unbounded_channel();
        let (x, y) = d.row_origin(1).expect("Order row rendered");
        d.handle_mouse(&mouse(x + 2, y), &tx);
        assert_eq!(d.order, SortOrder::Descending);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::SortChanged { .. })));
    }

    #[test]
    fn click_outside_sort_modal_does_nothing() {
        let mut d = sidebar_dialog();
        draw(&mut d);
        let (tx, mut rx) = unbounded_channel();
        d.handle_mouse(&mouse(0, 0), &tx);
        assert!(rx.try_recv().is_err());
        assert_eq!(d.field, SortField::Name);
    }

    #[test]
    fn enter_on_key_row_with_list_closed_closes_overlay() {
        let mut d = query_dialog(SortField::Property("due".into()));
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Down), &tx);
        d.handle_key(key(KeyCode::Down), &tx);
        d.handle_key(key(KeyCode::Enter), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
    }

    /// The browser target: property sorts allowed, no group row, no `s`.
    #[test]
    fn browser_dialog_cycles_to_property_without_group_row() {
        let mut d = SortDialog::new(SortTarget::Browser, state(SortField::Title, None), true);
        assert_eq!(d.row_count(), 2, "no group row without group_dirs");
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Property(String::new()));
        d.handle_key(key(KeyCode::Char('s')), &tx);
        assert!(rx.try_recv().is_err(), "browser has no save-default");
    }

    /// The sidebar sorts by property now: its dialog cycles to Property and
    /// shows the Key row next to the group row.
    #[test]
    fn sidebar_dialog_cycles_to_property_with_group_row() {
        let mut d = SortDialog::new(
            SortTarget::Sidebar,
            state(SortField::Title, Some(true)),
            true,
        );
        assert_eq!(d.row_count(), 3);
        let (tx, _rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Property(String::new()));
        assert_eq!(d.row_count(), 4, "field, order, key and group rows");
    }

    /// `allows_property = false` never reaches Property, whatever the target.
    #[test]
    fn disallowed_property_never_cycles_to_property() {
        let mut d = SortDialog::new(SortTarget::Query, state(SortField::Name, None), false);
        let (tx, _rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Name);
    }
}
