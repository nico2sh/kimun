//! The seams a `SearchList` varies across (see CONTEXT.md: SearchList, Row
//! source, Search row, Suggestion source). Everything else is folded into the
//! engine.

use std::sync::Arc;

use async_trait::async_trait;
use ratatui::widgets::ListItem;

use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// What a single row must tell its `SearchList` to be listed, filtered,
/// navigated and drawn. The only thing that varies with the row's type.
pub trait SearchRow: Clone + Send + Sync + 'static {
    /// Collapsed one-or-few-line rendering. `selected` lets a row self-style.
    fn to_list_item(&self, theme: &Theme, icons: &Icons, selected: bool) -> ListItem<'static>;

    /// Terminal rows this collapsed item occupies (mouse hit-testing / scroll).
    fn visual_height(&self) -> u16 {
        1
    }

    /// Haystack a LOCAL filter (`Filter::Fuzzy`/`Rank`) matches against.
    /// `None` => never removed by a local filter (e.g. an "Up .." / "Create"
    /// / pinned virtual row); ignored entirely by `Filter::SourceOrder`.
    fn match_text(&self) -> Option<&str> {
        None
    }
}

/// How rows arrive from a source. One-shot sources send one `Replace`;
/// streamed sources send many `Push` then `Done`.
pub enum Loaded<R> {
    Replace(Vec<R>),
    Push(R),
    Done,
}

/// How a loaded row set is narrowed/ordered for display. Three known
/// strategies; none need test substitution, so folded in here.
pub enum Filter<R: SearchRow> {
    /// Trust the source's order (server-side filter already applied).
    SourceOrder,
    /// Local nucleo fuzzy over `match_text`.
    Fuzzy,
    /// Local rank: `(rows, query) -> display indices` (lower = better; absent = hidden).
    Rank(std::sync::Arc<dyn Fn(&[R], &str) -> Vec<usize> + Send + Sync>),
}

/// The sink a `RowSource` writes rows into. Cheap to clone; carries the load
/// generation so the engine can drop results from a superseded load.
#[derive(Clone)]
pub struct Emit<R> {
    tx: std::sync::mpsc::Sender<(u64, Loaded<R>)>,
    generation: u64,
    redraw: Arc<dyn Fn() + Send + Sync>,
}

impl<R> Emit<R> {
    pub(super) fn new(
        tx: std::sync::mpsc::Sender<(u64, Loaded<R>)>,
        generation: u64,
        redraw: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self { tx, generation, redraw }
    }

    /// One-shot: deliver the whole set.
    pub fn replace(&self, rows: Vec<R>) {
        let _ = self.tx.send((self.generation, Loaded::Replace(rows)));
        (self.redraw)();
    }

    /// Streamed: one row at a time.
    pub fn push(&self, row: R) {
        let _ = self.tx.send((self.generation, Loaded::Push(row)));
        (self.redraw)();
    }

    /// Streamed: no more rows for this generation.
    pub fn done(&self) {
        let _ = self.tx.send((self.generation, Loaded::Done));
        (self.redraw)();
    }
}

/// One autocomplete candidate: the inserted/display text plus an optional
/// secondary line shown muted in the popup (a note path, a tag usage count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionItem {
    pub display: String,
    pub secondary: Option<String>,
}

impl SuggestionItem {
    pub fn plain(display: impl Into<String>) -> Self {
        Self { display: display.into(), secondary: None }
    }
}

/// Autocomplete candidates for the query input, kept separate from the vault
/// so the autocomplete host is testable in isolation.
#[async_trait]
pub trait SuggestionSource: Send + Sync + 'static {
    async fn notes_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem>;
    async fn tags_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem>;
}

/// Production adapter over the vault. Formats the secondary line (note path,
/// tag usage count) so the popup looks exactly as before.
pub struct VaultSuggestions {
    pub vault: std::sync::Arc<kimun_core::NoteVault>,
}

#[async_trait]
impl SuggestionSource for VaultSuggestions {
    async fn notes_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem> {
        self.vault
            .suggest_notes_by_prefix(prefix, limit)
            .await
            .map(|v| v.into_iter().map(|n| SuggestionItem {
                display: n.name,
                secondary: Some(n.path.to_string()),
            }).collect())
            .unwrap_or_default()
    }
    async fn tags_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem> {
        self.vault
            .suggest_tags_by_prefix(prefix, limit)
            .await
            .map(|v| v.into_iter().map(|t| SuggestionItem {
                display: t.label,
                secondary: Some(format!("{}×", t.usage_count)),
            }).collect())
            .unwrap_or_default()
    }
}

/// Where a `SearchList`'s rows come from. Vault-backed in the app, in-memory
/// in tests. Streaming vs one-shot is a delivery detail of the SAME seam.
#[async_trait]
pub trait RowSource<R: SearchRow>: Send + Sync + 'static {
    /// Called on construction and on every committed query change. Empty query
    /// = initial state. Write rows into `emit`. Cancel-safe: the engine drops
    /// the prior load on requery, so a slow source may be left unfinished.
    async fn load(&self, query: &str, emit: Emit<R>);

    /// An optional synthetic leading row (the "Create: <q>" affordance),
    /// prepended and exempt from local filtering. Keeps create-policy here.
    fn leading_row(&self, _query: &str) -> Option<R> {
        None
    }

    /// `true` (default): `load` is re-run on every query keystroke (server-side
    /// filter). `false`: `load` runs once with `""`, then a local `Filter`
    /// narrows the set per keystroke.
    fn reload_on_query(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod suggestion_tests {
    use super::*;
    struct Mem { notes: Vec<SuggestionItem>, tags: Vec<SuggestionItem> }
    #[async_trait]
    impl SuggestionSource for Mem {
        async fn notes_by_prefix(&self, p: &str, _n: usize) -> Vec<SuggestionItem> {
            self.notes.iter().filter(|x| x.display.starts_with(p)).cloned().collect()
        }
        async fn tags_by_prefix(&self, p: &str, _n: usize) -> Vec<SuggestionItem> {
            self.tags.iter().filter(|x| x.display.starts_with(p)).cloned().collect()
        }
    }
    #[tokio::test]
    async fn mem_suggestions_filter_by_prefix() {
        let m = Mem {
            notes: vec![SuggestionItem { display: "projects".into(), secondary: Some("work/projects".into()) }],
            tags: vec![SuggestionItem::plain("todo")],
        };
        assert_eq!(m.notes_by_prefix("pro", 9).await.len(), 1);
        assert_eq!(m.notes_by_prefix("pro", 9).await[0].display, "projects");
        assert_eq!(m.tags_by_prefix("to", 9).await[0].display, "todo");
    }
}
