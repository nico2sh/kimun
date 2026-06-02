use std::sync::Arc;
use std::sync::mpsc::Receiver;

use chrono::NaiveDate;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::autocomplete::AutocompleteMode;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, redraw_callback};
use crate::components::file_list::FileListEntry;
use crate::components::overlay::{Overlay, OverlayKind};
use crate::components::saved_search_breadcrumb::SavedSearchBreadcrumb;
use crate::components::search_list::{
    KeyReaction, RowSource, SearchList, SearchMouse, VaultSuggestions,
};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

pub mod file_finder_provider;
pub mod link_results_provider;
pub mod search_provider;

// ---------------------------------------------------------------------------
// NoteBrowserModal
// ---------------------------------------------------------------------------

/// The Ctrl+K note browser. It hosts a [`SearchList`] engine (query input +
/// async-loaded result list + hashtag autocomplete) and adds the two things
/// unique to the browser: a live preview pane for the selected note and the
/// open-on-enter glue that emits [`AppEvent::OpenPath`].
pub struct NoteBrowserModal {
    title: String,
    list: SearchList<FileListEntry>,
    vault: Arc<NoteVault>,
    tx: AppTx,
    preview_text: String,
    // Preview async loading
    preview_task: Option<tokio::task::JoinHandle<()>>,
    preview_rx: Option<Receiver<String>>,
    /// Path the preview pane is currently showing (or loading). Compared at
    /// render time against the engine's selected row so an async server-side
    /// reload that auto-selects a different row still refreshes the preview.
    preview_path: Option<VaultPath>,
    /// Used to resolve the save-current-query shortcut for the hint bar.
    key_bindings: KeyBindings,
    /// The saved-search breadcrumb shown on the search border. Owns its
    /// sticky/clear/edited state machine; the modal only forwards query events.
    /// See [`SavedSearchBreadcrumb`] and `adr/0006`.
    saved_search: SavedSearchBreadcrumb,
}

impl NoteBrowserModal {
    pub fn new(
        title: impl Into<String>,
        provider: impl RowSource<FileListEntry>,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
    ) -> Self {
        Self::new_with_query(
            title,
            provider,
            vault,
            key_bindings,
            icons,
            tx,
            String::new(),
        )
    }

    /// Construct the modal with a pre-filled search query.
    ///
    /// Behaves exactly like [`new`](Self::new) except the search input is
    /// pre-populated with `query` (cursor placed at the end) and the initial
    /// load is triggered for that query string.
    pub fn with_initial_query<S: Into<String>>(
        title: impl Into<String>,
        provider: impl RowSource<FileListEntry>,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
        query: S,
    ) -> Self {
        Self::new_with_query(
            title,
            provider,
            vault,
            key_bindings,
            icons,
            tx,
            query.into(),
        )
    }

    fn new_with_query(
        title: impl Into<String>,
        provider: impl RowSource<FileListEntry>,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
        initial_query: String,
    ) -> Self {
        let list = SearchList::builder(provider, redraw_callback(tx.clone()))
            .initial_query(initial_query)
            .icons(icons)
            .autocomplete(
                Arc::new(VaultSuggestions {
                    vault: vault.clone(),
                }),
                AutocompleteMode::SearchQuery,
            )
            .build();
        let mut modal = Self {
            title: title.into(),
            list,
            vault,
            tx,
            preview_text: String::new(),
            preview_task: None,
            preview_rx: None,
            preview_path: None,
            key_bindings,
            saved_search: SavedSearchBreadcrumb::default(),
        };
        modal.refresh_preview(None);
        modal
    }

    // ── Async preview loading ──────────────────────────────────────────────

    fn schedule_preview(&mut self, path: VaultPath) {
        if let Some(handle) = self.preview_task.take() {
            handle.abort();
        }
        let vault = Arc::clone(&self.vault);
        let tx = self.tx.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.preview_rx = Some(result_rx);

        let handle = tokio::spawn(async move {
            let text = vault.get_note_text(&path).await.unwrap_or_default();
            result_tx.send(text).ok();
            tx.send(AppEvent::Redraw).ok();
        });
        self.preview_task = Some(handle);
    }

    fn poll_preview(&mut self) {
        let Some(rx) = &self.preview_rx else { return };
        match rx.try_recv() {
            Ok(text) => {
                self.preview_text = text;
                self.preview_rx = None;
                self.preview_task = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.preview_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Called after selection changes to kick off a preview load for the
    /// highlighted note, or clear the preview if a non-note entry is selected.
    fn refresh_preview(&mut self, selected: Option<&FileListEntry>) {
        let maybe_path = selected.and_then(|e| match e {
            FileListEntry::Note { path, .. } => Some(path.clone()),
            _ => None,
        });
        if let Some(path) = maybe_path {
            self.schedule_preview(path);
        } else {
            self.preview_text.clear();
            if let Some(h) = self.preview_task.take() {
                h.abort();
            }
        }
    }

    /// The note path the engine currently has selected, if the selected row is
    /// a note (non-note rows yield `None`).
    fn selected_note_path(&self) -> Option<VaultPath> {
        self.list.selected_row().and_then(|e| match e {
            FileListEntry::Note { path, .. } => Some(path.clone()),
            _ => None,
        })
    }

    /// Refresh the preview for whatever the engine currently has selected.
    fn refresh_preview_from_list(&mut self) {
        let path = self.selected_note_path();
        self.preview_path = path.clone();
        match path {
            Some(path) => self.schedule_preview(path),
            None => {
                self.preview_text.clear();
                if let Some(h) = self.preview_task.take() {
                    h.abort();
                }
            }
        }
    }

    /// Open the engine's selected row: create-then-open for a `CreateNote`,
    /// or open directly for an existing `Note`. Emits only `OpenPath`; the
    /// editor's `OpenPath` handler closes this overlay (restoring focus to the
    /// editor), so no separate `CloseOverlay` is sent.
    fn open_selected(&self, tx: &AppTx) {
        let Some(entry) = self.list.selected_row() else {
            return;
        };
        if let FileListEntry::CreateNote { path, .. } = entry {
            let path = path.clone();
            let vault = Arc::clone(&self.vault);
            let tx = tx.clone();
            tokio::spawn(async move {
                vault.load_or_create_note(&path, None).await.ok();
                tx.send(AppEvent::OpenPath(path)).ok();
            });
            return;
        }
        let path = entry.path().clone();
        tx.send(AppEvent::OpenPath(path)).ok();
    }

    /// The saved-search breadcrumb label for the search border, or `None` when
    /// no saved search is active. See `adr/0006`.
    #[cfg(test)]
    fn saved_search_breadcrumb(&self) -> Option<String> {
        self.saved_search.label(self.list.query())
    }

    // ── Test-only accessors ────────────────────────────────────────────────

    /// Returns the current search input text. Test-only.
    #[cfg(test)]
    pub(super) fn query_text(&self) -> &str {
        self.list.query()
    }
}

// ---------------------------------------------------------------------------
// Overlay impl
// ---------------------------------------------------------------------------

impl Overlay for NoteBrowserModal {
    fn kind(&self) -> OverlayKind {
        OverlayKind::NoteBrowser
    }

    fn query(&self) -> Option<&str> {
        Some(self.list.query())
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Mouse(mouse) => match self.list.handle_mouse(mouse) {
                SearchMouse::Activated(_) => {
                    self.open_selected(tx);
                    EventState::Consumed
                }
                SearchMouse::Selected(_) | SearchMouse::Scrolled => {
                    self.refresh_preview_from_list();
                    EventState::Consumed
                }
                SearchMouse::None => EventState::NotConsumed,
            },
            InputEvent::Key(key) => match self.list.handle_key(key) {
                KeyReaction::Submit => {
                    self.open_selected(tx);
                    EventState::Consumed
                }
                KeyReaction::Cancel => {
                    tx.send(AppEvent::CloseOverlay).ok();
                    EventState::Consumed
                }
                KeyReaction::Consumed => {
                    // Forward the query event to the breadcrumb: a `?name`
                    // expansion pins it, an emptied field clears it, a manual
                    // edit keeps it (sticky). See `adr/0006`.
                    let accepted = self.list.take_accepted_saved_search();
                    let blank = self.list.query().trim().is_empty();
                    self.saved_search
                        .on_query_consumed(accepted, self.list.query(), blank);
                    self.refresh_preview_from_list();
                    EventState::Consumed
                }
                KeyReaction::Intercepted(_) | KeyReaction::Unhandled => EventState::NotConsumed,
            },
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.poll_preview();

        let popup_rect = crate::components::centered_rect(80, 75, area);

        // Clear the area behind the modal so the editor doesn't bleed through.
        f.render_widget(Clear, popup_rect);

        let outer_block = Block::default()
            .title(format!(" {} ", self.title))
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

        // ── Search box ────────────────────────────────────────────────────
        // A saved-search breadcrumb (`‹ name ›` / `‹ name • edited ›`) titles
        // the search box when a `?name` expansion is active. See `adr/0006`.
        let search_title = self
            .saved_search
            .border_title(self.list.query(), " Search ");
        let search_block = Block::default()
            .title(search_title)
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(theme.panel_style());
        let search_inner = search_block.inner(rows[0]);
        f.render_widget(search_block, rows[0]);
        self.list.render_query(f, search_inner, theme, true);

        // ── List + Preview ────────────────────────────────────────────────
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);

        // The engine hit-tests a click as `row - rect.y` against the recorded
        // rect, where row 0 is the first item. The list renders into the block's
        // INNER area, so record that same inner rect.
        let list_block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style(false))
            .style(theme.panel_style());
        let list_inner = list_block.inner(columns[0]);
        f.render_widget(list_block, columns[0]);
        self.list.render(f, list_inner, theme, false);
        self.list.set_list_rect(list_inner);

        // Authoritative preview trigger: `list.render` just polled, which is
        // where an async server-side reload lands and may auto-select a new
        // row 0. If the selected note path differs from what the preview is
        // showing, refresh. Guarded by the path diff so there's no redraw loop.
        if self.selected_note_path() != self.preview_path {
            self.refresh_preview_from_list();
        }

        let preview_block = Block::default()
            .title(" Preview ")
            .borders(Borders::ALL)
            .border_style(theme.border_style(false))
            .style(theme.panel_style());
        let preview_inner = preview_block.inner(columns[1]);
        f.render_widget(preview_block, columns[1]);
        f.render_widget(
            Paragraph::new(self.preview_text.as_str()).style(
                Style::default()
                    .fg(theme.fg.to_ratatui())
                    .bg(theme.bg.to_ratatui()),
            ),
            preview_inner,
        );

        // ── Hint bar ──────────────────────────────────────────────────────
        f.render_widget(
            Paragraph::new("↑↓: navigate  |  Enter: open  |  Esc: close")
                .style(Style::default().fg(theme.fg_secondary.to_ratatui())),
            rows[2],
        );

        // ── Autocomplete popup ───────────────────────────────────────────
        // Clamp to the modal's bounds so it never spills past the border.
        self.list.render_autocomplete(f, popup_rect, theme);
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        let mut hints = vec![
            ("↑↓".to_string(), "navigate".to_string()),
            ("Enter".to_string(), "open".to_string()),
            ("Esc".to_string(), "close".to_string()),
        ];
        if let Some(k) = self
            .key_bindings
            .first_combo_for(&ActionShortcuts::SaveCurrentQuery)
        {
            hints.push((k, "save query".to_string()));
        }
        hints
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn format_journal_date(date: NaiveDate) -> String {
    date.format("%A, %B %-d, %Y").to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::search_list::{Emit, RowSource};
    use crate::settings::AppSettings;
    use crate::test_support::temp_vault;
    use async_trait::async_trait;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    /// A one-shot source that yields a single existing note so submit has
    /// something to open.
    struct OneNoteSource {
        path: VaultPath,
    }

    #[async_trait]
    impl RowSource<FileListEntry> for OneNoteSource {
        async fn load(&self, _query: &str, emit: Emit<FileListEntry>) {
            emit.replace(vec![FileListEntry::Note {
                path: self.path.clone(),
                title: "Note".to_string(),
                filename: self.path.to_string(),
                journal_date: None,
            }]);
        }
    }

    async fn make_modal_with(source: impl RowSource<FileListEntry>, tx: AppTx) -> NoteBrowserModal {
        let vault = temp_vault("modal").await;
        let settings = AppSettings::default();
        NoteBrowserModal::new(
            "test",
            source,
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx,
        )
    }

    #[tokio::test]
    async fn modal_constructed_with_initial_query_prefills_input() {
        let vault = temp_vault("modal_iq").await;
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let modal = NoteBrowserModal::with_initial_query(
            "test",
            OneNoteSource {
                path: VaultPath::note_path_from("/a.md"),
            },
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx,
            "#important",
        );
        assert_eq!(modal.query_text(), "#important");
    }

    /// Pressing Enter on a selected note emits OpenPath only. The editor's
    /// OpenPath handler closes the overlay, so the modal does NOT also emit
    /// CloseOverlay (that would be redundant).
    #[tokio::test]
    async fn submit_opens_selected_note() {
        let (tx, mut rx) = unbounded_channel();
        let path = VaultPath::note_path_from("/a.md");
        let mut modal = make_modal_with(OneNoteSource { path: path.clone() }, tx.clone()).await;
        // Let the one-shot load deliver its row and the engine select it.
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
                .any(|e| matches!(e, AppEvent::OpenPath(p) if *p == path)),
            "expected OpenPath, got {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, AppEvent::CloseOverlay)),
            "select must not emit CloseOverlay; editor's OpenPath handler closes the overlay, got {events:?}"
        );
    }

    /// Selecting a note row updates the tracked `preview_path`; this is the
    /// state the render-time diff compares against to detect stale previews
    /// after an async reload.
    #[tokio::test]
    async fn refresh_preview_tracks_selected_path() {
        let (tx, _rx) = unbounded_channel();
        let path = VaultPath::note_path_from("/a.md");
        let mut modal = make_modal_with(OneNoteSource { path: path.clone() }, tx.clone()).await;
        modal.list.poll_until_idle().await;
        assert_eq!(modal.preview_path, None, "no path tracked before refresh");

        modal.refresh_preview_from_list();
        assert_eq!(
            modal.preview_path,
            Some(path),
            "preview_path should track the selected note"
        );
    }

    /// Pressing Esc closes the modal.
    #[tokio::test]
    async fn esc_closes_modal() {
        let (tx, mut rx) = unbounded_channel();
        let mut modal = make_modal_with(
            OneNoteSource {
                path: VaultPath::note_path_from("/a.md"),
            },
            tx.clone(),
        )
        .await;
        Overlay::handle_input(
            &mut modal,
            &InputEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &tx,
        );
        let mut sent = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::CloseOverlay) {
                sent = true;
            }
        }
        assert!(sent, "expected CloseOverlay on Esc");
    }

    /// Accepting a `?name` expansion in the Ctrl+K browser pins the saved-search
    /// breadcrumb and runs the stored query. See `adr/0006`.
    #[tokio::test(flavor = "multi_thread")]
    async fn accepting_saved_search_pins_breadcrumb() {
        let vault = temp_vault("modal-ss").await;
        vault.validate_and_init().await.unwrap();
        vault.save_search("todo-week", "#todo").await.unwrap();
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let mut modal = NoteBrowserModal::new(
            "test",
            OneNoteSource {
                path: VaultPath::note_path_from("/a.md"),
            },
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx.clone(),
        );

        // Type a leading `?` and a prefix, draining the async popup between
        // keystrokes so the suggestion lands before we accept.
        for ch in ['?', 't', 'o'] {
            Overlay::handle_input(
                &mut modal,
                &InputEvent::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
                &tx,
            );
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                modal.list.poll();
            }
        }
        Overlay::handle_input(
            &mut modal,
            &InputEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            &tx,
        );

        assert_eq!(modal.query_text(), "#todo");
        assert_eq!(
            modal.saved_search_breadcrumb().as_deref(),
            Some("todo-week")
        );
    }
}
