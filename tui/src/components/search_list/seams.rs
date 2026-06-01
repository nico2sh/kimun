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
