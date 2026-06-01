use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::components::autocomplete::{
    self, AutocompleteController, AutocompleteHost, AutocompleteMode, HandleKeyOutcome,
    TriggerOptions,
};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, redraw_callback};
use crate::components::file_list::{SortField, SortOrder};
use crate::components::query_vars::{query_has_variables, resolve_query};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::{KeyBindings, key_event_to_combo};
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
// SearchBoxHostSnapshot
// ---------------------------------------------------------------------------

/// Snapshot of the query input that satisfies `AutocompleteHost`.
/// Owned so the controller's borrow doesn't overlap with the input's
/// `&mut` borrow during key handling and replacement. Mirrors the
/// snapshot in `note_browser/mod.rs` (duplicated here intentionally;
/// the type is tiny and self-contained).
struct SearchBoxHostSnapshot {
    lines: Vec<String>,
    cursor: (usize, usize),
    caret_pos: Option<(u16, u16)>,
}

impl AutocompleteHost for SearchBoxHostSnapshot {
    fn buffer_snapshot(&self) -> crate::components::text_editor::snapshot::EditorSnapshot<'_> {
        use std::num::NonZeroU64;
        let dummy = NonZeroU64::new(1).unwrap();
        crate::components::text_editor::snapshot::EditorSnapshot::borrowed(
            &self.lines,
            self.cursor,
            dummy,
        )
    }
    fn cache_key(&self) -> Option<std::num::NonZeroU64> {
        None
    }
    fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
        self.caret_pos
    }
}

// ---------------------------------------------------------------------------
// QueryPanel
// ---------------------------------------------------------------------------

pub struct QueryPanel {
    entries: Vec<BacklinkEntry>,
    expand_states: Vec<ExpandState>,
    list_state: ListState,
    loading: bool,
    current_note: VaultPath,
    sort_field: SortField,
    sort_order: SortOrder,
    vault: Arc<NoteVault>,
    key_bindings: KeyBindings,
    /// Editable query line. Defaults to the backlinks query `>{note}`.
    query_input: SingleLineInput,
    /// Name of the saved search currently applied (drives the title);
    /// `None` for the default / a manually-edited query.
    saved_search_name: Option<String>,
    /// Autocomplete for the query line (`#` tags, `>` link targets).
    autocomplete: AutocompleteController,
    /// Scroll offset for full-expanded content view.
    content_scroll: usize,
    /// Maximum scroll offset (computed during render).
    content_scroll_max: usize,
}

impl QueryPanel {
    pub fn new(vault: Arc<NoteVault>, key_bindings: KeyBindings) -> Self {
        let autocomplete =
            AutocompleteController::new(std::sync::Arc::new(crate::components::search_list::VaultSuggestions { vault: vault.clone() }), AutocompleteMode::SearchQuery)
                .with_trigger_opts(TriggerOptions {
                    disambiguate_header: false,
                    apply_exclusion_zone: false,
                });
        Self {
            entries: Vec::new(),
            expand_states: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            current_note: VaultPath::empty(),
            sort_field: SortField::Name,
            sort_order: SortOrder::Ascending,
            vault,
            key_bindings,
            query_input: SingleLineInput::with_value(DEFAULT_QUERY),
            saved_search_name: None,
            // The redraw callback needs an `AppTx`, which `new` does not
            // receive (the panel is built before the app event channel in
            // some construction orders). It is wired lazily via
            // `ensure_redraw_callback` the first time a key arrives with a
            // `tx`, before the popup can open.
            autocomplete,
            content_scroll: 0,
            content_scroll_max: 0,
        }
    }

    /// Wire the autocomplete redraw callback. Idempotent-ish: callers
    /// invoke this once a `tx` is available (the panel is created before
    /// the app event channel in some construction orders).
    fn ensure_redraw_callback(&mut self, tx: &AppTx) {
        self.autocomplete
            .set_redraw_callback(redraw_callback(tx.clone()));
    }

    // ── Query accessors ─────────────────────────────────────────────────

    pub fn active_query(&self) -> &str {
        self.query_input.value()
    }

    pub fn set_active_query(&mut self, q: String) {
        self.query_input.set_value(q);
    }

    pub fn set_saved_search_name(&mut self, name: Option<String>) {
        self.saved_search_name = name;
    }

    /// Apply a query (e.g. from a saved search) and run it immediately.
    /// `name` drives the title (`None` for the default backlinks query).
    pub fn apply_query(&mut self, query: String, name: Option<String>, tx: AppTx) {
        self.set_active_query(query);
        self.set_saved_search_name(name);
        self.run_query(tx);
    }

    fn autocomplete_snapshot(&self) -> SearchBoxHostSnapshot {
        let value = self.query_input.value().to_string();
        let cursor_byte = self.query_input.cursor_byte();
        let col = value[..cursor_byte.min(value.len())].chars().count();
        SearchBoxHostSnapshot {
            lines: vec![value],
            cursor: (0, col),
            caret_pos: self.query_input.last_caret_pos(),
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Returns true if the selected entry is in full-expand mode (content takes
    /// the whole panel, up/down scrolls content).
    fn is_full_expanded(&self) -> bool {
        self.list_state
            .selected()
            .and_then(|i| self.expand_states.get(i))
            .is_some_and(|s| *s == ExpandState::Full)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn selected_path(&self) -> Option<&VaultPath> {
        self.list_state
            .selected()
            .and_then(|i| self.entries.get(i))
            .map(|e| &e.path)
    }

    // ── Loading ─────────────────────────────────────────────────────────

    /// Record the newly-open note. Re-runs the query only when it depends
    /// on `{note}` (otherwise the existing results stay untouched).
    pub fn set_note(&mut self, note_path: VaultPath, tx: AppTx) {
        self.current_note = note_path;
        if query_has_variables(self.active_query()) {
            self.run_query(tx);
        }
    }

    /// Resolve the active query against the current note and spawn a
    /// background search. Clears existing state and sets `loading = true`;
    /// the task sends `AppEvent::BacklinksLoaded` when finished.
    fn run_query(&mut self, tx: AppTx) {
        self.entries.clear();
        self.expand_states.clear();
        self.list_state.select(None);
        self.loading = true;
        self.content_scroll = 0;
        self.content_scroll_max = 0;

        let q = resolve_query(self.active_query(), Some(&self.current_note));
        let vault = Arc::clone(&self.vault);
        tokio::spawn(async move {
            let entries = load_query(&vault, &q).await;
            let _ = tx.send(AppEvent::BacklinksLoaded(entries));
        });
    }

    /// Called when the background task completes. Stores the entries, applies
    /// the current sort, and initialises expand states.
    pub fn on_loaded(&mut self, entries: Vec<BacklinkEntry>) {
        self.entries = entries;
        self.apply_sort();
        self.expand_states = vec![ExpandState::Collapsed; self.entries.len()];
        self.loading = false;
        if !self.entries.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    /// Sort `entries` (and their parallel `expand_states`) by the active
    /// sort field and order.
    pub fn apply_sort(&mut self) {
        let field = self.sort_field;
        let order = self.sort_order;

        // Build index permutation so we can reorder expand_states in sync.
        let mut indices: Vec<usize> = (0..self.entries.len()).collect();
        indices.sort_by(|&a, &b| {
            let cmp = match field {
                SortField::Name => self.entries[a]
                    .filename
                    .to_lowercase()
                    .cmp(&self.entries[b].filename.to_lowercase()),
                SortField::Title => self.entries[a]
                    .title
                    .to_lowercase()
                    .cmp(&self.entries[b].title.to_lowercase()),
            };
            match order {
                SortOrder::Ascending => cmp,
                SortOrder::Descending => cmp.reverse(),
            }
        });

        let sorted_entries: Vec<BacklinkEntry> =
            indices.iter().map(|&i| self.entries[i].clone()).collect();
        let sorted_states: Vec<ExpandState> = if self.expand_states.len() == self.entries.len() {
            indices.iter().map(|&i| self.expand_states[i]).collect()
        } else {
            vec![ExpandState::Collapsed; sorted_entries.len()]
        };

        self.entries = sorted_entries;
        self.expand_states = sorted_states;
    }

    // ── Input handling ──────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        // Lazily wire the redraw callback now that we have a tx.
        self.ensure_redraw_callback(tx);

        // Autocomplete popup gets first crack at the key when open:
        // Up/Down/Tab/Enter/Esc navigate / accept / dismiss the popup
        // instead of bubbling to the panel's list-nav handling.
        if self.autocomplete.is_open() {
            let snapshot = self.autocomplete_snapshot();
            match self.autocomplete.handle_key(*key, &snapshot) {
                HandleKeyOutcome::Accepted(action) => {
                    self.query_input.replace_range_bytes(
                        action.range.clone(),
                        &action.new_text,
                        action.new_cursor_byte,
                    );
                    self.saved_search_name = None;
                    self.run_query(tx.clone());
                    return EventState::Consumed;
                }
                HandleKeyOutcome::Dismissed | HandleKeyOutcome::Consumed => {
                    return EventState::Consumed;
                }
                HandleKeyOutcome::NotHandled => {}
            }
        }

        // Check for action shortcuts first.
        if let Some(combo) = key_event_to_combo(key) {
            match self.key_bindings.get_action(&combo) {
                Some(ActionShortcuts::CycleSortField) => {
                    self.sort_field = self.sort_field.cycle();
                    self.apply_sort();
                    self.expand_states = vec![ExpandState::Collapsed; self.entries.len()];
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SortReverseOrder) => {
                    self.sort_order = self.sort_order.toggle();
                    self.apply_sort();
                    self.expand_states = vec![ExpandState::Collapsed; self.entries.len()];
                    return EventState::Consumed;
                }
                // FocusSidebar / FocusEditor are intercepted at the
                // EditorScreen level for directional navigation.
                Some(ActionShortcuts::FollowLink) => {
                    if let Some(path) = self.selected_path().cloned() {
                        tx.send(AppEvent::OpenPath(path)).ok();
                    }
                    return EventState::Consumed;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Up => {
                if self.is_full_expanded() {
                    self.content_scroll = self.content_scroll.saturating_sub(1);
                } else {
                    self.move_selection(-1);
                }
                EventState::Consumed
            }
            KeyCode::Down => {
                if self.is_full_expanded() {
                    // Increment freely; render() clamps to content_scroll_max.
                    self.content_scroll += 1;
                } else {
                    self.move_selection(1);
                }
                EventState::Consumed
            }
            KeyCode::Enter => {
                self.toggle_expand();
                EventState::Consumed
            }
            KeyCode::Esc => {
                // If the popup is open it was already handled above; here
                // Esc bubbles to the editor for focus changes.
                EventState::NotConsumed
            }
            _ => {
                // Forward to the query input (text editing).
                let outcome = self.query_input.handle_key(key);
                let snapshot = self.autocomplete_snapshot();
                match outcome {
                    InputOutcome::Changed => {
                        // Manual edit drops the saved-search label.
                        self.saved_search_name = None;
                        self.autocomplete.sync(&snapshot);
                        self.run_query(tx.clone());
                        EventState::Consumed
                    }
                    InputOutcome::Consumed => {
                        self.autocomplete.refresh_if_open(&snapshot);
                        EventState::Consumed
                    }
                    // Submit (Enter) / Cancel (Esc) are handled by the
                    // explicit arms above and never reach here. Anything
                    // unrecognised bubbles to the editor.
                    InputOutcome::Submit | InputOutcome::Cancel => {
                        self.autocomplete.close();
                        EventState::NotConsumed
                    }
                    InputOutcome::NotConsumed => EventState::NotConsumed,
                }
            }
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.entries.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, self.entries.len() as i32 - 1) as usize;
        self.list_state.select(Some(next));
    }

    fn toggle_expand(&mut self) {
        let Some(idx) = self.list_state.selected() else {
            return;
        };
        if idx >= self.expand_states.len() {
            return;
        }

        match self.expand_states[idx] {
            ExpandState::Collapsed => {
                self.expand_states[idx] = ExpandState::Context;
            }
            ExpandState::Context => {
                self.content_scroll = 0;
                self.expand_states[idx] = ExpandState::Full;
            }
            ExpandState::Full => {
                self.content_scroll = 0;
                self.expand_states[idx] = ExpandState::Collapsed;
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
        self.autocomplete.poll_results();

        let border_style = theme.border_style(focused);
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let sort_indicator = format!("{}{}", self.sort_field.label(), self.sort_order.label());
        let title = if self.active_query() == DEFAULT_QUERY {
            format!("Backlinks ({}) {}", self.entries.len(), sort_indicator)
        } else if let Some(name) = &self.saved_search_name {
            format!("{} ({}) {}", name, self.entries.len(), sort_indicator)
        } else {
            format!("Query ({}) {}", self.entries.len(), sort_indicator)
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
        self.query_input
            .render(f, search_inner, Style::default().fg(fg).bg(bg), 0, focused);

        let inner = rows[1];

        if self.loading {
            f.render_widget(
                Paragraph::new("  Loading...").style(Style::default().fg(fg_muted).bg(bg)),
                inner,
            );
            self.render_autocomplete_popup(f, rect, theme);
            return;
        }

        if self.entries.is_empty() {
            f.render_widget(
                Paragraph::new("  No results").style(Style::default().fg(fg_muted).bg(bg)),
                inner,
            );
            self.render_autocomplete_popup(f, rect, theme);
            return;
        }

        let selected = self.list_state.selected();
        let selected_state = selected
            .and_then(|i| self.expand_states.get(i).copied())
            .unwrap_or(ExpandState::Collapsed);

        // Full mode: content takes the entire panel, no list visible.
        if selected_state == ExpandState::Full {
            if let Some(idx) = selected
                && let Some(entry) = self.entries.get(idx)
            {
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
                let needles = query_needles(&resolve_query(
                    self.active_query(),
                    Some(&self.current_note),
                ));

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
            self.render_autocomplete_popup(f, rect, theme);
            return;
        }

        // Context or Collapsed: show the list, optionally with preview below.
        let has_context = selected_state == ExpandState::Context;

        let (list_area, divider_area, content_area) = if has_context {
            let max_list = inner.height / 2;
            let list_height = (self.entries.len() as u16).min(max_list).max(1);
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

        // Build list items (1 line per entry).
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let is_selected = selected == Some(i);
                let title_style = if is_selected {
                    Style::default()
                        .fg(theme.fg_selected.to_ratatui())
                        .bg(theme.bg_selected.to_ratatui())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(fg).bg(bg)
                };
                let title_display = if entry.title.is_empty() {
                    &entry.filename
                } else {
                    &entry.title
                };
                let expand_marker = match self.expand_states.get(i) {
                    Some(ExpandState::Context) => "\u{25BC}",
                    _ => " ",
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} {} ", expand_marker, title_display), title_style),
                    Span::styled(
                        format!(" {}", entry.filename),
                        Style::default().fg(fg_muted).bg(if is_selected {
                            theme.bg_selected.to_ratatui()
                        } else {
                            bg
                        }),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items)
            .style(Style::default().bg(bg))
            .highlight_style(Style::default().bg(theme.bg_selected.to_ratatui()));

        f.render_stateful_widget(list, list_area, &mut self.list_state);

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
            && let Some(idx) = selected
            && let Some(entry) = self.entries.get(idx)
            && selected_state == ExpandState::Context
        {
            let text = entry.full_text.as_deref().unwrap_or(&entry.context);
            let indent = 2usize;
            let wrap_width = area.width.saturating_sub(indent as u16 + 1) as usize;
            let needles = query_needles(&resolve_query(
                self.active_query(),
                Some(&self.current_note),
            ));

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

        self.render_autocomplete_popup(f, rect, theme);
    }

    /// Re-anchor the autocomplete popup on the query input's freshly
    /// rendered caret and render it, clamped to the panel rect so it never
    /// spills past the panel border.
    fn render_autocomplete_popup(&mut self, f: &mut Frame, rect: Rect, theme: &Theme) {
        let live_anchor = self.query_input.last_caret_pos();
        if let (Some(state), Some(anchor)) = (self.autocomplete.state_mut(), live_anchor) {
            state.anchor = anchor;
        }
        if let Some(state) = self.autocomplete.state() {
            autocomplete::render(f, state, rect, theme);
        }
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

    fn fake_entry(name: &str) -> BacklinkEntry {
        BacklinkEntry {
            path: VaultPath::note_path_from(name),
            title: name.to_string(),
            filename: format!("{name}.md"),
            context: String::new(),
            full_text: Some(String::new()),
        }
    }

    /// A static query (no `{note}`) must survive navigation: `set_note` leaves
    /// its results and query text untouched (no clear, no re-run).
    #[tokio::test]
    async fn static_query_survives_navigation() {
        let vault = crate::test_support::temp_vault("nav-static").await;
        let kb = crate::settings::AppSettings::default().key_bindings.clone();
        let mut panel = QueryPanel::new(vault, kb);
        panel.set_active_query("#todo".to_string());
        panel.on_loaded(vec![fake_entry("a")]);
        assert_eq!(panel.entries.len(), 1);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_note(VaultPath::note_path_from("x.md"), tx);

        assert_eq!(panel.active_query(), "#todo"); // not reset to >{note}
        assert!(!panel.loading); // static query → no re-run
        assert_eq!(panel.entries.len(), 1); // results untouched
    }

    /// A `{note}` query re-runs on navigation: `set_note` kicks off a fresh
    /// search (clears results, sets loading).
    #[tokio::test]
    async fn note_variable_query_reruns_on_navigation() {
        let vault = crate::test_support::temp_vault("nav-var").await;
        let kb = crate::settings::AppSettings::default().key_bindings.clone();
        let mut panel = QueryPanel::new(vault, kb);
        panel.set_active_query(">{note}".to_string());
        panel.on_loaded(vec![fake_entry("a")]);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_note(VaultPath::note_path_from("x.md"), tx);

        assert!(panel.loading); // run_query started
    }
}
