//! `ListCore`: the toolkit-free half of a query-over-an-async-loaded list.
//! Rows, the display order (order → local filter → filter-exempt rows), the
//! query-fresh leading row, the selection and its seed/choice split, and the
//! generation-stamped load. A UI composes it with an input, a renderer and
//! hit-testing (the TUI's `SearchList`).

use std::sync::Arc;

use super::load::LoadEngine;
use super::seams::{Filter, ListRow, Loaded, OrderFn, RowSource};

/// Fuzzy-ranks the rows named by `base` (indices into `rows`, in the order
/// ties should keep) against `query`; absent = no match.
fn fuzzy_indices<R: ListRow>(rows: &[R], base: &[usize], query: &str) -> Vec<usize> {
    use nucleo::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo::{Matcher, Utf32Str};
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let pat = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut scored: Vec<(usize, u32)> = base
        .iter()
        .filter_map(|&i| {
            let hay = rows[i].match_text()?;
            let mut buf = Vec::new();
            let h = Utf32Str::new(hay, &mut buf);
            pat.score(h, &mut matcher).map(|s| (i, s))
        })
        .collect();
    // Stable: equal scores keep `base` order, so an `order_by` still decides
    // among ties.
    scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// Every row of `rows` ranked against `query` the way a [`Filter::Fuzzy`]
/// list ranks them: all rows in order for a blank query, else the matches,
/// best first (ties keep row order). For a frontend that filters a small,
/// static row set itself (the GUI's command palette).
pub fn fuzzy_rank<R: ListRow>(rows: &[R], query: &str) -> Vec<usize> {
    let base: Vec<usize> = (0..rows.len()).collect();
    if query.is_empty() {
        base
    } else {
        fuzzy_indices(rows, &base, query)
    }
}

/// How a list is set up before its rows arrive: the local filter, an optional
/// total order, and the opening query.
pub struct ListSetup<R: ListRow> {
    pub filter: Filter<R>,
    pub order: Option<OrderFn<R>>,
    pub query: String,
}

impl<R: ListRow> Default for ListSetup<R> {
    fn default() -> Self {
        Self {
            filter: Filter::SourceOrder,
            order: None,
            query: String::new(),
        }
    }
}

/// What a [`ListCore::poll`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Polled {
    /// Rows arrived (the display was recomputed).
    pub changed: bool,
    /// A newer load delivered its first events and the prior load's rows were
    /// dropped — a UI resets its viewport to the top.
    pub restarted: bool,
}

pub struct ListCore<R: ListRow> {
    source: Arc<dyn RowSource<R>>,
    rows: Vec<R>,
    /// Indices into `rows` in display order (after filtering/ranking).
    display: Vec<usize>,
    /// A synthetic, query-fresh, filter-exempt row pinned at visible position 0
    /// (the `Create: <q>` affordance / saved-searches virtual entry). Held
    /// separately from `rows` so it works regardless of delivery (one-shot
    /// `Replace` or streamed `Push`) and refreshes on every query change. See
    /// [`RowSource::leading_row`].
    leading: Option<R>,
    /// Index into the VISIBLE sequence `[leading?] ++ display` of the selected
    /// item.
    selected: Option<usize>,
    /// Whether `selected` is the user's own choice rather than the seed
    /// `recompute_and_seed` puts on the first row. A chosen selection tracks
    /// its row across a re-sort; a seeded one keeps the top slot, so a streamed
    /// listing highlights the top of the `order_by` order at every frame
    /// instead of whichever row the source happened to emit first.
    selection_pinned: bool,
    filter: Filter<R>,
    /// Optional total order applied before the local filter.
    order: Option<OrderFn<R>>,
    query: String,
    loader: LoadEngine<R>,
    /// Load generation whose rows are currently held. When a newer generation
    /// (a requery / reload) delivers its first event, `poll` clears the stale
    /// rows before applying it — required for streamed (`Push`) sources, which
    /// would otherwise append onto a superseded load's rows.
    applied_generation: u64,
}

/// Whatever keeps a [`ListCore`]: the core itself, for a UI that draws its
/// rows straight from it, or a UI list that wraps one with its own state (the
/// TUI's `SearchList`, with its input and viewport). Lets a feature model such
/// as [`FilesModel`](crate::logic::files::FilesModel) own its listing without
/// knowing which UI draws it.
pub trait ListHolder<R: ListRow> {
    fn core(&self) -> &ListCore<R>;
    fn core_mut(&mut self) -> &mut ListCore<R>;
    /// Apply whatever the current load delivered since the last poll. A
    /// wrapper overrides it to keep its own state in step (the TUI resets its
    /// viewport on [`Polled::restarted`]).
    fn poll(&mut self) -> Polled {
        self.core_mut().poll()
    }
}

impl<R: ListRow> ListHolder<R> for ListCore<R> {
    fn core(&self) -> &ListCore<R> {
        self
    }

    fn core_mut(&mut self) -> &mut ListCore<R> {
        self
    }

    fn poll(&mut self) -> Polled {
        ListCore::poll(self)
    }
}

impl<R: ListRow> ListCore<R> {
    /// The async path: kick off the initial load; rows land on a later
    /// [`poll`](Self::poll). Needs a tokio runtime.
    pub fn new(
        source: Arc<dyn RowSource<R>>,
        redraw: Arc<dyn Fn() + Send + Sync>,
        setup: ListSetup<R>,
    ) -> Self {
        let mut core = Self::idle(source, redraw, setup);
        core.reload();
        core
    }

    /// The synchronous path: apply an in-memory row set and seed the initial
    /// selection here and now — no async load, no channel, no redraw
    /// round-trip. The loader stays idle (`is_loading()` is false immediately),
    /// and the source's `load` is never invoked, so a caller can read
    /// `selected_row()`/`rows()` on the very next line. For static sources
    /// (`reload_on_query() == false`), where the query is a local filter over
    /// the built rows. See [`StaticRowSource`](super::StaticRowSource).
    pub fn with_rows(
        source: Arc<dyn RowSource<R>>,
        redraw: Arc<dyn Fn() + Send + Sync>,
        setup: ListSetup<R>,
        rows: Vec<R>,
    ) -> Self {
        let mut core = Self::idle(source, redraw, setup);
        core.rows = rows;
        core.recompute_and_seed();
        core
    }

    fn idle(
        source: Arc<dyn RowSource<R>>,
        redraw: Arc<dyn Fn() + Send + Sync>,
        setup: ListSetup<R>,
    ) -> Self {
        Self {
            source,
            rows: Vec::new(),
            display: Vec::new(),
            leading: None,
            selected: None,
            selection_pinned: false,
            filter: setup.filter,
            order: setup.order,
            query: setup.query,
            loader: LoadEngine::new(redraw),
            applied_generation: 0,
        }
    }

    /// Apply whatever the current load delivered since the last poll.
    pub fn poll(&mut self) -> Polled {
        let drained = self.loader.drain();
        let mut out = Polled::default();
        if drained.is_empty() {
            return out;
        }
        out.changed = true;
        // A newer load delivered its first event(s): drop the prior load's
        // rows so a streamed source starts from a clean slate (one-shot
        // `Replace` overwrites anyway, but `Push` would otherwise append).
        let current_gen = self.loader.generation();
        if current_gen != self.applied_generation {
            self.rows.clear();
            self.selected = None;
            self.selection_pinned = false;
            self.applied_generation = current_gen;
            out.restarted = true;
        }
        // Pushes only append, so row indices stay valid across the drain
        // and the selection can follow its row wherever `order` puts it.
        // A `Replace` swaps the whole set; indices mean nothing after it.
        let keep = if drained.iter().any(|ev| matches!(ev, Loaded::Replace(_))) {
            None
        } else {
            self.selection_to_carry()
        };
        for ev in drained {
            match ev {
                Loaded::Replace(rows) => {
                    self.rows = rows;
                }
                Loaded::Push(row) => {
                    self.rows.push(row);
                }
                Loaded::Done => {}
            }
        }
        self.recompute_and_seed();
        self.reselect(keep);
        out
    }

    /// Recompute the display order, then seed the selection to the first row
    /// when nothing is selected yet (e.g. after the first load or a filter that
    /// repopulated the list). The single place display + initial selection are
    /// brought in sync.
    fn recompute_and_seed(&mut self) {
        self.recompute_display();
        if self.selected.is_none() && self.visible_len() > 0 {
            self.selected = Some(0);
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

    /// Number of real matches — the visible rows minus the synthetic leading
    /// affordance ("Create: …"), for result-count displays.
    pub fn match_count(&self) -> usize {
        self.display.len()
    }

    /// Row at visible position `pos` in `[leading?] ++ display`.
    pub fn visible_row(&self, pos: usize) -> Option<&R> {
        if self.leading.is_some() && pos == 0 {
            self.leading.as_ref()
        } else {
            self.rows
                .get(*self.display.get(pos - self.leading_offset())?)
        }
    }

    /// The query-fresh leading row, if the source offers one for the current
    /// query.
    pub fn leading(&self) -> Option<&R> {
        self.leading.as_ref()
    }

    /// The source-delivered rows only (NOT the leading row). Prefer
    /// [`visible_len`](Self::visible_len)/[`visible_rows`](Self::visible_rows)
    /// for visible counts.
    pub fn rows(&self) -> &[R] {
        &self.rows
    }

    /// Visible position of the selection.
    pub fn selected(&self) -> Option<usize> {
        self.selected
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

    pub fn is_loading(&self) -> bool {
        self.loader.loading
    }

    /// Set the query and start a load (for `reload_on_query` sources) or
    /// recompute the display. The generation guard in the loader drops stale
    /// results.
    pub fn set_query(&mut self, q: impl Into<String>) {
        self.query = q.into();
        if self.source.reload_on_query() {
            self.loader.start(self.source.clone(), self.query.clone());
        }
        // Recompute now so the query-fresh leading row (and local filter, for
        // non-reload sources) reflect the new query in this frame. Reload
        // sources refresh again when their load drains in poll().
        self.recompute_and_seed();
    }

    /// Re-run the source load for the current query (e.g. after a mutation).
    pub fn reload(&mut self) {
        self.loader.start(self.source.clone(), self.query.clone());
    }

    /// Replace (or clear) the row order and re-sort the rows already loaded.
    /// A recompute, not a reload: the source is not consulted. A selection the
    /// user chose stays on the row it was on; a seeded one keeps the top slot
    /// of the NEW order — and stays a seed, so a still-streaming listing goes
    /// on re-seeding it (the `selection_pinned` split, as in `poll`).
    pub fn set_order(&mut self, cmp: Option<OrderFn<R>>) {
        let keep = self.selection_to_carry();
        self.order = cmp;
        self.recompute_display();
        self.reselect(keep);
    }

    /// The row a recompute has to put the selection back on: the row the user
    /// chose, or `None` for a seed — a seed belongs to the top of the order,
    /// wherever the recompute puts it. The one rule behind `poll`, `set_order`
    /// and `update_rows`.
    fn selection_to_carry(&self) -> Option<usize> {
        self.selection_pinned
            .then(|| self.selected_row_index())
            .flatten()
    }

    /// Index into `rows` of the selected row, if the selection is on a real
    /// (non-leading) row.
    fn selected_row_index(&self) -> Option<usize> {
        self.row_index_at(self.selected?)
    }

    /// Move the selection back onto the row at `rows[row]` after the display
    /// was recomputed, if that row is still visible; otherwise leave it where
    /// the recompute clamped it.
    fn reselect(&mut self, row: Option<usize>) {
        if let Some(row) = row
            && let Some(pos) = self.display.iter().position(|&i| i == row)
        {
            self.selected = Some(pos + self.leading_offset());
        }
    }

    /// Mutate rows in place. `mutate` is called for each row and returns `true`
    /// for each row it changed; if any did, the display is recomputed — which
    /// re-filters AND re-sorts, so an edit to a field the `order_by` reads
    /// moves its row. Returns whether anything changed.
    ///
    /// The selection is carried the same way every other recompute carries it:
    /// a chosen row stays chosen wherever it lands, a seed keeps the top slot.
    ///
    /// With [`replace_rows`](Self::replace_rows),
    /// [`swap_visible`](Self::swap_visible) and
    /// [`remove_visible`](Self::remove_visible), the seams that touch rows
    /// outside the [`RowSource`]; every other change rebuilds from the source.
    /// A reorder by a field is [`set_order`](Self::set_order).
    /// The list stays ignorant of the row type; callers layer the
    /// path-matched operations on top.
    pub fn update_rows(&mut self, mut mutate: impl FnMut(&mut R) -> bool) -> bool {
        let mut changed = false;
        for row in &mut self.rows {
            if mutate(row) {
                changed = true;
            }
        }
        if changed {
            let keep = self.selection_to_carry();
            self.recompute_display();
            self.reselect(keep);
        }
        changed
    }

    /// Apply a row set read outside the loader (a list that runs its own
    /// read, the GUI's pinned notes), here and now: as a one-shot `Replace`
    /// landing — the display recomputed, the selection kept at its position
    /// (clamped), or seeded on the first row when there was none. A load
    /// under way is not stopped.
    pub fn replace_rows(&mut self, rows: Vec<R>) {
        self.rows = rows;
        self.recompute_and_seed();
    }

    /// Swap the rows at visible positions `a` and `b`: a reorder the caller
    /// is writing through, shown before it lands. A selection on either row
    /// follows it. Only a source-ordered list keeps the swap — an `order_by`
    /// or a filter puts the rows back where it says. False (nothing done)
    /// when either position is not a real row (past the end, the leading
    /// row).
    pub fn swap_visible(&mut self, a: usize, b: usize) -> bool {
        let (Some(ia), Some(ib)) = (self.row_index_at(a), self.row_index_at(b)) else {
            return false;
        };
        let follow = self.selected_row_index().map(|i| match i {
            i if i == ia => ib,
            i if i == ib => ia,
            i => i,
        });
        self.rows.swap(ia, ib);
        self.recompute_display();
        self.reselect(follow);
        true
    }

    /// Remove the row at visible position `pos` and return it: a delete the
    /// caller is writing through, shown before it lands. The selection keeps
    /// its position, clamped to the rows left. `None` (nothing done) when
    /// `pos` is not a real row.
    pub fn remove_visible(&mut self, pos: usize) -> Option<R> {
        let index = self.row_index_at(pos)?;
        let row = self.rows.remove(index);
        self.recompute_display();
        Some(row)
    }

    /// Index into `rows` of the real (non-leading) row at visible `pos`.
    fn row_index_at(&self, pos: usize) -> Option<usize> {
        let pos = pos.checked_sub(self.leading_offset())?;
        self.display.get(pos).copied()
    }

    /// Select the visible row at `pos` (clamped to the visible range); clears
    /// the selection when the list is empty. The index-based counterpart to
    /// [`select_next`](Self::select_next)/[`select_prev`](Self::select_prev),
    /// for surfaces that point the cursor at a specific row (the Sources view's
    /// citation jump, a click).
    pub fn select(&mut self, pos: usize) {
        let n = self.visible_len();
        self.selected = if n == 0 { None } else { Some(pos.min(n - 1)) };
        // Unconditional: aiming at a row IS the choice, even when it happens
        // to be the row the seed already sat on. Only the blind nudges below
        // have to prove they moved.
        self.selection_pinned = self.selected.is_some();
    }

    pub fn select_next(&mut self) {
        let n = self.visible_len();
        if n == 0 {
            return;
        }
        self.move_selection(Some(self.selected.map_or(0, |i| (i + 1).min(n - 1))));
    }

    pub fn select_prev(&mut self) {
        if self.visible_len() == 0 {
            return;
        }
        self.move_selection(Some(self.selected.map_or(0, |i| i.saturating_sub(1))));
    }

    /// Move the selection, marking it the user's own choice only when it
    /// actually lands somewhere new. A nudge against either end of the list
    /// changes nothing, so it must not turn a seed into a choice — one stray
    /// `Up` on the first row of a streaming listing would otherwise freeze the
    /// highlight at position 0 and let rows arriving above it walk underneath.
    pub fn move_selection(&mut self, next: Option<usize>) {
        if next != self.selected {
            self.selected = next;
            self.selection_pinned = next.is_some();
        }
    }

    fn recompute_display(&mut self) {
        let q = self.query.trim();
        // The leading row is query-fresh: rebuilt on every poll AND on every
        // local-filter `set_query`, so it never goes stale.
        self.leading = self.source.leading_row(q);
        // Source order, or `order_by` order when one is set.
        let mut base: Vec<usize> = (0..self.rows.len()).collect();
        if let Some(cmp) = &self.order {
            let rows = &self.rows;
            base.sort_by(|&a, &b| cmp(&rows[a], &rows[b]));
        }
        let mut idx: Vec<usize> = match &self.filter {
            Filter::SourceOrder => base,
            Filter::Fuzzy if q.is_empty() => base,
            Filter::Fuzzy => fuzzy_indices(&self.rows, &base, q),
            Filter::Rank(_) if q.is_empty() => base,
            Filter::Rank(f) => {
                let f = f.clone();
                f(&self.rows, &base, q)
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

    /// Poll until the current load settles. See the TUI `SearchList`'s
    /// counterpart for why it sleeps.
    #[cfg(test)]
    pub(crate) async fn poll_until_idle(&mut self) {
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

#[cfg(test)]
mod tests {
    //! Engine-only tests: the half a non-terminal UI reuses. The TUI's
    //! `SearchList` tests exercise the same rules through keys and the mouse.
    use super::*;
    use crate::logic::list::{Emit, StaticRowSource};
    use async_trait::async_trait;

    #[derive(Clone, Debug, PartialEq)]
    enum Row {
        Item(&'static str),
        Up,
    }

    impl ListRow for Row {
        fn match_text(&self) -> Option<&str> {
            match self {
                Row::Item(s) => Some(s),
                Row::Up => None,
            }
        }
    }

    fn noop() -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }

    fn fuzzy() -> ListSetup<Row> {
        ListSetup {
            filter: Filter::Fuzzy,
            ..ListSetup::default()
        }
    }

    /// Streams `items` one row at a time, offers `Create:` as a leading row.
    struct Streamed(Vec<&'static str>);

    #[async_trait]
    impl RowSource<Row> for Streamed {
        async fn load(&self, _query: &str, emit: Emit<Row>) {
            for s in &self.0 {
                emit.push(Row::Item(s));
            }
            emit.done();
        }
        fn leading_row(&self, query: &str) -> Option<Row> {
            (!query.is_empty()).then_some(Row::Item("create"))
        }
        fn reload_on_query(&self) -> bool {
            false
        }
    }

    #[test]
    fn fuzzy_filter_keeps_filter_exempt_rows() {
        let mut core = ListCore::with_rows(
            Arc::new(StaticRowSource),
            noop(),
            fuzzy(),
            vec![Row::Item("alpha"), Row::Up, Row::Item("beta")],
        );
        assert_eq!(core.visible_len(), 3);
        core.set_query("alp");
        assert_eq!(core.visible_rows(), vec![&Row::Up, &Row::Item("alpha")]);
        assert_eq!(core.match_count(), 2);
    }

    #[tokio::test]
    async fn streamed_rows_land_in_order_with_a_query_fresh_leading_row() {
        let order: OrderFn<Row> = Arc::new(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        let mut core = ListCore::new(
            Arc::new(Streamed(vec!["zeta", "alpha"])),
            noop(),
            ListSetup {
                order: Some(order),
                ..fuzzy()
            },
        );
        core.poll_until_idle().await;
        assert_eq!(
            core.visible_rows(),
            vec![&Row::Item("alpha"), &Row::Item("zeta")]
        );
        // The seed sits on the top of the order, not the first row emitted.
        assert_eq!(core.selected_row(), Some(&Row::Item("alpha")));
        core.set_query("zet");
        assert_eq!(core.leading(), Some(&Row::Item("create")));
        assert_eq!(
            core.visible_rows(),
            vec![&Row::Item("create"), &Row::Item("zeta")]
        );
    }

    fn items(names: &[&'static str]) -> Vec<Row> {
        names.iter().map(|&n| Row::Item(n)).collect()
    }

    fn listed(core: &ListCore<Row>) -> Vec<&Row> {
        core.visible_rows()
    }

    #[test]
    fn replaced_rows_keep_the_selection_where_it_was_clamped() {
        let mut core = ListCore::with_rows(
            Arc::new(StaticRowSource),
            noop(),
            ListSetup::default(),
            items(&["a", "b", "c"]),
        );
        core.select(2);
        core.replace_rows(items(&["x", "y", "z"]));
        assert_eq!(core.selected_row(), Some(&Row::Item("z")), "same position");
        core.replace_rows(items(&["x"]));
        assert_eq!(core.selected(), Some(0), "clamped");
        core.replace_rows(Vec::new());
        assert_eq!(core.selected(), None);
        core.replace_rows(items(&["p", "q"]));
        assert_eq!(core.selected(), Some(0), "seeded again");
    }

    #[test]
    fn a_swap_moves_the_selected_row_and_the_cursor_follows() {
        let mut core = ListCore::with_rows(
            Arc::new(StaticRowSource),
            noop(),
            ListSetup::default(),
            items(&["a", "b", "c"]),
        );
        core.select(1);
        assert!(core.swap_visible(1, 2));
        assert_eq!(
            listed(&core),
            items(&["a", "c", "b"]).iter().collect::<Vec<_>>()
        );
        assert_eq!(core.selected_row(), Some(&Row::Item("b")));
        assert_eq!(core.selected(), Some(2));
        // A swap the selection is not on leaves it in place.
        assert!(core.swap_visible(0, 1));
        assert_eq!(core.selected_row(), Some(&Row::Item("b")));
        // Off the end: nothing.
        assert!(!core.swap_visible(2, 3));
        assert_eq!(
            listed(&core),
            items(&["c", "a", "b"]).iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_removed_row_goes_and_the_cursor_clamps() {
        let mut core = ListCore::with_rows(
            Arc::new(StaticRowSource),
            noop(),
            ListSetup::default(),
            items(&["a", "b"]),
        );
        core.select(1);
        assert_eq!(core.remove_visible(1), Some(Row::Item("b")));
        assert_eq!(core.selected_row(), Some(&Row::Item("a")), "clamped");
        assert_eq!(core.remove_visible(3), None);
        assert_eq!(core.remove_visible(0), Some(Row::Item("a")));
        assert_eq!(core.selected(), None);
        assert_eq!(core.visible_len(), 0);
    }

    #[tokio::test]
    async fn a_reload_reports_a_restart_and_drops_the_old_rows() {
        let mut core = ListCore::new(Arc::new(Streamed(vec!["a", "b"])), noop(), fuzzy());
        core.poll_until_idle().await;
        core.select(1);
        core.reload();
        let mut restarted = false;
        for _ in 0..600 {
            tokio::task::yield_now().await;
            let p = core.poll();
            restarted |= p.restarted;
            if !core.is_loading() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        assert!(restarted);
        assert_eq!(core.rows().len(), 2, "the reload must not append");
    }
}
