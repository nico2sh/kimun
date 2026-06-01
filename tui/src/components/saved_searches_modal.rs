//! Global "Saved Searches" picker modal.
//!
//! A query box on top of a list of the vault's saved searches, with a pinned
//! virtual "Backlinks (current note)" entry at the top. Typing filters by name
//! and by a leading 1–9 quick-select index (an exact index match ranks first).
//! Enter emits [`AppEvent::SavedSearchSelected`] (the editor runs the query in
//! the panel) and closes; Esc closes; Delete removes the selected user entry.
//!
//! Hosts a [`SearchList`] engine: the vault load is a load-once
//! [`RowSource`] (`reload_on_query == false`), name/index ranking is the
//! [`Filter::Rank`] closure, the pinned backlinks row is supplied as the
//! engine's `leading_row`, and Delete is intercepted by the modal.

use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::{NoteVault, SavedSearch};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, ListItem, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, redraw_callback};
use crate::components::search_list::{
    Emit, Filter, KeyReaction, RowSource, SearchList, SearchMouse, SearchRow,
};
use crate::keys::key_combo::KeyCombo;
use crate::keys::{KeyBindings, key_event_to_combo};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// Model (pure, unit-tested)
// ---------------------------------------------------------------------------

/// One row in the modal. `index` is the 1–9 quick-select number (only the
/// first nine USER searches get one). The virtual backlinks entry is pinned
/// at the top (supplied as the engine's `leading_row`) and is never numbered
/// or deletable.
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

impl SearchRow for SearchItem {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        let prefix = match self.index {
            Some(n) => format!("{n} "),
            None => "  ".to_string(),
        };
        let label = if self.is_virtual {
            format!("{prefix}* {}", self.name)
        } else {
            format!("{prefix}{}", self.name)
        };
        let style = if self.is_virtual {
            Style::default()
                .fg(theme.accent.to_ratatui())
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(theme.fg.to_ratatui())
        };
        ListItem::new(label).style(style)
    }

    fn visual_height(&self) -> u16 {
        1
    }

    /// The virtual backlinks row is filter-exempt: returning `None` makes the
    /// engine keep it present regardless of the query (it is also prepended by
    /// the engine when the rank closure drops it).
    fn match_text(&self) -> Option<&str> {
        if self.is_virtual {
            None
        } else {
            Some(&self.name)
        }
    }
}

pub const VIRTUAL_BACKLINKS_NAME: &str = "Backlinks (current note)";
pub const VIRTUAL_BACKLINKS_QUERY: &str = ">{note}";

pub struct SavedSearchesModel;

impl SavedSearchesModel {
    /// Build the USER rows from the vault's saved searches: the first nine get
    /// quick-select indices 1..=9, the rest are unnumbered. The pinned virtual
    /// backlinks row is NOT included here — the engine supplies it via
    /// [`RowSource::leading_row`].
    pub fn user_items(user: Vec<SavedSearch>) -> Vec<SearchItem> {
        user.into_iter()
            .enumerate()
            .map(|(i, s)| SearchItem {
                index: if i < 9 { Some((i + 1) as u8) } else { None },
                name: s.name,
                query: s.query,
                is_virtual: false,
            })
            .collect()
    }
}

/// Rank `rows` (USER rows only) by `filter`, returning DISPLAY INDICES into the
/// slice. An exact leading-index match (filter parses to a u8 equal to a row's
/// `index`) ranks that row first; otherwise a case-insensitive name substring
/// match. Stable order preserves the source order within a rank. Empty filter →
/// all indices in order. The engine re-adds any filter-exempt rows (the virtual
/// backlinks row) that this closure omits, so it may ignore the virtual row.
pub fn rank_to_indices(rows: &[SearchItem], filter: &str) -> Vec<usize> {
    let f = filter.trim();
    if f.is_empty() {
        return (0..rows.len()).collect();
    }
    let as_index: Option<u8> = f.parse().ok();
    let needle = f.to_lowercase();
    let mut ranked: Vec<(usize, u8)> = Vec::new(); // (index, rank: 0 = best)
    for (i, it) in rows.iter().enumerate() {
        let exact_index = as_index.is_some() && it.index == as_index;
        let name_match = it.name.to_lowercase().contains(&needle);
        if exact_index {
            ranked.push((i, 0));
        } else if name_match {
            ranked.push((i, 1));
        }
    }
    // stable sort by rank keeps original relative order within a rank
    ranked.sort_by_key(|(_, r)| *r);
    ranked.into_iter().map(|(i, _)| i).collect()
}

// ---------------------------------------------------------------------------
// RowSource
// ---------------------------------------------------------------------------

/// Loads the vault's saved searches once (`reload_on_query == false`); the
/// local [`Filter::Rank`] narrows the set per keystroke. The virtual backlinks
/// row is supplied by [`leading_row`](RowSource::leading_row), not the load.
struct SavedSearchSource {
    vault: Arc<NoteVault>,
}

#[async_trait]
impl RowSource<SearchItem> for SavedSearchSource {
    async fn load(&self, _query: &str, emit: Emit<SearchItem>) {
        let user = self.vault.list_saved_searches().await.unwrap_or_default();
        emit.replace(SavedSearchesModel::user_items(user));
    }

    fn leading_row(&self, _query: &str) -> Option<SearchItem> {
        Some(SearchItem {
            index: None,
            name: VIRTUAL_BACKLINKS_NAME.to_string(),
            query: VIRTUAL_BACKLINKS_QUERY.to_string(),
            is_virtual: true,
        })
    }

    fn reload_on_query(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// SavedSearchesModal widget
// ---------------------------------------------------------------------------

pub struct SavedSearchesModal {
    list: SearchList<SearchItem>,
    vault: Arc<NoteVault>,
    delete_combo: KeyCombo,
}

impl SavedSearchesModal {
    pub fn new(vault: Arc<NoteVault>, _key_bindings: KeyBindings, icons: Icons, tx: AppTx) -> Self {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let delete_combo = key_event_to_combo(&KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE))
            .expect("Delete maps to a key combo");
        let list = SearchList::builder(
            SavedSearchSource {
                vault: vault.clone(),
            },
            redraw_callback(tx),
        )
        .filter(Filter::Rank(Arc::new(rank_to_indices)))
        .icons(icons)
        .intercept(vec![delete_combo])
        .build();
        Self {
            list,
            vault,
            delete_combo,
        }
    }
}

// ---------------------------------------------------------------------------
// Component impl
// ---------------------------------------------------------------------------

impl Component for SavedSearchesModal {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Mouse(mouse) => match self.list.handle_mouse(mouse) {
                SearchMouse::Activated(_) => {
                    if let Some(item) = self.list.selected_row() {
                        tx.send(AppEvent::SavedSearchSelected {
                            query: item.query.clone(),
                            name: item.name.clone(),
                        })
                        .ok();
                        tx.send(AppEvent::CloseSavedSearches).ok();
                    }
                    EventState::Consumed
                }
                SearchMouse::Selected(_) | SearchMouse::Scrolled => EventState::Consumed,
                SearchMouse::None => EventState::NotConsumed,
            },
            InputEvent::Key(key) => match self.list.handle_key(key) {
                KeyReaction::Intercepted(c) if c == self.delete_combo => {
                    if let Some(item) = self.list.selected_row().filter(|i| !i.is_virtual) {
                        let name = item.name.clone();
                        let vault = self.vault.clone();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            vault.delete_saved_search(&name).await.ok();
                            tx2.send(AppEvent::Redraw).ok();
                        });
                        self.list.reload();
                    }
                    EventState::Consumed
                }
                KeyReaction::Submit => {
                    if let Some(item) = self.list.selected_row() {
                        tx.send(AppEvent::SavedSearchSelected {
                            query: item.query.clone(),
                            name: item.name.clone(),
                        })
                        .ok();
                        tx.send(AppEvent::CloseSavedSearches).ok();
                    }
                    EventState::Consumed
                }
                KeyReaction::Cancel => {
                    tx.send(AppEvent::CloseSavedSearches).ok();
                    EventState::Consumed
                }
                KeyReaction::Consumed => EventState::Consumed,
                KeyReaction::Intercepted(_) | KeyReaction::Unhandled => EventState::NotConsumed,
            },
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, _focused: bool) {
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
        self.list.render_query(f, filter_inner, theme, true);

        // ── List ─────────────────────────────────────────────────────────────
        let list_block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style(false))
            .style(theme.panel_style());
        let list_inner = list_block.inner(rows[1]);
        f.render_widget(list_block, rows[1]);
        self.list.render(f, list_inner, theme, false);
        // Hand the engine the block's OUTER rect so mouse hit-testing accounts
        // for the leading border row.
        self.list.set_list_rect(rows[1]);

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
    use crate::settings::AppSettings;
    use crate::test_support::temp_vault;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn user_items_skip_virtual_and_number_first_nine() {
        let user: Vec<SavedSearch> = (0..11)
            .map(|i| SavedSearch {
                name: format!("s{i}"),
                query: format!("#{i}"),
            })
            .collect();
        let items = SavedSearchesModel::user_items(user);
        // No virtual row here — it is supplied by leading_row.
        assert!(items.iter().all(|i| !i.is_virtual));
        assert_eq!(items[0].index, Some(1));
        assert_eq!(items[8].index, Some(9));
        assert_eq!(items[9].index, None); // 10th user search unnumbered
    }

    #[test]
    fn rank_exact_index_first() {
        let items = vec![
            SearchItem::saved(1, "todo", "#todo"),
            SearchItem::saved(2, "backlinks-ish", ">{note}"),
            SearchItem::saved(3, "two-things", "#a"),
        ];
        let idx = rank_to_indices(&items, "2");
        assert_eq!(items[idx[0]].name, "backlinks-ish"); // index 2 wins
        let idx = rank_to_indices(&items, "tod");
        assert_eq!(items[idx[0]].name, "todo");
    }

    #[test]
    fn rank_empty_filter_returns_all_in_order() {
        let items = vec![
            SearchItem::saved(1, "a", "#a"),
            SearchItem::saved(2, "b", "#b"),
        ];
        let idx = rank_to_indices(&items, "");
        assert_eq!(idx, vec![0, 1]);
    }

    #[test]
    fn rank_name_substring_only_matches() {
        let items = vec![
            SearchItem::saved(1, "todo", "#todo"),
            SearchItem::saved(2, "ideas", "#ideas"),
        ];
        let idx = rank_to_indices(&items, "ide");
        assert_eq!(idx.len(), 1);
        assert_eq!(items[idx[0]].name, "ideas");
    }

    #[tokio::test]
    async fn enter_emits_selected_and_close() {
        let vault = temp_vault("saved_searches_modal").await;
        vault
            .save_search("todo", "#todo")
            .await
            .expect("save search");
        vault
            .save_search("ideas", "#ideas")
            .await
            .expect("save search");
        let settings = AppSettings::default();
        let (tx, mut rx) = unbounded_channel();
        let mut modal = SavedSearchesModal::new(
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx.clone(),
        );
        modal.list.poll_until_idle().await;

        modal.handle_input(
            &InputEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AppEvent::SavedSearchSelected { .. })),
            "expected SavedSearchSelected, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AppEvent::CloseSavedSearches)),
            "expected CloseSavedSearches, got {events:?}"
        );
    }
}
