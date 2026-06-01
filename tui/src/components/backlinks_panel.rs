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

use crate::components::autocomplete::AutocompleteMode;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::file_list::{SortField, SortOrder};
use crate::components::query_vars::{query_has_variables, resolve_query};
use crate::components::search_list::{
    Emit, KeyReaction, RowSource, SearchList, SearchRow, VaultSuggestions,
};
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::KeyBindings;
use crate::keys::key_combo::KeyCombo;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// The default query the panel runs: backlinks to the current note.
const DEFAULT_QUERY: &str = ">{note}";

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
/// (e.g. `>{note}`, so the input shows the template); this source resolves
/// `{note}` against the shared current note at load time, preserving the exact
/// "input shows the template, results are backlinks of the current note" UX.
/// The shared sort handle lets the source order results by the panel's active
/// sort field/order (the engine itself uses `SourceOrder`).
struct BacklinkSource {
    vault: Arc<NoteVault>,
    current_note: Arc<Mutex<VaultPath>>,
    sort: Arc<Mutex<(SortField, SortOrder)>>,
}

#[async_trait]
impl RowSource<BacklinkEntry> for BacklinkSource {
    async fn load(&self, query: &str, emit: Emit<BacklinkEntry>) {
        // Clone the note out of the lock, then drop the guard before awaiting.
        let note = self.current_note.lock().unwrap().clone();
        let q = resolve_query(query, Some(&note));
        let mut entries = load_query(&self.vault, &q).await;
        let (field, order) = *self.sort.lock().unwrap();
        sort_entries(&mut entries, field, order);
        emit.replace(entries);
    }
}

/// Sort backlink entries in place by the given field and order.
fn sort_entries(entries: &mut [BacklinkEntry], field: SortField, order: SortOrder) {
    entries.sort_by(|a, b| {
        let cmp = match field {
            SortField::Name => a.filename.to_lowercase().cmp(&b.filename.to_lowercase()),
            SortField::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        };
        match order {
            SortOrder::Ascending => cmp,
            SortOrder::Descending => cmp.reverse(),
        }
    });
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
    /// Name of the saved search currently applied (drives the title); `None`
    /// for the default / a manually-edited query.
    saved_search_name: Option<String>,
    /// Shared sort field/order. `BacklinkSource::load` reads it to order
    /// results; the panel mutates it on sort shortcuts then reloads.
    sort: Arc<Mutex<(SortField, SortOrder)>>,
    /// Expand state of the currently-selected row. Reset to `Collapsed` on any
    /// navigation or query change.
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
    /// Combos that the engine intercepts: sort cycle / reverse / follow-link.
    sort_cycle_combos: Vec<KeyCombo>,
    sort_reverse_combos: Vec<KeyCombo>,
    follow_link_combos: Vec<KeyCombo>,
}

impl QueryPanel {
    pub fn new(vault: Arc<NoteVault>, key_bindings: KeyBindings) -> Self {
        let icons = Icons::new(false);
        let current_note = Arc::new(Mutex::new(VaultPath::empty()));
        let sort = Arc::new(Mutex::new((SortField::Name, SortOrder::Ascending)));
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
            sort: sort.clone(),
        };
        let combos = |action: &ActionShortcuts| -> Vec<KeyCombo> {
            key_bindings
                .to_hashmap()
                .get(action)
                .cloned()
                .unwrap_or_default()
        };
        let sort_cycle_combos = combos(&ActionShortcuts::CycleSortField);
        let sort_reverse_combos = combos(&ActionShortcuts::SortReverseOrder);
        let follow_link_combos = combos(&ActionShortcuts::FollowLink);

        let mut intercept = Vec::new();
        intercept.extend(sort_cycle_combos.iter().cloned());
        intercept.extend(sort_reverse_combos.iter().cloned());
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
            saved_search_name: None,
            sort,
            expand: ExpandState::Collapsed,
            expand_path: None,
            content_scroll: 0,
            content_scroll_max: 0,
            key_bindings,
            redraw_tx,
            sort_cycle_combos,
            sort_reverse_combos,
            follow_link_combos,
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

    pub fn set_saved_search_name(&mut self, name: Option<String>) {
        self.saved_search_name = name;
    }

    /// Apply a query template (e.g. from a saved search) and run it. The engine
    /// holds the template verbatim; `{note}` is resolved at load. `name` drives
    /// the title (`None` for the default backlinks query).
    pub fn apply_query(&mut self, query: String, name: Option<String>, tx: AppTx) {
        self.ensure_redraw_tx(&tx);
        self.set_active_query(query);
        self.set_saved_search_name(name);
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

    /// Re-anchor the expand state on the currently-selected row. If selection
    /// has moved to a different row, collapse it.
    fn sync_expand_anchor(&mut self) {
        let sel = self.list.selected_row().map(|e| e.path.clone());
        if sel != self.expand_path {
            self.expand = ExpandState::Collapsed;
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

    /// Advance the sort field, then reload so the source re-orders the results.
    fn cycle_sort(&mut self) {
        {
            let mut s = self.sort.lock().unwrap();
            s.0 = s.0.cycle();
        }
        self.list.reload();
        self.reset_expand();
    }

    /// Toggle the sort order, then reload so the source re-orders the results.
    fn reverse_sort(&mut self) {
        {
            let mut s = self.sort.lock().unwrap();
            s.1 = s.1.toggle();
        }
        self.list.reload();
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
        // Enter cycles expand (the engine would Submit; the panel's policy is to
        // toggle the expand state instead).
        if key.code == KeyCode::Enter {
            self.toggle_expand();
            return EventState::Consumed;
        }

        match self.list.handle_key(key) {
            KeyReaction::Intercepted(c) if self.sort_cycle_combos.contains(&c) => {
                self.cycle_sort();
                EventState::Consumed
            }
            KeyReaction::Intercepted(c) if self.sort_reverse_combos.contains(&c) => {
                self.reverse_sort();
                EventState::Consumed
            }
            KeyReaction::Intercepted(c) if self.follow_link_combos.contains(&c) => {
                if let Some(path) = self.selected_path().cloned() {
                    tx.send(AppEvent::OpenPath(path)).ok();
                }
                EventState::Consumed
            }
            KeyReaction::Consumed => {
                // A query edit or navigation: drop the saved-search label (a
                // manual edit is no longer the named search) and re-anchor the
                // expand state on the (possibly new) selection.
                if self.list.query() != DEFAULT_QUERY {
                    self.saved_search_name = None;
                }
                self.sync_expand_anchor();
                EventState::Consumed
            }
            KeyReaction::Submit => {
                // Unreachable: Enter is handled by the pre-check above. Kept for
                // completeness.
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
            (ActionShortcuts::CycleSortField, "sort"),
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
        let (sort_field, sort_order) = *self.sort.lock().unwrap();
        let sort_indicator = format!("{}{}", sort_field.label(), sort_order.label());
        let title = if self.list.query() == DEFAULT_QUERY {
            format!("Backlinks ({}) {}", count, sort_indicator)
        } else if let Some(name) = &self.saved_search_name {
            format!("{} ({}) {}", name, count, sort_indicator)
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
        let search_block = Block::default()
            .title(" Query")
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

/// Needles to highlight for a query: its free-text terms + link targets.
fn query_needles(query: &str) -> Vec<String> {
    let st = kimun_core::SearchTerms::from_query_string(query);
    let mut needles = st.terms.clone();
    needles.extend(st.links.clone());
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
        let n = query_needles("widget >spec");
        assert!(n.iter().any(|x| x == "widget"));
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

        // Query template untouched (not reset to >{note}); a static query is
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
