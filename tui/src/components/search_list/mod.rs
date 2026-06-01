//! `SearchList`: the one module behind every query-input-over-an-async-loaded
//! list surface in the TUI. See CONTEXT.md.

mod seams;
mod load;
#[cfg(test)]
mod adapters;

pub use seams::{Emit, Loaded, RowSource, SearchRow};

use std::sync::Arc;
use load::LoadEngine;
use seams::Loaded as LoadedInner;

pub struct SearchList<R: SearchRow> {
    source: Arc<dyn RowSource<R>>,
    rows: Vec<R>,
    selected: Option<usize>,
    query: String,
    loader: LoadEngine<R>,
}

pub struct SearchListBuilder<R: SearchRow> {
    source: Arc<dyn RowSource<R>>,
    redraw: Arc<dyn Fn() + Send + Sync>,
    initial_query: String,
}

impl<R: SearchRow> SearchList<R> {
    pub fn builder(source: impl RowSource<R>, redraw: Arc<dyn Fn() + Send + Sync>) -> SearchListBuilder<R> {
        SearchListBuilder { source: Arc::new(source), redraw, initial_query: String::new() }
    }

    fn new(b: SearchListBuilder<R>) -> Self {
        let mut loader = LoadEngine::new(b.redraw.clone());
        loader.start(b.source.clone(), b.initial_query.clone());
        Self { source: b.source, rows: Vec::new(), selected: None, query: b.initial_query, loader }
    }

    pub fn poll(&mut self) {
        for ev in self.loader.drain() {
            match ev {
                LoadedInner::Replace(rows) => { self.rows = rows; self.clamp_selection(); }
                LoadedInner::Push(row) => {
                    self.rows.push(row);
                    if self.selected.is_none() && !self.rows.is_empty() { self.selected = Some(0); }
                }
                LoadedInner::Done => {}
            }
        }
    }

    fn clamp_selection(&mut self) {
        self.selected = if self.rows.is_empty() { None } else { Some(self.selected.unwrap_or(0).min(self.rows.len() - 1)) };
    }

    pub fn rows(&self) -> &[R] { &self.rows }
    pub fn selected_row(&self) -> Option<&R> { self.selected.and_then(|i| self.rows.get(i)) }
    pub fn query(&self) -> &str { &self.query }
    pub fn is_loading(&self) -> bool { self.loader.loading }

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
    pub fn build(self) -> SearchList<R> { SearchList::new(self) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::adapters::{TestRow, VecSource};

    fn noop_redraw() -> std::sync::Arc<dyn Fn() + Send + Sync> {
        std::sync::Arc::new(|| {})
    }

    #[tokio::test]
    async fn initial_load_populates_rows() {
        let src = VecSource { rows: vec![TestRow::new("alpha"), TestRow::new("beta")], reload: true };
        let mut list = SearchList::builder(src, noop_redraw()).build();
        list.poll_until_idle().await;
        assert_eq!(list.rows().len(), 2);
        assert_eq!(list.selected_row().map(|r| r.name.as_str()), Some("alpha"));
    }
}
