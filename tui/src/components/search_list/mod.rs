//! `SearchList`: the one module behind every query-input-over-an-async-loaded
//! list surface in the TUI. See CONTEXT.md.

#[cfg(test)]
mod adapters;
mod host;
mod load;
mod seams;

pub use seams::{
    Emit, Filter, Loaded, RowSource, SearchRow, SuggestionItem, SuggestionSource, VaultSuggestions,
};

use crate::components::autocomplete::{
    AutocompleteController, AutocompleteMode, HandleKeyOutcome, TriggerOptions,
};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::keys::key_combo::KeyCombo;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;
use load::LoadEngine;
use ratatui::crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{List, ListItem, ListState},
};
use seams::Loaded as LoadedInner;
use std::sync::Arc;

fn fuzzy_indices<R: SearchRow>(rows: &[R], query: &str) -> Vec<usize> {
    use nucleo::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo::{Matcher, Utf32Str};
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let pat = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut scored: Vec<(usize, u32)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let hay = r.match_text()?;
            let mut buf = Vec::new();
            let h = Utf32Str::new(hay, &mut buf);
            pat.score(h, &mut matcher).map(|s| (i, s))
        })
        .collect();
    scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// Verdict returned by [`SearchList::handle_key`].
#[derive(Debug, PartialEq, Eq)]
pub enum KeyReaction {
    Consumed,
    Submit,
    Cancel,
    Intercepted(crate::keys::key_combo::KeyCombo),
    Unhandled,
}

pub struct SearchList<R: SearchRow> {
    source: Arc<dyn RowSource<R>>,
    rows: Vec<R>,
    /// Indices into `rows` in display order (after filtering/ranking).
    display: Vec<usize>,
    /// A synthetic, query-fresh, filter-exempt row pinned at visible position 0
    /// (the "Create: <q>" affordance / saved-searches virtual entry). Held
    /// separately from `rows` so it works regardless of delivery (one-shot
    /// `Replace` or streamed `Push`) and refreshes on every query change. See
    /// [`RowSource::leading_row`].
    leading: Option<R>,
    /// Index into the VISIBLE sequence `[leading?] ++ display` of the selected
    /// item.
    selected: Option<usize>,
    filter: Filter<R>,
    query: String,
    loader: LoadEngine<R>,
    input: SingleLineInput,
    autocomplete: Option<AutocompleteController>,
    /// Key combos the caller wants to intercept before the engine acts.
    intercept: Vec<KeyCombo>,
    icons: Icons,
    list_rect: Rect,
    /// Load generation whose rows are currently held. When a newer generation
    /// (a requery / reload) delivers its first event, `poll` clears the stale
    /// rows before applying it — required for streamed (`Push`) sources, which
    /// would otherwise append onto a superseded load's rows.
    applied_generation: u64,
}

/// Mouse interaction result from [`SearchList::handle_mouse`].
#[derive(Debug, PartialEq, Eq)]
pub enum SearchMouse {
    Selected(usize),
    Activated(usize),
    Scrolled,
    None,
}

pub struct SearchListBuilder<R: SearchRow> {
    source: Arc<dyn RowSource<R>>,
    redraw: Arc<dyn Fn() + Send + Sync>,
    initial_query: String,
    filter: Filter<R>,
    autocomplete: Option<(Arc<dyn SuggestionSource>, AutocompleteMode)>,
    intercept: Vec<KeyCombo>,
    icons: Icons,
    debounce: Option<std::time::Duration>,
}

impl<R: SearchRow> SearchList<R> {
    pub fn builder(
        source: impl RowSource<R>,
        redraw: Arc<dyn Fn() + Send + Sync>,
    ) -> SearchListBuilder<R> {
        SearchListBuilder {
            source: Arc::new(source),
            redraw,
            initial_query: String::new(),
            filter: Filter::SourceOrder,
            autocomplete: None,
            intercept: Vec::new(),
            icons: Icons::new(false),
            debounce: None,
        }
    }

    fn new(b: SearchListBuilder<R>) -> Self {
        let mut loader = LoadEngine::new(b.redraw.clone());
        loader.start(b.source.clone(), b.initial_query.clone());
        let input = SingleLineInput::with_value(&b.initial_query);
        let debounce = b.debounce;
        let autocomplete = b.autocomplete.map(|(suggestions, mode)| {
            let mut ac =
                AutocompleteController::new(suggestions, mode).with_trigger_opts(TriggerOptions {
                    disambiguate_header: false,
                    apply_exclusion_zone: false,
                });
            if let Some(d) = debounce {
                ac = ac.with_debounce(d);
            }
            ac.set_redraw_callback(b.redraw.clone());
            ac
        });
        Self {
            source: b.source,
            rows: Vec::new(),
            display: Vec::new(),
            leading: None,
            selected: None,
            filter: b.filter,
            query: b.initial_query,
            loader,
            input,
            autocomplete,
            intercept: b.intercept,
            icons: b.icons,
            list_rect: Rect::default(),
            applied_generation: 0,
        }
    }

    pub fn poll(&mut self) {
        let drained = self.loader.drain();
        if !drained.is_empty() {
            // A newer load delivered its first event(s): drop the prior load's
            // rows so a streamed source starts from a clean slate (one-shot
            // `Replace` overwrites anyway, but `Push` would otherwise append).
            let current_gen = self.loader.generation();
            if current_gen != self.applied_generation {
                self.rows.clear();
                self.selected = None;
                self.applied_generation = current_gen;
            }
        }
        for ev in drained {
            match ev {
                LoadedInner::Replace(rows) => {
                    self.rows = rows;
                }
                LoadedInner::Push(row) => {
                    self.rows.push(row);
                }
                LoadedInner::Done => {}
            }
        }
        self.recompute_display();
        if self.selected.is_none() && self.visible_len() > 0 {
            self.selected = Some(0);
        }
        if let Some(ac) = &mut self.autocomplete {
            ac.poll_results();
        }
    }

    /// Build a host snapshot from the current input state.
    /// Only reads `self.input` so the result can be stored in a local
    /// before taking `&mut self.autocomplete`, resolving the borrow conflict.
    fn autocomplete_snapshot(&self) -> host::SearchBoxHostSnapshot {
        let value = self.input.value().to_string();
        let cursor_byte = self.input.cursor_byte();
        let col = value[..cursor_byte.min(value.len())].chars().count();
        host::SearchBoxHostSnapshot {
            lines: vec![value],
            cursor: (0, col),
            caret_pos: self.input.last_caret_pos(),
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.visible_len();
        self.selected = if len == 0 {
            None
        } else {
            Some(self.selected.unwrap_or(0).min(len - 1))
        };
    }

    /// `1` when a leading row is pinned at visible position 0, else `0`.
    fn leading_offset(&self) -> usize {
        self.leading.is_some() as usize
    }

    /// Length of the visible sequence `[leading?] ++ display`.
    pub fn visible_len(&self) -> usize {
        self.leading_offset() + self.display.len()
    }

    /// Row at visible position `pos` in `[leading?] ++ display`.
    fn visible_row(&self, pos: usize) -> Option<&R> {
        if self.leading.is_some() && pos == 0 {
            self.leading.as_ref()
        } else {
            self.rows
                .get(*self.display.get(pos - self.leading_offset())?)
        }
    }

    /// The source-delivered rows only (NOT the leading row). Prefer
    /// [`visible_len`](Self::visible_len)/[`visible_rows`](Self::visible_rows)
    /// for visible counts.
    pub fn rows(&self) -> &[R] {
        &self.rows
    }

    pub fn selected_row(&self) -> Option<&R> {
        self.selected.and_then(|p| self.visible_row(p))
    }

    pub fn visible_rows(&self) -> Vec<&R> {
        (0..self.visible_len())
            .filter_map(|p| self.visible_row(p))
            .collect()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The visible text in the query input widget. Test-only: lets callers
    /// assert the input bar reflects a programmatic query change.
    #[cfg(test)]
    pub(crate) fn input_value(&self) -> &str {
        self.input.value()
    }
    pub fn is_loading(&self) -> bool {
        self.loader.loading
    }

    /// Set the query programmatically: updates the visible input widget (cursor
    /// to end) AND the query string, then starts a load (for `reload_on_query`
    /// sources) or recomputes the display. This is the setter every external
    /// caller wants — a saved search applied, a sort directive rewritten — so
    /// the input bar always reflects the query. The interactive keystroke path
    /// uses [`sync_query_from_input`](Self::sync_query_from_input) instead,
    /// because the input widget already holds the typed text (and its cursor
    /// must not jump back to the end on every keystroke).
    pub fn set_query(&mut self, q: impl Into<String>) {
        let q = q.into();
        self.input.set_value(q.clone());
        self.query = q;
        self.requery();
    }

    /// Pull the query string FROM the input widget without touching the widget
    /// (so the cursor stays put), then reload/recompute. The keystroke and
    /// autocomplete-accept paths use this after they have already mutated the
    /// input in place.
    fn sync_query_from_input(&mut self) {
        self.query = self.input.value().to_string();
        self.requery();
    }

    /// Start a fresh load for `reload_on_query` sources, else recompute the
    /// local display. The generation guard in `LoadEngine` drops stale results.
    fn requery(&mut self) {
        if self.source.reload_on_query() {
            self.loader.start(self.source.clone(), self.query.clone());
        } else {
            self.recompute_display();
        }
    }

    /// Re-run the source load for the current query (e.g. after a mutation).
    pub fn reload(&mut self) {
        self.loader.start(self.source.clone(), self.query.clone());
    }

    pub fn select_next(&mut self) {
        let n = self.visible_len();
        if n == 0 {
            return;
        }
        self.selected = Some(self.selected.map_or(0, |i| (i + 1).min(n - 1)));
    }

    pub fn select_prev(&mut self) {
        if self.visible_len() == 0 {
            return;
        }
        self.selected = Some(self.selected.map_or(0, |i| i.saturating_sub(1)));
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> KeyReaction {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        // Caller-registered intercepts get first crack — before autocomplete or
        // any built-in binding.
        if let Some(combo) = crate::keys::key_event_to_combo(key)
            && self.intercept.contains(&combo)
        {
            return KeyReaction::Intercepted(combo);
        }

        // Autocomplete popup gets first crack when open. Build snapshot before
        // taking &mut self.autocomplete to avoid borrow-checker conflict
        // (snapshot only reads self.input).
        if self.autocomplete.as_ref().is_some_and(|ac| ac.is_open()) {
            let snap = self.autocomplete_snapshot();
            if let Some(ac) = &mut self.autocomplete {
                match ac.handle_key(*key, &snap) {
                    HandleKeyOutcome::Accepted(action) => {
                        self.input.replace_range_bytes(
                            action.range.clone(),
                            &action.new_text,
                            action.new_cursor_byte,
                        );
                        self.sync_query_from_input();
                        return KeyReaction::Consumed;
                    }
                    HandleKeyOutcome::Dismissed | HandleKeyOutcome::Consumed => {
                        return KeyReaction::Consumed;
                    }
                    HandleKeyOutcome::NotHandled => {}
                }
            }
        }

        match key.code {
            KeyCode::Up => {
                self.select_prev();
                return KeyReaction::Consumed;
            }
            KeyCode::Down => {
                self.select_next();
                return KeyReaction::Consumed;
            }
            KeyCode::Enter => return KeyReaction::Submit,
            KeyCode::Esc => return KeyReaction::Cancel,
            _ => {}
        }
        // Drop Ctrl/Alt-modified chars so combos don't leak as text.
        if let KeyCode::Char(_) = key.code {
            let non_shift = key.modifiers - KeyModifiers::SHIFT;
            if !non_shift.is_empty() {
                return KeyReaction::Unhandled;
            }
        }
        let outcome = self.input.handle_key(key);
        // Sync/refresh/close the autocomplete popup based on the input outcome.
        // Build snapshot before taking &mut self.autocomplete (same borrow trick).
        let snap = self.autocomplete_snapshot();
        match outcome {
            InputOutcome::Changed => {
                if let Some(ac) = &mut self.autocomplete {
                    ac.sync(&snap);
                }
            }
            InputOutcome::Consumed => {
                if let Some(ac) = &mut self.autocomplete {
                    ac.refresh_if_open(&snap);
                }
            }
            InputOutcome::Cancel | InputOutcome::Submit => {
                if let Some(ac) = &mut self.autocomplete {
                    ac.close();
                }
            }
            InputOutcome::NotConsumed => {}
        }
        match outcome {
            InputOutcome::Changed => {
                self.sync_query_from_input();
                KeyReaction::Consumed
            }
            InputOutcome::Consumed => KeyReaction::Consumed,
            InputOutcome::Submit => KeyReaction::Submit,
            InputOutcome::Cancel => KeyReaction::Cancel,
            InputOutcome::NotConsumed => KeyReaction::Unhandled,
        }
    }

    pub fn render_query(&mut self, f: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        self.input.render(
            f,
            area,
            Style::default()
                .fg(theme.fg.to_ratatui())
                .bg(theme.bg_panel.to_ratatui()),
            0,
            focused,
        );
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        self.poll();
        let sel = self.selected;
        let items: Vec<ListItem> = (0..self.visible_len())
            .filter_map(|pos| {
                self.visible_row(pos)
                    .map(|r| r.to_list_item(theme, &self.icons, sel == Some(pos)))
            })
            .collect();
        let mut state = ListState::default();
        state.select(self.selected);
        let list =
            List::new(items).highlight_style(Style::default().bg(theme.bg_selected.to_ratatui()));
        f.render_stateful_widget(list, area, &mut state);
        self.list_rect = area;
        let _ = focused;
    }

    /// Override the rect used for mouse hit-testing. The recorded rect must be
    /// the area where list ITEMS actually render — row 0 is the first item, NOT
    /// a block border. Hosts that draw the list inside a bordered block pass the
    /// block's INNER rect; borderless hosts pass the list area directly. The
    /// recorded rect and the rendered-items rect MUST be identical, so
    /// [`handle_mouse`] maps a click at `row` to visual offset `row - rect.y`.
    ///
    /// [`handle_mouse`]: Self::handle_mouse
    pub fn set_list_rect(&mut self, rect: Rect) {
        self.list_rect = rect;
    }

    pub fn render_autocomplete(&mut self, f: &mut Frame, clamp: Rect, theme: &Theme) {
        if let Some(ac) = &mut self.autocomplete {
            ac.poll_results();
            let caret = self.input.last_caret_pos();
            if let (Some(state), Some(anchor)) = (ac.state_mut(), caret) {
                state.anchor = anchor;
            }
            if let Some(state) = ac.state() {
                crate::components::autocomplete::render(f, state, clamp, theme);
            }
        }
    }

    pub fn handle_mouse(&mut self, m: &ratatui::crossterm::event::MouseEvent) -> SearchMouse {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        use ratatui::layout::Position;
        // Any mouse interaction dismisses an open autocomplete popup (matches
        // the old modal: a click on the preview/border closes a stale popup).
        if let Some(ac) = &mut self.autocomplete {
            ac.close();
        }
        let r = self.list_rect;
        if !r.contains(Position {
            x: m.column,
            y: m.row,
        }) {
            return SearchMouse::None;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if m.row >= r.y => {
                let target_visual = m.row - r.y; // 0-based visual offset; row 0 = first item
                let mut acc: u16 = 0;
                let mut hit: Option<usize> = None;
                // Walk the VISIBLE sequence (leading row at position 0, then the
                // display rows) so visual offsets map to visible positions.
                for pos in 0..self.visible_len() {
                    let h = self
                        .visible_row(pos)
                        .map(|r| r.visual_height())
                        .unwrap_or(1);
                    if target_visual < acc + h {
                        hit = Some(pos);
                        break;
                    }
                    acc += h;
                }
                if let Some(pos) = hit {
                    let prev = self.selected;
                    self.selected = Some(pos);
                    return if prev == Some(pos) {
                        SearchMouse::Activated(pos)
                    } else {
                        SearchMouse::Selected(pos)
                    };
                }
                SearchMouse::None
            }
            MouseEventKind::ScrollUp => {
                self.select_prev();
                SearchMouse::Scrolled
            }
            MouseEventKind::ScrollDown => {
                self.select_next();
                SearchMouse::Scrolled
            }
            _ => SearchMouse::None,
        }
    }

    fn recompute_display(&mut self) {
        let q = self.query.trim();
        // The leading row is query-fresh: rebuilt on every poll AND on every
        // local-filter `set_query`, so it never goes stale.
        self.leading = self.source.leading_row(q);
        let mut idx: Vec<usize> = match &self.filter {
            Filter::SourceOrder => (0..self.rows.len()).collect(),
            Filter::Fuzzy if q.is_empty() => (0..self.rows.len()).collect(),
            Filter::Fuzzy => fuzzy_indices(&self.rows, q),
            Filter::Rank(_) if q.is_empty() => (0..self.rows.len()).collect(),
            Filter::Rank(f) => {
                let f = f.clone();
                f(&self.rows, q)
            }
        };
        // Filter-exempt rows (match_text() == None: Up / Create / virtual pinned)
        // are always present; prepend any that the filter dropped.
        for i in 0..self.rows.len() {
            if self.rows[i].match_text().is_none() && !idx.contains(&i) {
                idx.insert(0, i);
            }
        }
        self.display = idx;
        self.clamp_selection();
    }

    #[cfg(test)]
    pub(crate) async fn poll_until_idle(&mut self) {
        // In-memory sources settle on the first poll (no sleep paid). Vault-backed
        // sources run their read on a worker/blocking thread, which can starve
        // under the full parallel suite — so once still loading, sleep a little
        // between polls and use a generous ceiling. Early-breaks the instant the
        // load lands, keeping the common (in-memory) path fast.
        for _ in 0..600 {
            tokio::task::yield_now().await;
            self.poll();
            if !self.is_loading() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        self.poll();
    }
}

impl<R: SearchRow> SearchListBuilder<R> {
    pub fn initial_query(mut self, q: impl Into<String>) -> Self {
        self.initial_query = q.into();
        self
    }
    pub fn filter(mut self, f: Filter<R>) -> Self {
        self.filter = f;
        self
    }
    pub fn autocomplete(
        mut self,
        suggestions: Arc<dyn SuggestionSource>,
        mode: AutocompleteMode,
    ) -> Self {
        self.autocomplete = Some((suggestions, mode));
        self
    }
    pub fn intercept(mut self, v: Vec<KeyCombo>) -> Self {
        self.intercept = v;
        self
    }
    pub fn icons(mut self, icons: Icons) -> Self {
        self.icons = icons;
        self
    }
    /// Override the autocomplete controller's debounce. Tests use
    /// `Duration::ZERO` to get suggestions without waiting on the debounce timer.
    pub fn debounce(mut self, d: std::time::Duration) -> Self {
        self.debounce = Some(d);
        self
    }
    pub fn build(self) -> SearchList<R> {
        SearchList::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::adapters::{
        ScriptedStreamLeadSource, ScriptedStreamSource, StreamRow, TestRow, VecSource,
        VecSourceWithLead,
    };
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn noop_redraw() -> std::sync::Arc<dyn Fn() + Send + Sync> {
        std::sync::Arc::new(|| {})
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn mouse_down_at(col: u16, row: u16) -> ratatui::crossterm::event::MouseEvent {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    struct TallRow {
        name: String,
        height: u16,
    }
    impl SearchRow for TallRow {
        fn to_list_item(
            &self,
            _t: &crate::settings::themes::Theme,
            _i: &crate::settings::icons::Icons,
            _s: bool,
        ) -> ratatui::widgets::ListItem<'static> {
            ratatui::widgets::ListItem::new(self.name.clone())
        }
        fn visual_height(&self) -> u16 {
            self.height
        }
        fn match_text(&self) -> Option<&str> {
            Some(&self.name)
        }
    }
    struct TallSource(Vec<TallRow>);
    #[async_trait::async_trait]
    impl RowSource<TallRow> for TallSource {
        async fn load(&self, _q: &str, emit: Emit<TallRow>) {
            emit.replace(self.0.clone());
        }
    }

    #[tokio::test]
    async fn mouse_maps_visual_row_to_display_index_by_height() {
        // Row 0 occupies 3 visual rows, row 1 occupies 1. The recorded list rect
        // is the rendered-items area: row 0 == the FIRST item (no border row).
        let src = TallSource(vec![
            TallRow {
                name: "a".into(),
                height: 3,
            },
            TallRow {
                name: "b".into(),
                height: 1,
            },
        ]);
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        // Force the recorded list rect (render not run in test): items start at y=0.
        list.set_list_rect(ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        });
        // "a" occupies rows 0..=2; row 3 is the FIRST row of "b".
        let m = mouse_down_at(2, 3);
        assert!(matches!(list.handle_mouse(&m), SearchMouse::Selected(1)));
        assert_eq!(list.selected_row().unwrap().name, "b");
        // A click at row 1 = within "a" (rows 0..=2) -> display index 0.
        let m = mouse_down_at(2, 1);
        list.handle_mouse(&m);
        assert_eq!(list.selected_row().unwrap().name, "a");
    }

    #[tokio::test]
    async fn initial_load_populates_rows() {
        let src = VecSource {
            rows: vec![TestRow::new("alpha"), TestRow::new("beta")],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.rows().len(), 2);
        assert_eq!(list.selected_row().map(|r| r.name.as_str()), Some("alpha"));
    }

    #[tokio::test]
    async fn requery_supersedes_and_reloads() {
        let src = VecSource {
            rows: vec![
                TestRow::new("alpha"),
                TestRow::new("alps"),
                TestRow::new("beta"),
            ],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.rows().len(), 3);
        list.set_query("alp");
        list.poll_until_idle().await;
        assert_eq!(list.rows().len(), 2); // alpha, alps
        assert!(list.rows().iter().all(|r| r.name.contains("alp")));
    }

    #[tokio::test]
    async fn arrows_navigate_and_enter_submits() {
        let src = VecSource {
            rows: vec![TestRow::new("a"), TestRow::new("b")],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.handle_key(&key(KeyCode::Down)), KeyReaction::Consumed);
        assert_eq!(list.selected_row().unwrap().name, "b");
        assert_eq!(list.handle_key(&key(KeyCode::Enter)), KeyReaction::Submit);
        assert_eq!(list.handle_key(&key(KeyCode::Esc)), KeyReaction::Cancel);
    }

    #[tokio::test]
    async fn typing_a_char_changes_query() {
        let src = VecSource {
            rows: vec![TestRow::new("alpha"), TestRow::new("beta")],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(
            list.handle_key(&key(KeyCode::Char('a'))),
            KeyReaction::Consumed
        );
        list.poll_until_idle().await;
        assert_eq!(list.query(), "a");
    }

    #[tokio::test]
    async fn rank_filter_orders_by_closure() {
        let src = VecSource {
            rows: vec![
                TestRow::new("todo"),
                TestRow::new("today"),
                TestRow::new("misc"),
            ],
            reload: false,
        };
        let rank = std::sync::Arc::new(|rows: &[TestRow], q: &str| -> Vec<usize> {
            let mut idx: Vec<usize> = (0..rows.len())
                .filter(|&i| rows[i].name.contains(q))
                .collect();
            idx.sort_by_key(|&i| if rows[i].name == q { 0 } else { 1 });
            idx
        });
        let mut list = SearchList::builder(src, noop_redraw())
            .filter(Filter::Rank(rank))
            .build();
        list.poll_until_idle().await;
        list.set_query("today");
        list.poll();
        assert_eq!(list.selected_row().unwrap().name, "today");
    }

    #[tokio::test]
    async fn fuzzy_filter_narrows_local_set() {
        let src = VecSource {
            rows: vec![TestRow::new("alpha"), TestRow::new("beta")],
            reload: false,
        };
        let mut list = SearchList::builder(src, noop_redraw())
            .filter(Filter::Fuzzy)
            .build();
        list.poll_until_idle().await;
        list.set_query("alp");
        list.poll();
        assert_eq!(list.visible_rows().len(), 1);
        assert_eq!(list.selected_row().unwrap().name, "alpha");
    }

    #[tokio::test]
    async fn streamed_rows_arrive_then_done_and_filter_locally() {
        let src = ScriptedStreamSource {
            batches: vec![vec![TestRow::new("alpha")], vec![TestRow::new("beta")]],
        };
        let mut list = SearchList::builder(src, noop_redraw())
            .filter(Filter::Fuzzy)
            .build();
        list.poll_until_idle().await;
        assert_eq!(list.rows().len(), 2);
        assert!(!list.is_loading());
        list.set_query("alp");
        list.poll();
        assert_eq!(list.visible_rows().len(), 1);
    }

    #[tokio::test]
    async fn source_order_unfiltered_passthrough() {
        let src = VecSource {
            rows: vec![TestRow::new("a"), TestRow::new("b")],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw()).build(); // default Filter::SourceOrder
        list.poll_until_idle().await;
        assert_eq!(list.visible_rows().len(), 2);
        assert_eq!(list.selected_row().unwrap().name, "a");
    }

    #[tokio::test]
    async fn intercepted_combo_returns_intercepted_without_acting() {
        let src = VecSource {
            rows: vec![TestRow::new("a")],
            reload: true,
        };
        let combo = crate::keys::key_event_to_combo(&key(KeyCode::Enter)).unwrap();
        let mut list = SearchList::builder(src, noop_redraw())
            .intercept(vec![combo])
            .build();
        list.poll_until_idle().await;
        // Enter is intercepted: engine returns Intercepted, does NOT submit/act.
        assert_eq!(
            list.handle_key(&key(KeyCode::Enter)),
            KeyReaction::Intercepted(combo)
        );
    }

    #[tokio::test]
    async fn autocomplete_accept_rewrites_query_without_vault() {
        struct Mem;
        #[async_trait::async_trait]
        impl crate::components::search_list::SuggestionSource for Mem {
            async fn notes_by_prefix(
                &self,
                _p: &str,
                _n: usize,
            ) -> Vec<crate::components::search_list::SuggestionItem> {
                vec![]
            }
            async fn tags_by_prefix(
                &self,
                p: &str,
                _n: usize,
            ) -> Vec<crate::components::search_list::SuggestionItem> {
                if "projects".starts_with(p) {
                    vec![crate::components::search_list::SuggestionItem::plain(
                        "projects",
                    )]
                } else {
                    vec![]
                }
            }
        }
        let src = VecSource {
            rows: vec![],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw())
            .autocomplete(
                std::sync::Arc::new(Mem),
                crate::components::autocomplete::AutocompleteMode::SearchQuery,
            )
            .debounce(std::time::Duration::ZERO)
            .build();
        for c in ['#', 'p', 'r', 'o'] {
            let _ = list.handle_key(&key(KeyCode::Char(c)));
        }
        for _ in 0..50 {
            tokio::task::yield_now().await;
            list.poll();
        }
        let _ = list.handle_key(&key(KeyCode::Tab));
        assert_eq!(list.query(), "#projects");
    }

    // Regression: Enter (not just Tab) must accept an open autocomplete popup,
    // and the engine must report Consumed — NOT Submit — so a host does not
    // mistake the accept for a list submit. (A QueryPanel Enter pre-check used
    // to swallow this, breaking accept-on-Enter in the right sidebar.)
    #[tokio::test]
    async fn enter_accepts_open_popup_and_reports_consumed() {
        struct Mem;
        #[async_trait::async_trait]
        impl crate::components::search_list::SuggestionSource for Mem {
            async fn notes_by_prefix(
                &self,
                _p: &str,
                _n: usize,
            ) -> Vec<crate::components::search_list::SuggestionItem> {
                vec![]
            }
            async fn tags_by_prefix(
                &self,
                p: &str,
                _n: usize,
            ) -> Vec<crate::components::search_list::SuggestionItem> {
                if "projects".starts_with(p) {
                    vec![crate::components::search_list::SuggestionItem::plain(
                        "projects",
                    )]
                } else {
                    vec![]
                }
            }
        }
        let src = VecSource {
            rows: vec![],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw())
            .autocomplete(
                std::sync::Arc::new(Mem),
                crate::components::autocomplete::AutocompleteMode::SearchQuery,
            )
            .debounce(std::time::Duration::ZERO)
            .build();
        for c in ['#', 'p', 'r', 'o'] {
            let _ = list.handle_key(&key(KeyCode::Char(c)));
        }
        for _ in 0..50 {
            tokio::task::yield_now().await;
            list.poll();
        }
        // Popup is open: Enter accepts the suggestion and reports Consumed.
        assert_eq!(list.handle_key(&key(KeyCode::Enter)), KeyReaction::Consumed);
        assert_eq!(list.query(), "#projects");
        // Popup now closed: a second Enter falls through to Submit.
        assert_eq!(list.handle_key(&key(KeyCode::Enter)), KeyReaction::Submit);
    }

    // Regression (P0): a STREAMED source (sidebar shape) supplies a query-fresh
    // leading row. It must appear at visible position 0 even though rows arrive
    // via Push (never Replace), be present when the query matches no streamed
    // row, and refresh when the query changes (reload_on_query() == false).
    #[tokio::test]
    async fn streamed_source_leading_row_is_pinned_and_query_fresh() {
        let src = ScriptedStreamLeadSource {
            items: vec!["alpha".into(), "beta".into()],
        };
        let mut list = SearchList::builder(src, noop_redraw())
            .filter(Filter::Fuzzy)
            .initial_query("zz")
            .build();
        list.poll_until_idle().await;
        // Leading present even though "zz" matches no streamed Item.
        let vis = list.visible_rows();
        assert_eq!(vis[0], &StreamRow::Create("zz".into()));
        assert_eq!(list.visible_len(), 1); // just the leading; no Item matches
        // Query-fresh: changing the query rebuilds the leading and re-filters.
        list.set_query("alp");
        list.poll();
        let vis = list.visible_rows();
        assert_eq!(vis[0], &StreamRow::Create("alp".into()));
        assert_eq!(vis[1], &StreamRow::Item("alpha".into()));
        assert_eq!(list.visible_len(), 2);
        // Empty query: leading disappears, both Items show.
        list.set_query("");
        list.poll();
        assert!(
            list.visible_rows()
                .iter()
                .all(|r| matches!(r, StreamRow::Item(_)))
        );
        assert_eq!(list.visible_len(), 2);
    }

    // Regression guard for the saved-searches virtual entry: a one-shot
    // (Replace) source with a leading row still pins it at position 0.
    #[tokio::test]
    async fn oneshot_source_leading_row_still_works() {
        let src = VecSourceWithLead {
            rows: vec![TestRow::new("alpha"), TestRow::new("beta")],
        };
        let mut list = SearchList::builder(src, noop_redraw())
            .filter(Filter::Fuzzy)
            .initial_query("alp")
            .build();
        list.poll_until_idle().await;
        let vis = list.visible_rows();
        assert_eq!(vis[0].name, "create:alp");
        assert_eq!(vis[1].name, "alpha");
        assert_eq!(list.visible_len(), 2);
    }

    // Selection walks the VISIBLE sequence: position 0 is the leading row, and
    // select_next steps from the leading to the first real row.
    #[tokio::test]
    async fn selection_includes_leading_at_position_zero() {
        let src = VecSourceWithLead {
            rows: vec![TestRow::new("alpha"), TestRow::new("alps")],
        };
        let mut list = SearchList::builder(src, noop_redraw())
            .filter(Filter::Fuzzy)
            .initial_query("alp")
            .build();
        list.poll_until_idle().await;
        // Auto-selected position 0 -> the leading.
        assert_eq!(list.selected_row().unwrap().name, "create:alp");
        list.handle_key(&key(KeyCode::Down));
        assert_eq!(list.selected_row().unwrap().name, "alpha");
    }

    // A source with NO leading row has no off-by-one: visible_len == display.
    #[tokio::test]
    async fn no_leading_row_visible_len_matches_display() {
        let src = VecSource {
            rows: vec![TestRow::new("a"), TestRow::new("b")],
            reload: true,
        };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.visible_len(), 2);
        assert_eq!(list.visible_rows().len(), 2);
        assert_eq!(list.selected_row().unwrap().name, "a");
    }
}
