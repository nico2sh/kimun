//! The **Activity Rail** — the fixed-width icon strip on the far left of the
//! editor screen. Each cell names a drawer view; the active cell shows a
//! green edge bar and green glyph. CFG is pinned to the bottom.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::components::drawer::DrawerView;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::panel::panel_block;
use crate::settings::themes::Theme;

/// Total column width the rail occupies, borders included.
pub const RAIL_WIDTH: u16 = 7;

/// The rail items in presentation order. CFG is last and pinned to the
/// bottom of the strip by a spacer.
const ITEMS: [(&str, DrawerView); 6] = [
    ("FILES", DrawerView::Files),
    ("FIND", DrawerView::Find),
    ("TAGS", DrawerView::Tags),
    ("LINKS", DrawerView::Links),
    ("OUTL", DrawerView::Outline),
    ("CFG", DrawerView::Config),
];

/// The rail glyph for a drawer view, resolved through the icon set so the
/// nerd-font / ASCII fallback policy applies to the rail like everywhere else.
fn glyph_for(icons: &crate::settings::icons::Icons, view: DrawerView) -> &'static str {
    match view {
        DrawerView::Files => icons.rail_files,
        DrawerView::Find => icons.rail_find,
        DrawerView::Tags => icons.rail_tags,
        DrawerView::Links => icons.rail_links,
        DrawerView::Outline => icons.rail_outline,
        DrawerView::Config => icons.rail_config,
    }
}

/// Rows each rail cell occupies (glyph line + label line + gap).
const CELL_ROWS: u16 = 3;

pub struct ActivityRail {
    /// The item the keyboard cursor sits on (the item `Enter` opens).
    cursor: usize,
    /// The row each item was drawn at on the last render, for click
    /// hit-testing.
    item_rows: Vec<(DrawerView, Rect)>,
    /// Icon set resolving the rail glyphs (nerd-font / ASCII).
    icons: crate::settings::icons::Icons,
}

impl ActivityRail {
    pub fn new(icons: crate::settings::icons::Icons) -> Self {
        Self {
            cursor: 0,
            item_rows: Vec::new(),
            icons,
        }
    }

    /// The drawer view under the keyboard cursor.
    pub fn cursor_view(&self) -> DrawerView {
        ITEMS[self.cursor].1
    }

    /// Move the keyboard cursor onto `view` (e.g. after a click or a leader
    /// path switched the drawer), so rail navigation continues from there.
    pub fn set_cursor(&mut self, view: DrawerView) {
        if let Some(i) = ITEMS.iter().position(|(_, v)| *v == view) {
            self.cursor = i;
        }
    }

    /// The item at the given screen cell, from the last render.
    pub fn view_at(&self, column: u16, row: u16) -> Option<DrawerView> {
        self.item_rows
            .iter()
            .find(|(_, rect)| rect.contains(ratatui::layout::Position::new(column, row)))
            .map(|(view, _)| *view)
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![
            ("↑/↓".into(), "Move".into()),
            ("Enter".into(), "Open/close".into()),
        ]
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        // Click on a rail item → switch the drawer to it (spec §3); the
        // toggle-on-active-click refinement lands with Phase 03.
        if let InputEvent::Mouse(mouse) = event {
            use ratatui::crossterm::event::{MouseButton, MouseEventKind};
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(view) = self.view_at(mouse.column, mouse.row)
            {
                self.set_cursor(view);
                tx.send(AppEvent::OpenDrawerView(view)).ok();
                return EventState::Consumed;
            }
            return EventState::NotConsumed;
        }
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                EventState::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(ITEMS.len() - 1);
                EventState::Consumed
            }
            KeyCode::Enter => {
                tx.send(AppEvent::OpenDrawerView(self.cursor_view())).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    /// `active` is the drawer view currently shown (None when the drawer is
    /// hidden); it gets the green edge bar + glyph.
    pub fn render(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        theme: &Theme,
        focused: bool,
        active: Option<DrawerView>,
    ) {
        let block = panel_block("", theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        self.item_rows.clear();

        let accent = Style::default().fg(theme.focus_border.to_ratatui());
        let dim = Style::default().fg(theme.gray.to_ratatui());
        let cursor_style = Style::default()
            .fg(theme.fg_bright.to_ratatui())
            .add_modifier(Modifier::BOLD);

        // CFG (last item) is pinned to the bottom; the rest stack from the top.
        let (top_items, bottom_item) = ITEMS.split_at(ITEMS.len() - 1);

        let icons = self.icons.clone();
        let draw = |idx: usize,
                    label: &str,
                    view: DrawerView,
                    y: u16,
                    f: &mut Frame,
                    rows: &mut Vec<(DrawerView, Rect)>| {
            if y + 1 >= inner.bottom() {
                return;
            }
            let glyph = glyph_for(&icons, view);
            let is_active = active == Some(view);
            let is_cursor = focused && idx == self.cursor;
            let glyph_style = if is_active {
                accent
            } else if is_cursor {
                cursor_style
            } else {
                dim
            };
            let label_style = if is_cursor { cursor_style } else { dim };
            let cell = Rect::new(inner.x, y, inner.width, 2);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(format!(" {glyph} "), glyph_style)),
                    Line::from(Span::styled(format!(" {label}"), label_style)),
                ]),
                cell,
            );
            // CFG is drawn last; on cramped rails its cell can overlap a top
            // item — insert at the FRONT so hit-testing favors the
            // most-recently drawn (topmost) cell.
            rows.insert(0, (view, cell));
        };

        let mut y = inner.y;
        for (i, (label, view)) in top_items.iter().enumerate() {
            draw(i, label, *view, y, f, &mut self.item_rows);
            y += CELL_ROWS;
        }
        // Bottom-pinned CFG.
        let (label, view) = bottom_item[0];
        let cfg_y = inner.bottom().saturating_sub(2).max(y);
        draw(ITEMS.len() - 1, label, view, cfg_y, f, &mut self.item_rows);

        // The active item's marker is the rail's own left border: recolor the
        // border segment beside the active cell green (and thicken it), so
        // the highlight reads as part of the panel chrome rather than an
        // extra in-cell bar.
        if let Some((_, cell)) = self
            .item_rows
            .iter()
            .find(|(view, _)| active == Some(*view))
        {
            let buf = f.buffer_mut();
            for dy in 0..cell.height {
                let pos = ratatui::layout::Position::new(rect.x, cell.y + dy);
                if let Some(border_cell) = buf.cell_mut(pos) {
                    border_cell.set_symbol("┃");
                    border_cell.set_fg(theme.focus_border.to_ratatui());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn test_rail() -> ActivityRail {
        ActivityRail::new(crate::settings::icons::Icons::new(false))
    }

    #[test]
    fn cursor_moves_and_clamps() {
        let mut rail = test_rail();
        let (tx, _rx) = unbounded_channel();
        assert_eq!(rail.cursor_view(), DrawerView::Files);

        rail.handle_input(&key(KeyCode::Up), &tx);
        assert_eq!(rail.cursor_view(), DrawerView::Files); // clamped at top

        rail.handle_input(&key(KeyCode::Down), &tx);
        assert_eq!(rail.cursor_view(), DrawerView::Find);
        for _ in 0..10 {
            rail.handle_input(&key(KeyCode::Down), &tx);
        }
        assert_eq!(rail.cursor_view(), DrawerView::Config); // clamped at bottom
    }

    #[test]
    fn enter_emits_open_drawer_view() {
        let mut rail = test_rail();
        let (tx, mut rx) = unbounded_channel();
        rail.handle_input(&key(KeyCode::Down), &tx);
        rail.handle_input(&key(KeyCode::Enter), &tx);
        match rx.try_recv() {
            Ok(AppEvent::OpenDrawerView(view)) => assert_eq!(view, DrawerView::Find),
            other => panic!("expected OpenDrawerView, got {other:?}"),
        }
    }

    #[test]
    fn set_cursor_tracks_view() {
        let mut rail = test_rail();
        rail.set_cursor(DrawerView::Outline);
        assert_eq!(rail.cursor_view(), DrawerView::Outline);
    }
}
