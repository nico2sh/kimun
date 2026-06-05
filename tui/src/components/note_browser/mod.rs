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
/// What the modal is scoped to — drives the input prefix glyph and whether
/// the §9 query highlighter applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserScope {
    /// Full query syntax (Ctrl-K, tag/backlink leaves): `⌕` prefix +
    /// syntax highlighting.
    Query,
    /// Fuzzy file finding (Ctrl-O): plain input.
    Files,
}

pub struct NoteBrowserModal {
    scope: BrowserScope,
    /// Input prefix glyph for the scope (`⌕` query / `▤` files).
    prefix_glyph: &'static str,
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
    /// See [`SavedSearchBreadcrumb`].
    saved_search: SavedSearchBreadcrumb,
}

impl NoteBrowserModal {
    pub fn new(
        title: impl Into<String>,
        scope: BrowserScope,
        provider: impl RowSource<FileListEntry>,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
    ) -> Self {
        Self::new_with_query(
            title,
            scope,
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
    #[allow(clippy::too_many_arguments)]
    pub fn with_initial_query<S: Into<String>>(
        title: impl Into<String>,
        scope: BrowserScope,
        provider: impl RowSource<FileListEntry>,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
        query: S,
    ) -> Self {
        Self::new_with_query(
            title,
            scope,
            provider,
            vault,
            key_bindings,
            icons,
            tx,
            query.into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_query(
        title: impl Into<String>,
        scope: BrowserScope,
        provider: impl RowSource<FileListEntry>,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
        initial_query: String,
    ) -> Self {
        let prefix_glyph = match scope {
            BrowserScope::Query => icons.rail_find,
            BrowserScope::Files => icons.rail_files,
        };
        let mut builder = SearchList::builder(provider, redraw_callback(tx.clone()))
            .initial_query(initial_query)
            .icons(icons)
            .autocomplete(
                Arc::new(VaultSuggestions {
                    vault: vault.clone(),
                }),
                AutocompleteMode::SearchQuery,
            );
        if scope == BrowserScope::Query {
            builder = builder.highlight_query();
        }
        let list = builder.build();
        let mut modal = Self {
            scope,
            prefix_glyph,
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

    /// The lowercase text needles the preview emphasizes: the query's plain
    /// search terms (Query scope only — the fuzzy Files scope matches names,
    /// not content).
    fn preview_needles(&self) -> Vec<String> {
        if self.scope != BrowserScope::Query {
            return Vec::new();
        }
        let terms = kimun_core::SearchTerms::from_query_string(self.list.query());
        let mut needles: Vec<String> = terms
            .terms
            .iter()
            .map(|t| t.to_lowercase())
            // Labels match their in-body `#tag` form; link targets match the
            // wikilink text, so tag-follow and backlink queries get emphasis
            // too.
            .chain(
                terms
                    .labels
                    .iter()
                    .map(|l| format!("#{}", l.to_lowercase())),
            )
            .chain(terms.links.iter().map(|l| l.to_lowercase()))
            .chain(terms.forward_links.iter().map(|l| l.to_lowercase()))
            .filter(|t| !t.is_empty() && !t.contains('{'))
            .collect();
        needles.dedup();
        needles
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
    /// no saved search is active.
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

    fn saved_search_provenance(&self) -> Option<&str> {
        self.saved_search.name()
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
                // No content sub-region is recorded by this host, so these
                // are unreachable.
                SearchMouse::ContentScrollUp | SearchMouse::ContentScrollDown => {
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
                    // edit keeps it (sticky).
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

        let popup_rect = crate::components::centered_rect(75, 75, area);

        // Clear the area behind the modal so the editor doesn't bleed through.
        f.render_widget(Clear, popup_rect);

        // Modal chrome (spec §6): hard background, focus-green border.
        let modal_style = Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_hard.to_ratatui());
        let outer_block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(modal_style);
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
        // the search box when a `?name` expansion is active.
        let search_title = self
            .saved_search
            .border_title(self.list.query(), " Search ");
        let result_count = self.list.match_count();
        let search_block = Block::default()
            .title(search_title)
            .title(
                ratatui::text::Line::from(ratatui::text::Span::styled(
                    format!(" {result_count} results "),
                    Style::default().fg(theme.gray.to_ratatui()),
                ))
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(modal_style);
        let search_inner = search_block.inner(rows[0]);
        f.render_widget(search_block, rows[0]);
        // Scope prefix glyph to the input's left, the input shifted past it.
        let prefix = format!("{} ", self.prefix_glyph);
        let prefix_w = unicode_width::UnicodeWidthStr::width(prefix.as_str()) as u16;
        f.render_widget(
            Paragraph::new(prefix).style(
                Style::default()
                    .fg(theme.yellow.to_ratatui())
                    .bg(theme.bg_hard.to_ratatui()),
            ),
            Rect {
                width: prefix_w.min(search_inner.width),
                ..search_inner
            },
        );
        let input_rect = Rect {
            x: search_inner.x.saturating_add(prefix_w),
            width: search_inner.width.saturating_sub(prefix_w),
            ..search_inner
        };
        self.list.render_query(f, input_rect, theme, true);

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
            .style(modal_style);
        let list_inner = list_block.inner(columns[0]);
        f.render_widget(list_block, columns[0]);
        self.list.render(f, list_inner, theme, false);
        self.list.set_list_rect(list_inner);
        // The whole popup is wheel-scrollable (search box and preview included).
        self.list.set_panel_rect(popup_rect);

        // Authoritative preview trigger: `list.render` just polled, which is
        // where an async server-side reload lands and may auto-select a new
        // row 0. If the selected note path differs from what the preview is
        // showing, refresh. Guarded by the path diff so there's no redraw loop.
        if self.selected_note_path() != self.preview_path {
            self.refresh_preview_from_list();
        }

        // Preview header: filename, plus the match count when the query
        // carries text terms (spec §6: `filename · N matches`).
        let needles = self.preview_needles();
        let match_count = count_matches(&self.preview_text, &needles);
        let preview_title = match (&self.preview_path, match_count) {
            (Some(path), Some(n)) => {
                format!(" {} · {} matches ", path.get_name(), n)
            }
            (Some(path), None) => format!(" {} ", path.get_name()),
            (None, _) => " Preview ".to_string(),
        };
        let preview_block = Block::default()
            .title(preview_title)
            .borders(Borders::ALL)
            .border_style(theme.border_style(false))
            .style(modal_style);
        let preview_inner = preview_block.inner(columns[1]);
        f.render_widget(preview_block, columns[1]);
        f.render_widget(
            Paragraph::new(highlight_matches(
                &self.preview_text,
                &needles,
                theme,
                modal_style,
            )),
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

/// Total needle occurrences in `text` (case-insensitive), or `None` when
/// there are no needles — the preview header shows a count only for queries
/// with text terms.
fn count_matches(text: &str, needles: &[String]) -> Option<usize> {
    if needles.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    Some(
        needles
            .iter()
            .map(|n| lower.match_indices(n.as_str()).count())
            .sum(),
    )
}

/// The preview text with needle matches emphasized in `yellow` (spec §6).
/// Lines whose lowercase form changes byte length (rare non-ASCII case
/// folds) are rendered unhighlighted rather than risking misaligned spans.
fn highlight_matches<'a>(
    text: &'a str,
    needles: &[String],
    theme: &Theme,
    base: Style,
) -> ratatui::text::Text<'a> {
    use ratatui::text::{Line, Span};
    if needles.is_empty() {
        return ratatui::text::Text::styled(text, base);
    }
    let emphasis = base.patch(
        Style::default()
            .fg(theme.color_search_match.to_ratatui())
            .add_modifier(ratatui::style::Modifier::BOLD),
    );
    let mut lines = Vec::new();
    for line in text.lines() {
        let lower = line.to_lowercase();
        if lower.len() != line.len() {
            lines.push(Line::styled(line, base));
            continue;
        }
        // Collect non-overlapping match ranges across all needles.
        let mut ranges: Vec<(usize, usize)> = needles
            .iter()
            .flat_map(|n| {
                lower
                    .match_indices(n.as_str())
                    .map(|(i, m)| (i, i + m.len()))
            })
            .collect();
        // Longest match first at each start, so an overlapping shorter
        // needle never truncates a longer one.
        ranges.sort_unstable_by_key(|(s, e)| (*s, std::cmp::Reverse(*e)));
        ranges.dedup();
        let mut spans = Vec::new();
        let mut pos = 0;
        for (start, end) in ranges {
            if start < pos {
                continue; // overlapping with a previous needle — skip
            }
            if start > pos {
                spans.push(Span::styled(&line[pos..start], base));
            }
            spans.push(Span::styled(&line[start..end], emphasis));
            pos = end;
        }
        if pos < line.len() {
            spans.push(Span::styled(&line[pos..], base));
        }
        lines.push(Line::from(spans));
    }
    ratatui::text::Text::from(lines)
}

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
            BrowserScope::Query,
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
            BrowserScope::Query,
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
    /// breadcrumb and runs the stored query.
    #[tokio::test(flavor = "multi_thread")]
    async fn accepting_saved_search_pins_breadcrumb() {
        let vault = temp_vault("modal-ss").await;
        vault.validate_and_init().await.unwrap();
        vault.save_search("todo-week", "#todo").await.unwrap();
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let mut modal = NoteBrowserModal::new(
            "test",
            BrowserScope::Query,
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
        // The overlay exposes the provenance so the save-search dialog can
        // pre-fill its name field.
        assert_eq!(Overlay::saved_search_provenance(&modal), Some("todo-week"));
    }
}
