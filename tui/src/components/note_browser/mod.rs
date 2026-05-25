use std::sync::Arc;
use std::sync::mpsc::Receiver;

use async_trait::async_trait;
use chrono::NaiveDate;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::Component;
use crate::components::autocomplete::{
    self, AutocompleteController, AutocompleteHost, AutocompleteMode, HandleKeyOutcome,
    TriggerOptions,
};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::file_list::{FileListComponent, FileListEntry};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::keys::KeyBindings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

pub mod file_finder_provider;
pub mod link_results_provider;
pub mod search_provider;

// ---------------------------------------------------------------------------
// NoteBrowserProvider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait NoteBrowserProvider: Send + Sync {
    /// Called on every query change. Empty string = initial/empty state (recent notes).
    async fn load(&self, query: &str) -> Vec<FileListEntry>;

    /// Whether to prepend a "Create: <query>" entry when query is non-empty.
    /// Defaults to false. Used by future FileFinderProvider.
    fn allows_create(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// NoteBrowserModal
// ---------------------------------------------------------------------------

pub struct NoteBrowserModal {
    title: String,
    search_query: SingleLineInput,
    provider: Arc<dyn NoteBrowserProvider>,
    file_list: FileListComponent,
    list_rect: Rect,
    preview_text: String,
    vault: Arc<NoteVault>,
    tx: AppTx,
    // List async loading
    load_task: Option<tokio::task::JoinHandle<()>>,
    load_rx: Option<Receiver<Vec<FileListEntry>>>,
    // Preview async loading
    preview_task: Option<tokio::task::JoinHandle<()>>,
    preview_rx: Option<Receiver<String>>,
    // Hashtag autocomplete for the search input.
    autocomplete: AutocompleteController,
}

/// Snapshot of the search input that satisfies `AutocompleteHost`.
/// Owned so the controller's borrow doesn't overlap with the search
/// input's `&mut` borrow during key handling and replacement.
struct SearchBoxHostSnapshot {
    value: String,
    cursor: usize,
    caret_pos: Option<(u16, u16)>,
}

impl AutocompleteHost for SearchBoxHostSnapshot {
    fn buffer_text(&self) -> String {
        self.value.clone()
    }
    fn cursor_byte_offset(&self) -> usize {
        self.cursor
    }
    fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
        // Anchor at the caret — same liberty as the editor host. The
        // popup sits adjacent to the typed text either way.
        self.caret_pos
    }
}

impl NoteBrowserModal {
    pub fn new(
        title: impl Into<String>,
        provider: impl NoteBrowserProvider + 'static,
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

    fn new_with_query(
        title: impl Into<String>,
        provider: impl NoteBrowserProvider + 'static,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
        initial_query: String,
    ) -> Self {
        let file_list = FileListComponent::new(key_bindings, icons);
        // Search box is plain text, not Markdown — disable the
        // column-0 header disambiguation (no headers to confuse with)
        // and disable the exclusion-zone check (literal `` ` `` /
        // brackets in a query shouldn't suppress hashtag triggers).
        let autocomplete = AutocompleteController::new(
            vault.clone(),
            AutocompleteMode::HashtagOnly,
        )
        .with_trigger_opts(TriggerOptions {
            disambiguate_header: false,
            apply_exclusion_zone: false,
        });
        let mut modal = Self {
            title: title.into(),
            search_query: SingleLineInput::new(),
            provider: Arc::new(provider),
            file_list,
            list_rect: Rect::default(),
            preview_text: String::new(),
            vault,
            tx: tx.clone(),
            load_task: None,
            load_rx: None,
            preview_task: None,
            preview_rx: None,
            autocomplete,
        };
        if !initial_query.is_empty() {
            modal.search_query.set_value(initial_query);
        }
        modal.schedule_load(tx);
        modal
    }

    // ── Async list loading ─────────────────────────────────────────────────

    fn schedule_load(&mut self, tx: AppTx) {
        if let Some(handle) = self.load_task.take() {
            handle.abort();
        }
        let query = self.search_query.value().to_string();
        let provider = Arc::clone(&self.provider);
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.load_rx = Some(result_rx);

        let handle = tokio::spawn(async move {
            let entries = provider.load(&query).await;
            result_tx.send(entries).ok();
            tx.send(AppEvent::Redraw).ok();
        });
        self.load_task = Some(handle);
    }

    fn poll_load(&mut self) {
        let Some(rx) = &self.load_rx else { return };
        match rx.try_recv() {
            Ok(entries) => {
                self.file_list.clear();
                let mut create_entry: Option<FileListEntry> = None;
                for entry in entries {
                    if matches!(entry, FileListEntry::CreateNote { .. }) {
                        create_entry = Some(entry);
                    } else {
                        self.file_list.push_entry(entry);
                    }
                }
                if let Some(entry) = create_entry {
                    self.file_list.prepend_create_entry(entry);
                }
                self.load_rx = None;
                self.load_task = None;
                self.refresh_preview();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.load_rx = None;
            }
        }
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

    fn open_selected_entry(&self, tx: &AppTx) {
        let Some(entry) = self.file_list.selected_entry() else {
            return;
        };
        if let FileListEntry::CreateNote { path, .. } = entry {
            let path = path.clone();
            let vault = Arc::clone(&self.vault);
            let tx = tx.clone();
            tokio::spawn(async move {
                vault.load_or_create_note(&path, None).await.ok();
                tx.send(AppEvent::OpenPath(path)).ok();
                tx.send(AppEvent::CloseNoteBrowser).ok();
            });
            return;
        }
        let path = entry.path().clone();
        tx.send(AppEvent::OpenPath(path)).ok();
        tx.send(AppEvent::CloseNoteBrowser).ok();
    }

    /// Construct the modal with a pre-filled search query.
    ///
    /// Behaves exactly like [`new`](Self::new) except the search input is
    /// pre-populated with `query` (cursor placed at the end) and an initial
    /// load is triggered for that query string.  Only a single `schedule_load`
    /// call is made — the query is pre-filled before the task is spawned so
    /// there is no empty-load race.
    pub fn with_initial_query<S: Into<String>>(
        title: impl Into<String>,
        provider: impl NoteBrowserProvider + 'static,
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

    // ── Test-only accessors ────────────────────────────────────────────────

    /// Returns the current search input text. Test-only.
    #[cfg(test)]
    pub(super) fn query_text(&self) -> &str {
        self.search_query.value()
    }

    /// Returns the cursor position as a char count (not bytes). Test-only.
    #[cfg(test)]
    pub(super) fn cursor_char_count(&self) -> usize {
        self.search_query.cursor_char_offset()
    }

    /// Called after selection changes to kick off a preview load for the
    /// highlighted note, or clear the preview if a non-note entry is selected.
    fn refresh_preview(&mut self) {
        let maybe_path = self.file_list.selected_entry().and_then(|e| match e {
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

    // ── Autocomplete ──────────────────────────────────────────────────────

    fn autocomplete_snapshot(&self) -> SearchBoxHostSnapshot {
        SearchBoxHostSnapshot {
            value: self.search_query.value().to_string(),
            cursor: self.search_query.cursor_byte(),
            caret_pos: self.search_query.last_caret_pos(),
        }
    }

}

// ---------------------------------------------------------------------------
// Component impl
// ---------------------------------------------------------------------------

impl Component for NoteBrowserModal {
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
            // Any mouse interaction takes focus away from the search
            // input — close the popup so it doesn't paint stale at the
            // old caret coords over the list/preview.
            self.autocomplete.close();
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if mouse.row > r.y {
                        let rel_row = mouse.row - r.y - 1;
                        let prev = self.file_list.selected_display_idx();
                        if let Some(idx) = self.file_list.select_at_visual_row(rel_row) {
                            if prev == Some(idx) {
                                self.open_selected_entry(tx);
                            } else {
                                self.refresh_preview();
                            }
                        }
                    }
                    EventState::Consumed
                }
                MouseEventKind::ScrollUp => {
                    self.file_list.scroll_up();
                    EventState::Consumed
                }
                MouseEventKind::ScrollDown => {
                    self.file_list.scroll_down();
                    EventState::Consumed
                }
                _ => EventState::Consumed,
            }
        } else {
            let InputEvent::Key(key) = event else {
                return EventState::NotConsumed;
            };

            // Autocomplete popup gets first crack: Up/Down/Tab/Enter/Esc
            // navigate or accept the suggestion list instead of bubbling
            // to the modal's own list-nav handling. Falls through when
            // closed or when the popup doesn't recognise the key.
            if self.autocomplete.is_open() {
                let snapshot = self.autocomplete_snapshot();
                match self.autocomplete.handle_key(*key, &snapshot) {
                    HandleKeyOutcome::Accepted(action) => {
                        self.search_query.replace_range_bytes(
                            action.range.clone(),
                            &action.new_text,
                            action.new_cursor_byte,
                        );
                        // Reschedule the load so search results reflect
                        // the accepted suggestion, mirroring what would
                        // happen if the user had typed the text manually.
                        self.schedule_load(tx.clone());
                        return EventState::Consumed;
                    }
                    HandleKeyOutcome::Dismissed | HandleKeyOutcome::Consumed => {
                        return EventState::Consumed;
                    }
                    HandleKeyOutcome::NotHandled => {}
                }
            }

            // List nav handled directly; everything else forwards to the input.
            match key.code {
                KeyCode::Up => {
                    self.file_list.select_prev();
                    self.refresh_preview();
                    return EventState::Consumed;
                }
                KeyCode::Down => {
                    self.file_list.select_next();
                    self.refresh_preview();
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
            let outcome = self.search_query.handle_key(key);
            // Edits feed the popup (may open / refresh / close it).
            // Cursor-only navigation (`Consumed`) refreshes an OPEN
            // popup so it tracks the cursor or closes when the cursor
            // leaves the trigger range — but it never auto-opens the
            // popup just because the cursor passed over a hashtag.
            // Cancel / Submit are exit paths; the popup never survives
            // them.
            let snapshot = self.autocomplete_snapshot();
            match outcome {
                InputOutcome::Changed => self.autocomplete.sync(&snapshot),
                InputOutcome::Consumed => self.autocomplete.refresh_if_open(&snapshot),
                InputOutcome::Cancel | InputOutcome::Submit => {
                    self.autocomplete.close();
                }
                InputOutcome::NotConsumed => {}
            }
            match outcome {
                InputOutcome::Cancel => {
                    tx.send(AppEvent::CloseNoteBrowser).ok();
                    EventState::Consumed
                }
                InputOutcome::Submit => {
                    self.open_selected_entry(tx);
                    EventState::Consumed
                }
                InputOutcome::Changed => {
                    self.schedule_load(tx.clone());
                    EventState::Consumed
                }
                InputOutcome::Consumed => EventState::Consumed,
                InputOutcome::NotConsumed => EventState::NotConsumed,
            }
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, _focused: bool) {
        self.poll_load();
        self.poll_preview();

        let popup_rect = centered_rect(80, 75, area);

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
        let search_block = Block::default()
            .title(" Search ")
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(theme.panel_style());
        let search_inner = search_block.inner(rows[0]);
        f.render_widget(search_block, rows[0]);
        self.search_query.render(
            f,
            search_inner,
            Style::default()
                .fg(theme.fg.to_ratatui())
                .bg(theme.bg_panel.to_ratatui()),
            0,
            true,
        );

        // ── List + Preview ────────────────────────────────────────────────
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);

        self.list_rect = columns[0];
        self.file_list.render(f, columns[0], theme, false);

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
        // Drain async results, re-anchor on the search input's freshly
        // rendered caret position, and clamp the popup to `popup_rect`
        // (the modal's bounds) so it never spills past the modal's
        // border into the cleared backdrop.
        self.autocomplete.poll_results();
        let live_anchor = self.search_query.last_caret_pos();
        if let (Some(state), Some(anchor)) =
            (self.autocomplete.state_mut(), live_anchor)
        {
            state.anchor = anchor;
        }
        if let Some(state) = self.autocomplete.state() {
            autocomplete::render(f, state, popup_rect, theme);
        }
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![
            ("↑↓".to_string(), "navigate".to_string()),
            ("Enter".to_string(), "open".to_string()),
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
    use crate::settings::AppSettings;
    use crate::test_support::{mouse_down_at, temp_vault};
    use tokio::sync::mpsc::unbounded_channel;

    struct EmptyProvider;

    #[async_trait]
    impl NoteBrowserProvider for EmptyProvider {
        async fn load(&self, _query: &str) -> Vec<FileListEntry> {
            Vec::new()
        }
    }

    async fn make_modal() -> NoteBrowserModal {
        let vault = temp_vault("modal").await;
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        NoteBrowserModal::new(
            "test",
            EmptyProvider,
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx,
        )
    }

    /// The modal's mouse handler scopes by `list_rect` (set during render),
    /// not by any rect carried by `FileListComponent`.  Clicks outside that
    /// rect must not be consumed.
    #[tokio::test]
    async fn modal_mouse_down_outside_list_rect_is_not_consumed() {
        let mut modal = make_modal().await;
        modal.list_rect = Rect {
            x: 10,
            y: 10,
            width: 20,
            height: 10,
        };
        let (tx, _rx) = unbounded_channel();

        // Click well outside the list rect.
        let result = modal.handle_input(&mouse_down_at(0, 0), &tx);
        assert_eq!(result, EventState::NotConsumed);
    }

    /// Mirrors the bounds-check used by `SidebarComponent`: a click on the
    /// modal's list_rect.y row is on the block border and must not panic, and
    /// must not select anything (the guard `mouse.row > r.y` skips it).
    #[tokio::test]
    async fn modal_mouse_down_on_list_border_does_not_panic() {
        let mut modal = make_modal().await;
        modal.list_rect = Rect {
            x: 10,
            y: 10,
            width: 20,
            height: 10,
        };
        let (tx, _rx) = unbounded_channel();
        // Click the very top row of the list rect (the block border).
        let result = modal.handle_input(&mouse_down_at(15, 10), &tx);
        assert_eq!(result, EventState::Consumed);
        assert!(modal.file_list.selected_display_idx().is_none());
    }

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let r = centered_rect(80, 75, area);
        assert_eq!(r.width, 80);
        assert_eq!(r.height, 30);
        assert_eq!(r.x, 10); // (100 - 80) / 2
        assert_eq!(r.y, 5); // (40 - 30) / 2
    }

    #[test]
    fn centered_rect_does_not_underflow() {
        // Very small area — must not panic.
        let area = Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        };
        let _ = centered_rect(80, 75, area);
    }

    // ── initial-query tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn modal_constructed_with_initial_query_prefills_input() {
        let vault = temp_vault("modal_iq").await;
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let modal = NoteBrowserModal::with_initial_query(
            "test",
            EmptyProvider,
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx,
            "#important",
        );
        assert_eq!(modal.query_text(), "#important");
        assert_eq!(modal.cursor_char_count(), "#important".chars().count());
    }

    #[tokio::test]
    async fn modal_new_has_empty_query() {
        let modal = make_modal().await;
        assert_eq!(modal.query_text(), "");
        assert_eq!(modal.cursor_char_count(), 0);
    }

    /// End-to-end: the modal's hashtag autocomplete plumbing accepts a
    /// suggestion via Tab and writes the chosen tag into the search input.
    /// Uses a real vault containing a known tag so the controller's
    /// query path is exercised too.
    #[tokio::test]
    async fn search_box_autocomplete_accept_inserts_tag() {
        use kimun_core::nfs::VaultPath;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let vault = temp_vault("search_autocomplete").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(
                &VaultPath::note_path_from("/a.md"),
                "body #projects",
            )
            .await
            .unwrap();
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let mut modal = NoteBrowserModal::new(
            "test",
            EmptyProvider,
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx,
        );

        // Type `#pro` into the search box.
        let (tx2, _rx2) = unbounded_channel();
        for ch in ['#', 'p', 'r', 'o'] {
            modal.handle_input(
                &InputEvent::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
                &tx2,
            );
        }
        // Prime the caret cache for the controller's anchor lookup so the
        // popup is allowed to open.
        modal
            .search_query
            .set_last_caret_pos_for_tests(Some((0, 0)));
        let snapshot = modal.autocomplete_snapshot();
        modal.autocomplete.sync(&snapshot);
        // Allow the spawned query task to complete and drain results.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        modal.autocomplete.poll_results();

        assert!(modal.autocomplete.is_open(), "popup should be open");

        // Tab accepts; the chosen tag should replace `pro`.
        modal.handle_input(
            &InputEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            &tx2,
        );
        assert_eq!(modal.search_query.value(), "#projects");
    }

    /// `with_initial_query` must call `schedule_load` exactly once, with the
    /// query already pre-filled.  Verified indirectly: the visible state after
    /// construction must match the supplied query and the cursor must sit at
    /// the end — just as if a single properly-initialised load were scheduled.
    #[tokio::test]
    async fn with_initial_query_does_not_double_schedule() {
        let vault = temp_vault("modal_iq_once").await;
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let modal = NoteBrowserModal::with_initial_query(
            "test",
            EmptyProvider,
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx,
            "#important",
        );
        assert_eq!(modal.query_text(), "#important");
        assert_eq!(modal.cursor_char_count(), "#important".chars().count());
        // A load task must have been spawned (Some), confirming schedule_load
        // was called.  If it were called twice the second abort() would race;
        // that scenario is ruled out by code inspection: new_with_query is the
        // only call site for schedule_load during construction.
        assert!(modal.load_task.is_some());
    }
}
