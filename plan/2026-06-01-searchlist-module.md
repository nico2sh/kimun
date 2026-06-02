# SearchList Module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the four hand-rolled "query input + async-loaded, keyboard-navigable note list" surfaces (note browser, saved-searches modal, Query panel, directory sidebar) onto one deep `SearchList` module behind three seams, so the input/nav/async/autocomplete plumbing lives — and is tested — in one place.

**Architecture:** A generic `SearchList<R: SearchRow>` owns the query input, keyboard navigation, the async-load lifecycle (local channel + poll, generation-stamped to kill races — *not* the global `AppEvent` bus), the autocomplete host, and selection. Three seams vary per call site: `RowSource<R>` (where rows come from), `SuggestionSource` (autocomplete candidates, so the host is testable vault-free), and a folded-in `Filter` enum (SourceOrder / Fuzzy / Rank). Heavy presentation (Query panel's 3-state expand + preview, note browser's preview pane) **composes on top** — the caller pre-registers intercepted keys (`.intercept`) and reads `selected_row()`; `SearchList` emits nothing global.

**Tech Stack:** Rust, ratatui (TUI), tokio (async), nucleo (fuzzy matching), async-trait. Crate: `kimun-notes` (in `tui/`). No `kimun_core` changes.

**Design source:** the architecture review grilling (this branch's `CONTEXT.md` adds `SearchList` / `Row source` / `Search row` / `Suggestion source`). Glossary: `CONTEXT.md`. The four call sites today: `note_browser/mod.rs`, `saved_searches_modal.rs`, `backlinks_panel.rs` (QueryPanel), `sidebar.rs` + `file_list.rs`.

**Branch:** `refactor/searchlist-module` (off `feat/sidebar-saved-queries`; the refactor depends on QueryPanel / saved-searches / the `>` autocomplete trigger that only exist there).

**Verification note:** `app_screen/*` tests live in the BINARY target. Use `cargo test -p kimun-notes` (whole package) — not `--lib` — when an editor/app_screen test is involved. Component tests under `components/` run under `--lib`.

---

## File Structure

**Create** (new module `tui/src/components/search_list/`):
- `search_list/mod.rs` — `SearchList<R>` engine + `SearchListBuilder` + `KeyReaction`/`SearchMouse`. Re-exports the seams.
- `search_list/seams.rs` — `SearchRow`, `RowSource`, `Emit`, `Loaded`, `SuggestionSource`, `Filter`.
- `search_list/load.rs` — async-load lifecycle (generation-stamped channel + poll), shared by one-shot and streamed delivery.
- `search_list/host.rs` — the single canonical `SearchBoxHostSnapshot` (the duplicate that lived in note_browser + backlinks_panel).
- `search_list/adapters.rs` — `#[cfg(test)]` in-memory `VecSource`, `ScriptedStreamSource`, `VecSuggestions` test adapters.

**Modify (incrementally, one call site per phase):**
- `autocomplete/controller.rs` — `AutocompleteController::new` takes `Arc<dyn SuggestionSource>` instead of `Arc<NoteVault>`; `fire_query`/`link_filter_suggestions` call the port.
- `note_browser/mod.rs` — becomes a thin `SearchList` host + preview pane; providers become `RowSource`s; delete its `SearchBoxHostSnapshot`.
- `saved_searches_modal.rs` — `rank_items` → `Filter::Rank`; virtual row → `RowSource::leading_row`; host the engine.
- `backlinks_panel.rs` (QueryPanel) — host the engine via `.intercept`; delete its `SearchBoxHostSnapshot`; drop `AppEvent::BacklinksLoaded`.
- `sidebar.rs` + `file_list.rs` — `FileListComponent` splits: list/nav engine absorbed into `SearchList`, nucleo → `Filter::Fuzzy`, `FileListEntry: SearchRow`; streamed delivery.
- `components/mod.rs` — `pub mod search_list;`.

---

## PHASE 1 — The engine + seams (TDD, vault-free)

> Build the whole module against in-memory adapters first. The interface is the test surface; no vault, no `AppTx`, no global events.

### Task 1: Seams — `SearchRow`, `Loaded`, `Emit`, `RowSource`

**Files:**
- Create: `tui/src/components/search_list/seams.rs`
- Create: `tui/src/components/search_list/mod.rs` (module wiring only this task)
- Modify: `tui/src/components/mod.rs` (`pub mod search_list;`)

- [ ] **Step 1: Write the seams**

`tui/src/components/search_list/seams.rs`:

```rust
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
```

`tui/src/components/search_list/mod.rs`:

```rust
//! `SearchList`: the one module behind every query-input-over-an-async-loaded
//! list surface in the TUI. See CONTEXT.md.

mod seams;

pub use seams::{Emit, Loaded, RowSource, SearchRow};
```

Add to `tui/src/components/mod.rs` (alongside the other `pub mod`s):

```rust
pub mod search_list;
```

- [ ] **Step 2: Build**

Run: `cargo build -p kimun-notes`
Expected: compiles (unused-warnings on the new items are fine).

- [ ] **Step 3: Commit**

```bash
git add tui/src/components/search_list/ tui/src/components/mod.rs
git commit -m "feat(tui): SearchList seams — SearchRow, RowSource, Emit, Loaded"
```

### Task 2: Engine skeleton + one-shot load lifecycle

**Files:**
- Create: `tui/src/components/search_list/load.rs`
- Create: `tui/src/components/search_list/adapters.rs` (test adapters)
- Modify: `tui/src/components/search_list/mod.rs`

- [ ] **Step 1: Write the test adapter + failing test**

`tui/src/components/search_list/adapters.rs`:

```rust
//! In-memory adapters for testing `SearchList` without a vault.
#![cfg(test)]

use async_trait::async_trait;

use super::seams::{Emit, RowSource, SearchRow};

/// A row that is just a name. Enough to test the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct TestRow {
    pub name: String,
}

impl TestRow {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

impl SearchRow for TestRow {
    fn to_list_item(
        &self,
        _t: &crate::settings::themes::Theme,
        _i: &crate::settings::icons::Icons,
        _sel: bool,
    ) -> ratatui::widgets::ListItem<'static> {
        ratatui::widgets::ListItem::new(self.name.clone())
    }
    fn match_text(&self) -> Option<&str> {
        Some(&self.name)
    }
}

/// One-shot source: returns rows whose name contains the query (server-side
/// filter analogue), or all rows for an empty query.
pub struct VecSource {
    pub rows: Vec<TestRow>,
    pub reload: bool,
}

#[async_trait]
impl RowSource<TestRow> for VecSource {
    async fn load(&self, query: &str, emit: Emit<TestRow>) {
        let out: Vec<TestRow> = if self.reload && !query.is_empty() {
            self.rows.iter().filter(|r| r.name.contains(query)).cloned().collect()
        } else {
            self.rows.clone()
        };
        emit.replace(out);
    }
    fn reload_on_query(&self) -> bool {
        self.reload
    }
}
```

Add a test module to `mod.rs` (engine not built yet — this is the red):

```rust
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
        list.poll_until_idle().await; // test helper: await the spawned load + drain
        assert_eq!(list.rows().len(), 2);
        assert_eq!(list.selected_row().map(|r| r.name.as_str()), Some("alpha"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun-notes --lib search_list::tests::initial_load_populates_rows`
Expected: FAIL — `SearchList`/`builder`/`poll_until_idle` not found.

- [ ] **Step 3: Implement the load lifecycle + engine skeleton**

`tui/src/components/search_list/load.rs`:

```rust
//! Generation-stamped async-load lifecycle shared by one-shot and streamed
//! delivery. A new load bumps the generation and aborts the prior task;
//! `poll` discards any results stamped with a stale generation.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use super::seams::{Emit, Loaded, RowSource, SearchRow};

pub(super) struct LoadEngine<R: SearchRow> {
    generation: u64,
    rx: Receiver<(u64, Loaded<R>)>,
    tx: Sender<(u64, Loaded<R>)>,
    task: Option<tokio::task::JoinHandle<()>>,
    redraw: Arc<dyn Fn() + Send + Sync>,
    pub(super) loading: bool,
}

impl<R: SearchRow> LoadEngine<R> {
    pub(super) fn new(redraw: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = channel();
        Self { generation: 0, rx, tx, task: None, redraw, loading: false }
    }

    /// Start a fresh load for `query`, aborting any in-flight one.
    pub(super) fn start(&mut self, source: Arc<dyn RowSource<R>>, query: String) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        self.generation += 1;
        self.loading = true;
        let emit = Emit::new(self.tx.clone(), self.generation, self.redraw.clone());
        self.task = Some(tokio::spawn(async move {
            source.load(&query, emit).await;
        }));
    }

    /// Drain ready results for the CURRENT generation. Returns the batch of
    /// `Loaded` events (caller applies them to its row set). Stale-generation
    /// events are dropped. Sets `loading=false` on `Done` or one-shot `Replace`.
    pub(super) fn drain(&mut self) -> Vec<Loaded<R>> {
        let mut out = Vec::new();
        while let Ok((gen, ev)) = self.rx.try_recv() {
            if gen != self.generation {
                continue; // superseded
            }
            match &ev {
                Loaded::Replace(_) | Loaded::Done => self.loading = false,
                Loaded::Push(_) => {}
            }
            out.push(ev);
        }
        out
    }
}
```

In `mod.rs`, add the engine. (This task implements construction + one-shot load + `rows`/`selected_row` + the test helper; nav/filter/render/autocomplete come in later tasks.)

```rust
mod adapters;
mod load;
mod seams;

use std::sync::Arc;

pub use seams::{Emit, Loaded, RowSource, SearchRow};

use load::LoadEngine;

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
    pub fn builder(
        source: impl RowSource<R>,
        redraw: Arc<dyn Fn() + Send + Sync>,
    ) -> SearchListBuilder<R> {
        SearchListBuilder { source: Arc::new(source), redraw, initial_query: String::new() }
    }

    fn new(b: SearchListBuilder<R>) -> Self {
        let mut loader = LoadEngine::new(b.redraw.clone());
        loader.start(b.source.clone(), b.initial_query.clone());
        Self { source: b.source, rows: Vec::new(), selected: None, query: b.initial_query, loader }
    }

    /// Drain async results into the row set. Call once per frame before render.
    pub fn poll(&mut self) {
        for ev in self.loader.drain() {
            match ev {
                Loaded::Replace(rows) => {
                    self.rows = rows;
                    self.clamp_selection();
                }
                Loaded::Push(row) => {
                    self.rows.push(row);
                    if self.selected.is_none() && !self.rows.is_empty() {
                        self.selected = Some(0);
                    }
                }
                Loaded::Done => {}
            }
        }
    }

    fn clamp_selection(&mut self) {
        self.selected = if self.rows.is_empty() { None } else { Some(self.selected.unwrap_or(0).min(self.rows.len() - 1)) };
    }

    pub fn rows(&self) -> &[R] {
        &self.rows
    }
    pub fn selected_row(&self) -> Option<&R> {
        self.selected.and_then(|i| self.rows.get(i))
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn is_loading(&self) -> bool {
        self.loader.loading
    }

    /// Test helper: yield to the runtime until the load task finishes, then poll.
    #[cfg(test)]
    pub(crate) async fn poll_until_idle(&mut self) {
        for _ in 0..50 {
            tokio::task::yield_now().await;
            self.poll();
            if !self.is_loading() {
                break;
            }
        }
    }
}

impl<R: SearchRow> SearchListBuilder<R> {
    pub fn initial_query(mut self, q: impl Into<String>) -> Self {
        self.initial_query = q.into();
        self
    }
    pub fn build(self) -> SearchList<R> {
        SearchList::new(self)
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p kimun-notes --lib search_list::tests::initial_load_populates_rows`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/search_list/
git commit -m "feat(tui): SearchList engine skeleton + generation-stamped one-shot load"
```

### Task 3: Generation guard — requery aborts stale results

**Files:**
- Modify: `tui/src/components/search_list/mod.rs` (add `set_query`)
- Test: inline

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun-notes --lib requery_supersedes`
Expected: FAIL — no `set_query`.

- [ ] **Step 3: Implement**

Add to `impl SearchList<R>`:

```rust
    /// Set the query and (for `reload_on_query` sources) start a fresh load.
    pub fn set_query(&mut self, q: impl Into<String>) {
        self.query = q.into();
        if self.source.reload_on_query() {
            self.loader.start(self.source.clone(), self.query.clone());
        }
        // Local-filter sources re-filter in `poll`/`apply_filter` (Task 5).
    }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p kimun-notes --lib requery_supersedes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/search_list/mod.rs
git commit -m "feat(tui): SearchList requery aborts the prior load (generation guard)"
```

### Task 4: Keyboard navigation + `KeyReaction`

**Files:**
- Modify: `tui/src/components/search_list/mod.rs`
- Test: inline

- [ ] **Step 1: Write the failing test**

```rust
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    fn key(c: KeyCode) -> KeyEvent { KeyEvent::new(c, KeyModifiers::NONE) }

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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun-notes --lib arrows_navigate_and_enter_submits`
Expected: FAIL — no `handle_key`/`KeyReaction`.

- [ ] **Step 3: Implement**

Add the verdict enum + input field. Add `use crate::components::single_line_input::{InputOutcome, SingleLineInput};` and a `input: SingleLineInput` field (init `SingleLineInput::with_value(&b.initial_query)` in `new`). Then:

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum KeyReaction {
    Consumed,
    Submit,
    Cancel,
    Intercepted(crate::keys::key_combo::KeyCombo),
    Unhandled,
}

impl<R: SearchRow> SearchList<R> {
    pub fn select_next(&mut self) {
        if self.rows.is_empty() { return; }
        let n = self.rows.len();
        self.selected = Some(self.selected.map_or(0, |i| (i + 1).min(n - 1)));
    }
    pub fn select_prev(&mut self) {
        if self.rows.is_empty() { return; }
        self.selected = Some(self.selected.map_or(0, |i| i.saturating_sub(1)));
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> KeyReaction {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        // (autocomplete first-crack + .intercept added in later tasks)
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
            InputOutcome::Changed => {
                self.set_query(self.input.value().to_string());
                KeyReaction::Consumed
            }
            InputOutcome::Consumed => KeyReaction::Consumed,
            InputOutcome::Submit => KeyReaction::Submit,
            InputOutcome::Cancel => KeyReaction::Cancel,
            InputOutcome::NotConsumed => KeyReaction::Unhandled,
        }
    }
}
```

> Note: `set_query` is now called from two places (the input edit here, and the external `set_query`). Keep both — the external one is for programmatic query changes (saved-search apply).

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p kimun-notes --lib search_list`
Expected: PASS (all engine tests).

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/search_list/mod.rs
git commit -m "feat(tui): SearchList keyboard nav + KeyReaction verdict"
```

### Task 5: `Filter` enum (SourceOrder / Fuzzy / Rank) + leading row

**Files:**
- Modify: `tui/src/components/search_list/seams.rs` (add `Filter`), `mod.rs` (apply it)
- Test: inline

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn rank_filter_orders_by_closure() {
        // Load-once source; local Rank: exact-name match first, else substring.
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun-notes --lib rank_filter_orders_by_closure`
Expected: FAIL — no `Filter`/`.filter`/`visible_rows`.

- [ ] **Step 3: Implement**

The engine now distinguishes the *backing* set (`rows`, what the source returned) from the *display* order (`display: Vec<usize>` into `rows`, after filter + leading row). `selected` indexes the display sequence.

Add to `seams.rs`:

```rust
/// How a loaded row set is narrowed/ordered for display. Folded in (not a
/// port): three known strategies, none needing test substitution.
pub enum Filter<R: SearchRow> {
    /// Trust the source's order (server-side filter already applied).
    SourceOrder,
    /// Local nucleo fuzzy over `match_text`.
    Fuzzy,
    /// Local rank: `(rows, query) -> display indices` (lower = better; absent = hidden).
    Rank(std::sync::Arc<dyn Fn(&[R], &str) -> Vec<usize> + Send + Sync>),
}
```

In `mod.rs`: add `filter: Filter<R>` (builder default `Filter::SourceOrder`) and `display: Vec<usize>`. Replace direct `rows` indexing in `selected_row`/nav with `display`-based indexing. After every `poll` apply / on `set_query` for non-reload sources, call:

```rust
    fn recompute_display(&mut self) {
        let q = self.query.trim();
        let mut idx: Vec<usize> = match &self.filter {
            Filter::SourceOrder => (0..self.rows.len()).collect(),
            Filter::Fuzzy if q.is_empty() => (0..self.rows.len()).collect(),
            Filter::Fuzzy => fuzzy_indices(&self.rows, q), // nucleo over match_text(); see below
            Filter::Rank(f) if q.is_empty() => (0..self.rows.len()).collect(),
            Filter::Rank(f) => f(&self.rows, q),
        };
        // Rows with match_text()==None are filter-exempt: always present.
        for i in 0..self.rows.len() {
            if self.rows[i].match_text().is_none() && !idx.contains(&i) {
                idx.insert(0, i);
            }
        }
        self.display = idx;
        self.clamp_selection();
    }

    pub fn visible_rows(&self) -> Vec<&R> {
        self.display.iter().filter_map(|&i| self.rows.get(i)).collect()
    }
```

`fuzzy_indices` lifts the nucleo setup out of `file_list.rs` (Task 14 deletes the original):

```rust
fn fuzzy_indices<R: SearchRow>(rows: &[R], query: &str) -> Vec<usize> {
    use nucleo::pattern::{CaseMatching, Normalization, Pattern};
    let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
    let pat = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut scored: Vec<(usize, u32)> = rows.iter().enumerate().filter_map(|(i, r)| {
        let hay = r.match_text()?;
        let mut buf = Vec::new();
        let h = nucleo::Utf32Str::new(hay, &mut buf);
        pat.score(h, &mut matcher).map(|s| (i, s))
    }).collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(i, _)| i).collect()
}
```

Update `selected_row`, `select_next/prev`, `clamp_selection`, and the `Loaded::Push`/`Replace` handlers to operate through `display` (selection indexes display; `display.len()` is the bound). Call `recompute_display()` after applying drained events and inside `set_query` for non-reload sources. Add `leading_row` handling: in `recompute_display`, if `self.source.leading_row(&self.query)` is `Some`, store it in a `leading: Option<R>` field and prepend its display slot (render reads it from `leading` rather than `rows`). For simplicity in this task, fold the leading row into `rows` at position 0 on load and mark it via `match_text()==None`; refine only if a call site needs dynamic create-text (sidebar, Task 14).

Add `filter` to the builder:

```rust
    pub fn filter(mut self, f: Filter<R>) -> Self {
        self.filter = f;
        self
    }
```

Re-export `Filter` from `mod.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p kimun-notes --lib search_list`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/search_list/
git commit -m "feat(tui): SearchList Filter enum (SourceOrder/Fuzzy/Rank) + display indices"
```

### Task 6: `SuggestionSource` port + retarget `AutocompleteController`

**Files:**
- Modify: `tui/src/components/search_list/seams.rs` (add `SuggestionSource`)
- Modify: `tui/src/components/autocomplete/controller.rs` (`new` takes the port; `fire_query`/`link_filter_suggestions` call it)
- Modify: call sites of `AutocompleteController::new` (`note_browser/mod.rs:144`, `backlinks_panel.rs:118`) to pass a vault-backed adapter
- Test: inline in `seams.rs` + adjust the controller's existing `link_filter_suggestions_include_note_var_and_names` test to use an in-memory adapter

- [ ] **Step 1: Add the port + a vault adapter (with test)**

Add to `seams.rs`:

```rust
/// Autocomplete candidates for the query input, kept separate from the vault
/// so the autocomplete host is testable in isolation.
#[async_trait]
pub trait SuggestionSource: Send + Sync + 'static {
    async fn notes_by_prefix(&self, prefix: &str, limit: usize) -> Vec<String>;
    async fn tags_by_prefix(&self, prefix: &str, limit: usize) -> Vec<String>;
}

/// Production adapter over the vault.
pub struct VaultSuggestions {
    pub vault: std::sync::Arc<kimun_core::NoteVault>,
}

#[async_trait]
impl SuggestionSource for VaultSuggestions {
    async fn notes_by_prefix(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.vault.suggest_notes_by_prefix(prefix, limit).await
            .map(|v| v.into_iter().map(|n| n.name).collect())
            .unwrap_or_default()
    }
    async fn tags_by_prefix(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.vault.suggest_tags_by_prefix(prefix, limit).await
            .map(|v| v.into_iter().map(|t| t.name).collect()) // adjust field name to TagSuggestion's
            .unwrap_or_default()
    }
}
```

> Verify `TagSuggestion`'s field for the label (`grep -n "pub struct TagSuggestion" core/src/db/mod.rs`) and use it; `NoteSuggestion.name` is confirmed.

Test in `seams.rs`:

```rust
#[cfg(test)]
mod suggestion_tests {
    use super::*;
    struct Mem { notes: Vec<String>, tags: Vec<String> }
    #[async_trait]
    impl SuggestionSource for Mem {
        async fn notes_by_prefix(&self, p: &str, _n: usize) -> Vec<String> {
            self.notes.iter().filter(|x| x.starts_with(p)).cloned().collect()
        }
        async fn tags_by_prefix(&self, p: &str, _n: usize) -> Vec<String> {
            self.tags.iter().filter(|x| x.starts_with(p)).cloned().collect()
        }
    }
    #[tokio::test]
    async fn mem_suggestions_filter_by_prefix() {
        let m = Mem { notes: vec!["projects".into()], tags: vec!["todo".into()] };
        assert_eq!(m.notes_by_prefix("pro", 9).await, vec!["projects"]);
        assert_eq!(m.tags_by_prefix("to", 9).await, vec!["todo"]);
    }
}
```

- [ ] **Step 2: Run to verify the port compiles/tests**

Run: `cargo test -p kimun-notes --lib search_list::seams::suggestion_tests`
Expected: PASS.

- [ ] **Step 3: Retarget `AutocompleteController`**

In `controller.rs`: change the field `vault: Arc<NoteVault>` to `suggestions: Arc<dyn SuggestionSource>`. Change `pub fn new(vault: Arc<NoteVault>, mode)` → `pub fn new(suggestions: Arc<dyn crate::components::search_list::SuggestionSource>, mode: AutocompleteMode)`. In `fire_query`, replace the three vault calls:
- `TriggerKind::Wikilink => self.suggestions.notes_by_prefix(&query, limit).await`
- `TriggerKind::LinkFilter => Self::link_filter_suggestions(&*self.suggestions, &query).await`
- `TriggerKind::Hashtag => self.suggestions.tags_by_prefix(&query, limit).await`

Change `link_filter_suggestions(vault: &NoteVault, ...)` → `link_filter_suggestions(s: &dyn SuggestionSource, prefix: &str) -> Vec<String>` calling `s.notes_by_prefix(prefix, 20)`. Update its test (`controller.rs:1204`) to construct a `Mem`-style in-memory `SuggestionSource` instead of a `temp_vault` — deleting the `create_note`/`validate_and_init` setup.

- [ ] **Step 4: Fix the two construction call sites**

`note_browser/mod.rs:144` and `backlinks_panel.rs:118`: replace `AutocompleteController::new(vault.clone(), AutocompleteMode::SearchQuery)` with
`AutocompleteController::new(Arc::new(VaultSuggestions { vault: vault.clone() }), AutocompleteMode::SearchQuery)` (import `VaultSuggestions`).

- [ ] **Step 5: Run the suite**

Run: `cargo test -p kimun-notes --lib autocomplete && cargo build -p kimun-notes`
Expected: PASS + compiles. The retargeted `link_filter_suggestions` test no longer touches a vault.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(tui): SuggestionSource port — autocomplete decoupled from the vault"
```

### Task 7: Autocomplete host inside `SearchList` (single canonical snapshot)

**Files:**
- Create: `tui/src/components/search_list/host.rs` (the one `SearchBoxHostSnapshot`)
- Modify: `tui/src/components/search_list/mod.rs` (own an `Option<AutocompleteController>`, wire it into `handle_key`)
- Test: inline (drive autocomplete via in-memory `SuggestionSource`, assert accept rewrites the query)

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn autocomplete_accept_rewrites_query_without_vault() {
        struct Mem;
        #[async_trait::async_trait]
        impl crate::components::search_list::SuggestionSource for Mem {
            async fn notes_by_prefix(&self, _p: &str, _n: usize) -> Vec<String> { vec![] }
            async fn tags_by_prefix(&self, p: &str, _n: usize) -> Vec<String> {
                if "projects".starts_with(p) { vec!["projects".into()] } else { vec![] }
            }
        }
        let src = VecSource { rows: vec![], reload: true };
        let mut list = SearchList::builder(src, noop_redraw())
            .autocomplete(std::sync::Arc::new(Mem), crate::components::autocomplete::AutocompleteMode::SearchQuery)
            .build();
        for c in ['#','p','r','o'] { list.handle_key(&key(KeyCode::Char(c))); }
        // allow the spawned suggestion query to land
        for _ in 0..50 { tokio::task::yield_now().await; list.poll(); }
        list.handle_key(&key(KeyCode::Tab)); // accept
        assert_eq!(list.query(), "#projects");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun-notes --lib autocomplete_accept_rewrites_query_without_vault`
Expected: FAIL — no `.autocomplete` builder / host wiring.

- [ ] **Step 3: Implement**

Move the `SearchBoxHostSnapshot` struct + its `AutocompleteHost` impl verbatim from `note_browser/mod.rs:74-107` into `host.rs` (make it `pub(super)`). Add to `SearchList`: `autocomplete: Option<AutocompleteController>`. Builder `.autocomplete(suggestions, mode)` constructs the controller with the trigger opts the search box uses (`TriggerOptions { disambiguate_header: false, apply_exclusion_zone: false }`) and `set_redraw_callback(self.redraw.clone())`. Add `autocomplete_snapshot()` (move from note_browser:332). In `handle_key`, before the nav match, give the popup first crack (mirror note_browser:402-422): if open, `handle_key` → `Accepted(action)` rewrites the input via `replace_range_bytes` + `set_query`; `Dismissed`/`Consumed` → return `Consumed`. After an input `Changed`, call `self.autocomplete.as_mut().map(|a| a.sync(&snapshot))`. In `poll`, call `autocomplete.poll_results()`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p kimun-notes --lib search_list`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/search_list/
git commit -m "feat(tui): autocomplete host inside SearchList (one canonical snapshot)"
```

### Task 8: `.intercept` (compose-on-top key pre-emption) + render

**Files:**
- Modify: `tui/src/components/search_list/mod.rs`
- Test: inline

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn intercepted_combo_returns_intercepted_without_acting() {
        use crate::keys::key_combo::KeyCombo;
        let combo = KeyCombo::from(&key(KeyCode::Enter)); // adjust to KeyCombo's real ctor
        let src = VecSource { rows: vec![TestRow::new("a")], reload: true };
        let mut list = SearchList::builder(src, noop_redraw()).intercept(vec![combo.clone()]).build();
        list.poll_until_idle().await;
        // Enter is intercepted: engine returns Intercepted, does NOT submit.
        assert_eq!(list.handle_key(&key(KeyCode::Enter)), KeyReaction::Intercepted(combo));
    }
```

> Check `KeyCombo`'s constructor (`tui/src/keys/key_combo.rs` + `key_event_to_combo`) and build the combo the real way; adapt the test's combo construction accordingly.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun-notes --lib intercepted_combo`
Expected: FAIL — no `.intercept`.

- [ ] **Step 3: Implement**

Add `intercept: Vec<KeyCombo>` (builder `.intercept(v)`). At the TOP of `handle_key` (before autocomplete/nav), `if let Some(combo) = key_event_to_combo(key) { if self.intercept.contains(&combo) { return KeyReaction::Intercepted(combo); } }`. Then add `render`/`render_query`/`render_autocomplete`:

```rust
    pub fn render_query(&mut self, f: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        self.input.render(f, area, Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg_panel.to_ratatui()), 0, focused);
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
        self.poll();
        let items: Vec<ListItem> = self.display.iter()
            .filter_map(|&i| self.rows.get(i))
            .enumerate()
            .map(|(disp, r)| r.to_list_item(theme, &self.icons, self.selected == Some(disp)))
            .collect();
        let mut state = ListState::default();
        state.select(self.selected);
        f.render_stateful_widget(List::new(items).highlight_style(...), area, &mut state);
        self.list_rect = area; // for handle_mouse
    }

    pub fn render_autocomplete(&mut self, f: &mut Frame, clamp: Rect, theme: &Theme) {
        if let Some(ac) = &mut self.autocomplete {
            ac.poll_results();
            if let (Some(state), Some(anchor)) = (ac.state_mut(), self.input.last_caret_pos()) { state.anchor = anchor; }
            if let Some(state) = ac.state() { crate::components::autocomplete::render(f, state, clamp, theme); }
        }
    }
```

> Add `icons: Icons` to the builder (`.icons(Icons)` or pass in `builder`), and a `list_rect: Rect` field for `handle_mouse` (mirror note_browser). Add `handle_mouse` mirroring note_browser:354-392 returning a `SearchMouse { Selected(usize), Scrolled, None }`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p kimun-notes --lib search_list && cargo build -p kimun-notes`
Expected: PASS + compiles.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/search_list/mod.rs
git commit -m "feat(tui): SearchList .intercept + render/render_query/render_autocomplete"
```

> **End of Phase 1.** `SearchList` is a complete deep module tested entirely through its interface with in-memory adapters — zero vault. Phases 2–5 each adopt it at one call site, deleting that site's plumbing.

---

## PHASE 2 — Adopt in the note browser (first real adapter)

### Task 9: `NoteBrowserProvider` → `RowSource`; modal hosts `SearchList`

**Files:**
- Modify: `tui/src/components/note_browser/mod.rs`, `note_browser/{search_provider,file_finder_provider,link_results_provider}.rs`
- Modify: `tui/src/components/file_list.rs` (add `impl SearchRow for FileListEntry`)
- Test: replace the modal's tests with `SearchList`-interface tests; keep behaviour-pinning

- [ ] **Step 1: `FileListEntry: SearchRow`**

In `file_list.rs`, add `impl SearchRow for FileListEntry` whose `to_list_item` delegates to the existing `to_list_item`/row-render method (move that body), `visual_height` returns the journal-aware height already computed there, and `match_text` returns the filename for `Note`/`CreateNote`, `None` for `Up`. Build.

- [ ] **Step 2: Providers become `RowSource<FileListEntry>`**

Change the `NoteBrowserProvider` trait usage: each provider (`SearchNotesProvider`, `FileFinderProvider`, `LinkResultsProvider`) gets `impl RowSource<FileListEntry>`: its async `load(&self, query, emit)` calls the existing body and `emit.replace(entries)`. Move the `allows_create()` create-row into `RowSource::leading_row` (returns the `CreateNote` entry when the query is non-empty, for the finder). Keep `reload_on_query()` = true (default).

- [ ] **Step 3: Rewrite `NoteBrowserModal` as a `SearchList` host**

Replace the modal's `search_query` + `file_list` + `load_task`/`load_rx` + `poll_load` + `autocomplete` + `SearchBoxHostSnapshot` + key ladder with one `list: SearchList<FileListEntry>`. Keep the preview pane (`preview_text`/`preview_task`/`preview_rx`/`refresh_preview`). Construct:

```rust
let list = SearchList::builder(provider, redraw_callback(tx.clone()))
    .initial_query(initial_query)
    .icons(icons)
    .autocomplete(Arc::new(VaultSuggestions { vault: vault.clone() }), AutocompleteMode::SearchQuery)
    .build();
```

`handle_input`: delegate to `list.handle_key`:
```rust
match self.list.handle_key(key) {
    KeyReaction::Submit => self.open_selected(tx),               // reads list.selected_row()
    KeyReaction::Cancel => { tx.send(AppEvent::CloseNoteBrowser).ok(); }
    KeyReaction::Consumed => { self.refresh_preview(self.list.selected_row()); }
    _ => {}
}
```
`render`: `list.render_query(...)`, `list.render(...)`, own preview pane, `list.render_autocomplete(...)`. Mouse → `list.handle_mouse`.

- [ ] **Step 4: Replace tests**

Delete the modal tests that reached into `search_query`/`file_list`/`load_task` private state (`note_browser/mod.rs` tests, incl. `search_box_autocomplete_accept_inserts_tag` — now covered by Task 7's vault-free engine test). Add a small modal-level test: build the modal with a `temp_vault`, drive a key, assert `OpenPath`/`CloseNoteBrowser` emit on Submit. Keep it thin — the engine is tested in Phase 1.

- [ ] **Step 5: Verify + commit**

Run: `cargo test -p kimun-notes && cargo build -p kimun-notes`
Expected: PASS.
```bash
git add -A
git commit -m "refactor(tui): note browser hosts SearchList; providers are RowSources"
```

---

## PHASE 3 — Adopt in the saved-searches modal (two adapters → seam justified)

### Task 10: `rank_items` → `Filter::Rank`; virtual row → `leading_row`

**Files:**
- Modify: `tui/src/components/saved_searches_modal.rs`
- Test: keep the `rank_items` unit tests (now a closure); modal test asserts select/delete

- [ ] **Step 1: `SearchItem: SearchRow`**

`impl SearchRow for SearchItem`: `to_list_item` = the existing row render (index prefix + name + virtual `*`), `match_text` = `Some(&name)`. Build.

- [ ] **Step 2: `SavedSearchSource: RowSource<SearchItem>`**

`load(_query, emit)` calls `vault.list_saved_searches()`, builds `SavedSearchesModel`'s user items, `emit.replace(items)`. `leading_row(_)` returns the pinned virtual backlinks `SearchItem` (so it's always shown, filter-exempt). `reload_on_query()` = false (load once; local rank).

- [ ] **Step 3: Host `SearchList` with `Filter::Rank`**

Replace the modal's `filter`/`list_state`/`load_*`/`poll_load`/`rank_items`-call with:
```rust
let rank = Arc::new(|rows: &[SearchItem], q: &str| rank_to_indices(rows, q)); // rank_items, returning Vec<usize>
let list = SearchList::builder(SavedSearchSource { vault }, redraw_callback(tx.clone()))
    .filter(Filter::Rank(rank))
    .icons(icons)
    .build();
```
Adapt `rank_items` to return display indices (it currently returns `Vec<&SearchItem>`; make a `rank_to_indices(&[SearchItem], &str) -> Vec<usize>` and keep its 4 unit tests, updated to assert on indices). `handle_input`: intercept `Delete` BEFORE delegating (mirror current); on `KeyReaction::Submit` emit `SavedSearchSelected{query,name}` + `CloseSavedSearches`; on `Cancel` emit `CloseSavedSearches`. Delete → `vault.delete_saved_search` then `list.reload()` (add a `reload()` to SearchList that re-runs `load` for the current query).

- [ ] **Step 4: Verify + commit**

Run: `cargo test -p kimun-notes && cargo build -p kimun-notes`
Expected: PASS.
```bash
git add -A
git commit -m "refactor(tui): saved searches modal hosts SearchList (Filter::Rank + leading_row)"
```

---

## PHASE 4 — Adopt in the Query panel (compose-on-top)

### Task 11: QueryPanel hosts `SearchList`; expand/preview compose via `.intercept`

**Files:**
- Modify: `tui/src/components/backlinks_panel.rs`
- Modify: `tui/src/components/events.rs` (remove `BacklinksLoaded` once unused), `app_screen/editor.rs` (drop its `BacklinksLoaded` arm)
- Test: keep the expand/needle tests; add a SearchList-hosted nav test

- [ ] **Step 1: `BacklinkEntry: SearchRow`**

`impl SearchRow for BacklinkEntry`: `to_list_item` = the existing collapsed-row render; `match_text` = `Some(&filename)`. The expand/context/full rendering + needle highlight + content scroll STAY in the panel (compose-on-top). Build.

- [ ] **Step 2: `BacklinkSource: RowSource<BacklinkEntry>`**

`load(query, emit)` calls the existing `load_query(&vault, query)` (the query is already `{note}`-resolved by the caller via `set_query`) and `emit.replace(entries)`. `reload_on_query()` = true.

- [ ] **Step 3: Host the engine, intercept expand/sort**

Replace `query_input`/`autocomplete`/`SearchBoxHostSnapshot`/the `BacklinksLoaded` spawn with `list: SearchList<BacklinkEntry>`, built with `.autocomplete(VaultSuggestions, SearchQuery)` and `.intercept([Enter-combo, sort combos])`. `set_note`: resolve `{note}` → `list.set_query(resolved)` only when `query_has_variables`. `handle_key`:
```rust
// full-expand content scroll vetoes nav BEFORE the engine:
if self.is_full_expanded() && matches!(key.code, Up|Down) { self.scroll_content(key); return Consumed; }
match self.list.handle_key(key) {
    KeyReaction::Intercepted(c) if c == enter_combo => self.toggle_expand(),
    KeyReaction::Intercepted(c) if c == sort_cycle => { self.cycle_sort(); }
    KeyReaction::Submit => self.toggle_expand(),
    KeyReaction::Consumed => { self.saved_search_name = None; } // a query edit drops the saved name
    _ => {}
}
```
> Because full-expand must remap Up/Down to scroll, those are handled by the panel's own pre-check (above) — NOT registered in `.intercept` (intercept is for combos the engine should never act on at all; Up/Down must still nav when NOT expanded). Keep Enter + sort combos in `.intercept`.

Render: `list.render_query` + (collapsed) `list.render` when not full-expanded; the panel draws the expanded/full pane itself reading `list.selected_row()`. Title logic (Backlinks/saved-name/Query) stays in the panel.

- [ ] **Step 4: Remove `BacklinksLoaded`**

Once the panel no longer sends/receives it: delete `AppEvent::BacklinksLoaded` (events.rs:114) and its `handle_app_message` arm in editor.rs. Build to confirm no other references.

- [ ] **Step 5: Verify + commit**

Run: `cargo test -p kimun-notes && cargo build -p kimun-notes`
Expected: PASS.
```bash
git add -A
git commit -m "refactor(tui): Query panel hosts SearchList; expand/preview compose on top; drop BacklinksLoaded"
```

---

## PHASE 5 — Adopt in the sidebar; split & retire `FileListComponent`

### Task 12: Streamed `RowSource` for the directory listing

**Files:**
- Modify: `tui/src/components/sidebar.rs`
- Test: a streamed-source engine test (in `search_list` tests, using `ScriptedStreamSource`) + a sidebar smoke test

- [ ] **Step 1: Engine streaming test (in search_list adapters/tests)**

Add `ScriptedStreamSource { batches: Vec<Vec<TestRow>> }` to `adapters.rs`: `load` pushes each row via `emit.push` then `emit.done()`; `reload_on_query()` = false. Test:
```rust
    #[tokio::test]
    async fn streamed_rows_arrive_incrementally_then_done() {
        let src = ScriptedStreamSource { batches: vec![vec![TestRow::new("a")], vec![TestRow::new("b")]] };
        let mut list = SearchList::builder(src, noop_redraw()).filter(Filter::Fuzzy).build();
        list.poll_until_idle().await;
        assert_eq!(list.rows().len(), 2);
        assert!(!list.is_loading());
    }
```
Run → fails only if streaming/`Done` handling is wrong; it should already pass from Phase 1 (Task 2/5). If green immediately, this is a regression guard — keep it.

- [ ] **Step 2: `DirListingSource`**

In `sidebar.rs`, wrap the existing `VaultBrowseOptionsBuilder` + `Receiver<SearchResult>` flow as a `RowSource<FileListEntry>` whose `load` forwards each `SearchResult` → `FileListEntry::from_result` → `emit.push`, then `emit.done()` on disconnect. `leading_row` returns the `CreateNote` row when the query is non-empty (the current `sync_create_entry`). `reload_on_query()` = false (load once per directory; local fuzzy filters).

- [ ] **Step 3: Sidebar hosts `SearchList`**

Replace `file_list: FileListComponent` with `list: SearchList<FileListEntry>` built with `.filter(Filter::Fuzzy)`, no autocomplete. Directory navigation (Up-entry activation, `OpenPath`, focus) stays in the sidebar, reading `list.selected_row()` and handling `KeyReaction::{Submit,Unhandled}`. The "Up .." row is a `FileListEntry::Up` pushed first by the source (filter-exempt via `match_text()==None`).

- [ ] **Step 4: Verify + commit**

Run: `cargo test -p kimun-notes && cargo build -p kimun-notes`
Expected: PASS.
```bash
git add -A
git commit -m "refactor(tui): sidebar hosts SearchList (streamed source + Filter::Fuzzy)"
```

### Task 13: Retire `FileListComponent`

**Files:**
- Modify/Delete: `tui/src/components/file_list.rs`
- Modify: `tui/src/components/mod.rs`

- [ ] **Step 1: Confirm no remaining users**

Run: `grep -rn "FileListComponent" tui/src`
Expected: only `file_list.rs` itself (and `FileListEntry`, which SURVIVES as the `SearchRow` impl). If any call site remains, it wasn't migrated — stop and migrate it.

- [ ] **Step 2: Strip the component, keep `FileListEntry`**

Delete `FileListComponent` (the struct + impl + its nucleo filter + nav + render + tests). KEEP `FileListEntry` (enum + `from_result` + `path()`/`filename()` + its `SearchRow` impl). The nucleo logic now lives in `search_list::fuzzy_indices`; the nav/list mechanics live in `SearchList`. Move `FileListEntry` to its own `file_list_entry.rs` if `file_list.rs` is now mostly empty.

- [ ] **Step 3: Verify + commit**

Run: `cargo test -p kimun-notes && cargo build -p kimun-notes && cargo clippy -p kimun-notes --all-targets 2>&1 | tail -20`
Expected: PASS + clippy clean.
```bash
git add -A
git commit -m "refactor(tui): retire FileListComponent — list engine absorbed by SearchList"
```

---

## Final verification

- [ ] **Whole suite**

Run: `cargo test`
Expected: all green.

- [ ] **Clippy + fmt**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: clean.

- [ ] **Seam-count sanity (deletion test held):** `grep -rn "SearchBoxHostSnapshot" tui/src` returns exactly ONE definition (in `search_list/host.rs`). `grep -rn "schedule_load\|poll_load\|poll_loading\|poll_filter" tui/src` returns nothing outside `search_list/`. `grep -rn "BacklinksLoaded" tui/src` returns nothing.

- [ ] **Manual smoke (`cargo run -p kimun-notes` / the `run` skill):** Ctrl+K browse + autocomplete + open; right panel backlinks + `#todo` query + expand; saved-searches modal filter + 1–9 + select + delete; sidebar directory browse + fuzzy filter + Up/Create. All behave as before the refactor.

---

## Self-Review notes (spec coverage)

- Deep `SearchList` owning input/nav/async/autocomplete/select → Phase 1 (Tasks 1–8).
- `RowSource` seam (1 trait, ≥2 adapters) → Tasks 1, 9, 10, 11, 12.
- `SuggestionSource` port (autocomplete vault-free) → Task 6.
- `Filter` enum, not a port → Task 5 (SourceOrder/Fuzzy/Rank used in 9/12, 12, 10).
- Compose-on-top via `.intercept` + `selected_row()` → Task 8; exercised in 11 (expand) and 10 (Delete).
- Emits nothing global; caller decides action → Tasks 4, 9, 10, 11, 12.
- Generation-stamped loads (race fix) → Tasks 2, 3.
- `FileListComponent` split (list engine vs nucleo filter) → Tasks 5, 12, 13.
- `BacklinksLoaded` lifted off the global AppEvent → Task 11.
- Migration order engine→browser→saved→panel→sidebar, each green/committable → Phases 1–5.

**Known follow-ups (not in this plan):** `handle_mouse`/`SearchMouse` is sketched in Task 8 — flesh out per call site during 9/12 if mouse parity needs it. The `leading_row` dynamic create-text (sidebar) may need the `leading: Option<R>` field refinement noted in Task 5 if folding it into `rows` proves awkward.
