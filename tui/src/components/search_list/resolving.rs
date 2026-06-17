//! `ResolvingRowSource`: the one adapter that resolves a query template's
//! variables before any inner [`RowSource`] sees it. It reads a fresh
//! [`QueryContext`] per load (so a panel whose open note changes between loads
//! resolves against the current note), substitutes `{note}` and the
//! bare-operator sugar, and applies a fallback when the template needs a note
//! none is available. Inner sources speak only resolved queries and never
//! import the query-variable logic. See CONTEXT.md "Resolving row source".

use std::sync::Arc;

use async_trait::async_trait;

use super::{Emit, RowSource, SearchRow};
use crate::components::query_vars::{QueryContext, query_is_unresolvable, resolve_query};

/// What to do when a template is purely note-dependent but no note is available
/// to resolve it (the startup state, or a browser launched from the root).
/// Running the bare prefix against core is a dead-end empty round-trip, so each
/// surface picks how to degrade instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unresolvable {
    /// Emit nothing (the Query panel: an unresolvable backlinks query has no
    /// results to show yet).
    Empty,
    /// Run the inner source as if the query were empty (the note browser: fall
    /// back to its recent-notes view).
    AsEmptyQuery,
}

/// A [`RowSource`] that resolves query variables against a per-load
/// [`QueryContext`] before delegating to `inner`. Generic over the row type, so
/// the same adapter serves every query-variable surface.
pub struct ResolvingRowSource<R: SearchRow> {
    inner: Arc<dyn RowSource<R>>,
    ctx: Arc<dyn Fn() -> QueryContext + Send + Sync>,
    on_unresolvable: Unresolvable,
}

impl<R: SearchRow> ResolvingRowSource<R> {
    pub fn new(
        inner: Arc<dyn RowSource<R>>,
        ctx: impl Fn() -> QueryContext + Send + Sync + 'static,
        on_unresolvable: Unresolvable,
    ) -> Self {
        Self {
            inner,
            ctx: Arc::new(ctx),
            on_unresolvable,
        }
    }
}

#[async_trait]
impl<R: SearchRow> RowSource<R> for ResolvingRowSource<R> {
    async fn load(&self, query: &str, emit: Emit<R>) {
        let ctx = (self.ctx)();
        if query_is_unresolvable(query, &ctx) {
            match self.on_unresolvable {
                Unresolvable::Empty => emit.replace(Vec::new()),
                Unresolvable::AsEmptyQuery => self.inner.load("", emit).await,
            }
            return;
        }
        self.inner.load(&resolve_query(query, &ctx), emit).await;
    }

    // The leading-row affordance ("Create: …") and the reload strategy are the
    // inner source's policy — forward them so wrapping is transparent. Note the
    // leading row sees the RAW template (what the user typed), while `load`
    // sees the resolved query.
    fn leading_row(&self, query: &str) -> Option<R> {
        self.inner.leading_row(query)
    }

    fn reload_on_query(&self) -> bool {
        self.inner.reload_on_query()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::redraw_callback;
    use crate::components::search_list::SearchList;
    use crate::settings::icons::Icons;
    use crate::settings::themes::Theme;
    use kimun_core::nfs::VaultPath;
    use ratatui::widgets::ListItem;
    use std::sync::Mutex;
    use tokio::sync::mpsc::unbounded_channel;

    /// A row that just records the query string the inner source was asked to
    /// load — the assertion surface for "what did the wrapper hand through?".
    #[derive(Clone)]
    struct EchoRow(String);
    impl SearchRow for EchoRow {
        fn to_list_item(&self, _: &Theme, _: &Icons, _: bool) -> ListItem<'static> {
            ListItem::new(self.0.clone())
        }
        fn match_text(&self) -> Option<&str> {
            Some(&self.0)
        }
    }

    /// Inner source that emits one row carrying whatever query it received, and
    /// records every query it saw (so we can assert the fallback handed `""`).
    struct EchoSource {
        seen: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl RowSource<EchoRow> for EchoSource {
        async fn load(&self, query: &str, emit: Emit<EchoRow>) {
            self.seen.lock().unwrap().push(query.to_string());
            emit.replace(vec![EchoRow(query.to_string())]);
        }
    }

    fn note(name: &str) -> QueryContext {
        QueryContext::with_note(Some(VaultPath::note_path_from(name)))
    }

    async fn run(initial: &str, ctx: QueryContext, on: Unresolvable) -> (Vec<String>, Vec<String>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::new(EchoSource { seen: seen.clone() });
        let source = ResolvingRowSource::new(inner, move || ctx.clone(), on);
        let (tx, _rx) = unbounded_channel();
        let mut list = SearchList::builder(source, redraw_callback(tx))
            .initial_query(initial)
            .build();
        list.poll_until_idle().await;
        let rows = list
            .visible_rows()
            .iter()
            .map(|r| r.0.clone())
            .collect::<Vec<_>>();
        let seen = seen.lock().unwrap().clone();
        (rows, seen)
    }

    #[tokio::test]
    async fn resolves_template_before_inner_sees_it() {
        // `<{note}` with note "spec" must reach the inner source as "<spec".
        let (_rows, seen) = run("<{note}", note("work/spec.md"), Unresolvable::Empty).await;
        assert!(
            seen.contains(&"<spec".to_string()),
            "inner should see the resolved query, got {seen:?}"
        );
        assert!(
            !seen.iter().any(|q| q.contains("{note}")),
            "`{{note}}` must never reach the inner source, got {seen:?}"
        );
    }

    #[tokio::test]
    async fn unresolvable_empty_emits_nothing_and_skips_inner() {
        // Purely note-dependent query, no note: Empty policy emits nothing and
        // the inner source is never asked to load.
        let (rows, seen) = run("<", QueryContext::default(), Unresolvable::Empty).await;
        assert!(rows.is_empty(), "expected no rows, got {rows:?}");
        assert!(seen.is_empty(), "inner must not be loaded, got {seen:?}");
    }

    #[tokio::test]
    async fn unresolvable_as_empty_query_delegates_empty_to_inner() {
        // Same query, AsEmptyQuery policy: the inner source IS loaded, with the
        // empty query (its recent-notes fallback).
        let (_rows, seen) = run("<", QueryContext::default(), Unresolvable::AsEmptyQuery).await;
        assert_eq!(seen, vec!["".to_string()], "inner should be loaded with \"\"");
    }

    #[tokio::test]
    async fn mixed_query_with_no_note_still_resolves_and_runs() {
        // `widget <` is not unresolvable (it has a concrete term), so it runs.
        // With no note, `{note}` resolves to an empty target, leaving the bare
        // operator `<` in place — the inner source sees "widget <".
        let (_rows, seen) = run("widget <", QueryContext::default(), Unresolvable::Empty).await;
        assert_eq!(seen, vec!["widget <".to_string()]);
    }
}
