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
use crate::components::preview_highlight;
use crate::components::query_vars::{QueryContext, query_has_variables, resolve_query};
use crate::components::saved_search_breadcrumb::SavedSearchBreadcrumb;
use crate::components::search_list::{
    Emit, KeyReaction, ResolvingRowSource, RowSource, SearchList, SearchMouse, SearchRow,
    Unresolvable, VaultSuggestions,
};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_combo::KeyCombo;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// The canonical backlinks query (`<` / `lk:`; `>` is forward links). The
/// panel no longer starts on it — the LINKS drawer owns backlinks — but any
/// spelling of it still titles the panel "Backlinks".
const DEFAULT_QUERY: &str = "<{note}";
/// The long-form spelling of [`DEFAULT_QUERY`] (`lk:` is the documented
/// synonym of `<`), recognized so it also reads as the default.
const DEFAULT_QUERY_LONG: &str = "lk:{note}";

/// True when `query` is the default backlinks query in any spelling: the
/// canonical `<{note}`, the bare `<` sugar, the long form `lk:` — with or
/// without an order directive. Drives the "Backlinks" title and the
/// breadcrumb's blank-query condition, so every synonym reads as the default.
fn is_default_query(query: &str) -> bool {
    let expanded = kimun_core::expand_bare_note_prefixes(
        &kimun_core::strip_order_directive(query),
        crate::components::query_vars::VAR_NOTE,
    );
    expanded == DEFAULT_QUERY || expanded == DEFAULT_QUERY_LONG
}

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
    fn to_list_item(&self, theme: &Theme, icons: &Icons, selected: bool) -> ListItem<'static> {
        let title_display = if self.title.is_empty() {
            &self.filename
        } else {
            &self.title
        };
        let title_style = if selected {
            Style::default()
                .fg(theme.selection_fg.to_ratatui())
                .bg(theme.selection_bg.to_ratatui())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.fg.to_ratatui())
                .bg(theme.bg_panel.to_ratatui())
        };
        crate::components::rich_row::RichRow::new(icons.note, title_display.clone())
            .title_style(title_style)
            .meta(self.filename.clone())
            .into_list_item(theme)
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

/// Row source for the Query panel. It receives an already-resolved query
/// string — [`ResolvingRowSource`] substitutes `{note}` and short-circuits the
/// purely-note-dependent-but-no-note case to an empty list ([`Unresolvable::Empty`])
/// before this source is asked to load. Result ordering comes from the query
/// string's order directive, applied by the vault DB — the source no longer
/// sorts in memory beyond the no-directive default.
struct BacklinkSource {
    vault: Arc<NoteVault>,
}

#[async_trait]
impl RowSource<BacklinkEntry> for BacklinkSource {
    async fn load(&self, query: &str, emit: Emit<BacklinkEntry>) {
        let mut entries = load_query(&self.vault, query).await;
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
// ContentScroll (private)
// ---------------------------------------------------------------------------

/// Scroll state shared by the expanded content views (Full mode and the
/// half-height Context preview). The offset is either *anchored* — the
/// Context render recomputes it from the first needle match each frame — or
/// user-owned after a scroll. Every transition (take-over, re-anchor, clamp)
/// lives here, so paths that should re-anchor have one decision point and
/// the offset is never out of range between events.
#[derive(Clone, Copy)]
struct ContentScroll {
    /// True while the render owns the offset (anchor on the first needle
    /// match). The first tick that actually moves the view flips it;
    /// re-anchoring events set it back.
    anchored: bool,
    /// The rendered scroll offset (first visible content line).
    offset: usize,
    /// Maximum offset, recorded by render from content/viewport size.
    max: usize,
}

impl ContentScroll {
    fn new() -> Self {
        Self {
            anchored: true,
            offset: 0,
            max: 0,
        }
    }

    /// Back to the top, offset handed back to the auto-anchor.
    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Re-arm the auto-anchor without touching the offset (the next anchored
    /// render overwrites it).
    fn re_anchor(&mut self) {
        self.anchored = true;
    }

    /// One wheel/key tick up, clamped at the top. Only a tick that moves the
    /// view takes the offset over from the anchor — a saturated no-op must
    /// not silently disarm it.
    fn scroll_up(&mut self) {
        if self.offset > 0 {
            self.offset -= 1;
            self.anchored = false;
        }
    }

    /// One wheel/key tick down, clamped at `max` at mutation time so the
    /// offset is never out of range. Same no-op rule as [`scroll_up`].
    ///
    /// [`scroll_up`]: Self::scroll_up
    fn scroll_down(&mut self) {
        if self.offset < self.max {
            self.offset += 1;
            self.anchored = false;
        }
    }

    /// Render-time sync: record the current max offset and clamp — a resize
    /// can shrink the content below the held offset.
    fn set_max(&mut self, max: usize) {
        self.max = max;
        self.offset = self.offset.min(max);
    }

    /// Render-time anchor: while anchored, place the offset (clamped). A
    /// user-owned offset is left alone.
    fn anchor_to(&mut self, offset: usize) {
        if self.anchored {
            self.offset = offset.min(self.max);
        }
    }
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
    /// Scroll state for the expanded content views (Full takes the whole
    /// panel; Context is the half-height preview below the list). See
    /// [`ContentScroll`] for the anchored/user-owned life cycle.
    scroll: ContentScroll,
    /// The full-expand header's screen area (the fixed title line), recorded
    /// each render so a click on it collapses the view, mirroring Enter.
    /// Empty whenever full mode is not on screen.
    full_header_rect: Rect,
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
    /// Memoised `is_default_query` result for `order_cache_query` — the title
    /// reads it every frame, and the helper allocates (strip + expand), so it
    /// is refreshed in the same query-changed gate as `order_cache`.
    is_default_cache: bool,
    /// Memoised highlight needles derived from the resolved query, plus the
    /// (query template, note) pair they were computed from. The expand/context
    /// preview branches of `render` read needles every frame; recomputing them
    /// means resolving the template and a full query parse, so they are cached
    /// like `order_cache` and refreshed only when a key changes.
    needles_cache: Vec<String>,
    needles_cache_key: (String, VaultPath),
}

impl QueryPanel {
    pub fn new(vault: Arc<NoteVault>, key_bindings: KeyBindings, icons: Icons) -> Self {
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
        // Resolve `{note}` against the shared (live) current note at load time;
        // a purely note-dependent query with no note open yet shows nothing
        // (the panel has no recent-notes fallback). See [`ResolvingRowSource`].
        let source = ResolvingRowSource::new(
            Arc::new(BacklinkSource {
                vault: vault.clone(),
            }),
            {
                let note = current_note.clone();
                move || QueryContext::with_note(Some(note.lock().unwrap().clone()))
            },
            Unresolvable::Empty,
        );
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
            .highlight_query()
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
            scroll: ContentScroll::new(),
            full_header_rect: Rect::default(),
            key_bindings,
            redraw_tx,
            follow_link_combos,
            // An empty query carries no order directive → (Name, Ascending).
            order_cache: (SortField::Name, SortOrder::Ascending),
            order_cache_query: String::new(),
            // The panel starts empty; the first render's query-changed gate
            // recomputes this anyway.
            is_default_cache: false,
            needles_cache: Vec::new(),
            needles_cache_key: (String::new(), VaultPath::empty()),
        }
    }

    // ── Query accessors ─────────────────────────────────────────────────

    pub fn active_query(&self) -> &str {
        self.list.query()
    }

    /// The emphasis payload an open from this panel carries: the resolved
    /// query's needles (spec §5.1) — resolved, not the template, so `{note}`
    /// never leaks.
    fn emphasis(&self) -> Option<Vec<String>> {
        let resolved = resolve_query(self.list.query(), &self.query_ctx());
        let needles = crate::components::query_highlight::emphasis_needles(&resolved);
        (!needles.is_empty()).then_some(needles)
    }

    /// Number of results currently listed — the status bar's match count.
    pub fn result_count(&self) -> usize {
        self.list.match_count()
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

    /// The saved-search name the active query came from (breadcrumb
    /// provenance, no edited marker), or `None`. Pre-fills the save-search
    /// dialog's name field.
    pub fn saved_search_name(&self) -> Option<&str> {
        self.saved_search.name()
    }

    /// Re-pin the breadcrumb to a just-saved search: the saved identity is
    /// the provenance from now on, so the edited marker drops on an update
    /// and the name switches on a save-as-new.
    pub fn repin_saved_search(&mut self, name: String, query: &str) {
        self.saved_search.set(Some(name), query);
    }

    /// `true` when the live query carries no saved-search provenance worth
    /// showing — an empty field, or the default backlinks query (which the
    /// panel title already renders as "Backlinks", so a breadcrumb there would
    /// contradict it). Drives the breadcrumb's clear condition.
    fn query_is_blank(&self) -> bool {
        let q = self.list.query();
        q.trim().is_empty() || is_default_query(q)
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

    /// The query-resolution context for this panel: the open note. Mirrors what
    /// the panel's [`ResolvingRowSource`] reads at load time, so the panel's own
    /// `{note}` resolutions (emphasis, needles) match the loaded results.
    fn query_ctx(&self) -> QueryContext {
        QueryContext::with_note(Some(self.current_note()))
    }

    /// Fill the shared redraw slot so the engine's async loads / autocomplete
    /// wake the render loop. Idempotent.
    fn ensure_redraw_tx(&self, tx: &AppTx) {
        let mut slot = self.redraw_tx.lock().unwrap();
        if slot.is_none() {
            *slot = Some(tx.clone());
        }
    }

    /// The highlight needles for the active query, memoised on the
    /// (query template, current note) pair — `render` reads these every frame
    /// while a preview is open, and deriving them costs a template resolution
    /// plus a full query parse.
    fn cached_needles(&mut self) -> &[String] {
        let note = self.current_note();
        if self.needles_cache_key.0 != self.list.query() || self.needles_cache_key.1 != note {
            let resolved = resolve_query(self.list.query(), &self.query_ctx());
            self.needles_cache = query_needles(&resolved);
            self.needles_cache_key = (self.list.query().to_string(), note);
        }
        &self.needles_cache
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

    /// Drop the engine's content sub-region and the full-expand header rect.
    /// Every path that changes the expand state calls this: the recorded
    /// regions describe the PREVIOUS frame's content view, and the event
    /// loop drains queued events between renders — a mouse event arriving in
    /// the same batch as the state change must not be routed against a rect
    /// that no longer matches what is on screen.
    fn clear_content_regions(&mut self) {
        self.list.set_content_rect(Rect::default());
        self.full_header_rect = Rect::default();
    }

    fn reset_expand(&mut self) {
        self.expand = ExpandState::Collapsed;
        self.expand_path = None;
        self.scroll.reset();
        self.clear_content_regions();
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
            self.scroll.reset();
            self.clear_content_regions();
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
        // Ctrl+Enter opens the selected note (kitty-protocol terminals; the
        // FollowLink combo below is the always-works path). Pre-checked here
        // because Enter-with-modifiers never participates in the engine's
        // autocomplete/Submit flow.
        if key.code == KeyCode::Enter
            && key
                .modifiers
                .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
        {
            if let Some(path) = self.selected_path().cloned() {
                tx.send(AppEvent::OpenPath {
                    path,
                    emphasis: self.emphasis(),
                })
                .ok();
            }
            return EventState::Consumed;
        }
        // NOTE: plain Enter is NOT pre-checked here. It must reach the engine
        // so an open autocomplete popup can accept on Enter; only when the
        // popup is closed does the engine return `Submit`, which toggles
        // expand below.
        let prev_query = self.list.query().to_string();
        match self.list.handle_key(key) {
            KeyReaction::Intercepted(c) if self.follow_link_combos.contains(&c) => {
                if let Some(path) = self.selected_path().cloned() {
                    tx.send(AppEvent::OpenPath {
                        path,
                        emphasis: self.emphasis(),
                    })
                    .ok();
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
                // A query edit moves the needle highlights, so the preview
                // scroll goes back to the link auto-anchor — a user scroll
                // position is stale against the new matches. (Programmatic
                // query changes re-arm via `reset_expand`.)
                if self.list.query() != prev_query {
                    self.scroll.re_anchor();
                }
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

    /// Mouse behavior: the wheel scrolls — the result list (viewport moves,
    /// selection keeps its screen position), the half-height Context preview
    /// when hovering over it, or, in full-expand, the content — anywhere
    /// within the panel; clicks select/activate list rows (a second click on
    /// the selected row cycles its expand state, mirroring Enter). The engine
    /// owns the wheel routing: render records the content view (preview or
    /// full) as its content sub-region, which wins over the panel bounds and
    /// comes back as `ContentScroll*`.
    pub fn handle_mouse(
        &mut self,
        mouse: &ratatui::crossterm::event::MouseEvent,
        tx: &AppTx,
    ) -> EventState {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        use ratatui::layout::Position;
        self.ensure_redraw_tx(tx);
        // Read BEFORE the sync: a selection that vanished in this same event
        // batch collapses the expand state, but the screen still shows the
        // full view — the event must be handled against what the user saw,
        // not let through to the engine's stale list rect.
        let was_full = self.is_full_expanded();
        self.sync_expand_anchor();
        // In full-expand the list is not rendered (its recorded rect is
        // stale, from the last non-full frame), so only the wheel may reach
        // the engine — it routes via the content rect, which covers the
        // whole panel in full-expand. Everything else is the panel's;
        // closing the popup here keeps the any-mouse-interaction-dismisses
        // rule for events the engine never sees.
        if was_full {
            match mouse.kind {
                // Fall through to the engine below.
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {}
                // A click on the header collapses the view, mirroring Enter.
                // (A sync collapse above already cleared the header rect, so
                // this cannot toggle a no-longer-full view.)
                MouseEventKind::Down(MouseButton::Left)
                    if self.full_header_rect.contains(Position {
                        x: mouse.column,
                        y: mouse.row,
                    }) =>
                {
                    self.list.close_autocomplete();
                    self.toggle_expand();
                    return EventState::Consumed;
                }
                _ => {
                    self.list.close_autocomplete();
                    return EventState::Consumed;
                }
            }
        }
        match self.list.handle_mouse(mouse) {
            SearchMouse::ContentScrollUp => {
                self.scroll.scroll_up();
                EventState::Consumed
            }
            SearchMouse::ContentScrollDown => {
                self.scroll.scroll_down();
                EventState::Consumed
            }
            SearchMouse::Activated(_) => {
                self.toggle_expand();
                EventState::Consumed
            }
            // Right-click on a result row → file/note context menu (spec §10).
            SearchMouse::Context(_) => {
                if let Some(path) = self.selected_path().cloned() {
                    tx.send(AppEvent::ShowFileOpsMenu(path)).ok();
                }
                EventState::Consumed
            }
            SearchMouse::Selected(_) | SearchMouse::Scrolled => {
                self.sync_expand_anchor();
                EventState::Consumed
            }
            SearchMouse::None => EventState::NotConsumed,
        }
    }

    fn scroll_content(&mut self, key: &KeyEvent) {
        match key.code {
            KeyCode::Up => self.scroll.scroll_up(),
            KeyCode::Down => self.scroll.scroll_down(),
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
                self.scroll.re_anchor();
            }
            ExpandState::Context => {
                self.scroll.reset();
                self.expand = ExpandState::Full;
            }
            ExpandState::Full => {
                self.scroll.reset();
                self.expand = ExpandState::Collapsed;
            }
        }
        self.clear_content_regions();
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        crate::components::hints::hints_for(
            &self.key_bindings,
            &[
                (ActionShortcuts::FocusSidebar, "\u{2190} editor"),
                (ActionShortcuts::FollowLink, "open note"),
                (ActionShortcuts::SaveCurrentQuery, "save query"),
                (ActionShortcuts::OpenSavedSearches, "searches"),
                (ActionShortcuts::OpenSortDialog, "sort"),
            ],
        )
    }

    // ── Rendering ──────────────────────────────────────────────────────

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.list.poll();
        self.sync_expand_anchor();
        // The whole panel is wheel-scrollable (query box and preview included);
        // recorded up front so it is fresh on every expand-state branch.
        self.list.set_panel_rect(rect);
        // Cleared every frame; only the branches that draw a content view
        // (Context preview, full-expand) record it, so the engine's wheel
        // routing never sees a stale sub-region from a frame where no
        // content view was drawn. Same life cycle for the full-expand header.
        self.list.set_content_rect(Rect::default());
        self.full_header_rect = Rect::default();

        let border_style = theme.border_style(focused);
        let gray = theme.gray.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let count = self.list.visible_rows().len();
        // Reparse the order only when the query changed (memoised) — render runs
        // every frame and `from_query_string` is a full allocating parse.
        if self.list.query() != self.order_cache_query {
            self.order_cache = self.current_order();
            self.is_default_cache = is_default_query(self.list.query());
            self.order_cache_query = self.list.query().to_string();
        }
        let (sort_field, sort_order) = self.order_cache;
        let sort_indicator = format!("{}{}", sort_field.label(), sort_order.label());
        // The saved-search name lives on the query searchbox border (the
        // breadcrumb below), not here, so the outer title stays generic.
        // `is_default_query` ignores the order directive and recognizes every
        // spelling of the default (`<{note}`, bare `<`, `lk:`), so sorting or
        // typing a synonym still reads as "Backlinks". Memoised above — the
        // helper allocates and this runs every frame.
        let title = if self.list.query().trim().is_empty() {
            "Find".to_string()
        } else if self.is_default_cache {
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
        let mut search_block = Block::default()
            .title(search_title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        // Parse problems surface as a second, red title segment — the input
        // itself never blocks (spec §9).
        if let Some(reason) = crate::components::query_highlight::error_reason(self.list.query()) {
            search_block = search_block.title(
                ratatui::text::Line::from(ratatui::text::Span::styled(
                    format!(" ⚠ {reason} "),
                    Style::default().fg(theme.red.to_ratatui()),
                ))
                .right_aligned(),
            );
        }
        let search_inner = search_block.inner(rows[0]);
        f.render_widget(search_block, rows[0]);
        self.list.render_query(f, search_inner, theme, focused);

        let inner = rows[1];

        if self.list.is_loading() {
            f.render_widget(
                Paragraph::new("  Loading...").style(Style::default().fg(gray).bg(bg)),
                inner,
            );
            self.list.render_autocomplete(f, rect, theme);
            return;
        }

        if self.list.visible_rows().is_empty() {
            f.render_widget(
                Paragraph::new("  No results").style(Style::default().fg(gray).bg(bg)),
                inner,
            );
            self.list.render_autocomplete(f, rect, theme);
            return;
        }

        let selected_state = self.expand;

        // Full mode: content takes the entire panel, no list visible. The
        // wheel scrolls the content from anywhere in the panel, so the whole
        // panel is the engine's content sub-region.
        if selected_state == ExpandState::Full {
            self.list.set_content_rect(rect);
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

                // Fixed title header. Clicking it collapses the view
                // (mirroring Enter) — record where it was drawn.
                self.full_header_rect = parts[0];
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!("\u{25BC} {} ", title_display),
                            Style::default()
                                .fg(theme.selection_fg.to_ratatui())
                                .bg(bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {}", entry.filename),
                            Style::default().fg(gray).bg(bg),
                        ),
                    ]))
                    .style(Style::default().bg(bg)),
                    parts[0],
                );

                // Fixed divider.
                f.render_widget(
                    Paragraph::new("\u{2500}".repeat(parts[1].width as usize))
                        .style(Style::default().fg(gray).bg(bg)),
                    parts[1],
                );

                // Scrollable content.
                let indent = 2usize;
                let wrap_width = parts[2].width.saturating_sub(indent as u16 + 1) as usize;
                let needles = self.cached_needles();

                let mut lines = Vec::new();
                for line in text.lines() {
                    let wrapped = preview_highlight::wrap_line(line, wrap_width);
                    for wline in wrapped {
                        let spans = highlight_needles(&wline, needles, gray, bg, theme);
                        let mut indented =
                            vec![Span::styled(" ".repeat(indent), Style::default().bg(bg))];
                        indented.extend(spans);
                        lines.push(Line::from(indented));
                    }
                }

                let total_lines = lines.len();
                let viewport = parts[2].height as usize;
                self.scroll.set_max(total_lines.saturating_sub(viewport));

                f.render_widget(
                    Paragraph::new(lines)
                        .scroll((self.scroll.offset as u16, 0))
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
        if self.list.query().trim().is_empty() {
            // Empty-state: a short query-syntax primer instead of a blank
            // list (spec §9 discoverability; the panel no longer pre-fills
            // a backlinks query — the LINKS drawer owns those).
            let dim = Style::default().fg(theme.gray.to_ratatui());
            let key = Style::default().fg(theme.yellow.to_ratatui());
            let lines = vec![
                ratatui::text::Line::from(Span::styled("type to search the vault", dim)),
                ratatui::text::Line::default(),
                ratatui::text::Line::from(vec![
                    Span::styled(" #tag      ", key),
                    Span::styled("label", dim),
                ]),
                ratatui::text::Line::from(vec![
                    Span::styled(" <  >      ", key),
                    Span::styled("backlinks · links", dim),
                ]),
                ratatui::text::Line::from(vec![
                    Span::styled(" \"phrase\"  ", key),
                    Span::styled("exact match", dim),
                ]),
                ratatui::text::Line::from(vec![
                    Span::styled(" =date     ", key),
                    Span::styled("modified", dim),
                ]),
                ratatui::text::Line::from(vec![
                    Span::styled(" ?name     ", key),
                    Span::styled("saved search", dim),
                ]),
            ];
            f.render_widget(ratatui::widgets::Paragraph::new(lines), list_area);
        } else {
            self.list.render(f, list_area, theme, focused);
        }
        self.list.set_list_rect(list_area);

        // Divider between list and content.
        if let Some(div) = divider_area {
            f.render_widget(
                Paragraph::new("\u{2500}".repeat(div.width as usize))
                    .style(Style::default().fg(gray).bg(bg)),
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
            // Copied out before `cached_needles` borrows self; gates the
            // link-line scan below, whose result is only consumed while the
            // anchor owns the offset.
            let anchored = self.scroll.anchored;
            let needles = self.cached_needles();

            let mut lines = Vec::new();

            // Track which rendered line contains the first needle match —
            // only while anchored: a user-owned scroll never reads it, so
            // the per-line needle scan would be wasted work.
            let mut link_line: Option<usize> = None;

            for line in text.lines() {
                let wrapped = preview_highlight::wrap_line(line, wrap_width);
                for wline in wrapped {
                    if anchored
                        && link_line.is_none()
                        && !preview_highlight::match_ranges(&wline, needles).is_empty()
                    {
                        link_line = Some(lines.len());
                    }
                    let spans = highlight_needles(&wline, needles, gray, bg, theme);
                    let mut indented =
                        vec![Span::styled(" ".repeat(indent), Style::default().bg(bg))];
                    indented.extend(spans);
                    lines.push(Line::from(indented));
                }
            }

            // Anchor scroll: show the link with context above. If the content
            // from the link to the end fits within the viewport, scroll back
            // further to fill the available space. A user-owned offset is
            // left where it is (anchor_to is a no-op), just clamped by
            // set_max.
            let viewport = area.height as usize;
            let total = lines.len();
            self.scroll.set_max(total.saturating_sub(viewport));
            let link_pos = link_line.unwrap_or(0);
            let lines_after_link = total.saturating_sub(link_pos);
            self.scroll.anchor_to(if lines_after_link <= viewport {
                // Content from link to end fits — scroll back to fill the
                // viewport.
                self.scroll.max
            } else {
                // More content below the link — show 2 lines of context
                // above.
                link_pos.saturating_sub(2)
            });

            f.render_widget(
                Paragraph::new(lines)
                    .scroll((self.scroll.offset as u16, 0))
                    .style(Style::default().bg(bg)),
                area,
            );
            // The preview is the engine's content sub-region: wheel events
            // inside it come back as ContentScroll* instead of moving the
            // list.
            self.list.set_content_rect(area);
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

/// Highlight every occurrence of any needle in `line` (bold accent), the rest
/// muted. Matching is byte-safe via [`preview_highlight::match_ranges`].
fn highlight_needles(
    line: &str,
    needles: &[String],
    gray: ratatui::style::Color,
    bg: ratatui::style::Color,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let normal = Style::default().fg(gray).bg(bg);
    let bold = Style::default()
        .fg(theme.accent.to_ratatui())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let ranges = preview_highlight::match_ranges(line, needles);
    if ranges.is_empty() {
        return vec![Span::styled(line.to_string(), normal)];
    }
    let mut spans = Vec::new();
    let mut pos = 0;
    for (start, end) in ranges {
        if start > pos {
            spans.push(Span::styled(line[pos..start].to_string(), normal));
        }
        spans.push(Span::styled(line[start..end].to_string(), bold));
        pos = end;
    }
    if pos < line.len() {
        spans.push(Span::styled(line[pos..].to_string(), normal));
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
    fn extract_context_matches_any_needle() {
        let text = "# Title\n\nIntro line.\n\nA paragraph mentioning widget here.\n";
        let result = extract_context_multi(text, &["widget".to_string()]);
        assert!(result.contains("widget"));
    }

    #[test]
    fn highlight_needles_bolds_every_match() {
        let spans = highlight_needles(
            "see widget and gadget",
            &["widget".to_string(), "gadget".to_string()],
            ratatui::style::Color::Gray,
            ratatui::style::Color::Black,
            &crate::settings::themes::Theme::default(),
        );
        let bolded: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .map(|s| s.content.as_ref())
            .collect();
        // Both needles are bolded, not just the earliest one.
        assert!(bolded.contains(&"widget"), "widget should be bold: {bolded:?}");
        assert!(
            spans
                .iter()
                .any(|s| s.content.contains("gadget")
                    && s.style.add_modifier.contains(Modifier::BOLD))
        );
    }

    #[test]
    fn default_query_recognized_in_all_spellings() {
        // Bare `<` and the long form are first-class synonyms of the default
        // backlinks query: the panel title must read "Backlinks" and the
        // breadcrumb clear condition must treat them as blank.
        assert!(is_default_query(DEFAULT_QUERY));
        assert!(is_default_query("<"));
        assert!(is_default_query("lk:"));
        assert!(is_default_query("< or:title"));
        assert!(is_default_query("<{note} -or:file"));
        assert!(!is_default_query("<projects"));
        assert!(!is_default_query(">"));
        assert!(!is_default_query(""));
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
        QueryPanel::new(vault, kb, Icons::new(false))
    }

    /// Ctrl+Enter opens the selected result (kitty-protocol terminals) —
    /// regression: it must not fall through to the engine as a plain key.
    #[tokio::test(flavor = "multi_thread")]
    async fn ctrl_enter_opens_selected_result() {
        let vault = crate::test_support::temp_vault("qp-ctrl-enter").await;
        vault.validate_and_init().await.unwrap();
        vault
            .save_note(&VaultPath::note_path_from("target"), "the note body")
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Query for the note and let the async load land.
        panel.apply_query("target".to_string(), None, tx.clone());
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            panel.list.poll();
        }
        assert!(
            panel.selected_path().is_some(),
            "result loaded and selected"
        );

        panel.handle_key(
            &KeyEvent::new(
                KeyCode::Enter,
                ratatui::crossterm::event::KeyModifiers::CONTROL,
            ),
            &tx,
        );

        let mut opened = None;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::OpenPath { path, .. } = ev {
                opened = Some(path);
            }
        }
        assert_eq!(opened, Some(VaultPath::note_path_from("target")));
    }

    /// The memoised highlight needles must follow both cache keys: recompute
    /// when the current note changes and when the query template changes.
    #[tokio::test]
    async fn cached_needles_track_query_and_note() {
        let vault = crate::test_support::temp_vault("qp_needles").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        panel.list.set_query(DEFAULT_QUERY);

        // Backlinks query `<{note}` resolved against "spec".
        *panel.current_note.lock().unwrap() = VaultPath::note_path_from("spec");
        assert!(panel.cached_needles().iter().any(|n| n == "spec"));

        // Note change invalidates.
        *panel.current_note.lock().unwrap() = VaultPath::note_path_from("other");
        assert!(panel.cached_needles().iter().any(|n| n == "other"));

        // Query change invalidates.
        panel.list.set_query("widget".to_string());
        let needles = panel.cached_needles();
        assert!(needles.iter().any(|n| n == "widget"));
        assert!(!needles.iter().any(|n| n == "other"));
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

    /// Applying a sort rewrites the query's order directive — the breadcrumb
    /// stays sticky but gains the edited marker, because the stored query is
    /// saved verbatim and any text divergence counts as an edit.
    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_marks_saved_search_breadcrumb_edited() {
        let vault = crate::test_support::temp_vault("qp-sort-name").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.apply_query("#todo".to_string(), Some("todo".to_string()), tx.clone());

        panel.apply_sort(SortField::Title, SortOrder::Ascending, &tx);
        assert_eq!(panel.active_query(), "#todo or:title");
        assert_eq!(
            panel.saved_search_breadcrumb().as_deref(),
            Some("todo • edited"),
            "sorting diverges from the stored query, so the breadcrumb is edited"
        );
    }

    /// Saving the live query re-pins the breadcrumb to the saved identity:
    /// the edited marker drops, and a save-as-new switches the name.
    #[tokio::test(flavor = "multi_thread")]
    async fn repin_after_save_adopts_saved_identity() {
        let vault = crate::test_support::temp_vault("qp-repin").await;
        vault.validate_and_init().await.unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.apply_query("#todo".to_string(), Some("todo".to_string()), tx.clone());

        panel.set_active_query("#todo and #urgent".to_string());
        assert_eq!(
            panel.saved_search_breadcrumb().as_deref(),
            Some("todo • edited")
        );

        panel.repin_saved_search("urgent-todos".to_string(), "#todo and #urgent");
        assert_eq!(
            panel.saved_search_breadcrumb().as_deref(),
            Some("urgent-todos"),
            "after a save the saved identity is the provenance — no edited marker"
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

    /// The wheel over the half-height Context preview scrolls the preview
    /// text (taking over from the link auto-anchor); over the list it keeps
    /// scrolling the list and leaves the preview's scroll untouched.
    #[tokio::test(flavor = "multi_thread")]
    async fn context_preview_wheel_scrolls_preview_not_list() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

        let vault = crate::test_support::temp_vault("qp-preview-wheel").await;
        vault.validate_and_init().await.unwrap();
        // Long note so the preview content overflows its half-height viewport;
        // the needle on the first line anchors the auto-scroll at 0.
        let mut body = String::from("#todo first line\n");
        for i in 0..40 {
            body.push_str(&format!("line {}\n", i));
        }
        vault
            .create_note(&VaultPath::note_path_from("/long.md"), &body)
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("#todo".to_string());
        settle(&mut panel).await;
        assert!(panel.list.selected_row().is_some());

        // Open the half-height Context preview and render once to record the
        // list/preview rects.
        panel.toggle_expand();
        assert!(panel.expand == ExpandState::Context);
        let theme = crate::settings::themes::Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();
        terminal
            .draw(|f| panel.render(f, f.area(), &theme, true))
            .unwrap();
        let preview = panel.list.content_rect();
        assert!(!preview.is_empty(), "preview rect recorded");
        assert_eq!(panel.scroll.offset, 0, "auto-anchor at the top needle");
        assert!(panel.scroll.max > 0, "content overflows viewport");

        let wheel = move |y: u16| MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: preview.x + 1,
            row: y,
            modifiers: KeyModifiers::NONE,
        };

        // Wheel over the LIST area: list scroll path, preview untouched.
        let over_list = wheel(preview.y.saturating_sub(3));
        panel.handle_mouse(&over_list, &tx);
        assert_eq!(panel.scroll.offset, 0, "list wheel must not move preview");
        assert!(panel.scroll.anchored, "anchor stays armed");

        // Wheel over the PREVIEW area: preview scrolls, anchor hands over.
        let over_preview = wheel(preview.y + 1);
        panel.handle_mouse(&over_preview, &tx);
        assert_eq!(panel.scroll.offset, 1, "preview wheel scrolls content");
        assert!(!panel.scroll.anchored, "user owns the scroll now");

        // Re-render keeps the user position (no re-anchor) and clamps.
        terminal
            .draw(|f| panel.render(f, f.area(), &theme, true))
            .unwrap();
        assert_eq!(panel.scroll.offset, 1);

        // Scrolling up past the top saturates at 0.
        let up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: preview.x + 1,
            row: preview.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        panel.handle_mouse(&up, &tx);
        panel.handle_mouse(&up, &tx);
        assert_eq!(panel.scroll.offset, 0);
    }

    /// A wheel tick that cannot move the preview (content fits the viewport,
    /// or already at the top) is a no-op and must NOT disarm the link
    /// auto-anchor.
    #[tokio::test(flavor = "multi_thread")]
    async fn noop_preview_wheel_keeps_autoscroll_armed() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

        let vault = crate::test_support::temp_vault("qp-noop-wheel").await;
        vault.validate_and_init().await.unwrap();
        // Short note: the preview content fits the half-height viewport.
        vault
            .create_note(&VaultPath::note_path_from("/short.md"), "#todo only line")
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("#todo".to_string());
        settle(&mut panel).await;
        panel.toggle_expand();
        let theme = crate::settings::themes::Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();
        terminal
            .draw(|f| panel.render(f, f.area(), &theme, true))
            .unwrap();
        assert_eq!(panel.scroll.max, 0, "content fits the viewport");

        let preview = panel.list.content_rect();
        let down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: preview.x + 1,
            row: preview.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        panel.handle_mouse(&down, &tx);
        assert!(
            panel.scroll.anchored,
            "no-op wheel tick must not disarm the auto-anchor"
        );
    }

    /// Editing the query by keystroke moves the needle highlights, so a
    /// wheel-scrolled Context preview hands the scroll back to the
    /// auto-anchor.
    #[tokio::test(flavor = "multi_thread")]
    async fn query_keystroke_rearms_preview_autoscroll() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let vault = crate::test_support::temp_vault("qp-rearm").await;
        vault.validate_and_init().await.unwrap();
        let mut body = String::from("#todo first line\n");
        for i in 0..40 {
            body.push_str(&format!("line {}\n", i));
        }
        vault
            .create_note(&VaultPath::note_path_from("/long.md"), &body)
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("#todo".to_string());
        settle(&mut panel).await;
        panel.toggle_expand();
        // Simulate a user-owned scroll.
        panel.scroll.anchored = false;

        panel.handle_key(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), &tx);
        assert_eq!(panel.active_query(), "#todox");
        assert!(
            panel.scroll.anchored,
            "a query edit must re-arm the preview auto-anchor"
        );
    }

    /// A wheel over the Context preview consumes the event without reaching
    /// the engine, but must still dismiss an open autocomplete popup (the
    /// any-mouse-interaction-dismisses rule).
    #[tokio::test(flavor = "multi_thread")]
    async fn preview_wheel_closes_autocomplete_popup() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
        };

        let vault = crate::test_support::temp_vault("qp-wheel-popup").await;
        vault.validate_and_init().await.unwrap();
        let mut body = String::from("#todo first line\n");
        for i in 0..40 {
            body.push_str(&format!("line {}\n", i));
        }
        vault
            .create_note(&VaultPath::note_path_from("/long.md"), &body)
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("#todo".to_string());
        settle(&mut panel).await;
        panel.toggle_expand();
        let theme = crate::settings::themes::Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();
        terminal
            .draw(|f| panel.render(f, f.area(), &theme, true))
            .unwrap();
        let preview = panel.list.content_rect();
        assert!(!preview.is_empty());

        // Type ` #` to open the hashtag autocomplete popup (the note's #todo
        // tag is a suggestion), draining the async suggestion load.
        for ch in [' ', '#'] {
            panel.handle_key(&KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), &tx);
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                panel.list.poll();
            }
        }
        assert!(panel.list.autocomplete_is_open(), "popup open after `#`");

        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: preview.x + 1,
            row: preview.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        panel.handle_mouse(&wheel, &tx);
        assert!(
            !panel.list.autocomplete_is_open(),
            "wheel over the preview must dismiss the popup"
        );
    }

    /// In full-expand, a click on the fixed title header collapses the view
    /// (mirroring Enter); clicks elsewhere are swallowed (the list under the
    /// content is not rendered).
    #[tokio::test(flavor = "multi_thread")]
    async fn full_expand_header_click_collapses() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let vault = crate::test_support::temp_vault("qp-header-click").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/long.md"), "#todo body")
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("#todo".to_string());
        settle(&mut panel).await;
        // Collapsed -> Context -> Full.
        panel.toggle_expand();
        panel.toggle_expand();
        assert!(panel.is_full_expanded());
        let theme = crate::settings::themes::Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();
        terminal
            .draw(|f| panel.render(f, f.area(), &theme, true))
            .unwrap();
        let header = panel.full_header_rect;
        assert!(!header.is_empty(), "header rect recorded in full mode");

        let click = |x: u16, y: u16| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };

        // A click below the header (over the content) is swallowed.
        panel.handle_mouse(&click(header.x + 1, header.y + 3), &tx);
        assert!(panel.is_full_expanded(), "content click must not collapse");

        // A click on the header collapses, like Enter.
        panel.handle_mouse(&click(header.x + 1, header.y), &tx);
        assert!(!panel.is_full_expanded());
        assert!(panel.expand == ExpandState::Collapsed);
    }

    /// Every expand-state change must drop the recorded content regions: the
    /// event loop drains queued events between renders, so a mouse event in
    /// the same batch as the toggle must not be routed against rects from
    /// the previous frame's content view.
    #[tokio::test(flavor = "multi_thread")]
    async fn toggling_expand_clears_stale_content_regions() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let vault = crate::test_support::temp_vault("qp-stale-regions").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/long.md"), "#todo body")
            .await
            .unwrap();
        let mut panel = make_panel(vault);
        panel.set_active_query("#todo".to_string());
        settle(&mut panel).await;
        let theme = crate::settings::themes::Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 30)).unwrap();

        // Render in Full so both regions are recorded.
        panel.toggle_expand();
        panel.toggle_expand();
        terminal
            .draw(|f| panel.render(f, f.area(), &theme, true))
            .unwrap();
        assert!(!panel.list.content_rect().is_empty());
        assert!(!panel.full_header_rect.is_empty());

        // Toggle (Full -> Collapsed) WITHOUT a render in between — as when
        // Enter and a mouse event are drained in the same batch.
        panel.toggle_expand();
        assert!(
            panel.list.content_rect().is_empty(),
            "stale content rect must not survive a state change"
        );
        assert!(
            panel.full_header_rect.is_empty(),
            "stale header rect must not survive a state change"
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
        // The panel starts empty (LINKS owns backlinks); type the backlinks
        // query to exercise the `{note}` re-resolution machinery.
        assert_eq!(panel.active_query(), "");
        panel.list.set_query(DEFAULT_QUERY);

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
        panel.list.set_query(DEFAULT_QUERY);
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
