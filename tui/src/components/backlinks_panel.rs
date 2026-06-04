use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, ListItem, Paragraph};

use kimun_core::{OrderBy, OrderField, with_order_directive};

use crate::components::autocomplete::AutocompleteMode;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::file_list::{SortField, SortOrder};
use crate::components::query_vars::{query_has_variables, resolve_query};
use crate::components::saved_search_breadcrumb::SavedSearchBreadcrumb;
use crate::components::search_list::{
    Emit, KeyReaction, RowSource, SearchList, SearchRow, VaultSuggestions,
};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_combo::KeyCombo;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// The default query the panel runs: backlinks to the current note.
/// Backlinks are `<` / `lk:` (`>` is now forward links).
const DEFAULT_QUERY: &str = "<{note}";

// ---------------------------------------------------------------------------
// BacklinkEntry
// ---------------------------------------------------------------------------

/// A single backlink entry with preloaded context.
#[derive(Debug, Clone)]
pub struct BacklinkEntry {
    pub path: VaultPath,
    pub title: String,
    pub filename: String,
    /// The paragraph in this note that contains the link to the current note.
    pub context: String,
    /// Full note text, loaded when backlinks are fetched.
    pub full_text: Option<String>,
}

impl SearchRow for BacklinkEntry {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, selected: bool) -> ListItem<'static> {
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();
        let title_style = if selected {
            Style::default()
                .fg(theme.fg_selected.to_ratatui())
                .bg(theme.bg_selected.to_ratatui())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(fg).bg(bg)
        };
        let title_display = if self.title.is_empty() {
            &self.filename
        } else {
            &self.title
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!("  {} ", title_display), title_style),
            Span::styled(
                format!(" {}", self.filename),
                Style::default().fg(fg_muted).bg(if selected {
                    theme.bg_selected.to_ratatui()
                } else {
                    bg
                }),
            ),
        ]))
    }

    fn match_text(&self) -> Option<&str> {
        Some(&self.filename)
    }

    fn visual_height(&self) -> u16 {
        1
    }
}

// ---------------------------------------------------------------------------
// BacklinkSource
// ---------------------------------------------------------------------------

/// Row source for the Query panel. The engine holds the query TEMPLATE verbatim
/// (e.g. `<{note}`, so the input shows the template); this source resolves
/// `{note}` against the shared current note at load time, preserving the exact
/// "input shows the template, results are backlinks of the current note" UX.
/// Result ordering comes from the query string's order directive, applied by
/// the vault DB — the source no longer sorts in memory.
struct BacklinkSource {
    vault: Arc<NoteVault>,
    current_note: Arc<Mutex<VaultPath>>,
}

#[async_trait]
impl RowSource<BacklinkEntry> for BacklinkSource {
    async fn load(&self, query: &str, emit: Emit<BacklinkEntry>) {
        // Clone the note out of the lock, then drop the guard before awaiting.
        let note = self.current_note.lock().unwrap().clone();
        // Skip the search when the query contains a variable but no note is
        // open yet (startup state). Running `load_query(vault, "<")` against an
        // empty note is a wasted DB round-trip that returns nothing; once
        // `set_note` provides a real note the normal reload fires.
        if query_has_variables(query) && note.is_root_or_empty() {
            emit.replace(Vec::new());
            return;
        }
        let q = resolve_query(query, Some(&note));
        let mut entries = load_query(&self.vault, &q).await;
        // The DB orders results only when the query carries an `or:` directive
        // (core applies the sort iff `order_by` is non-empty). Keep that
        // directive as the source of truth, but fall back to a stable
        // Name-ascending order when the query has none — otherwise default
        // backlinks come back in arbitrary DB scan order, and the sort dialog's
        // reported default (Name/Ascending) would not match the displayed list.
        if kimun_core::SearchTerms::from_query_string(query)
            .order_by
            .is_empty()
        {
            entries.sort_by_key(|e| e.filename.to_lowercase());
        }
        emit.replace(entries);
    }
}

// ---------------------------------------------------------------------------
// ExpandState (private)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ExpandState {
    Collapsed,
    Context,
    Full,
}

// ---------------------------------------------------------------------------
// QueryPanel
// ---------------------------------------------------------------------------

pub struct QueryPanel {
    /// The SearchList engine: owns the query input, the result list, and the
    /// hashtag/link autocomplete.
    list: SearchList<BacklinkEntry>,
    /// Shared handle to the current note. `BacklinkSource::load` reads this to
    /// resolve `{note}` in the query template.
    current_note: Arc<Mutex<VaultPath>>,
    /// The saved-search breadcrumb shown on the query searchbox border. Owns
    /// its own sticky/clear/edited state machine; this panel only forwards
    /// query events to it. See [`SavedSearchBreadcrumb`].
    saved_search: SavedSearchBreadcrumb,
    /// Expand state of the currently-selected row. `Context` sticks across
    /// navigation (re-anchored on the new row); `Full` and query changes reset
    /// to `Collapsed`.
    expand: ExpandState,
    /// The path the `expand` state belongs to, used to detect selection changes
    /// (the engine owns the list, so we re-anchor expand on the selected row).
    expand_path: Option<VaultPath>,
    /// Scroll offset for full-expanded content view.
    content_scroll: usize,
    /// Maximum scroll offset (computed during render).
    content_scroll_max: usize,
    key_bindings: KeyBindings,
    /// Shared sender filled the first time a `tx` arrives. The engine's redraw
    /// callback reads this slot, so async loads/autocomplete wake the render
    /// loop once the app event channel is wired (the panel is built before the
    /// channel exists in some construction orders).
    redraw_tx: Arc<Mutex<Option<AppTx>>>,
    /// Combos that the engine intercepts: follow-link.
    follow_link_combos: Vec<KeyCombo>,
    /// Memoised sort field/order parsed from the query's order directive, plus
    /// the query string it was parsed from. `render` reparses only when the
    /// query changes, so the per-frame title indicator avoids a full query
    /// parse every frame.
    order_cache: (SortField, SortOrder),
    order_cache_query: String,
}

impl QueryPanel {
    pub fn new(vault: Arc<NoteVault>, key_bindings: KeyBindings) -> Self {
        let icons = Icons::new(false);
        let current_note = Arc::new(Mutex::new(VaultPath::empty()));
        // The redraw callback reads a shared slot that `set_note`/`handle_key`
        // fill once a `tx` is available (the panel is constructed before the
        // app event channel in some orders). Until then it is a no-op.
        let redraw_tx: Arc<Mutex<Option<AppTx>>> = Arc::new(Mutex::new(None));
        let redraw: Arc<dyn Fn() + Send + Sync> = {
            let slot = redraw_tx.clone();
            Arc::new(move || {
                if let Some(tx) = slot.lock().unwrap().as_ref() {
                    let _ = tx.send(AppEvent::Redraw);
                }
            })
        };
        let source = BacklinkSource {
            vault: vault.clone(),
            current_note: current_note.clone(),
        };
        let combos = |action: &ActionShortcuts| -> Vec<KeyCombo> {
            key_bindings
                .to_hashmap()
                .get(action)
                .cloned()
                .unwrap_or_default()
        };
        let follow_link_combos = combos(&ActionShortcuts::FollowLink);

        let mut intercept = Vec::new();
        intercept.extend(follow_link_combos.iter().cloned());

        let list = SearchList::builder(source, redraw)
            .initial_query(DEFAULT_QUERY)
            .icons(icons.clone())
            .autocomplete(
                Arc::new(VaultSuggestions {
                    vault: vault.clone(),
                }),
                AutocompleteMode::SearchQuery,
            )
            .intercept(intercept)
            .build();

        Self {
            list,
            current_note,
            saved_search: SavedSearchBreadcrumb::default(),
            expand: ExpandState::Collapsed,
            expand_path: None,
            content_scroll: 0,
            content_scroll_max: 0,
            key_bindings,
            redraw_tx,
            follow_link_combos,
            // DEFAULT_QUERY carries no order directive → (Name, Ascending).
            order_cache: (SortField::Name, SortOrder::Ascending),
            order_cache_query: String::new(),
        }
    }

    // ── Query accessors ─────────────────────────────────────────────────

    pub fn active_query(&self) -> &str {
        self.list.query()
    }

    pub fn set_active_query(&mut self, q: String) {
        self.list.set_query(q);
        self.reset_expand();
    }

    /// The breadcrumb label for the query searchbox border, or `None` when no
    /// saved search is active.
    pub fn saved_search_breadcrumb(&self) -> Option<String> {
        self.saved_search.label(self.list.query())
    }

    /// `true` when the live query carries no saved-search provenance worth
    /// showing — an empty field, or the default backlinks query (which the
    /// panel title already renders as "Backlinks", so a breadcrumb there would
    /// contradict it). Drives the breadcrumb's clear condition.
    fn query_is_blank(&self) -> bool {
        let q = self.list.query();
        q.trim().is_empty() || kimun_core::strip_order_directive(q) == DEFAULT_QUERY
    }

    /// Apply a query template (e.g. from a saved search) and run it. The engine
    /// holds the template verbatim; `{note}` is resolved at load. `name` pins
    /// the breadcrumb (`None` for the default backlinks query).
    pub fn apply_query(&mut self, query: String, name: Option<String>, tx: AppTx) {
        self.ensure_redraw_tx(&tx);
        self.set_active_query(query.clone());
        self.saved_search.set(name, &query);
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    fn current_note(&self) -> VaultPath {
        self.current_note.lock().unwrap().clone()
    }

    /// Fill the shared redraw slot so the engine's async loads / autocomplete
    /// wake the render loop. Idempotent.
    fn ensure_redraw_tx(&self, tx: &AppTx) {
        let mut slot = self.redraw_tx.lock().unwrap();
        if slot.is_none() {
            *slot = Some(tx.clone());
        }
    }

    /// Resolve the active query template against the current note (the form the
    /// source actually searches; used to derive highlight needles).
    fn resolved_query(&self) -> String {
        resolve_query(self.list.query(), Some(&self.current_note()))
    }

    /// Returns true if the selected entry is in full-expand mode (content takes
    /// the whole panel, up/down scrolls content).
    fn is_full_expanded(&self) -> bool {
        self.list.selected_row().is_some() && self.expand == ExpandState::Full
    }

    pub fn is_empty(&self) -> bool {
        self.list.rows().is_empty()
    }

    pub fn selected_path(&self) -> Option<&VaultPath> {
        self.list.selected_row().map(|e| &e.path)
    }

    fn reset_expand(&mut self) {
        self.expand = ExpandState::Collapsed;
        self.expand_path = None;
        self.content_scroll = 0;
        self.content_scroll_max = 0;
    }

    /// Re-anchor the expand state on the currently-selected row. The Context
    /// (half-height) preview sticks across selection moves: it stays open and
    /// re-anchors on the new row, so Down/Up browse previews in place. Full
    /// collapses, and a vanished selection always collapses.
    fn sync_expand_anchor(&mut self) {
        let sel = self.list.selected_row().map(|e| e.path.clone());
        if sel != self.expand_path {
            if self.expand != ExpandState::Context || sel.is_none() {
                self.expand = ExpandState::Collapsed;
            }
            self.expand_path = sel;
            self.content_scroll = 0;
        }
    }

    // ── Loading ─────────────────────────────────────────────────────────

    /// Record the newly-open note. Re-runs the query only when it depends on
    /// `{note}` (otherwise the existing results stay untouched).
    pub fn set_note(&mut self, note_path: VaultPath, tx: AppTx) {
        self.ensure_redraw_tx(&tx);
        *self.current_note.lock().unwrap() = note_path;
        if query_has_variables(self.list.query()) {
            self.list.reload();
            self.reset_expand();
        }
    }

    /// Current sort field/order, derived from the active query's order
    /// directive. Defaults to (Name, Ascending) when the query has none.
    /// Parses the query each call — cheap for the rare callers (dialog open).
    /// The per-frame render path uses the memoised `order_cache` instead.
    pub fn current_order(&self) -> (SortField, SortOrder) {
        let st = kimun_core::SearchTerms::from_query_string(self.list.query());
        match st.order_by.first() {
            Some(OrderBy::Title { asc }) => (
                SortField::Title,
                if *asc {
                    SortOrder::Ascending
                } else {
                    SortOrder::Descending
                },
            ),
            Some(OrderBy::FileName { asc }) => (
                SortField::Name,
                if *asc {
                    SortOrder::Ascending
                } else {
                    SortOrder::Descending
                },
            ),
            None => (SortField::Name, SortOrder::Ascending),
        }
    }

    /// Apply a sort selection from the sort dialog: rewrite the query's order
    /// directive (the query string is the single source of truth) and reload.
    pub fn apply_sort(&mut self, field: SortField, order: SortOrder, tx: &AppTx) {
        self.ensure_redraw_tx(tx);
        let order_field = match field {
            SortField::Name => OrderField::FileName,
            SortField::Title => OrderField::Title,
        };
        let asc = matches!(order, SortOrder::Ascending);
        let rewritten = with_order_directive(self.list.query(), order_field, asc);
        self.list.set_query(rewritten);
        // A sort only rewrites the order directive — the breadcrumb stays
        // (and `saved_search_breadcrumb` ignores the directive, so it is not
        // marked edited).
        self.reset_expand();
    }

    // ── Input handling ──────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        self.ensure_redraw_tx(tx);
        self.sync_expand_anchor();

        // Full-expand takes over Up/Down for content scroll BEFORE the engine
        // sees them.
        if self.is_full_expanded() && matches!(key.code, KeyCode::Up | KeyCode::Down) {
            self.scroll_content(key);
            return EventState::Consumed;
        }
        // NOTE: Enter is NOT pre-checked here. It must reach the engine so an
        // open autocomplete popup can accept on Enter; only when the popup is
        // closed does the engine return `Submit`, which toggles expand below.
        match self.list.handle_key(key) {
            KeyReaction::Intercepted(c) if self.follow_link_combos.contains(&c) => {
                if let Some(path) = self.selected_path().cloned() {
                    tx.send(AppEvent::OpenPath(path)).ok();
                }
                EventState::Consumed
            }
            KeyReaction::Consumed => {
                // Forward the query event to the breadcrumb: a `?name`
                // expansion pins it, a blank query clears it, a manual edit
                // keeps it (sticky).
                let accepted = self.list.take_accepted_saved_search();
                let blank = self.query_is_blank();
                self.saved_search
                    .on_query_consumed(accepted, self.list.query(), blank);
                self.sync_expand_anchor();
                EventState::Consumed
            }
            KeyReaction::Submit => {
                // Enter with the autocomplete popup closed: the panel's policy
                // is to cycle the expand state of the selected row.
                self.toggle_expand();
                EventState::Consumed
            }
            // Esc bubbles to the editor for focus changes.
            KeyReaction::Cancel => EventState::NotConsumed,
            KeyReaction::Unhandled => EventState::NotConsumed,
            KeyReaction::Intercepted(_) => EventState::Consumed,
        }
    }

    fn scroll_content(&mut self, key: &KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.content_scroll = self.content_scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                // Increment freely; render() clamps to content_scroll_max.
                self.content_scroll += 1;
            }
            _ => {}
        }
    }

    fn toggle_expand(&mut self) {
        if self.list.selected_row().is_none() {
            return;
        }
        self.expand_path = self.list.selected_row().map(|e| e.path.clone());
        match self.expand {
            ExpandState::Collapsed => {
                self.expand = ExpandState::Context;
            }
            ExpandState::Context => {
                self.content_scroll = 0;
                self.expand = ExpandState::Full;
            }
            ExpandState::Full => {
                self.content_scroll = 0;
                self.expand = ExpandState::Collapsed;
            }
        }
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        [
            (ActionShortcuts::FocusSidebar, "\u{2190} editor"),
            (ActionShortcuts::FollowLink, "open note"),
            (ActionShortcuts::SaveCurrentQuery, "save query"),
            (ActionShortcuts::OpenSavedSearches, "searches"),
            (ActionShortcuts::OpenSortDialog, "sort"),
        ]
        .iter()
        .filter_map(|(action, label)| {
            self.key_bindings
                .first_combo_for(action)
                .map(|k| (k, label.to_string()))
        })
        .collect()
    }

    // ── Rendering ──────────────────────────────────────────────────────

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.list.poll();
        self.sync_expand_anchor();

        let border_style = theme.border_style(focused);
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let count = self.list.visible_rows().len();
        // Reparse the order only when the query changed (memoised) — render runs
        // every frame and `from_query_string` is a full allocating parse.
        if self.list.query() != self.order_cache_query {
            self.order_cache = self.current_order();
            self.order_cache_query = self.list.query().to_string();
        }
        let (sort_field, sort_order) = self.order_cache;
        let sort_indicator = format!("{}{}", sort_field.label(), sort_order.label());
        // Compare ignoring the order directive so that sorting the default
        // backlinks query (`<{note} or:title`) still reads as "Backlinks".
        let base_query = kimun_core::strip_order_directive(self.list.query());
        // The saved-search name lives on the query searchbox border (the
        // breadcrumb below), not here, so the outer title stays generic.
        let title = if base_query == DEFAULT_QUERY {
            format!("Backlinks ({}) {}", count, sort_indicator)
        } else {
            format!("Query ({}) {}", count, sort_indicator)
        };

        let outer = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let outer_inner = outer.inner(rect);
        f.render_widget(outer, rect);

        // Split off the query line (top) from the list/preview (rest).
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(outer_inner);
        // The saved-search breadcrumb (`‹ name ›` / `‹ name • edited ›`) titles
        // the query searchbox when a saved search is active.
        let search_title = self.saved_search.border_title(self.list.query(), " Query");
        let search_block = Block::default()
            .title(search_title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let search_inner = search_block.inner(rows[0]);
        f.render_widget(search_block, rows[0]);
        self.list.render_query(f, search_inner, theme, focused);

        let inner = rows[1];

        if self.list.is_loading() {
            f.render_widget(
                Paragraph::new("  Loading...").style(Style::default().fg(fg_muted).bg(bg)),
                inner,
            );
            self.list.render_autocomplete(f, rect, theme);
            return;
        }

        if self.list.visible_rows().is_empty() {
            f.render_widget(
                Paragraph::new("  No results").style(Style::default().fg(fg_muted).bg(bg)),
                inner,
            );
            self.list.render_autocomplete(f, rect, theme);
            return;
        }

        let selected_state = self.expand;

        // Full mode: content takes the entire panel, no list visible.
        if selected_state == ExpandState::Full {
            if let Some(entry) = self.list.selected_row() {
                let entry = entry.clone();
                let text = entry.full_text.as_deref().unwrap_or(&entry.context);

                // Split into fixed header (title + divider) and scrollable content.
                let title_display = if entry.title.is_empty() {
                    &entry.filename
                } else {
                    &entry.title
                };

                let parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1), // title
                        Constraint::Length(1), // divider
                        Constraint::Min(0),    // content
                    ])
                    .split(inner);

                // Fixed title header.
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!("\u{25BC} {} ", title_display),
                            Style::default()
                                .fg(theme.fg_selected.to_ratatui())
                                .bg(bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {}", entry.filename),
                            Style::default().fg(fg_muted).bg(bg),
                        ),
                    ]))
                    .style(Style::default().bg(bg)),
                    parts[0],
                );

                // Fixed divider.
                f.render_widget(
                    Paragraph::new("\u{2500}".repeat(parts[1].width as usize))
                        .style(Style::default().fg(fg_muted).bg(bg)),
                    parts[1],
                );

                // Scrollable content.
                let indent = 2usize;
                let wrap_width = parts[2].width.saturating_sub(indent as u16 + 1) as usize;
                let needles = query_needles(&self.resolved_query());

                let mut lines = Vec::new();
                for line in text.lines() {
                    let wrapped = wrap_line(line, wrap_width);
                    for wline in wrapped {
                        let spans = highlight_needles(&wline, &needles, fg_muted, bg, theme);
                        let mut indented =
                            vec![Span::styled(" ".repeat(indent), Style::default().bg(bg))];
                        indented.extend(spans);
                        lines.push(Line::from(indented));
                    }
                }

                let total_lines = lines.len();
                let viewport = parts[2].height as usize;
                self.content_scroll_max = total_lines.saturating_sub(viewport);
                self.content_scroll = self.content_scroll.min(self.content_scroll_max);

                f.render_widget(
                    Paragraph::new(lines)
                        .scroll((self.content_scroll as u16, 0))
                        .style(Style::default().bg(bg)),
                    parts[2],
                );
            }
            self.list.render_autocomplete(f, rect, theme);
            return;
        }

        // Context or Collapsed: show the list, optionally with preview below.
        let has_context = selected_state == ExpandState::Context;

        let (list_area, divider_area, content_area) = if has_context {
            let max_list = inner.height / 2;
            let list_height = (count as u16).min(max_list).max(1);
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(list_height),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .split(inner);
            (areas[0], Some(areas[1]), Some(areas[2]))
        } else {
            (inner, None, None)
        };

        // The engine draws the collapsed list (1 line per row, with the
        // selected-row marker handled in `to_list_item`).
        self.list.render(f, list_area, theme, focused);
        self.list.set_list_rect(list_area);

        // Divider between list and content.
        if let Some(div) = divider_area {
            f.render_widget(
                Paragraph::new("\u{2500}".repeat(div.width as usize))
                    .style(Style::default().fg(fg_muted).bg(bg)),
                div,
            );
        }

        // Render context preview below the list: show the full note text
        // scrolled so the first link occurrence is visible with context above.
        if let Some(area) = content_area
            && selected_state == ExpandState::Context
            && let Some(entry) = self.list.selected_row()
        {
            let entry = entry.clone();
            let text = entry.full_text.as_deref().unwrap_or(&entry.context);
            let indent = 2usize;
            let wrap_width = area.width.saturating_sub(indent as u16 + 1) as usize;
            let needles = query_needles(&self.resolved_query());

            let mut lines = Vec::new();

            // Track which rendered line contains the first needle match.
            let mut link_line: Option<usize> = None;

            for line in text.lines() {
                let wrapped = wrap_line(line, wrap_width);
                for wline in wrapped {
                    if link_line.is_none()
                        && needles
                            .iter()
                            .any(|n| !n.is_empty() && find_case_insensitive(&wline, n).is_some())
                    {
                        link_line = Some(lines.len());
                    }
                    let spans = highlight_needles(&wline, &needles, fg_muted, bg, theme);
                    let mut indented =
                        vec![Span::styled(" ".repeat(indent), Style::default().bg(bg))];
                    indented.extend(spans);
                    lines.push(Line::from(indented));
                }
            }

            // Scroll to show the link with context above. If the content from
            // the link to the end fits within the viewport, scroll back further
            // to fill the available space.
            let viewport = area.height as usize;
            let total = lines.len();
            let link_pos = link_line.unwrap_or(0);
            let lines_after_link = total.saturating_sub(link_pos);
            let scroll_to = if lines_after_link <= viewport {
                // Content from link to end fits — scroll back to fill the viewport.
                total.saturating_sub(viewport)
            } else {
                // More content below the link — show 2 lines of context above.
                link_pos.saturating_sub(2)
            } as u16;

            f.render_widget(
                Paragraph::new(lines)
                    .scroll((scroll_to, 0))
                    .style(Style::default().bg(bg)),
                area,
            );
        }

        self.list.render_autocomplete(f, rect, theme);
    }
}

// ---------------------------------------------------------------------------
// Standalone async helpers
// ---------------------------------------------------------------------------

/// Run `query` (already a resolved plain query string) and build entries.
/// Sources from full-text / query search via `vault.search_notes`.
async fn load_query(vault: &NoteVault, query: &str) -> Vec<BacklinkEntry> {
    let needles = query_needles(query);
    let results = vault.search_notes(query).await.unwrap_or_default();
    let mut entries = Vec::with_capacity(results.len());
    for (entry_data, content_data) in results {
        let text = vault
            .get_note_text(&entry_data.path)
            .await
            .unwrap_or_default();
        let context = extract_context_multi(&text, &needles);
        let (_p, filename) = entry_data.path.get_parent_path();
        entries.push(BacklinkEntry {
            path: entry_data.path,
            title: content_data.title,
            filename,
            context,
            full_text: Some(text),
        });
    }
    entries
}

/// Split text into paragraphs. A paragraph is one or more consecutive
/// non-blank lines. Blank lines act as separators.
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }

    paragraphs
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Wrap a single line into multiple lines that fit within `max_width` characters.
/// Uses character count (not byte length) for width. Wraps at word boundaries
/// when possible, hard-breaks otherwise.
fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || line.chars().count() <= max_width {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut remaining = line;

    while remaining.chars().count() > max_width {
        // Find the byte index of the max_width-th character.
        let byte_limit = remaining
            .char_indices()
            .nth(max_width)
            .map(|(i, _)| i)
            .unwrap_or(remaining.len());

        // Try to find a space to break at (within the allowed character range).
        let break_at = remaining[..byte_limit]
            .rfind(' ')
            .map(|i| i + 1) // include the space on the current line
            .unwrap_or(byte_limit); // hard break if no space
        result.push(remaining[..break_at].trim_end().to_string());
        remaining = &remaining[break_at..];
    }
    if !remaining.is_empty() {
        result.push(remaining.to_string());
    }
    result
}

/// Case-insensitive search for `needle` in `haystack`, returning the byte
/// range `(start, end)` in `haystack` where the match occurs. Compares
/// char-by-char via `to_lowercase()` so byte lengths are always derived from
/// the original string, avoiding the case-folding byte-mismatch problem.
fn find_case_insensitive(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let needle_chars: Vec<char> = needle.chars().collect();
    if needle_chars.is_empty() {
        return None;
    }
    let hay_indices: Vec<(usize, char)> = haystack.char_indices().collect();
    'outer: for start_idx in 0..hay_indices.len() {
        if start_idx + needle_chars.len() > hay_indices.len() {
            break;
        }
        for (j, &nc) in needle_chars.iter().enumerate() {
            let hc = hay_indices[start_idx + j].1;
            // Compare lowercased chars.
            let mut h_lower = hc.to_lowercase();
            let mut n_lower = nc.to_lowercase();
            if h_lower.next() != n_lower.next() {
                continue 'outer;
            }
        }
        // Match found — compute byte range from haystack char indices.
        let byte_start = hay_indices[start_idx].0;
        let byte_end = if start_idx + needle_chars.len() < hay_indices.len() {
            hay_indices[start_idx + needle_chars.len()].0
        } else {
            haystack.len()
        };
        return Some((byte_start, byte_end));
    }
    None
}

/// Find the first paragraph containing any of `needles` (case-insensitive);
/// fall back to the first non-blank line.
fn extract_context_multi(text: &str, needles: &[String]) -> String {
    let lowered: Vec<String> = needles.iter().map(|n| n.to_lowercase()).collect();
    for para in &split_paragraphs(text) {
        let lower = para.to_lowercase();
        if lowered.iter().any(|n| !n.is_empty() && lower.contains(n)) {
            return para.clone();
        }
    }
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string()
}

/// Highlight the earliest occurrence of any needle in `line` (bold accent).
fn highlight_needles(
    line: &str,
    needles: &[String],
    fg_muted: ratatui::style::Color,
    bg: ratatui::style::Color,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let normal = Style::default().fg(fg_muted).bg(bg);
    let bold = Style::default()
        .fg(theme.accent.to_ratatui())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let mut best: Option<(usize, usize)> = None;
    for needle in needles {
        if needle.is_empty() {
            continue;
        }
        if let Some((s, e)) = find_case_insensitive(line, needle)
            && (best.is_none() || s < best.unwrap().0)
        {
            best = Some((s, e));
        }
    }
    let Some((start, end)) = best else {
        return vec![Span::styled(line.to_string(), normal)];
    };
    let mut spans = Vec::new();
    if start > 0 {
        spans.push(Span::styled(line[..start].to_string(), normal));
    }
    spans.push(Span::styled(line[start..end].to_string(), bold));
    if end < line.len() {
        spans.push(Span::styled(line[end..].to_string(), normal));
    }
    spans
}

/// Needles to highlight for a query: its free-text terms + link targets
/// (both backlink and forward-link targets).
fn query_needles(query: &str) -> Vec<String> {
    let st = kimun_core::SearchTerms::from_query_string(query);
    let mut needles = st.terms.clone();
    needles.extend(st.links.clone());
    needles.extend(st.forward_links.clone());
    needles
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_line_fits_within_width() {
        let result = wrap_line("short", 20);
        assert_eq!(result, vec!["short"]);
    }

    #[test]
    fn wrap_line_breaks_at_word_boundary() {
        let result = wrap_line("hello world foo bar", 12);
        assert_eq!(result, vec!["hello world", "foo bar"]);
    }

    #[test]
    fn wrap_line_hard_breaks_long_word() {
        let result = wrap_line("abcdefghij", 5);
        assert_eq!(result, vec!["abcde", "fghij"]);
    }

    #[test]
    fn wrap_line_handles_multibyte_chars() {
        // 5 CJK characters — each is 1 char, should wrap at char boundary
        let result = wrap_line("日本語テスト", 3);
        assert_eq!(result, vec!["日本語", "テスト"]);
    }

    #[test]
    fn wrap_line_empty_string() {
        let result = wrap_line("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn extract_context_matches_any_needle() {
        let text = "# Title\n\nIntro line.\n\nA paragraph mentioning widget here.\n";
        let result = extract_context_multi(text, &["widget".to_string()]);
        assert!(result.contains("widget"));
    }

    #[test]
    fn highlight_needles_highlights_first_match() {
        let spans = highlight_needles(
            "see widget and gadget",
            &["gadget".to_string()],
            ratatui::style::Color::Gray,
            ratatui::style::Color::Black,
            &crate::settings::themes::Theme::default(),
        );
        assert!(
            spans
                .iter()
                .any(|s| s.content.contains("gadget")
                    && s.style.add_modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn query_needles_extracts_terms_and_links() {
        let n = query_needles("widget <spec");
        assert!(n.iter().any(|x| x == "widget"));
        assert!(n.iter().any(|x| x == "spec"));
    }

    #[test]
    fn query_needles_extracts_forward_links() {
        // A forward-link query (`>target`) must contribute its target as a
        // highlight needle, just like a backlink query (`<target`).
        let n = query_needles(">spec");
        assert!(n.iter().any(|x| x == "spec"));
    }

    #[tokio::test]
    async fn query_panel_load_query_lists_matches() {
        let vault = crate::test_support::temp_vault("qp").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/a.md"), "alpha #todo")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/b.md"), "beta")
            .await
            .unwrap();
        let entries = load_query(&vault, "#todo").await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].filename.contains("a"));
    }

    fn make_panel(vault: Arc<NoteVault>) -> QueryPanel {
        let kb = crate::settings::AppSettings::default().key_bindings.clone();
        QueryPanel::new(vault, kb)
    }

    /// Drive the engine until its async load settles. Unlike the engine's
    /// `poll_until_idle` (tight `yield_now` loop), this gives the spawned
    /// sqlite-backed search task real wall-clock time to complete — `load_query`
    /// awaits a full-text search plus per-result `get_note_text`, which a
    /// yield-only loop does not advance fast enough.
    async fn settle(panel: &mut QueryPanel) {
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            panel.list.poll();
            if !panel.list.is_loading() {
                break;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_rewrites_query_order_directive() {
        let vault = crate::test_support::temp_vault("qp-sort").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("widget".to_string());

        panel.apply_sort(SortField::Title, SortOrder::Ascending, &tx);
        assert_eq!(panel.active_query(), "widget or:title");

        panel.apply_sort(SortField::Name, SortOrder::Descending, &tx);
        assert_eq!(panel.active_query(), "widget -or:file");
    }

    /// Regression: a directive-less query must still return a stable
    /// Name-ascending order (the DB only sorts when an `or:` directive is
    /// present). Without the fallback, results came back in arbitrary DB order.
    #[tokio::test(flavor = "multi_thread")]
    async fn directiveless_query_is_name_ascending() {
        let vault = crate::test_support::temp_vault("qp-defaultorder").await;
        vault.validate_and_init().await.unwrap();
        // Create in non-alphabetical order; all share the term "widget".
        for name in ["/charlie.md", "/alpha.md", "/bravo.md"] {
            vault
                .create_note(&VaultPath::note_path_from(name), "widget")
                .await
                .unwrap();
        }
        let mut panel = make_panel(vault);
        panel.set_active_query("widget".to_string()); // no order directive
        settle(&mut panel).await;

        let names: Vec<String> = panel
            .list
            .visible_rows()
            .iter()
            .map(|e| e.filename.clone())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "directive-less query must be name-ascending");
    }

    /// Accepting a `?name` expansion through the panel pins the saved-search
    /// breadcrumb to the accepted name and runs the stored query.
    #[tokio::test(flavor = "multi_thread")]
    async fn accepting_saved_search_pins_breadcrumb() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let vault = crate::test_support::temp_vault("qp-ss-accept").await;
        vault.validate_and_init().await.unwrap();
        vault.save_search("todo-week", "#todo").await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Clear the default query so `?` is the leading char, then type a
        // prefix, draining the async popup load between keystrokes so the
        // suggestion lands before we accept.
        panel.set_active_query(String::new());
        for ch in ['?', 't', 'o'] {
            panel.handle_key(&KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), &tx);
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                panel.list.poll();
            }
        }
        panel.handle_key(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &tx);

        assert_eq!(panel.active_query(), "#todo");
        assert_eq!(
            panel.saved_search_breadcrumb().as_deref(),
            Some("todo-week")
        );
    }

    /// Editing the expanded query keeps the breadcrumb (sticky provenance) and
    /// marks it `• edited` once the text diverges from the stored query.
    #[tokio::test(flavor = "multi_thread")]
    async fn editing_expanded_query_keeps_breadcrumb_marked_edited() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let vault = crate::test_support::temp_vault("qp-ss-edit").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.apply_query("#todo".to_string(), Some("todo".to_string()), tx.clone());
        assert_eq!(panel.saved_search_breadcrumb().as_deref(), Some("todo"));

        // A manual edit must NOT drop the breadcrumb; it gains the marker.
        panel.handle_key(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), &tx);
        assert_eq!(panel.active_query(), "#todox");
        assert_eq!(
            panel.saved_search_breadcrumb().as_deref(),
            Some("todo • edited")
        );
    }

    /// Emptying the query field clears the breadcrumb entirely (one of the two
    /// clear triggers, the other being a fresh expansion).
    #[tokio::test(flavor = "multi_thread")]
    async fn emptying_field_clears_breadcrumb() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let vault = crate::test_support::temp_vault("qp-ss-empty").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.apply_query("#todo".to_string(), Some("todo".to_string()), tx.clone());

        // Backspace the whole "#todo" away.
        for _ in 0.."#todo".len() {
            panel.handle_key(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), &tx);
        }
        assert_eq!(panel.active_query(), "");
        assert_eq!(panel.saved_search_breadcrumb(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_query_pins_breadcrumb() {
        let vault = crate::test_support::temp_vault("qp-name").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.apply_query("#todo".to_string(), Some("todo".to_string()), tx.clone());
        assert_eq!(panel.saved_search_breadcrumb().as_deref(), Some("todo"));
    }

    /// Applying a sort rewrites the query's order directive but keeps the
    /// breadcrumb sticky — and NOT marked edited, because the edited check
    /// ignores the order directive.
    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_keeps_saved_search_breadcrumb() {
        let vault = crate::test_support::temp_vault("qp-sort-name").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.apply_query("#todo".to_string(), Some("todo".to_string()), tx.clone());

        panel.apply_sort(SortField::Title, SortOrder::Ascending, &tx);
        assert_eq!(panel.active_query(), "#todo or:title");
        assert_eq!(
            panel.saved_search_breadcrumb().as_deref(),
            Some("todo"),
            "sorting keeps the unedited breadcrumb"
        );
    }

    /// Regression: a programmatic sort change must update the VISIBLE input bar,
    /// not just the internal query string. (Previously `set_query` left the
    /// input widget stale, so the bar didn't show the `or:` directive.)
    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_updates_visible_input_bar() {
        let vault = crate::test_support::temp_vault("qp-bar").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("widget".to_string());
        assert_eq!(
            panel.list.input_value(),
            "widget",
            "set_active_query syncs the bar"
        );

        panel.apply_sort(SortField::Title, SortOrder::Ascending, &tx);
        assert_eq!(panel.active_query(), "widget or:title");
        assert_eq!(
            panel.list.input_value(),
            "widget or:title",
            "the input bar must reflect the rewritten query"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn current_order_reads_query_directive() {
        let vault = crate::test_support::temp_vault("qp-order").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        panel.set_active_query("widget -or:title".to_string());
        assert_eq!(
            panel.current_order(),
            (SortField::Title, SortOrder::Descending)
        );
        panel.set_active_query("widget".to_string());
        assert_eq!(
            panel.current_order(),
            (SortField::Name, SortOrder::Ascending)
        );
    }

    /// A static query (no `{note}`) must survive navigation: `set_note` leaves
    /// its query template untouched and does NOT reload the engine.
    // Multi-thread flavour: the engine drives the source load on a spawned
    // task, and `search_notes` awaits a sqlite pool that needs the IO driver
    // (a current-thread runtime only advances the spawned task on `yield_now`).
    #[tokio::test(flavor = "multi_thread")]
    async fn static_query_survives_navigation() {
        let vault = crate::test_support::temp_vault("nav-static").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/a.md"), "alpha #todo")
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        panel.set_active_query("#todo".to_string());
        settle(&mut panel).await;
        assert_eq!(panel.list.visible_rows().len(), 1);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_note(VaultPath::note_path_from("x.md"), tx);

        // Query template untouched (not reset to <{note}); a static query is
        // not reloaded, so it is not in a loading state.
        assert_eq!(panel.active_query(), "#todo");
        assert!(!panel.list.is_loading());
        settle(&mut panel).await;
        assert_eq!(panel.list.visible_rows().len(), 1); // results untouched
    }

    /// A `{note}` query re-runs on navigation: `set_note` resolves `{note}`
    /// against the new note and reloads, so results follow the open note.
    #[tokio::test(flavor = "multi_thread")]
    async fn note_variable_query_reruns_on_navigation() {
        let vault = crate::test_support::temp_vault("nav-var").await;
        vault.validate_and_init().await.unwrap();
        // `target` is linked from `linker`; opening `target` should surface
        // `linker` as a backlink.
        vault
            .create_note(&VaultPath::note_path_from("/target.md"), "I am the target")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/linker.md"), "see [[target]]")
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        assert_eq!(panel.active_query(), DEFAULT_QUERY);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_note(VaultPath::note_path_from("/target.md"), tx);
        settle(&mut panel).await;

        // The `{note}` query resolved against `target` and found the backlink.
        assert!(
            panel
                .list
                .visible_rows()
                .iter()
                .any(|e| e.filename.contains("linker")),
            "expected linker as a backlink, got {:?}",
            panel
                .list
                .visible_rows()
                .iter()
                .map(|e| e.filename.clone())
                .collect::<Vec<_>>()
        );
    }

    /// Navigating to a different `{note}` re-resolves and changes results.
    #[tokio::test(flavor = "multi_thread")]
    async fn note_variable_query_changes_with_note() {
        let vault = crate::test_support::temp_vault("nav-var2").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/a.md"), "I am a")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/b.md"), "I am b")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/links_a.md"), "see [[a]]")
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        panel.set_note(VaultPath::note_path_from("/a.md"), tx.clone());
        settle(&mut panel).await;
        assert!(
            panel
                .list
                .visible_rows()
                .iter()
                .any(|e| e.filename.contains("links_a"))
        );

        panel.set_note(VaultPath::note_path_from("/b.md"), tx);
        settle(&mut panel).await;
        assert!(
            !panel
                .list
                .visible_rows()
                .iter()
                .any(|e| e.filename.contains("links_a")),
            "b has no backlinks, expected empty"
        );
    }
}
