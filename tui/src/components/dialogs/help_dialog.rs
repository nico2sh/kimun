use std::collections::BTreeMap;

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ShortcutCategory;
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// HelpRow
// ---------------------------------------------------------------------------

pub enum HelpRow {
    Header(String),
    Separator,
    Binding { keys: String, label: String },
    Blank,
}

// ---------------------------------------------------------------------------
// HelpDialog
// ---------------------------------------------------------------------------

pub struct HelpDialog {
    pub rows: Vec<HelpRow>,
    scroll: usize,
    /// Cached body height from last render, used for PageUp/PageDown page size.
    last_body_height: u16,
}

impl HelpDialog {
    pub fn new(key_bindings: &KeyBindings) -> Self {
        let mut by_category: BTreeMap<ShortcutCategory, Vec<(String, String)>> = BTreeMap::new();

        let map = key_bindings.to_hashmap();
        let mut entries: Vec<_> = map.into_iter().collect();
        entries.sort_by_key(|(action, _)| action.to_string());

        for (action, mut combos) in entries {
            combos.sort();
            let keys = combos
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" / ");
            let label = action.label();
            by_category
                .entry(action.category())
                .or_default()
                .push((keys, label));
        }

        let mut rows: Vec<HelpRow> = Vec::new();
        for (category, bindings) in by_category {
            if bindings.is_empty() {
                continue;
            }
            rows.push(HelpRow::Blank);
            rows.push(HelpRow::Header(category.to_string()));
            rows.push(HelpRow::Separator);
            for (keys, label) in bindings {
                rows.push(HelpRow::Binding { keys, label });
            }
        }
        rows.push(HelpRow::Blank);

        Self {
            rows,
            scroll: 0,
            last_body_height: 20,
        }
    }

    fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    fn scroll_down(&mut self) {
        // Clamped to rows.len() so render's slice is always valid even if called
        // between renders.
        self.scroll = self
            .scroll
            .saturating_add(1)
            .min(self.rows.len().saturating_sub(1));
    }

    fn page_up(&mut self) {
        let page = (self.last_body_height as usize).max(1);
        self.scroll = self.scroll.saturating_sub(page);
    }

    fn page_down(&mut self) {
        let page = (self.last_body_height as usize).max(1);
        self.scroll = self
            .scroll
            .saturating_add(page)
            .min(self.rows.len().saturating_sub(1));
    }

    /// Key handler — mirrors the `handle_key` pattern used by all other dialog types.
    pub fn handle_key(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        tx: &AppTx,
    ) -> EventState {
        match key.code {
            KeyCode::Esc => {
                tx.send(AppEvent::CloseDialog).ok();
            }
            KeyCode::Up => self.scroll_up(),
            KeyCode::Down => self.scroll_down(),
            KeyCode::PageUp => self.page_up(),
            KeyCode::PageDown => self.page_down(),
            _ => {}
        }
        EventState::Consumed
    }
}

const OUTER_WIDTH: u16 = 50;
const KEYS_COL_WIDTH: u16 = 18;

impl Component for HelpDialog {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        self.handle_key(*key, tx)
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let content_rows = self.rows.len() as u16;
        let desired_height = content_rows + 4; // borders(2) + footer(1) + bottom blank(1)
        let max_height = (rect.height * 60 / 100).max(10);
        let outer_height = desired_height.min(max_height);

        let popup_area = super::fixed_centered_rect(OUTER_WIDTH, outer_height, rect);
        f.render_widget(Clear, popup_area);

        let outer_block = Block::default()
            .title(" Keyboard Shortcuts ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.fg.to_ratatui()))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        if inner.height < 2 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let body_area = chunks[0];
        let footer_area = chunks[1];

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let fg_accent = theme.fg_selected.to_ratatui();

        // Cache for PageUp/PageDown.
        self.last_body_height = body_area.height;

        // Clamp scroll.
        let body_height = body_area.height as usize;
        let max_scroll = self.rows.len().saturating_sub(body_height);
        self.scroll = self.scroll.min(max_scroll);

        // Render visible rows.
        let visible = &self.rows[self.scroll..];
        for (y, row) in (body_area.y..).zip(visible.iter()) {
            if y >= body_area.y + body_area.height {
                break;
            }
            let row_rect = Rect {
                x: body_area.x,
                y,
                width: body_area.width,
                height: 1,
            };
            match row {
                HelpRow::Blank => {}
                HelpRow::Header(title) => {
                    f.render_widget(
                        Paragraph::new(format!("  {title}")).style(
                            Style::default()
                                .fg(fg_accent)
                                .bg(bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        row_rect,
                    );
                }
                HelpRow::Separator => {
                    super::render_separator(f, row_rect, fg_muted, bg);
                }
                HelpRow::Binding { keys, label } => {
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(2),
                            Constraint::Length(KEYS_COL_WIDTH),
                            Constraint::Min(1),
                        ])
                        .split(row_rect);
                    f.render_widget(
                        Paragraph::new(keys.as_str()).style(Style::default().fg(fg_accent).bg(bg)),
                        cols[1],
                    );
                    f.render_widget(
                        Paragraph::new(label.as_str()).style(Style::default().fg(fg).bg(bg)),
                        cols[2],
                    );
                }
            }
        }

        f.render_widget(
            Paragraph::new("  [↑↓ PgUp/PgDn] Scroll   [Esc] Close")
                .style(Style::default().fg(fg_muted).bg(bg)),
            footer_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyBindings;
    use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
    use crate::keys::key_strike::KeyStrike;

    fn bindings_with_bold_and_quit() -> KeyBindings {
        let mut kb = KeyBindings::empty();
        kb.batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyB, ActionShortcuts::Text(TextAction::Bold))
            .add(KeyStrike::KeyQ, ActionShortcuts::Quit);
        kb
    }

    #[test]
    fn rows_contain_both_categories() {
        let dialog = HelpDialog::new(&bindings_with_bold_and_quit());
        let headers: Vec<String> = dialog
            .rows
            .iter()
            .filter_map(|r| {
                if let HelpRow::Header(s) = r {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(headers.contains(&"Text Editing".to_string()));
        assert!(headers.contains(&"Other".to_string()));
        assert!(!headers.contains(&"Navigation".to_string()));
        assert!(!headers.contains(&"Notes".to_string()));
    }

    #[test]
    fn binding_row_has_correct_keys_and_label() {
        let dialog = HelpDialog::new(&bindings_with_bold_and_quit());
        let binding = dialog.rows.iter().find_map(|r| {
            if let HelpRow::Binding { keys, label } = r
                && label == "Bold"
            {
                return Some(keys.clone());
            }
            None
        });
        assert!(binding.is_some(), "expected a Bold binding row");
        assert_eq!(binding.unwrap(), "ctrl&B");
    }

    #[test]
    fn empty_keybindings_produces_no_rows() {
        let dialog = HelpDialog::new(&KeyBindings::empty());
        assert!(
            !dialog
                .rows
                .iter()
                .any(|r| matches!(r, HelpRow::Binding { .. }))
        );
        assert!(!dialog.rows.iter().any(|r| matches!(r, HelpRow::Header(_))));
    }
}
