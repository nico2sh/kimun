//! Global "Saved Searches" picker modal.
//!
//! A query box on top of a list of the vault's saved searches, with a pinned
//! virtual "Backlinks (current note)" entry at the top. Typing filters by name
//! and by a leading 1–9 quick-select index (an exact index match ranks first).
//! Enter emits [`SavedSearchFlow::Selected`] (the editor runs the query in
//! the panel and closes this overlay itself); Esc emits
//! [`AppMessage::CloseOverlay`]; Delete removes the selected user entry.
//!
//! Hosts a [`SearchList`] engine: the vault load is a load-once
//! [`RowSource`](crate::logic::list::RowSource) (`reload_on_query == false`),
//! name/index ranking is the [`Filter::Rank`] closure, the pinned backlinks
//! row is supplied as the engine's `leading_row`, and Delete is intercepted by
//! the modal. The rows, ranking and source live in
//! [`crate::logic::saved_search`] (re-exported here).

use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, ListItem, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::components::overlay::{Overlay, OverlayKind};
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::components::search_list::{Filter, KeyReaction, SearchList, SearchMouse, SearchRow};
use crate::keys::key_combo::KeyCombo;
use crate::keys::{KeyBindings, key_event_to_combo};
use crate::logic::message::{AppMessage, AppTx, SavedSearchFlow, redraw_callback};
use crate::logic::saved_search::SavedSearchSource;
pub use crate::logic::saved_search::{
    SavedSearchesModel, SearchItem, VIRTUAL_BACKLINKS_NAME, VIRTUAL_BACKLINKS_QUERY,
    rank_to_indices,
};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

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
}

// ---------------------------------------------------------------------------
// SavedSearchesModal widget
// ---------------------------------------------------------------------------

pub struct SavedSearchesModal {
    list: SearchList<SearchItem>,
    /// Shared with the [`SavedSearchSource`]: setting this then calling
    /// `list.reload()` makes the source delete-then-list in one ordered load.
    pending_delete: Arc<std::sync::Mutex<Option<String>>>,
    delete_combo: KeyCombo,
}

impl SavedSearchesModal {
    pub fn new(vault: Arc<NoteVault>, key_bindings: KeyBindings, icons: Icons, tx: AppTx) -> Self {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let delete_combo = key_event_to_combo(&KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE))
            .expect("Delete maps to a key combo");
        let pending_delete = Arc::new(std::sync::Mutex::new(None));
        let list = SearchList::builder(
            SavedSearchSource::new(vault, pending_delete.clone()),
            redraw_callback(tx),
        )
        .filter(Filter::Rank(Arc::new(rank_to_indices)))
        .yank_combos_from(&key_bindings)
        .icons(icons)
        .intercept(vec![delete_combo])
        .build();
        Self {
            list,
            pending_delete,
            delete_combo,
        }
    }
}

// ---------------------------------------------------------------------------
// Overlay impl
// ---------------------------------------------------------------------------

impl Overlay for SavedSearchesModal {
    fn kind(&self) -> OverlayKind {
        OverlayKind::SavedSearches
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Mouse(mouse) => match self.list.handle_mouse(mouse) {
                SearchMouse::Activated(_) | SearchMouse::DoubleClicked { repeat: false, .. } => {
                    if let Some(item) = self.list.selected_row() {
                        tx.send(AppMessage::SavedSearch(SavedSearchFlow::Selected {
                            query: item.query.clone(),
                            name: item.name.clone(),
                        }))
                        .ok();
                    }
                    EventState::Consumed
                }
                SearchMouse::Context(_) | SearchMouse::Selected(_) | SearchMouse::Scrolled => {
                    EventState::Consumed
                }
                // No content sub-region is recorded by this host, so these
                // are unreachable.
                SearchMouse::ContentScrollUp | SearchMouse::ContentScrollDown => {
                    EventState::Consumed
                }
                // A repeat double-click: its first press already activated.
                SearchMouse::InputFocused
                | SearchMouse::Autocomplete { .. }
                | SearchMouse::DoubleClicked { repeat: true, .. } => EventState::Consumed,
                SearchMouse::None => EventState::NotConsumed,
            },
            InputEvent::Key(key) => match self.list.handle_key(key) {
                KeyReaction::Intercepted(c) if c == self.delete_combo => {
                    if let Some(item) = self.list.selected_row().filter(|i| !i.is_virtual) {
                        // Hand the name to the source and re-run the load: the
                        // source deletes-then-lists in one ordered async step, so
                        // the new rows can never reflect pre-delete state.
                        *self.pending_delete.lock().unwrap() = Some(item.name.clone());
                        self.list.reload();
                    }
                    EventState::Consumed
                }
                KeyReaction::Submit => {
                    if let Some(item) = self.list.selected_row() {
                        tx.send(AppMessage::SavedSearch(SavedSearchFlow::Selected {
                            query: item.query.clone(),
                            name: item.name.clone(),
                        }))
                        .ok();
                    }
                    EventState::Consumed
                }
                KeyReaction::Cancel => {
                    tx.send(AppMessage::CloseOverlay).ok();
                    EventState::Consumed
                }
                KeyReaction::Consumed => EventState::Consumed,
                KeyReaction::Yank(target) => {
                    crate::components::yank_row(target, tx);
                    EventState::Consumed
                }
                KeyReaction::Intercepted(_) | KeyReaction::ListVerb(_) | KeyReaction::Unhandled => {
                    EventState::NotConsumed
                }
            },
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let popup_rect = crate::components::centered_rect(60, 60, area);

        let inner = modal_chrome(
            f,
            popup_rect,
            theme,
            ModalSpec {
                title: Some(" Saved Searches "),
                ..Default::default()
            },
        );

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
        // The engine hit-tests `row - rect.y` (row 0 = first item); the list
        // renders into the block's inner area, so record that same rect.
        self.list.set_list_rect(list_inner);
        // The whole popup is wheel-scrollable (filter box and hint bar included).
        self.list.set_panel_rect(popup_rect);

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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use crate::test_support::temp_vault;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    /// Drive the modal's engine to idle, giving the background load real
    /// wall-clock time to land (the vault read runs on a worker thread under
    /// the multi-thread runtime).
    async fn poll_engine_idle(modal: &mut SavedSearchesModal) {
        // Generous ceiling: the spawned vault read runs on a worker thread, and
        // under the full parallel suite those workers are contended, so a tight
        // budget races the load. Early-breaks the instant the load lands, so the
        // common path stays fast; the high cap only matters under heavy load.
        for _ in 0..600 {
            modal.list.poll();
            if !modal.list.is_loading() {
                break;
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        modal.list.poll();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_removes_row_via_ordered_reload() {
        let vault = temp_vault("saved_searches_delete").await;
        vault
            .save_search("todo", "#todo")
            .await
            .expect("save search");
        vault
            .save_search("ideas", "#ideas")
            .await
            .expect("save search");
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let mut modal = SavedSearchesModal::new(
            vault.clone(),
            settings.key_bindings.clone(),
            settings.icons(),
            tx.clone(),
        );
        poll_engine_idle(&mut modal).await;

        // Select the first USER row (skip the pinned virtual backlinks row).
        Overlay::handle_input(
            &mut modal,
            &InputEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            &tx,
        );
        let target = modal
            .list
            .selected_row()
            .filter(|i| !i.is_virtual)
            .expect("a non-virtual row is selected")
            .name
            .clone();

        // Delete: this sets pending_delete and reloads (delete-then-list, ordered).
        Overlay::handle_input(
            &mut modal,
            &InputEvent::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            &tx,
        );
        poll_engine_idle(&mut modal).await;

        // Vault state: the deleted name is gone, one user search remains.
        let remaining = vault.list_saved_searches().await.expect("list");
        assert_eq!(remaining.len(), 1, "one saved search should remain");
        assert!(
            !remaining.iter().any(|s| s.name == target),
            "deleted name {target} should be gone from the vault"
        );

        // Visible list no longer contains the deleted row.
        let visible: Vec<String> = modal
            .list
            .visible_rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert!(
            !visible.contains(&target),
            "deleted name {target} should be gone from the visible rows, got {visible:?}"
        );
    }

    /// Pressing Enter emits SavedSearchSelected only. The editor's handler for
    /// that event closes the overlay itself (focusing the Query panel), so the
    /// modal does NOT also emit CloseOverlay (that would be redundant).
    #[tokio::test]
    async fn enter_emits_selected_not_close() {
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

        Overlay::handle_input(
            &mut modal,
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
                .any(|e| matches!(e, AppMessage::SavedSearch(SavedSearchFlow::Selected { .. }))),
            "expected SavedSearchSelected, got {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, AppMessage::CloseOverlay)),
            "select must not emit CloseOverlay; editor's SavedSearchSelected handler closes the overlay, got {events:?}"
        );
    }
}
