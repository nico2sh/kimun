//! `SearchList`: the one module behind every query-input-over-an-async-loaded
//! list surface in the TUI. See CONTEXT.md.

mod seams;
mod load;
mod host;
#[cfg(test)]
mod adapters;

pub use seams::{Emit, Filter, Loaded, RowSource, SearchRow, SuggestionItem, SuggestionSource, VaultSuggestions};

use std::sync::Arc;
use load::LoadEngine;
use seams::Loaded as LoadedInner;
use crate::components::autocomplete::{AutocompleteController, AutocompleteMode, HandleKeyOutcome, TriggerOptions};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::keys::key_combo::KeyCombo;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;
use ratatui::crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect, style::Style, widgets::{List, ListItem, ListState}};

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
    scored.sort_by(|a, b| b.1.cmp(&a.1));
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
    /// Index into `display` (not `rows`) of the selected item.
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
}

impl<R: SearchRow> SearchList<R> {
    pub fn builder(source: impl RowSource<R>, redraw: Arc<dyn Fn() + Send + Sync>) -> SearchListBuilder<R> {
        SearchListBuilder {
            source: Arc::new(source),
            redraw,
            initial_query: String::new(),
            filter: Filter::SourceOrder,
            autocomplete: None,
            intercept: Vec::new(),
            icons: Icons::new(false),
        }
    }

    fn new(b: SearchListBuilder<R>) -> Self {
        let mut loader = LoadEngine::new(b.redraw.clone());
        loader.start(b.source.clone(), b.initial_query.clone());
        let input = SingleLineInput::with_value(&b.initial_query);
        let autocomplete = b.autocomplete.map(|(suggestions, mode)| {
            #[allow(unused_mut)]
            let mut ac = AutocompleteController::new(suggestions, mode)
                .with_trigger_opts(TriggerOptions { disambiguate_header: false, apply_exclusion_zone: false });
            #[cfg(test)]
            let mut ac = ac.with_debounce(std::time::Duration::ZERO);
            ac.set_redraw_callback(b.redraw.clone());
            ac
        });
        Self {
            source: b.source,
            rows: Vec::new(),
            display: Vec::new(),
            selected: None,
            filter: b.filter,
            query: b.initial_query,
            loader,
            input,
            autocomplete,
            intercept: b.intercept,
            icons: b.icons,
            list_rect: Rect::default(),
        }
    }

    pub fn poll(&mut self) {
        for ev in self.loader.drain() {
            match ev {
                LoadedInner::Replace(rows) => {
                    let mut rows = rows;
                    if let Some(lead) = self.source.leading_row(&self.query) {
                        rows.insert(0, lead);
                    }
                    self.rows = rows;
                }
                LoadedInner::Push(row) => {
                    self.rows.push(row);
                }
                LoadedInner::Done => {}
            }
        }
        self.recompute_display();
        if self.selected.is_none() && !self.display.is_empty() {
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
        self.selected = if self.display.is_empty() {
            None
        } else {
            Some(self.selected.unwrap_or(0).min(self.display.len() - 1))
        };
    }

    pub fn rows(&self) -> &[R] { &self.rows }

    pub fn selected_row(&self) -> Option<&R> {
        self.selected
            .and_then(|i| self.display.get(i))
            .and_then(|&ri| self.rows.get(ri))
    }

    pub fn visible_rows(&self) -> Vec<&R> {
        self.display.iter().filter_map(|&i| self.rows.get(i)).collect()
    }

    pub fn query(&self) -> &str { &self.query }
    pub fn is_loading(&self) -> bool { self.loader.loading }

    /// Set the query and (for `reload_on_query` sources) start a fresh load.
    /// The generation guard in `LoadEngine` drops any in-flight stale results.
    pub fn set_query(&mut self, q: impl Into<String>) {
        self.query = q.into();
        if self.source.reload_on_query() {
            self.loader.start(self.source.clone(), self.query.clone());
        } else {
            self.recompute_display();
        }
    }

    pub fn select_next(&mut self) {
        if self.display.is_empty() { return; }
        let n = self.display.len();
        self.selected = Some(self.selected.map_or(0, |i| (i + 1).min(n - 1)));
    }

    pub fn select_prev(&mut self) {
        if self.display.is_empty() { return; }
        self.selected = Some(self.selected.map_or(0, |i| i.saturating_sub(1)));
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> KeyReaction {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        // Caller-registered intercepts get first crack — before autocomplete or
        // any built-in binding.
        if let Some(combo) = crate::keys::key_event_to_combo(key) {
            if self.intercept.contains(&combo) {
                return KeyReaction::Intercepted(combo);
            }
        }

        // Autocomplete popup gets first crack when open. Build snapshot before
        // taking &mut self.autocomplete to avoid borrow-checker conflict
        // (snapshot only reads self.input).
        if self.autocomplete.as_ref().map_or(false, |ac| ac.is_open()) {
            let snap = self.autocomplete_snapshot();
            if let Some(ac) = &mut self.autocomplete {
                match ac.handle_key(*key, &snap) {
                    HandleKeyOutcome::Accepted(action) => {
                        self.input.replace_range_bytes(action.range.clone(), &action.new_text, action.new_cursor_byte);
                        self.set_query(self.input.value().to_string());
                        return KeyReaction::Consumed;
                    }
                    HandleKeyOutcome::Dismissed | HandleKeyOutcome::Consumed => return KeyReaction::Consumed,
                    HandleKeyOutcome::NotHandled => {}
                }
            }
        }

        match key.code {
            KeyCode::Up => { self.select_prev(); return KeyReaction::Consumed; }
            KeyCode::Down => { self.select_next(); return KeyReaction::Consumed; }
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
                if let Some(ac) = &mut self.autocomplete { ac.sync(&snap); }
            }
            InputOutcome::Consumed => {
                if let Some(ac) = &mut self.autocomplete { ac.refresh_if_open(&snap); }
            }
            InputOutcome::Cancel | InputOutcome::Submit => {
                if let Some(ac) = &mut self.autocomplete { ac.close(); }
            }
            InputOutcome::NotConsumed => {}
        }
        match outcome {
            InputOutcome::Changed => { self.set_query(self.input.value().to_string()); KeyReaction::Consumed }
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
            Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg_panel.to_ratatui()),
            0,
            focused,
        );
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        self.poll();
        let sel = self.selected;
        let items: Vec<ListItem> = self.display.iter()
            .enumerate()
            .filter_map(|(disp_idx, &row_idx)| {
                self.rows.get(row_idx).map(|r| r.to_list_item(theme, &self.icons, sel == Some(disp_idx)))
            })
            .collect();
        let mut state = ListState::default();
        state.select(self.selected);
        let list = List::new(items)
            .highlight_style(Style::default().bg(theme.bg_selected.to_ratatui()));
        f.render_stateful_widget(list, area, &mut state);
        self.list_rect = area;
        let _ = focused;
    }

    /// Override the rect used for mouse hit-testing. Hosts that draw the list
    /// inside their own bordered block render into the block's inner area but
    /// must record the block's OUTER rect, because [`handle_mouse`] hit-tests
    /// as `row - rect.y - 1` (assuming the first row of `rect` is a border).
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
        let r = self.list_rect;
        if !r.contains(Position { x: m.column, y: m.row }) { return SearchMouse::None; }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if m.row > r.y => {
                let rel = (m.row - r.y - 1) as usize; // border-aware, consistent with note_browser
                if rel < self.display.len() {
                    let prev = self.selected;
                    self.selected = Some(rel);
                    return if prev == Some(rel) { SearchMouse::Activated(rel) } else { SearchMouse::Selected(rel) };
                }
                SearchMouse::None
            }
            MouseEventKind::ScrollUp => { self.select_prev(); SearchMouse::Scrolled }
            MouseEventKind::ScrollDown => { self.select_next(); SearchMouse::Scrolled }
            _ => SearchMouse::None,
        }
    }

    fn recompute_display(&mut self) {
        let q = self.query.trim();
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
        for _ in 0..50 {
            tokio::task::yield_now().await;
            self.poll();
            if !self.is_loading() { break; }
        }
    }
}

impl<R: SearchRow> SearchListBuilder<R> {
    pub fn initial_query(mut self, q: impl Into<String>) -> Self { self.initial_query = q.into(); self }
    pub fn filter(mut self, f: Filter<R>) -> Self { self.filter = f; self }
    pub fn autocomplete(mut self, suggestions: Arc<dyn SuggestionSource>, mode: AutocompleteMode) -> Self {
        self.autocomplete = Some((suggestions, mode));
        self
    }
    pub fn intercept(mut self, v: Vec<KeyCombo>) -> Self { self.intercept = v; self }
    pub fn icons(mut self, icons: Icons) -> Self { self.icons = icons; self }
    pub fn build(self) -> SearchList<R> { SearchList::new(self) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::adapters::{TestRow, VecSource};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn noop_redraw() -> std::sync::Arc<dyn Fn() + Send + Sync> {
        std::sync::Arc::new(|| {})
    }

    fn key(c: KeyCode) -> KeyEvent { KeyEvent::new(c, KeyModifiers::NONE) }

    #[tokio::test]
    async fn initial_load_populates_rows() {
        let src = VecSource { rows: vec![TestRow::new("alpha"), TestRow::new("beta")], reload: true };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.rows().len(), 2);
        assert_eq!(list.selected_row().map(|r| r.name.as_str()), Some("alpha"));
    }

    #[tokio::test]
    async fn requery_supersedes_and_reloads() {
        let src = VecSource { rows: vec![TestRow::new("alpha"), TestRow::new("alps"), TestRow::new("beta")], reload: true };
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
        let src = VecSource { rows: vec![TestRow::new("a"), TestRow::new("b")], reload: true };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.handle_key(&key(KeyCode::Down)), KeyReaction::Consumed);
        assert_eq!(list.selected_row().unwrap().name, "b");
        assert_eq!(list.handle_key(&key(KeyCode::Enter)), KeyReaction::Submit);
        assert_eq!(list.handle_key(&key(KeyCode::Esc)), KeyReaction::Cancel);
    }

    #[tokio::test]
    async fn typing_a_char_changes_query() {
        let src = VecSource { rows: vec![TestRow::new("alpha"), TestRow::new("beta")], reload: true };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.handle_key(&key(KeyCode::Char('a'))), KeyReaction::Consumed);
        list.poll_until_idle().await;
        assert_eq!(list.query(), "a");
    }

    #[tokio::test]
    async fn rank_filter_orders_by_closure() {
        let src = VecSource { rows: vec![TestRow::new("todo"), TestRow::new("today"), TestRow::new("misc")], reload: false };
        let rank = std::sync::Arc::new(|rows: &[TestRow], q: &str| -> Vec<usize> {
            let mut idx: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].name.contains(q)).collect();
            idx.sort_by_key(|&i| if rows[i].name == q { 0 } else { 1 });
            idx
        });
        let mut list = SearchList::builder(src, noop_redraw()).filter(Filter::Rank(rank)).build();
        list.poll_until_idle().await;
        list.set_query("today");
        list.poll();
        assert_eq!(list.selected_row().unwrap().name, "today");
    }

    #[tokio::test]
    async fn fuzzy_filter_narrows_local_set() {
        let src = VecSource { rows: vec![TestRow::new("alpha"), TestRow::new("beta")], reload: false };
        let mut list = SearchList::builder(src, noop_redraw()).filter(Filter::Fuzzy).build();
        list.poll_until_idle().await;
        list.set_query("alp");
        list.poll();
        assert_eq!(list.visible_rows().len(), 1);
        assert_eq!(list.selected_row().unwrap().name, "alpha");
    }

    #[tokio::test]
    async fn source_order_unfiltered_passthrough() {
        let src = VecSource { rows: vec![TestRow::new("a"), TestRow::new("b")], reload: true };
        let mut list = SearchList::builder(src, noop_redraw()).build(); // default Filter::SourceOrder
        list.poll_until_idle().await;
        assert_eq!(list.visible_rows().len(), 2);
        assert_eq!(list.selected_row().unwrap().name, "a");
    }

    #[tokio::test]
    async fn intercepted_combo_returns_intercepted_without_acting() {
        let src = VecSource { rows: vec![TestRow::new("a")], reload: true };
        let combo = crate::keys::key_event_to_combo(&key(KeyCode::Enter)).unwrap();
        let mut list = SearchList::builder(src, noop_redraw()).intercept(vec![combo.clone()]).build();
        list.poll_until_idle().await;
        // Enter is intercepted: engine returns Intercepted, does NOT submit/act.
        assert_eq!(list.handle_key(&key(KeyCode::Enter)), KeyReaction::Intercepted(combo));
    }

    #[tokio::test]
    async fn autocomplete_accept_rewrites_query_without_vault() {
        struct Mem;
        #[async_trait::async_trait]
        impl crate::components::search_list::SuggestionSource for Mem {
            async fn notes_by_prefix(&self, _p: &str, _n: usize) -> Vec<crate::components::search_list::SuggestionItem> { vec![] }
            async fn tags_by_prefix(&self, p: &str, _n: usize) -> Vec<crate::components::search_list::SuggestionItem> {
                if "projects".starts_with(p) { vec![crate::components::search_list::SuggestionItem::plain("projects")] } else { vec![] }
            }
        }
        let src = VecSource { rows: vec![], reload: true };
        let mut list = SearchList::builder(src, noop_redraw())
            .autocomplete(std::sync::Arc::new(Mem), crate::components::autocomplete::AutocompleteMode::SearchQuery)
            .build();
        for c in ['#','p','r','o'] { let _ = list.handle_key(&key(KeyCode::Char(c))); }
        for _ in 0..50 { tokio::task::yield_now().await; list.poll(); }
        let _ = list.handle_key(&key(KeyCode::Tab));
        assert_eq!(list.query(), "#projects");
    }
}
