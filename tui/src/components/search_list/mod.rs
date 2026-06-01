//! `SearchList`: the one module behind every query-input-over-an-async-loaded
//! list surface in the TUI. See CONTEXT.md.

mod seams;
mod load;
#[cfg(test)]
mod adapters;

pub use seams::{Emit, Filter, Loaded, RowSource, SearchRow};

use std::sync::Arc;
use load::LoadEngine;
use seams::Loaded as LoadedInner;
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use ratatui::crossterm::event::KeyEvent;

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
}

pub struct SearchListBuilder<R: SearchRow> {
    source: Arc<dyn RowSource<R>>,
    redraw: Arc<dyn Fn() + Send + Sync>,
    initial_query: String,
    filter: Filter<R>,
}

impl<R: SearchRow> SearchList<R> {
    pub fn builder(source: impl RowSource<R>, redraw: Arc<dyn Fn() + Send + Sync>) -> SearchListBuilder<R> {
        SearchListBuilder { source: Arc::new(source), redraw, initial_query: String::new(), filter: Filter::SourceOrder }
    }

    fn new(b: SearchListBuilder<R>) -> Self {
        let mut loader = LoadEngine::new(b.redraw.clone());
        loader.start(b.source.clone(), b.initial_query.clone());
        let input = SingleLineInput::with_value(&b.initial_query);
        Self {
            source: b.source,
            rows: Vec::new(),
            display: Vec::new(),
            selected: None,
            filter: b.filter,
            query: b.initial_query,
            loader,
            input,
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
        match self.input.handle_key(key) {
            InputOutcome::Changed => { self.set_query(self.input.value().to_string()); KeyReaction::Consumed }
            InputOutcome::Consumed => KeyReaction::Consumed,
            InputOutcome::Submit => KeyReaction::Submit,
            InputOutcome::Cancel => KeyReaction::Cancel,
            InputOutcome::NotConsumed => KeyReaction::Unhandled,
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
}
