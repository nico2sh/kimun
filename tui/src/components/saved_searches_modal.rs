//! Global "Saved Searches" picker modal.
//!
//! A filter box on top of a list of the vault's saved searches, with a pinned
//! virtual "Backlinks (current note)" entry at the top. Typing filters by name
//! and by a leading 1–9 quick-select index (an exact index match ranks first).
//! Enter emits [`AppEvent::SavedSearchSelected`] (the editor runs the query in
//! the panel) and closes; Esc closes; Delete removes the selected user entry.
//!
//! Structurally mirrors [`crate::components::note_browser::NoteBrowserModal`]
//! (minus the preview pane) — async load via a spawned task + `mpsc` channel,
//! polled in `render`.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use kimun_core::{NoteVault, SavedSearch};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::keys::KeyBindings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// Model (pure, unit-tested)
// ---------------------------------------------------------------------------

/// One row in the modal. `index` is the 1–9 quick-select number (only the
/// first nine USER searches get one). The virtual backlinks entry is pinned
/// at the top and is never numbered or deletable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchItem {
    pub index: Option<u8>,
    pub name: String,
    pub query: String,
    pub is_virtual: bool,
}

impl SearchItem {
    /// A normal (user) saved-search item with a quick-select index.
    pub fn saved(index: u8, name: &str, query: &str) -> Self {
        Self {
            index: Some(index),
            name: name.to_string(),
            query: query.to_string(),
            is_virtual: false,
        }
    }
}

pub const VIRTUAL_BACKLINKS_NAME: &str = "Backlinks (current note)";
pub const VIRTUAL_BACKLINKS_QUERY: &str = ">{note}";

pub struct SavedSearchesModel {
    items: Vec<SearchItem>,
}

impl SavedSearchesModel {
    /// Build from the vault's user searches: a pinned virtual backlinks entry
    /// first (no index, not deletable), then user searches with the first nine
    /// assigned indices 1..=9.
    pub fn new(user: Vec<SavedSearch>) -> Self {
        let mut items = Vec::with_capacity(user.len() + 1);
        items.push(SearchItem {
            index: None,
            name: VIRTUAL_BACKLINKS_NAME.to_string(),
            query: VIRTUAL_BACKLINKS_QUERY.to_string(),
            is_virtual: true,
        });
        for (i, s) in user.into_iter().enumerate() {
            let index = if i < 9 { Some((i + 1) as u8) } else { None };
            items.push(SearchItem {
                index,
                name: s.name,
                query: s.query,
                is_virtual: false,
            });
        }
        Self { items }
    }

    pub fn items(&self) -> &[SearchItem] {
        &self.items
    }
}

/// Filter + rank `items` by `filter`. An exact leading-index match (filter
/// parses to a u8 equal to an item's `index`) ranks that item first.
/// Otherwise: case-insensitive substring match on the name. Stable order
/// (preserves the model's order among equal-rank items). Empty filter →
/// all items, original order.
pub fn rank_items<'a>(items: &'a [SearchItem], filter: &str) -> Vec<&'a SearchItem> {
    let f = filter.trim();
    if f.is_empty() {
        return items.iter().collect();
    }
    let as_index: Option<u8> = f.parse().ok();
    let mut ranked: Vec<(&SearchItem, u8)> = Vec::new(); // (item, rank: 0 = best)
    for it in items {
        let exact_index = as_index.is_some() && it.index == as_index;
        let name_match = it.name.to_lowercase().contains(&f.to_lowercase());
        if exact_index {
            ranked.push((it, 0));
        } else if name_match {
            ranked.push((it, 1));
        }
    }
    // stable sort by rank keeps original relative order within a rank
    ranked.sort_by_key(|(_, r)| *r);
    ranked.into_iter().map(|(it, _)| it).collect()
}

// ---------------------------------------------------------------------------
// SavedSearchesModal widget
// ---------------------------------------------------------------------------

pub struct SavedSearchesModal {
    filter: SingleLineInput,
    model: SavedSearchesModel,
    list_state: ListState,
    list_rect: Rect,
    vault: Arc<NoteVault>,
    // Async-load plumbing (mirrors note_browser).
    load_task: Option<tokio::task::JoinHandle<()>>,
    load_rx: Option<Receiver<Vec<SavedSearch>>>,
    // Carried for parity with other modals; reserved for future use.
    #[allow(dead_code)]
    key_bindings: KeyBindings,
    #[allow(dead_code)]
    icons: Icons,
}

impl SavedSearchesModal {
    pub fn new(vault: Arc<NoteVault>, key_bindings: KeyBindings, icons: Icons, tx: AppTx) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut modal = Self {
            filter: SingleLineInput::new(),
            model: SavedSearchesModel::new(Vec::new()),
            list_state,
            list_rect: Rect::default(),
            vault,
            load_task: None,
            load_rx: None,
            key_bindings,
            icons,
        };
        modal.schedule_load(tx);
        modal
    }

    // ── Async list loading ─────────────────────────────────────────────────

    fn schedule_load(&mut self, tx: AppTx) {
        if let Some(handle) = self.load_task.take() {
            handle.abort();
        }
        let vault = Arc::clone(&self.vault);
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.load_rx = Some(result_rx);

        let handle = tokio::spawn(async move {
            let searches = vault.list_saved_searches().await.unwrap_or_default();
            result_tx.send(searches).ok();
            tx.send(AppEvent::Redraw).ok();
        });
        self.load_task = Some(handle);
    }

    fn poll_load(&mut self) {
        let Some(rx) = &self.load_rx else { return };
        match rx.try_recv() {
            Ok(searches) => {
                self.model = SavedSearchesModel::new(searches);
                self.load_rx = None;
                self.load_task = None;
                self.reset_selection();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.load_rx = None;
                self.load_task = None;
            }
        }
    }

    // ── Selection helpers (operate on the ranked/filtered view) ─────────────

    /// Number of rows in the current ranked view.
    fn ranked_len(&self) -> usize {
        rank_items(self.model.items(), self.filter.value()).len()
    }

    /// Reset the selection to the first row (or clear it if the view is empty).
    fn reset_selection(&mut self) {
        if self.ranked_len() == 0 {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn select_prev(&mut self) {
        let len = self.ranked_len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0);
        let next = if cur == 0 { len - 1 } else { cur - 1 };
        self.list_state.select(Some(next));
    }

    fn select_next(&mut self) {
        let len = self.ranked_len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0);
        let next = if cur + 1 >= len { 0 } else { cur + 1 };
        self.list_state.select(Some(next));
    }

    /// The currently selected item in the ranked view, if any.
    fn selected_item(&self) -> Option<SearchItem> {
        let ranked = rank_items(self.model.items(), self.filter.value());
        let idx = self.list_state.selected()?;
        ranked.get(idx).map(|it| (*it).clone())
    }

    fn open_selected(&self, tx: &AppTx) {
        if let Some(item) = self.selected_item() {
            tx.send(AppEvent::SavedSearchSelected {
                query: item.query.clone(),
                name: item.name.clone(),
            })
            .ok();
            tx.send(AppEvent::CloseSavedSearches).ok();
        }
    }

    /// Delete the selected user entry (ignored for the virtual entry), then
    /// reload the list. Spawns the vault delete in the background and
    /// re-schedules the load.
    fn delete_selected(&mut self, tx: &AppTx) {
        let Some(item) = self.selected_item() else {
            return;
        };
        if item.is_virtual {
            return;
        }
        let vault = Arc::clone(&self.vault);
        let name = item.name.clone();
        let tx_for_task = tx.clone();
        tokio::spawn(async move {
            vault.delete_saved_search(&name).await.ok();
            tx_for_task.send(AppEvent::Redraw).ok();
        });
        // Re-load the list so the deleted entry disappears. The delete and
        // the subsequent list read both hit the same on-disk file; the
        // reload reflects whatever state the file is in once the read runs.
        self.schedule_load(tx.clone());
    }
}

// ---------------------------------------------------------------------------
// Component impl
// ---------------------------------------------------------------------------

impl Component for SavedSearchesModal {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};

        if let InputEvent::Mouse(mouse) = event {
            let r = self.list_rect;
            if !r.contains(Position {
                x: mouse.column,
                y: mouse.row,
            }) {
                return EventState::NotConsumed;
            }
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if mouse.row > r.y {
                        let rel_row = (mouse.row - r.y - 1) as usize;
                        let len = self.ranked_len();
                        if rel_row < len {
                            let prev = self.list_state.selected();
                            self.list_state.select(Some(rel_row));
                            if prev == Some(rel_row) {
                                self.open_selected(tx);
                            }
                        }
                    }
                    EventState::Consumed
                }
                MouseEventKind::ScrollUp => {
                    self.select_prev();
                    EventState::Consumed
                }
                MouseEventKind::ScrollDown => {
                    self.select_next();
                    EventState::Consumed
                }
                _ => EventState::Consumed,
            }
        } else {
            let InputEvent::Key(key) = event else {
                return EventState::NotConsumed;
            };

            match key.code {
                KeyCode::Up => {
                    self.select_prev();
                    return EventState::Consumed;
                }
                KeyCode::Down => {
                    self.select_next();
                    return EventState::Consumed;
                }
                KeyCode::Delete => {
                    self.delete_selected(tx);
                    return EventState::Consumed;
                }
                _ => {}
            }
            // Drop Ctrl/Alt-modified chars so combos don't leak as text.
            if let KeyCode::Char(_) = key.code {
                let non_shift = key.modifiers - KeyModifiers::SHIFT;
                if !non_shift.is_empty() {
                    return EventState::Consumed;
                }
            }
            let outcome = self.filter.handle_key(key);
            match outcome {
                InputOutcome::Cancel => {
                    tx.send(AppEvent::CloseSavedSearches).ok();
                    EventState::Consumed
                }
                InputOutcome::Submit => {
                    self.open_selected(tx);
                    EventState::Consumed
                }
                InputOutcome::Changed => {
                    self.reset_selection();
                    EventState::Consumed
                }
                InputOutcome::Consumed => EventState::Consumed,
                InputOutcome::NotConsumed => EventState::NotConsumed,
            }
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, _focused: bool) {
        self.poll_load();

        let popup_rect = centered_rect(60, 60, area);

        // Clear the area behind the modal so the editor doesn't bleed through.
        f.render_widget(Clear, popup_rect);

        let outer_block = Block::default()
            .title(" Saved Searches ")
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_rect);
        f.render_widget(outer_block, popup_rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        // ── Filter box ──────────────────────────────────────────────────────
        let filter_block = Block::default()
            .title(" Filter ")
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(theme.panel_style());
        let filter_inner = filter_block.inner(rows[0]);
        f.render_widget(filter_block, rows[0]);
        self.filter.render(
            f,
            filter_inner,
            Style::default()
                .fg(theme.fg.to_ratatui())
                .bg(theme.bg_panel.to_ratatui()),
            0,
            true,
        );

        // ── List ─────────────────────────────────────────────────────────────
        let list_block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style(false))
            .style(theme.panel_style());
        self.list_rect = rows[1];

        let ranked = rank_items(self.model.items(), self.filter.value());
        let items: Vec<ListItem> = ranked
            .iter()
            .map(|it| {
                let prefix = match it.index {
                    Some(n) => format!("{n} "),
                    None => "  ".to_string(),
                };
                let label = if it.is_virtual {
                    format!("{prefix}* {}", it.name)
                } else {
                    format!("{prefix}{}", it.name)
                };
                let style = if it.is_virtual {
                    Style::default()
                        .fg(theme.accent.to_ratatui())
                        .add_modifier(Modifier::ITALIC)
                } else {
                    Style::default().fg(theme.fg.to_ratatui())
                };
                ListItem::new(label).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(list_block)
            .highlight_style(
                Style::default()
                    .fg(theme.fg_selected.to_ratatui())
                    .bg(theme.bg_selected.to_ratatui()),
            )
            .highlight_symbol("> ");
        f.render_stateful_widget(list, rows[1], &mut self.list_state);

        // ── Hint bar ──────────────────────────────────────────────────────────
        f.render_widget(
            Paragraph::new("↑↓ navigate | Enter open | Del delete | Esc close")
                .style(Style::default().fg(theme.fg_secondary.to_ratatui())),
            rows[2],
        );
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![
            ("↑↓".to_string(), "navigate".to_string()),
            ("Enter".to_string(), "open".to_string()),
            ("Del".to_string(), "delete".to_string()),
            ("Esc".to_string(), "close".to_string()),
        ]
    }
}

// ---------------------------------------------------------------------------
// Layout helper
// ---------------------------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_height = area.height * percent_y / 100;
    let popup_width = area.width * percent_x / 100;
    Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_backlinks_entry_present_and_not_deletable() {
        let model = SavedSearchesModel::new(vec![]);
        assert!(model.items()[0].is_virtual);
        assert_eq!(model.items()[0].query, ">{note}");
        assert_eq!(model.items()[0].index, None);
    }

    #[test]
    fn first_nine_user_searches_numbered() {
        let user: Vec<SavedSearch> = (0..11)
            .map(|i| SavedSearch {
                name: format!("s{i}"),
                query: format!("#{i}"),
            })
            .collect();
        let model = SavedSearchesModel::new(user);
        // item[0] is virtual; user items start at item[1].
        assert_eq!(model.items()[1].index, Some(1));
        assert_eq!(model.items()[9].index, Some(9));
        assert_eq!(model.items()[10].index, None); // 10th user search unnumbered
    }

    #[test]
    fn filter_ranks_exact_index_first() {
        let items = vec![
            SearchItem::saved(1, "todo", "#todo"),
            SearchItem::saved(2, "backlinks-ish", ">{note}"),
            SearchItem::saved(3, "two-things", "#a"),
        ];
        let ranked = rank_items(&items, "2");
        assert_eq!(ranked[0].name, "backlinks-ish"); // index 2 wins
        let ranked = rank_items(&items, "tod");
        assert_eq!(ranked[0].name, "todo");
    }

    #[test]
    fn empty_filter_returns_all_in_order() {
        let items = vec![
            SearchItem::saved(1, "a", "#a"),
            SearchItem::saved(2, "b", "#b"),
        ];
        let ranked = rank_items(&items, "");
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].name, "a");
    }
}
