//! The seams a list engine varies across (see CONTEXT.md: SearchList, Row
//! source, Search row). Toolkit-free: how a row is *drawn* is the UI layer's
//! extension of [`ListRow`] (the TUI's `SearchRow`).

use std::sync::Arc;

use async_trait::async_trait;

/// What a single row must tell a [`ListCore`](super::ListCore) to be listed,
/// filtered, navigated and yanked. Drawing is not here: each UI layers its own
/// renderer trait on top (the TUI's `SearchRow` adds `to_list_item` and
/// `visual_height`).
pub trait ListRow: Clone + Send + Sync + 'static {
    /// Haystack a LOCAL filter (`Filter::Fuzzy`/`Rank`) matches against.
    /// `None` => never removed by a local filter (e.g. an "Up .." / "Create"
    /// / pinned virtual row); ignored entirely by `Filter::SourceOrder`.
    fn match_text(&self) -> Option<&str> {
        None
    }

    /// What this row offers to the OS clipboard, or `None` when it has nothing
    /// worth copying (a command entry, a virtual "Up .." row).
    ///
    /// Declared by the row rather than by the surface displaying it, so every
    /// list built on the engine inherits the yank instead of each panel wiring
    /// its own key — which is how the note browser ended up without one while
    /// the Query panel had it.
    fn yank_target(&self) -> Option<YankTarget> {
        None
    }
}

/// A row's contribution to the OS clipboard: the text, plus the noun naming it
/// so the confirmation says *what* was copied ("path copied", "tag copied")
/// rather than a bare "copied" that would be a lie where nothing was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YankTarget {
    pub text: String,
    pub noun: &'static str,
}

impl YankTarget {
    pub fn new(text: impl Into<String>, noun: &'static str) -> Self {
        Self {
            text: text.into(),
            noun,
        }
    }

    /// The overwhelmingly common case: a row that stands for a note.
    pub fn path(text: impl Into<String>) -> Self {
        Self::new(text, "path")
    }
}

/// How rows arrive from a source. One-shot sources send one `Replace`;
/// streamed sources send many `Push` then `Done`.
pub enum Loaded<R> {
    Replace(Vec<R>),
    Push(R),
    Done,
}

/// Ranking function for `Filter::Rank`: takes the full row slice, the candidate
/// indices into it (`base` — already in `order_by` order, so ranking may fall
/// back on it for ties), and the current query string; returns display indices
/// in preferred order (absent = hidden).
pub type RankFn<R> = std::sync::Arc<dyn Fn(&[R], &[usize], &str) -> Vec<usize> + Send + Sync>;

/// Total order over rows for a list's `order_by`: applied to the row set before
/// any local filter, so streamed rows land in place as they arrive and a sort
/// change is a recompute, not a reload.
pub type OrderFn<R> = std::sync::Arc<dyn Fn(&R, &R) -> std::cmp::Ordering + Send + Sync>;

/// How a loaded row set is narrowed/ordered for display. Three known
/// strategies; none need test substitution, so folded in here.
pub enum Filter<R: ListRow> {
    /// Trust the source's order (server-side filter already applied).
    SourceOrder,
    /// Local nucleo fuzzy over `match_text`.
    Fuzzy,
    /// Local rank: `(rows, base, query) -> display indices` (lower = better;
    /// absent = hidden). `base` carries the `order_by` order, so honoring it
    /// for equally-ranked rows is what makes the two compose.
    Rank(RankFn<R>),
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
        Self {
            tx,
            generation,
            redraw,
        }
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

    /// Test sink: an `Emit` whose every event lands on the returned receiver,
    /// so a source's delivery sequence can be asserted without an engine.
    #[cfg(test)]
    pub(crate) fn capture() -> (Self, std::sync::mpsc::Receiver<(u64, Loaded<R>)>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self::new(tx, 0, Arc::new(|| {})), rx)
    }
}

/// Where a list's rows come from. Vault-backed in the app, in-memory in tests.
/// Streaming vs one-shot is a delivery detail of the SAME seam.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait RowSource<R: ListRow>: Send + Sync + 'static {
    /// Called on construction and on every committed query change. Empty query
    /// = initial state. Write rows into `emit`. Cancel-safe: the engine drops
    /// the prior load on requery, so a slow source may be left unfinished.
    async fn load(&self, query: &str, emit: Emit<R>);

    /// An optional synthetic leading row (the `Create: <q>` affordance),
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

/// A [`RowSource`] for the synchronous build path: its rows are supplied at
/// build time via [`ListCore::with_rows`](super::ListCore::with_rows) (the
/// TUI's `SearchListBuilder::build_with_rows`), so its async `load` is never
/// called. Pairs with any static, in-memory row set
/// (`reload_on_query() == false` — the query is a local filter over the built
/// rows), replacing a hand-rolled one-shot `emit.replace(rows.clone())` source.
pub struct StaticRowSource;

#[async_trait]
impl<R: ListRow> RowSource<R> for StaticRowSource {
    async fn load(&self, _query: &str, _emit: Emit<R>) {}
    fn reload_on_query(&self) -> bool {
        false
    }
}
