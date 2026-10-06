use std::sync::Arc;
use std::sync::mpsc::Receiver;

use chrono::NaiveDate;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::autocomplete::AutocompleteMode;
use crate::components::event_state::EventState;
use crate::components::events::{
    AppEvent, AppTx, AppTxExt, InputEvent, OverlayData, redraw_callback,
};
use crate::components::file_list::{
    FileListEntry, PropertyValues, SortField, SortOrder, entry_order,
};
use crate::components::overlay::{Overlay, OverlayKind, OverlayMsg};
use crate::components::panel::{ModalBg, ModalSpec, modal_chrome};
use crate::components::preview_highlight;
use crate::components::saved_search_breadcrumb::SavedSearchBreadcrumb;
use crate::components::search_list::{
    KeyReaction, RowSource, SearchList, SearchMouse, VaultSuggestions,
};
use crate::components::sortable::{
    PropertySort, SortState, SortableList, is_blank_property, order_of_query, query_with_sort,
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
    /// Last create/open error (e.g. a failed `Create: …`), shown in the hint
    /// bar until the next keystroke. Cleared on input.
    error: Option<String>,
    /// Row sorting for a Files-scope list (the Ctrl+O finder) — see
    /// [`Self::with_row_sort`]. `None`: not sortable (link results), or the
    /// Query scope, whose sort lives in the query string.
    row_sort: Option<RowSort>,
}

/// The file finder's sort: `state` is `None` until the user picks one (rows
/// keep the provider's order — recency, or fuzzy rank while typing).
#[derive(Default)]
struct RowSort {
    state: Option<SortState>,
    property: PropertySort,
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
            .yank_combos_from(&key_bindings)
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
            error: None,
            row_sort: None,
        };
        modal.refresh_preview(None);
        modal
    }

    /// Make a Files-scope list sortable from the sort dialog (the Ctrl+O
    /// finder): rows re-sort by name, title or a property's indexed value.
    /// A no-op for the Query scope, which sorts through its query.
    pub fn with_row_sort(mut self) -> Self {
        if self.scope == BrowserScope::Files {
            self.row_sort = Some(RowSort::default());
        }
        self
    }

    /// Re-sort the loaded rows by the row sort (no reload). A property sort
    /// still waiting for its values keeps the current order.
    fn reorder_rows(&mut self) {
        let Some(RowSort {
            state: Some(state),
            property,
        }) = &self.row_sort
        else {
            return;
        };
        if property.is_awaiting(&state.field) {
            return;
        }
        let values = property.values_for_field(&state.field);
        let order = entry_order(state.field.clone(), state.order, false, values);
        self.list.set_order(Some(order));
        self.refresh_preview_from_list();
    }

    /// Property values fetched for `key` landed: re-sort if they belong to
    /// the active sort. `false` when they are stale and were dropped.
    pub(crate) fn on_property_sort_values(&mut self, key: &str, values: PropertyValues) -> bool {
        let accepted = self
            .row_sort
            .as_mut()
            .is_some_and(|rs| rs.property.receive(key, values));
        if accepted {
            self.reorder_rows();
        }
        accepted
    }

    /// Apply property values that arrived since the last frame. Runs from
    /// `render`, which a parked finder (under the sort dialog) still gets.
    fn poll_row_sort(&mut self) {
        if let Some((key, values)) = self.row_sort.as_mut().and_then(|rs| rs.property.poll()) {
            self.on_property_sort_values(&key, values);
        }
    }

    #[cfg(test)]
    fn row_sort_pending(&self) -> bool {
        self.row_sort
            .as_ref()
            .is_some_and(|rs| rs.property.is_pending())
    }

    /// The lowercase text needles the preview emphasizes: the query's plain
    /// search terms (Query scope only — the fuzzy Files scope matches names,
    /// not content).
    fn preview_needles(&self) -> Vec<String> {
        if self.scope != BrowserScope::Query {
            return Vec::new();
        }
        crate::components::query_highlight::emphasis_needles(self.list.query())
    }

    /// The emphasis payload an open from this modal carries: the query's
    /// needles (spec §5.1), Query scope only.
    fn emphasis(&self) -> Option<Vec<String>> {
        let needles = self.preview_needles();
        (!needles.is_empty()).then_some(needles)
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
                match vault.load_or_create_note(&path, None).await {
                    Ok((_, created)) => tx.announce_and_open(path, created),
                    Err(e) => {
                        tx.send(AppEvent::OverlayData(OverlayData::Error(e.to_string())))
                            .ok();
                    }
                }
            });
            return;
        }
        let path = entry.path().clone();
        tx.send(AppEvent::OpenPath {
            path,
            emphasis: self.emphasis(),
        })
        .ok();
    }

    fn is_sortable(&self) -> bool {
        self.scope == BrowserScope::Query || self.row_sort.is_some()
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
// SortableList impl (search scope — see `Overlay::as_sortable`)
// ---------------------------------------------------------------------------

impl SortableList for NoteBrowserModal {
    /// The query's order directive (search browser), or the row sort
    /// (file finder; Name ascending until one is picked — reported as
    /// unsorted, see [`SortableList::is_unsorted`]).
    fn sort_state(&self) -> SortState {
        if let Some(rs) = &self.row_sort {
            return rs.state.clone().unwrap_or(SortState {
                field: SortField::Name,
                order: SortOrder::Ascending,
                group_dirs: None,
            });
        }
        let (field, order) = order_of_query(self.list.query());
        SortState {
            field,
            order,
            group_dirs: None,
        }
    }

    /// Search browser: rewrite the query's order directive and reload, so
    /// the results re-sort live behind the sort dialog. Like the Query panel,
    /// the saved-search breadcrumb stays pinned and shows `• edited` (the
    /// stored query is saved verbatim). File finder: re-sort the rows; a
    /// property sort on a new key fetches its values and re-sorts when they
    /// land (the same key reuses them).
    fn apply_sort(&mut self, state: &SortState, tx: &AppTx) {
        if let Some(rs) = &mut self.row_sort {
            if is_blank_property(&state.field) {
                return;
            }
            rs.state = Some(state.clone());
            rs.property.sync(&self.vault, &state.field, tx);
            self.reorder_rows();
            return;
        }
        if let Some(rewritten) = query_with_sort(self.list.query(), &state.field, state.order) {
            self.list.set_query(rewritten);
            self.refresh_preview_from_list();
        }
    }

    fn allows_property(&self) -> bool {
        true
    }

    /// File finder: until a sort is picked (rows in recency / fuzzy order).
    /// Search browser: on the empty query, which lists the recent notes in
    /// recency order.
    fn is_unsorted(&self) -> bool {
        match &self.row_sort {
            Some(rs) => rs.state.is_none(),
            None => self.scope == BrowserScope::Query && self.list.query().trim().is_empty(),
        }
    }
}

// ---------------------------------------------------------------------------
// Overlay impl
// ---------------------------------------------------------------------------

impl NoteBrowserModal {
    /// After the query changed (a key, or a clicked suggestion): forward the
    /// event to the breadcrumb — a `?name` expansion pins it, an emptied
    /// field clears it, a manual edit keeps it (sticky) — and refresh the
    /// preview.
    fn after_query_edit(&mut self) {
        let accepted = self.list.take_accepted_saved_search();
        let blank = self.list.query().trim().is_empty();
        self.saved_search
            .on_query_consumed(accepted, self.list.query(), blank);
        self.refresh_preview_from_list();
    }
}

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

    /// The search browser sorts through its query's order directive; the
    /// file finder re-sorts its rows ([`NoteBrowserModal::with_row_sort`]).
    /// The link-results list does not sort.
    fn as_sortable(&self) -> Option<&dyn SortableList> {
        self.is_sortable().then_some(self as &dyn SortableList)
    }

    fn as_sortable_mut(&mut self) -> Option<&mut dyn SortableList> {
        if self.is_sortable() { Some(self) } else { None }
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Mouse(mouse) => match self.list.handle_mouse(mouse) {
                SearchMouse::Activated(_) | SearchMouse::DoubleClicked { repeat: false, .. } => {
                    self.open_selected(tx);
                    EventState::Consumed
                }
                SearchMouse::Context(_) | SearchMouse::Selected(_) | SearchMouse::Scrolled => {
                    self.refresh_preview_from_list();
                    EventState::Consumed
                }
                // No content sub-region is recorded by this host, so these
                // are unreachable.
                SearchMouse::ContentScrollUp | SearchMouse::ContentScrollDown => {
                    EventState::Consumed
                }
                SearchMouse::Autocomplete { edited: true } => {
                    self.after_query_edit();
                    EventState::Consumed
                }
                // The repeat half of a double-click whose first press already
                // opened: acting again would open twice.
                SearchMouse::InputFocused
                | SearchMouse::Autocomplete { edited: false }
                | SearchMouse::DoubleClicked { repeat: true, .. } => EventState::Consumed,
                SearchMouse::None => EventState::NotConsumed,
            },
            InputEvent::Key(key) => {
                // Any keypress clears a stale create/open error.
                self.error = None;
                match self.list.handle_key(key) {
                    KeyReaction::Submit => {
                        self.open_selected(tx);
                        EventState::Consumed
                    }
                    KeyReaction::Cancel => {
                        tx.send(AppEvent::CloseOverlay).ok();
                        EventState::Consumed
                    }
                    KeyReaction::Consumed => {
                        self.after_query_edit();
                        EventState::Consumed
                    }
                    KeyReaction::Yank(target) => {
                        crate::components::yank_row(target, tx);
                        EventState::Consumed
                    }
                    KeyReaction::Intercepted(_)
                    | KeyReaction::ListVerb(_)
                    | KeyReaction::Unhandled => EventState::NotConsumed,
                }
            }
            _ => EventState::NotConsumed,
        }
    }

    fn handle_data(
        &mut self,
        data: &OverlayData,
        _vault: &Arc<NoteVault>,
        _tx: &AppTx,
    ) -> OverlayMsg {
        // A failed `Create: …` (or other open error) surfaces here while the
        // modal stays open, so the user sees why nothing happened.
        if let OverlayData::Error(text) = data {
            self.error = Some(text.clone());
            OverlayMsg::Consumed
        } else {
            OverlayMsg::NotConsumed
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.poll_preview();
        self.poll_row_sort();

        let popup_rect = crate::components::centered_rect(75, 75, area);

        // Modal chrome (spec §6): hard background, focus-green border.
        let modal_style = Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_hard.to_ratatui());
        let title = format!(" {} ", self.title);
        let inner = modal_chrome(
            f,
            popup_rect,
            theme,
            ModalSpec {
                title: Some(&title),
                bg: ModalBg::Hard,
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

        // ── Hint bar (or last error) ──────────────────────────────────────
        let hint = match &self.error {
            Some(err) => Paragraph::new(format!("⚠ {err}"))
                .style(Style::default().fg(theme.red.to_ratatui())),
            None => Paragraph::new("↑↓: navigate  |  Enter: open  |  Esc: close")
                .style(Style::default().fg(theme.fg_secondary.to_ratatui())),
        };
        f.render_widget(hint, rows[2]);

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

pub(crate) fn format_journal_date(date: NaiveDate) -> String {
    date.format("%A, %B %-d, %Y").to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The number of highlighted matches in `text`, or `None` when there are no
/// needles (the preview header shows a count only for queries with text terms).
/// Counts the same ranges [`highlight_matches`] bolds — via the shared
/// [`preview_highlight::match_ranges`] — so the header never disagrees with the
/// visible highlights (overlapping needles are deduped, folds counted).
fn count_matches(text: &str, needles: &[String]) -> Option<usize> {
    if needles.is_empty() {
        return None;
    }
    Some(preview_highlight::match_ranges(text, needles).len())
}

/// The preview text with needle matches emphasized in `yellow` (spec §6).
/// Matching is byte-safe via [`preview_highlight::match_ranges`], so non-ASCII
/// case folds (e.g. `İ`, `ẞ`) are highlighted too, not dropped.
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
        let ranges = preview_highlight::match_ranges(line, needles);
        if ranges.is_empty() {
            lines.push(Line::styled(line, base));
            continue;
        }
        // Borrowed spans into `text` ('a) — zero-copy; shared segment walk.
        let spans = preview_highlight::style_ranges(line, &ranges, |s, hit| {
            Span::styled(s, if hit { emphasis } else { base })
        });
        lines.push(Line::from(spans));
    }
    ratatui::text::Text::from(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::file_list::{SortField, SortOrder};
    use crate::components::search_list::{Emit, RowSource};
    use crate::components::sortable::{SortState, SortableList};
    use crate::settings::AppSettings;
    use crate::test_support::temp_vault;
    use async_trait::async_trait;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn count_matches_matches_highlighted_ranges() {
        // No needles → no header count.
        assert_eq!(count_matches("anything", &[]), None);
        // Overlapping needles count once (deduped, longest-first) — the header
        // must equal the number of bolded ranges, not the raw per-needle sum.
        let needles = vec!["foo".to_string(), "foobar".to_string()];
        assert_eq!(count_matches("foobar", &needles), Some(1));
        // Distinct occurrences each count.
        assert_eq!(count_matches("foo and foo", &["foo".to_string()]), Some(2));
    }

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
                is_open: false,
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
    async fn dialog_error_surfaces_then_clears_on_keystroke() {
        let (tx, _rx) = unbounded_channel();
        let path = VaultPath::note_path_from("/a.md");
        let mut modal = make_modal_with(OneNoteSource { path }, tx.clone()).await;
        let vault = temp_vault("modal_err").await;

        let consumed = modal.handle_data(&OverlayData::Error("boom".to_string()), &vault, &tx);
        assert!(matches!(consumed, OverlayMsg::Consumed));
        assert_eq!(modal.error.as_deref(), Some("boom"));

        // Any keystroke clears the error.
        modal.handle_input(
            &InputEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(modal.error, None, "keystroke should clear the error");
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
                .any(|e| matches!(e, AppEvent::OpenPath { path: p, .. } if *p == path)),
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

    // ── Sorting (Ctrl+R over the search browser) ──────────────────────────

    async fn search_modal(scope: BrowserScope, query: &str) -> (NoteBrowserModal, AppTx) {
        let vault = temp_vault("modal_sort").await;
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let modal = NoteBrowserModal::with_initial_query(
            "test",
            scope,
            OneNoteSource {
                path: VaultPath::note_path_from("/a.md"),
            },
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx.clone(),
            query,
        );
        (modal, tx)
    }

    #[tokio::test]
    async fn apply_sort_rewrites_the_query_order_directive() {
        let (mut modal, tx) = search_modal(BrowserScope::Query, "#work").await;
        let state = SortState {
            field: SortField::Property("due".into()),
            order: SortOrder::Descending,
            group_dirs: None,
        };
        modal.apply_sort(&state, &tx);
        assert_eq!(modal.query_text(), "#work -or:prop:due");
        assert_eq!(modal.sort_state(), state, "sort_state reads it back");
    }

    /// Ctrl+K over the recent notes: picking a Title sort in the dialog keeps
    /// the same recent set and only reorders it.
    #[tokio::test]
    async fn title_sort_reorders_the_recent_notes_in_place() {
        use crate::components::note_browser::search_provider::resolving_search_source;
        let vault = temp_vault("modal_sort_recents").await;
        vault.validate_and_init().await.unwrap();
        for (file, body) in [
            ("a", "# Charlie\nx"),
            ("b", "# Alpha\nx"),
            ("c", "# Bravo\nx"),
        ] {
            vault
                .create_note(&VaultPath::note_path_from(file), body)
                .await
                .unwrap();
        }
        vault
            .create_note(&VaultPath::note_path_from("other"), "# Aaa\nx")
            .await
            .unwrap();
        let recents: Vec<VaultPath> = ["c", "a", "b"]
            .into_iter()
            .map(VaultPath::note_path_from)
            .collect();
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let mut modal = NoteBrowserModal::new(
            "test",
            BrowserScope::Query,
            resolving_search_source(vault.clone(), recents, None),
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx.clone(),
        );
        let listed = |m: &NoteBrowserModal| -> Vec<String> {
            m.list
                .visible_rows()
                .iter()
                .filter_map(|r| match r {
                    FileListEntry::Note { path, .. } => Some(path.get_clean_name()),
                    _ => None,
                })
                .collect()
        };
        modal.list.poll_until_idle().await;
        assert_eq!(listed(&modal), ["c", "a", "b"]);

        modal.apply_sort(
            &SortState {
                field: SortField::Title,
                order: SortOrder::Ascending,
                group_dirs: None,
            },
            &tx,
        );
        modal.list.poll_until_idle().await;
        assert_eq!(modal.query_text(), "or:title");
        assert_eq!(listed(&modal), ["b", "c", "a"], "same set, by title");
    }

    #[tokio::test]
    async fn apply_sort_ignores_an_empty_property_key() {
        let (mut modal, tx) = search_modal(BrowserScope::Query, "x -or:title").await;
        for key in ["", "  "] {
            let state = SortState {
                field: SortField::Property(key.into()),
                order: SortOrder::Ascending,
                group_dirs: None,
            };
            modal.apply_sort(&state, &tx);
        }
        assert_eq!(modal.query_text(), "x -or:title");
    }

    #[tokio::test]
    async fn sort_state_defaults_to_name_ascending() {
        let (modal, _tx) = search_modal(BrowserScope::Query, "#work").await;
        assert_eq!(
            modal.sort_state(),
            SortState {
                field: SortField::Name,
                order: SortOrder::Ascending,
                group_dirs: None,
            }
        );
        assert!(modal.allows_property());
    }

    /// The search browser and the file finder sort; a plain Files-scope
    /// list (link results) does not.
    #[tokio::test]
    async fn the_search_browser_and_the_file_finder_are_sortable() {
        let (mut search, _) = search_modal(BrowserScope::Query, "").await;
        let (mut links, _) = search_modal(BrowserScope::Files, "").await;
        let (finder, _) = search_modal(BrowserScope::Files, "").await;
        let mut finder = finder.with_row_sort();
        assert!(search.as_sortable().is_some());
        assert!(search.as_sortable_mut().is_some());
        assert!(links.as_sortable().is_none());
        assert!(links.as_sortable_mut().is_none());
        assert!(finder.as_sortable().is_some());
        assert!(finder.as_sortable_mut().is_some());
        assert!(finder.allows_property());
    }

    /// A sort counts as an edit of a pinned saved search: the breadcrumb
    /// shows `• edited` once the order directive is rewritten.
    #[tokio::test]
    async fn apply_sort_marks_the_saved_search_breadcrumb_edited() {
        let (mut modal, tx) = search_modal(BrowserScope::Query, "#todo").await;
        modal.saved_search.set(Some("todo".into()), "#todo");
        assert_eq!(modal.saved_search_breadcrumb().as_deref(), Some("todo"));
        modal.apply_sort(
            &SortState {
                field: SortField::Title,
                order: SortOrder::Ascending,
                group_dirs: None,
            },
            &tx,
        );
        assert_eq!(
            modal.saved_search_breadcrumb().as_deref(),
            Some("todo • edited")
        );
    }

    /// The Ctrl+O finder over a real vault, with row sorting on.
    async fn finder_modal(prefix: &str) -> (NoteBrowserModal, AppTx) {
        use crate::components::note_browser::file_finder_provider::FileFinderProvider;
        let vault = temp_vault(prefix).await;
        vault.validate_and_init().await.unwrap();
        for (name, body) in [
            ("alpha", "---\nrank: 3\n---\nbody"),
            ("bravo", "---\nrank: 1\n---\nbody"),
            ("charlie", "body"),
        ] {
            vault
                .create_note(&VaultPath::note_path_from(name), body)
                .await
                .unwrap();
        }
        let settings = AppSettings::default();
        let (tx, _rx) = unbounded_channel();
        let provider = FileFinderProvider::new(vault.clone(), VaultPath::root());
        let mut modal = NoteBrowserModal::new(
            "Find Note",
            BrowserScope::Files,
            provider,
            vault,
            settings.key_bindings.clone(),
            settings.icons(),
            tx.clone(),
        )
        .with_row_sort();
        modal.list.poll_until_idle().await;
        (modal, tx)
    }

    fn finder_names(modal: &NoteBrowserModal) -> Vec<String> {
        modal
            .list
            .visible_rows()
            .iter()
            .map(|r| r.path().get_name())
            .collect()
    }

    fn sort(field: SortField, order: SortOrder) -> SortState {
        SortState {
            field,
            order,
            group_dirs: None,
        }
    }

    /// Poll until the finder's property values have landed.
    async fn poll_finder_values(modal: &mut NoteBrowserModal) {
        for _ in 0..100 {
            modal.poll_row_sort();
            if !modal.row_sort_pending() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("property values never arrived");
    }

    /// Ctrl+R on the finder re-sorts its rows: by name, then by a property
    /// once the values arrive (the note without the key last).
    #[tokio::test(flavor = "multi_thread")]
    async fn file_finder_sorts_rows_by_name_and_property() {
        let (mut modal, tx) = finder_modal("finder-sort").await;
        modal.apply_sort(&sort(SortField::Name, SortOrder::Descending), &tx);
        assert_eq!(finder_names(&modal), ["charlie.md", "bravo.md", "alpha.md"]);
        assert_eq!(
            modal.sort_state(),
            sort(SortField::Name, SortOrder::Descending)
        );

        let rank = SortField::Property("rank".into());
        modal.apply_sort(&sort(rank.clone(), SortOrder::Ascending), &tx);
        assert_eq!(
            finder_names(&modal),
            ["charlie.md", "bravo.md", "alpha.md"],
            "current order kept until the values arrive"
        );
        poll_finder_values(&mut modal).await;
        assert_eq!(finder_names(&modal), ["bravo.md", "alpha.md", "charlie.md"]);
        modal.apply_sort(&sort(rank.clone(), SortOrder::Descending), &tx);
        assert_eq!(finder_names(&modal), ["alpha.md", "bravo.md", "charlie.md"]);
        assert_eq!(modal.sort_state(), sort(rank, SortOrder::Descending));
    }

    /// Values for a key the finder no longer sorts by are dropped.
    #[tokio::test(flavor = "multi_thread")]
    async fn file_finder_ignores_stale_property_values() {
        use kimun_core::PropertySortValue::Number;
        let (mut modal, tx) = finder_modal("finder-stale").await;
        modal.apply_sort(&sort(SortField::Name, SortOrder::Ascending), &tx);
        modal.apply_sort(
            &sort(SortField::Property("rank".into()), SortOrder::Ascending),
            &tx,
        );
        let stale = Arc::new(std::collections::HashMap::from([(
            VaultPath::note_path_from("/charlie"),
            Number(0.0),
        )]));
        assert!(!modal.on_property_sort_values("other", stale));
        assert_eq!(finder_names(&modal), ["alpha.md", "bravo.md", "charlie.md"]);
    }

    /// The finder reports "unsorted" (recency / match order) until a sort
    /// is picked; then it reports the pick.
    #[tokio::test(flavor = "multi_thread")]
    async fn file_finder_is_unsorted_until_a_sort_is_picked() {
        let (mut modal, tx) = finder_modal("finder-unsorted").await;
        assert!(modal.is_unsorted());
        modal.apply_sort(&sort(SortField::Name, SortOrder::Ascending), &tx);
        assert!(!modal.is_unsorted());
    }

    /// The Ctrl+K browser shows the recent notes in recency order for an
    /// empty query: unsorted. A term or an order directive is not.
    #[tokio::test]
    async fn search_browser_is_unsorted_only_on_the_recents() {
        let (empty, _) = search_modal(BrowserScope::Query, "").await;
        assert!(empty.is_unsorted());
        let (term, _) = search_modal(BrowserScope::Query, "#work").await;
        assert!(!term.is_unsorted());
        let (sorted, _) = search_modal(BrowserScope::Query, "or:title").await;
        assert!(!sorted.is_unsorted());
    }

    /// Toggling the order of the same property key reuses the cached
    /// values; another key fetches.
    #[tokio::test(flavor = "multi_thread")]
    async fn file_finder_order_toggles_reuse_the_property_values() {
        let (mut modal, tx) = finder_modal("finder-prop-cache").await;
        let rank = SortField::Property("rank".into());
        modal.apply_sort(&sort(rank.clone(), SortOrder::Ascending), &tx);
        poll_finder_values(&mut modal).await;
        let fetches = |m: &NoteBrowserModal| m.row_sort.as_ref().unwrap().property.fetches;
        assert_eq!(fetches(&modal), 1);
        modal.apply_sort(&sort(rank.clone(), SortOrder::Descending), &tx);
        modal.apply_sort(&sort(rank, SortOrder::Ascending), &tx);
        assert_eq!(fetches(&modal), 1, "order toggles: no refetch");
        assert!(!modal.row_sort_pending());
        assert_eq!(finder_names(&modal), ["bravo.md", "alpha.md", "charlie.md"]);
        modal.apply_sort(
            &sort(SortField::Property("due".into()), SortOrder::Ascending),
            &tx,
        );
        assert_eq!(fetches(&modal), 2, "a new key fetches");
        modal.apply_sort(&sort(SortField::Title, SortOrder::Ascending), &tx);
        modal.apply_sort(
            &sort(SortField::Property("due".into()), SortOrder::Ascending),
            &tx,
        );
        assert_eq!(fetches(&modal), 3, "back to the key after Title refetches");
    }
}
