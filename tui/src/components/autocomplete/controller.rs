use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_core::note::ExclusionZones;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::host::AutocompleteHost;
use super::popup::{PopupAction, PopupOutcome, handle_key as popup_handle_key};
use super::state::{AutocompleteState, DEFAULT_MAX_VISIBLE_ROWS, Suggestion};
use super::trigger::{TriggerKind, TriggerOptions, ZoneOracle, detect_trigger_with_oracle};
#[cfg(test)]
use crate::components::text_editor::snapshot::EditorSnapshot;
use crate::util::single_slot_task::SingleSlotTask;

/// Hard cap on suggestions fetched from core per query. The popup itself
/// only shows `max_visible_rows` at a time and scrolls inside the fetched
/// set, so a few dozen rows is plenty.
const DEFAULT_FETCH_LIMIT: usize = 50;

/// Wait this long after a query-refinement keystroke before hitting the
/// vault. Two cases:
///
/// - Fast typing (inter-keystroke gap < `DEFAULT_DEBOUNCE`): each new
///   keystroke aborts the previous in-flight task while it is still
///   inside `tokio::time::sleep`, so only the final keystroke's query
///   reaches SQLite. This is the case the debounce is optimised for.
/// - Normal typing (inter-keystroke gap ≥ `DEFAULT_DEBOUNCE`): every
///   keystroke still runs its own query, with `DEFAULT_DEBOUNCE` of added
///   latency between keystroke and popup update. The debounce does NOT
///   reduce work in this regime — it bounds responsiveness.
///
/// The first query of a popup (kind change / popup opening) skips the
/// debounce entirely so the popup feels instant on open.
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(80);

/// Whether wikilink triggers are honoured. The editor uses
/// `Both`; the search box uses `HashtagOnly` because the search syntax has
/// no `[[…]]` operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutocompleteMode {
    Both,
    HashtagOnly,
}

/// Owns the popup lifecycle and the (debounced via generation tokens)
/// query plumbing. The host calls `sync(host)` after every edit; the
/// controller decides whether to open, refresh, or close the popup.
pub struct AutocompleteController {
    state: Option<AutocompleteState>,
    vault: Arc<NoteVault>,
    mode: AutocompleteMode,
    /// Trigger-detection options passed to `detect_trigger_with` on every
    /// `sync`. The editor leaves header disambiguation on; the search box
    /// switches it off because its input has no Markdown headers.
    trigger_opts: TriggerOptions,
    /// Monotonic counter incremented on every fired query. Responses that
    /// arrive with a stale generation are discarded.
    generation: u64,
    result_tx: UnboundedSender<QueryResult>,
    result_rx: UnboundedReceiver<QueryResult>,
    fetch_limit: usize,
    max_visible_rows: usize,
    /// Handle of the most recently spawned query task. Spawning a new
    /// query into this slot aborts the previous one, so a burst of
    /// keystrokes does not pile up N concurrent SQLite queries holding
    /// the vault `Arc` open. The slot's `Drop` also aborts, so the
    /// task cannot outlive the controller.
    in_flight: SingleSlotTask<()>,
    /// Delay inserted before each refinement query hits the vault. A burst
    /// of typing aborts the prior in-flight task during this window, so
    /// only the final keystroke's query reaches SQLite. The first query of
    /// a popup (kind change / open) bypasses the debounce. Tests override
    /// to `Duration::ZERO` via `with_debounce`.
    debounce: Duration,
    /// The joined buffer text keyed on the host's `content_revision`,
    /// plus its `ExclusionZones` computed LAZILY (`None` until the
    /// trigger veto first needs them). The text is rebuilt only when the
    /// revision moves; cursor moves reuse it. The zones — a full-buffer
    /// pulldown-cmark + regex scan — are computed only when a `[[`/`#`
    /// opener is found, then memoized here so a later cursor move at the
    /// same revision reuses them. Hosts with no stable revision identity
    /// (search-box modal) return `None` from `content_revision` and never
    /// populate this slot.
    cached_text: Option<(NonZeroU64, String, Option<ExclusionZones>)>,
    /// Optional callback used to wake the host's render loop after an
    /// async query posts its result. Decoupled from any specific event
    /// bus so the controller stays usable wherever the host can
    /// trigger a redraw.
    redraw_cb: Option<RedrawCallback>,
}

/// Fire-and-forget redraw signal owned by the controller and invoked
/// from the spawned query task. The host wires this to its event loop
/// (e.g. `tx.send(AppEvent::Redraw)`).
pub type RedrawCallback = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Debug)]
struct QueryResult {
    generation: u64,
    kind: TriggerKind,
    items: Vec<Suggestion>,
}

/// [`ZoneOracle`] that computes `ExclusionZones` from `text` on first
/// query and memoizes the result back into the borrowed slot — so the
/// full-buffer scan runs at most once per buffer revision, and only when
/// a trigger candidate actually reaches the exclusion veto.
struct LazyZoneOracle<'a> {
    text: &'a str,
    zones: &'a mut Option<ExclusionZones>,
}

impl ZoneOracle for LazyZoneOracle<'_> {
    fn contains(&mut self, cursor: usize) -> bool {
        let text = self.text;
        self.zones
            .get_or_insert_with(|| ExclusionZones::from_text(text))
            .contains(cursor)
    }

    fn contains_code_link_or_frontmatter(&mut self, cursor: usize) -> bool {
        let text = self.text;
        self.zones
            .get_or_insert_with(|| ExclusionZones::from_text(text))
            .contains_code_link_or_frontmatter(cursor)
    }
}

impl AutocompleteController {
    pub fn new(vault: Arc<NoteVault>, mode: AutocompleteMode) -> Self {
        let (result_tx, result_rx) = unbounded_channel();
        Self {
            state: None,
            vault,
            mode,
            trigger_opts: TriggerOptions::default(),
            generation: 0,
            result_tx,
            result_rx,
            fetch_limit: DEFAULT_FETCH_LIMIT,
            max_visible_rows: DEFAULT_MAX_VISIBLE_ROWS,
            in_flight: SingleSlotTask::empty(),
            debounce: DEFAULT_DEBOUNCE,
            cached_text: None,
            redraw_cb: None,
        }
    }

    /// Override the trigger-detection options. Used by the search-box
    /// controller to disable the column-0 header disambiguation rule
    /// (Markdown headers don't exist in a search input).
    pub fn with_trigger_opts(mut self, opts: TriggerOptions) -> Self {
        self.trigger_opts = opts;
        self
    }

    /// Override the per-refinement debounce window. Tests pass
    /// `Duration::ZERO` so query results land promptly inside `drain_results`.
    #[cfg(test)]
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// Register a redraw callback. Without one, the popup state updates
    /// on background threads but the render loop has no signal to
    /// wake. Idempotent — safe to call from a host's first
    /// `handle_input` to lazily bind once the host has a way to
    /// trigger redraws.
    pub fn set_redraw_callback(&mut self, cb: RedrawCallback) {
        self.redraw_cb = Some(cb);
    }

    /// Whether the popup is currently *interactive* — held state AND at
    /// least one visible suggestion. Returns `false` while a query is
    /// in flight (state exists but items not yet arrived) or when a
    /// query returned no matches: in both cases the popup is not drawn
    /// and must not intercept key events, so Esc/Up/Down/Tab fall
    /// through to the host (modal Esc closes the modal, list Up/Down
    /// navigates files, etc).
    pub fn is_open(&self) -> bool {
        self.state.as_ref().is_some_and(|s| !s.items.is_empty())
    }

    /// Borrow the popup state for read-only inspection (rendering,
    /// query introspection, tests). Returns `None` whenever the popup
    /// is not active.
    pub fn state(&self) -> Option<&AutocompleteState> {
        self.state.as_ref()
    }

    /// Borrow the popup state mutably. The only legitimate
    /// caller-side use today is the host's render path, which
    /// re-anchors `state.anchor` from the freshly rendered caret
    /// position so the popup follows the cursor without a one-frame
    /// lag. Mutating `items` / `highlighted` / `scroll_offset` from
    /// outside the controller will desync the popup; use the
    /// dedicated `sync` / `refresh_if_open` / `handle_key` entry
    /// points for those.
    pub fn state_mut(&mut self) -> Option<&mut AutocompleteState> {
        self.state.as_mut()
    }

    /// Close the popup immediately. Safe to call when already closed.
    /// Use whenever focus moves away from the host or the host
    /// triggers a buffer-replacement that invalidates the trigger
    /// context (e.g. `set_text`).
    ///
    /// Also aborts any in-flight query task — without this, pressing Esc
    /// during the 80ms debounce window leaks a spawned tokio task that
    /// continues to the SQLite hit and posts a result discarded later
    /// via generation mismatch.
    pub fn close(&mut self) {
        self.state = None;
        self.in_flight.abort();
        // Drop the cached buffer text + zones. On a multi-MB note these
        // hold a full clone of the buffer + parsed exclusion-zone
        // ranges; without this clear they survive popup dismissal
        // until the next text edit overwrites the slot.
        self.cached_text = None;
    }

    /// Route a key event through the popup when one is open. Returns a
    /// `HandleKeyOutcome` so the host can decide whether to apply an
    /// accept, fall through to its own key handling, etc. The controller
    /// never mutates the host's buffer directly — on accept it returns an
    /// `AcceptAction` describing the replacement.
    pub fn handle_key<H: AutocompleteHost>(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        host: &H,
    ) -> HandleKeyOutcome {
        let Some(state) = self.state.as_mut() else {
            return HandleKeyOutcome::NotHandled;
        };
        let outcome = popup_handle_key(state, key);
        match outcome {
            PopupOutcome::Consumed(PopupAction::None) => HandleKeyOutcome::Consumed,
            PopupOutcome::Consumed(PopupAction::Accept) => {
                // Compute the accept BEFORE closing so a stale-range
                // failure (None) can fall through to the host's normal
                // key handling instead of silently swallowing the key:
                // user pressed Tab expecting an indent or Enter
                // expecting a newline; if the accept can't run we
                // should still give them the key back.
                match self.compute_accept(host) {
                    Some(action) => {
                        self.close();
                        HandleKeyOutcome::Accepted(action)
                    }
                    None => {
                        self.close();
                        HandleKeyOutcome::NotHandled
                    }
                }
            }
            PopupOutcome::Consumed(PopupAction::Dismiss) => {
                self.close();
                HandleKeyOutcome::Dismissed
            }
            PopupOutcome::NotHandled => HandleKeyOutcome::NotHandled,
        }
    }

    /// Inspect the host's current buffer + cursor and reconcile the popup
    /// state. Call this after a **text edit** (insert / delete / paste /
    /// any change that modifies the buffer). Will open a fresh popup
    /// when the cursor lands inside a trigger context, refresh an open
    /// popup's range/query/anchor, or close the popup when the trigger
    /// is gone.
    pub fn sync<H: AutocompleteHost>(&mut self, host: &H) {
        self.reconcile(host, true);
    }

    /// Refresh the popup state for a **cursor-only** event (arrow keys,
    /// click, Home/End, etc). If the popup is closed, this is a no-op —
    /// cursor movement never opens a new popup. If the popup is open,
    /// it follows the cursor: query, range, and anchor update; the
    /// popup closes when the cursor leaves the trigger range.
    pub fn refresh_if_open<H: AutocompleteHost>(&mut self, host: &H) {
        if self.state.is_some() {
            self.reconcile(host, false);
        }
    }

    fn reconcile<H: AutocompleteHost>(&mut self, host: &H, allow_open: bool) {
        // Single borrow of the host's buffer + cursor. The snapshot
        // is borrowed (Textarea backend) so no per-keystroke lines
        // clone happens here.
        let snap = host.buffer_snapshot();
        let cursor = snap.cursor_byte_offset();
        let cache_key = host.cache_key();
        // Rebuild the joined buffer text only when the host's cache key
        // has moved on; cursor moves reuse it. The expensive
        // `ExclusionZones` scan (full-buffer pulldown-cmark + regex)
        // stays LAZY — the oracle below computes it only if the local
        // trigger scan finds a `[[`/`#` opener that needs the exclusion
        // veto, and memoizes it into the cache slot so a later cursor
        // move at the same revision reuses it. Normal prose keystrokes
        // (no opener at the caret) never pay the scan at all.
        //
        // `cache_key == None` opts the host out (search-box modal): join
        // locally, use a throwaway lazy memo, and never touch the cache.
        let trigger = match cache_key {
            Some(rev) => {
                let hit = matches!(&self.cached_text, Some((r, _, _)) if *r == rev);
                if !hit {
                    self.cached_text = Some((rev, snap.lines.join("\n"), None));
                }
                let (_, text, zones_slot) = self.cached_text.as_mut().expect("just populated");
                let text: &str = text;
                let mut oracle = LazyZoneOracle {
                    text,
                    zones: zones_slot,
                };
                detect_trigger_with_oracle(text, cursor, self.trigger_opts, &mut oracle)
            }
            None => {
                let text = snap.lines.join("\n");
                let mut zones: Option<ExclusionZones> = None;
                let mut oracle = LazyZoneOracle {
                    text: &text,
                    zones: &mut zones,
                };
                detect_trigger_with_oracle(&text, cursor, self.trigger_opts, &mut oracle)
            }
        };

        // Filter by mode before deciding anything else.
        let trigger = trigger.filter(|t| match (self.mode, t.kind) {
            (AutocompleteMode::Both, _) => true,
            (AutocompleteMode::HashtagOnly, TriggerKind::Hashtag) => true,
            (AutocompleteMode::HashtagOnly, TriggerKind::Wikilink | TriggerKind::LinkFilter) => {
                false
            }
        });

        let Some(trigger) = trigger else {
            self.close();
            return;
        };

        let Some(anchor) = host.screen_anchor_for(trigger.anchor_col) else {
            self.close();
            return;
        };

        let query_changed;
        let kind_changed;
        match self.state.as_ref() {
            None => {
                kind_changed = true;
                query_changed = true;
            }
            Some(existing) => {
                kind_changed = existing.kind != trigger.kind;
                query_changed = kind_changed || existing.query != trigger.query;
            }
        }

        // On a cursor-only reconcile (allow_open=false), neither
        // opening a brand-new popup NOR replacing an existing popup
        // with a different trigger kind counts as a refresh — the
        // user did not type into the new context. Close instead so
        // the popup doesn't materialise from a mouse click or arrow
        // move into a different trigger zone.
        if !allow_open && (self.state.is_none() || kind_changed) {
            self.close();
            return;
        }

        if self.state.is_none() || kind_changed {
            let mut st = AutocompleteState::new(trigger.kind, anchor);
            st.max_visible_rows = self.max_visible_rows;
            self.state = Some(st);
        }

        if let Some(state) = self.state.as_mut() {
            state.kind = trigger.kind;
            state.query = trigger.query.clone();
            state.replace_range = trigger.replace_range.clone();
            state.anchor = anchor;
        }

        if query_changed {
            // First query of a popup (kind change / open) fires instantly
            // for snappy UX. Refinement queries on the same popup are
            // debounced so a burst of typing only hits the vault once.
            let instant = kind_changed;
            self.fire_query(trigger.kind, trigger.query, instant);
        }
    }

    /// Drain pending query responses and apply the latest one whose
    /// generation matches the controller's current generation. Older
    /// responses (stale) are discarded.
    pub fn poll_results(&mut self) {
        while let Ok(result) = self.result_rx.try_recv() {
            if result.generation != self.generation {
                continue;
            }
            let Some(state) = self.state.as_mut() else {
                continue;
            };
            if state.kind != result.kind {
                continue;
            }
            state.set_items(result.items);
        }
    }

    fn fire_query(&mut self, kind: TriggerKind, query: String, instant: bool) {
        // `SingleSlotTask::spawn` aborts the previous in-flight task —
        // its result would be discarded on receive (generation
        // mismatch) but the SQLite hit would still happen and the
        // vault `Arc` would stay alive until the task drained.
        self.generation = self.generation.wrapping_add(1);
        let req_gen = self.generation;
        let tx = self.result_tx.clone();
        let redraw = self.redraw_cb.clone();
        let vault = self.vault.clone();
        let limit = self.fetch_limit;
        let debounce = if instant {
            Duration::ZERO
        } else {
            self.debounce
        };
        self.in_flight.spawn(async move {
            // Aborted by the next `fire_query` before this sleep completes
            // for a burst of typing — the SQLite hit below never runs.
            if !debounce.is_zero() {
                tokio::time::sleep(debounce).await;
            }
            let items: Vec<Suggestion> = match kind {
                TriggerKind::Wikilink => match vault.suggest_notes_by_prefix(&query, limit).await {
                    Ok(notes) => notes
                        .into_iter()
                        .map(|n| Suggestion {
                            display: n.name,
                            secondary: Some(n.path.to_string()),
                        })
                        .collect(),
                    Err(e) => {
                        log::warn!(
                            "autocomplete: suggest_notes_by_prefix({:?}) failed: {}",
                            query,
                            e
                        );
                        Vec::new()
                    }
                },
                // LinkFilter suggestion source is wired in a later task;
                // for now fall through to an empty list so detection can
                // be tested end-to-end without suggestion plumbing.
                TriggerKind::LinkFilter => Vec::new(),
                TriggerKind::Hashtag => match vault.suggest_tags_by_prefix(&query, limit).await {
                    Ok(tags) => tags
                        .into_iter()
                        .map(|t| Suggestion {
                            display: t.label,
                            secondary: Some(format!("{}×", t.usage_count)),
                        })
                        .collect(),
                    Err(e) => {
                        log::warn!(
                            "autocomplete: suggest_tags_by_prefix({:?}) failed: {}",
                            query,
                            e
                        );
                        Vec::new()
                    }
                },
            };
            let _ = tx.send(QueryResult {
                generation: req_gen,
                kind,
                items,
            });
            // Wake the host's render loop so the popup actually paints
            // with the new items. `redraw_cb` may be None in unit
            // tests; production hosts bind it via `set_redraw_callback`.
            if let Some(redraw) = redraw {
                redraw();
            }
        });
    }

    fn compute_accept<H: AutocompleteHost>(&self, host: &H) -> Option<AcceptAction> {
        let state = self.state.as_ref()?;
        let suggestion = state.selected()?.clone();
        let kind = state.kind;
        let range = state.replace_range.clone();
        // Accept is a once-per-popup-acceptance path; allocating the
        // joined buffer text here is fine. The hot path is reconcile,
        // which uses the cached `text` slot built once per text edit.
        let buffer = host.buffer_snapshot().lines.join("\n");

        // Guard against a stale snapshot — if the live buffer shrank
        // below the trigger range, drop the accept rather than producing
        // a malformed insertion (or panicking on String::replace_range
        // in the search-box host).
        if range.start > range.end || range.end > buffer.len() {
            return None;
        }
        if !buffer.is_char_boundary(range.start) || !buffer.is_char_boundary(range.end) {
            return None;
        }

        match kind {
            TriggerKind::Wikilink => {
                let extent = scan_wikilink_extent(&buffer, range.end);
                // Replace from the trigger's start through the end of
                // the stale wikilink-target region. This consumes any
                // characters the user already typed past the cursor up
                // to (but not including) `]]`, `|`, a newline, `[`, or
                // EOF — preventing artefacts like `[[meeting]]e]]` when
                // the popup is reopened mid-target.
                let new_range = range.start..extent.end;
                let needs_close = !extent.existing_close && !extent.has_alias;
                let new_text = if needs_close {
                    format!("{}]]", suggestion.display)
                } else {
                    suggestion.display.clone()
                };
                let cursor_offset_in_target = suggestion.display.len();
                let new_cursor_byte = if extent.has_alias {
                    // Keep the cursor right before `|alias]]` so the
                    // user can edit the alias next.
                    range.start.saturating_add(cursor_offset_in_target)
                } else {
                    // Land just past `]]` — whether we appended it or
                    // it already existed.
                    range
                        .start
                        .saturating_add(cursor_offset_in_target)
                        .saturating_add(2)
                };
                Some(AcceptAction {
                    range: new_range,
                    new_text,
                    new_cursor_byte,
                })
            }
            TriggerKind::Hashtag | TriggerKind::LinkFilter => {
                let new_cursor_byte = range.start.saturating_add(suggestion.display.len());
                Some(AcceptAction {
                    range,
                    new_text: suggestion.display,
                    new_cursor_byte,
                })
            }
        }
    }
}

/// Where the stale wikilink-target region around the cursor ends, plus
/// what kind of suffix is already present.
struct WikilinkExtent {
    /// Byte offset (≥ `start`) of the first character that is NOT part
    /// of the target region: either the first `]` of an existing `]]`,
    /// a `|` alias separator, a newline, a `[`, or EOF.
    end: usize,
    /// `true` when an existing `]]` follows immediately at `end`.
    existing_close: bool,
    /// `true` when `end` points at a `|` separator — the user has
    /// already started typing an alias which we must preserve.
    has_alias: bool,
}

/// Walk forward from `start` over the bytes that look like wikilink
/// target characters (anything except `]`, `|`, `\n`, `\r`, `[`).
/// Lone `]` bytes (without a following `]`) are treated as stale
/// characters and consumed — invalid inside a wikilink target anyway.
///
/// All decision bytes are ASCII so byte-level scanning is UTF-8 safe.
fn scan_wikilink_extent(buffer: &str, start: usize) -> WikilinkExtent {
    let bytes = buffer.as_bytes();
    let mut i = start.min(bytes.len());
    while i < bytes.len() {
        match bytes[i] {
            b']' => {
                if bytes.get(i + 1) == Some(&b']') {
                    return WikilinkExtent {
                        end: i,
                        existing_close: true,
                        has_alias: false,
                    };
                }
                // Lone `]` — consume and keep scanning. Invalid inside
                // a wikilink target so dropping it is the safe default.
                i += 1;
            }
            b'|' => {
                return WikilinkExtent {
                    end: i,
                    existing_close: false,
                    has_alias: true,
                };
            }
            b'\n' | b'\r' | b'[' => {
                return WikilinkExtent {
                    end: i,
                    existing_close: false,
                    has_alias: false,
                };
            }
            _ => i += 1,
        }
    }
    WikilinkExtent {
        end: i,
        existing_close: false,
        has_alias: false,
    }
}

/// What the controller decided when forwarded a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleKeyOutcome {
    /// Popup was open and consumed the key as navigation.
    Consumed,
    /// Popup was open; user dismissed with Esc.
    Dismissed,
    /// Popup was open; user accepted. The host should apply the action.
    Accepted(AcceptAction),
    /// Popup was either closed or did not handle this key — host should
    /// process it as a normal key event, then call `sync()` afterward.
    NotHandled,
}

/// A buffer replacement the host needs to perform after an accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptAction {
    pub range: Range<usize>,
    pub new_text: String,
    pub new_cursor_byte: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::nfs::VaultPath;
    use kimun_core::{NoteVault, VaultConfig};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tempfile::TempDir;

    /// Global per-test counter so each `FakeHost::new` returns a distinct
    /// `content_revision` and the controller's cache is invalidated between
    /// successive sync calls in the same test (mirrors production where
    /// rebuilding the buffer always advances the editor's revision).
    static FAKE_REV: AtomicU64 = AtomicU64::new(1);

    struct FakeHost {
        buffer: String,
        cursor: usize,
        /// `Some(rev)` to participate in the controller's cache.
        /// `None` mirrors the search-box modal opting out.
        revision: Option<NonZeroU64>,
    }

    impl FakeHost {
        fn new(buffer: &str, cursor: usize) -> Self {
            Self {
                buffer: buffer.to_string(),
                cursor,
                revision: NonZeroU64::new(FAKE_REV.fetch_add(1, Ordering::SeqCst)),
            }
        }

        fn apply(&mut self, action: &AcceptAction) {
            self.buffer
                .replace_range(action.range.clone(), &action.new_text);
            self.cursor = action.new_cursor_byte;
            self.revision = self
                .revision
                .and_then(|r| NonZeroU64::new(r.get().wrapping_add(1)));
        }

        /// Split `self.buffer` into lines and convert the byte cursor
        /// into `(row, char_col)`. Re-derived on every
        /// `buffer_snapshot()` call so tests that mutate
        /// `host.buffer` / `host.cursor` directly stay in sync.
        fn lines_and_cursor(&self) -> (Vec<String>, (usize, usize)) {
            let lines: Vec<String> = self.buffer.split('\n').map(|s| s.to_string()).collect();
            let mut byte_running = 0;
            for (row, line) in lines.iter().enumerate() {
                let line_end = byte_running + line.len();
                if self.cursor <= line_end {
                    let col_byte = self.cursor - byte_running;
                    let col = line[..col_byte].chars().count();
                    return (lines, (row, col));
                }
                byte_running = line_end + 1; // +1 for '\n'
            }
            // Past EOF — clamp to last row's end.
            let row = lines.len().saturating_sub(1);
            let col = lines.get(row).map(|l| l.chars().count()).unwrap_or(0);
            (lines, (row, col))
        }
    }

    impl AutocompleteHost for FakeHost {
        fn buffer_snapshot(&self) -> EditorSnapshot<'_> {
            let rev = self.revision.unwrap_or_else(|| NonZeroU64::new(1).unwrap());
            let (lines, cursor) = self.lines_and_cursor();
            // Owned because we constructed `lines` locally — tests
            // don't hold the snapshot long enough to care about the
            // allocation.
            EditorSnapshot::owned(lines, cursor, rev)
        }
        fn cache_key(&self) -> Option<NonZeroU64> {
            self.revision
        }
        fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
            Some((0, 0))
        }
    }

    async fn new_vault_with(
        notes: &[&str],
        tag_notes: &[(&str, &str)],
    ) -> (TempDir, Arc<NoteVault>) {
        let tmp = TempDir::new().unwrap();
        let cfg = VaultConfig::new(tmp.path().to_path_buf());
        let vault = NoteVault::new(cfg).await.unwrap();
        vault.validate_and_init().await.unwrap();
        for name in notes {
            vault
                .create_note(&VaultPath::note_path_from(format!("/{name}.md")), "body")
                .await
                .unwrap();
        }
        for (path, body) in tag_notes {
            vault
                .create_note(&VaultPath::note_path_from(format!("/{path}.md")), *body)
                .await
                .unwrap();
        }
        (tmp, Arc::new(vault))
    }

    async fn drain_results(controller: &mut AutocompleteController) {
        // The spawned query task completes promptly on an in-memory DB;
        // yield once to let it run and post the result.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        controller.poll_results();
    }

    /// Builds a controller with debounce disabled so tests don't pay the
    /// 80ms refinement window before each query reaches the in-memory DB.
    fn make_controller(vault: Arc<NoteVault>, mode: AutocompleteMode) -> AutocompleteController {
        AutocompleteController::new(vault, mode).with_debounce(Duration::ZERO)
    }

    // ---- Lifecycle ----

    #[tokio::test]
    async fn no_trigger_keeps_popup_closed() {
        let (_tmp, vault) = new_vault_with(&[], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let host = FakeHost::new("plain text", 5);
        c.sync(&host);
        assert!(!c.is_open());
    }

    #[tokio::test]
    async fn wikilink_trigger_opens_popup_and_loads_results() {
        let (_tmp, vault) = new_vault_with(&["meeting", "music", "novel"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        // State exists immediately; is_open() flips to true only once
        // items have arrived (the popup is not "interactive" while a
        // query is in flight).
        assert!(c.state().is_some());
        assert!(!c.is_open());
        drain_results(&mut c).await;
        assert!(c.is_open());
        let st = c.state().unwrap();
        assert_eq!(st.kind, TriggerKind::Wikilink);
        assert_eq!(st.query, "me");
        let names: Vec<&str> = st.items.iter().map(|s| s.display.as_str()).collect();
        assert!(names.contains(&"meeting"));
        assert!(!names.contains(&"novel"));
    }

    #[tokio::test]
    async fn refresh_if_open_closes_on_kind_change() {
        // Popup is open for Hashtag; cursor-only move (refresh_if_open)
        // into a Wikilink context must NOT replace the popup with a
        // wikilink one — close it instead. Opening a wikilink popup
        // on cursor movement violates the refresh-only contract.
        let (_tmp, vault) = new_vault_with(&["meeting"], &[("a", "x #proj")]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("#pro [[me", 4); // cursor after `#pro`
        c.sync(&host);
        drain_results(&mut c).await;
        assert!(c.is_open());
        assert_eq!(c.state().unwrap().kind, TriggerKind::Hashtag);
        // Cursor jumps into the wikilink target (mouse click simulation).
        host.cursor = 9;
        c.refresh_if_open(&host);
        assert!(c.state().is_none(), "kind change on movement must close");
    }

    #[tokio::test]
    async fn accept_with_stale_range_falls_through_not_consumed() {
        // Previously: Tab/Enter on a stale-range accept returned
        // Consumed and silently ate the keystroke. Now returns
        // NotHandled so the host can give the user back their Tab
        // indent / Enter newline.
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        // Live buffer shrinks below the trigger range between sync
        // and accept (e.g. an async event truncated the buffer).
        host.buffer = "see [".into();
        host.cursor = 5;
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        assert_eq!(outcome, HandleKeyOutcome::NotHandled);
        assert!(c.state().is_none(), "popup must close even on fallthrough");
    }

    #[tokio::test]
    async fn refresh_if_open_does_not_open_new_popup() {
        // Cursor moves into a fresh trigger context without any text
        // edit: refresh_if_open must NOT open a popup. This is the
        // behaviour that prevents cursor-only navigation over an
        // existing wikilink from re-popping the suggestions.
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        // Cursor inside an existing wikilink — but the popup is closed.
        let host = FakeHost::new("[[meeting]]", 4);
        c.refresh_if_open(&host);
        assert!(c.state().is_none());
    }

    #[tokio::test]
    async fn refresh_if_open_closes_popup_when_cursor_leaves_trigger() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        assert!(c.is_open());
        // Cursor moves before the `[[` — trigger context is gone.
        host.cursor = 0;
        c.refresh_if_open(&host);
        assert!(c.state().is_none());
    }

    #[tokio::test]
    async fn popup_with_zero_results_is_not_interactive() {
        // Trigger fires but the query returns nothing → state exists but
        // is_open() is false so Esc/Up/Down/Tab fall through to the
        // modal/editor instead of being swallowed.
        let (_tmp, vault) = new_vault_with(&[], &[]).await; // empty vault
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let host = FakeHost::new("see [[xyz", 9);
        c.sync(&host);
        drain_results(&mut c).await;
        assert!(c.state().is_some());
        assert_eq!(c.state().unwrap().items.len(), 0);
        assert!(!c.is_open());
    }

    #[tokio::test]
    async fn hashtag_trigger_opens_popup_and_loads_results() {
        let (_tmp, vault) = new_vault_with(&[], &[("a", "x #projects"), ("b", "y #pro")]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let host = FakeHost::new("about #pro", 10);
        c.sync(&host);
        drain_results(&mut c).await;
        let st = c.state().unwrap();
        assert_eq!(st.kind, TriggerKind::Hashtag);
        let labels: Vec<&str> = st.items.iter().map(|s| s.display.as_str()).collect();
        assert!(labels.contains(&"pro"));
        assert!(labels.contains(&"projects"));
    }

    #[tokio::test]
    async fn hashtag_only_mode_ignores_wikilinks() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::HashtagOnly);
        let host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        assert!(!c.is_open());
    }

    #[tokio::test]
    async fn losing_trigger_context_closes_popup() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        assert!(c.is_open());
        // User types a space — hashtag context for the typed query is now
        // broken; for wikilinks, the trigger stays alive but the query
        // gains a space. Here we simulate the cursor jumping outside.
        host.buffer = "see [[me\n".into();
        host.cursor = 9;
        c.sync(&host);
        assert!(!c.is_open());
    }

    // ---- Accept actions ----

    #[tokio::test]
    async fn accepting_wikilink_inserts_name_and_closes_brackets() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "see [[meeting]]");
        assert_eq!(host.cursor, host.buffer.len());
        assert!(!c.is_open());
    }

    #[tokio::test]
    async fn accepting_wikilink_consumes_stale_chars_before_existing_close() {
        // Reopened mid-target: the user moved the cursor back inside an
        // already-closed wikilink and is replacing the target. The stale
        // characters between the cursor and `]]` must be consumed, not
        // left as `[[meeting]]e]]`.
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me]]", 7); // cursor between `m` and `e`
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "see [[meeting]]");
        assert_eq!(host.cursor, host.buffer.len());
    }

    #[tokio::test]
    async fn accepting_wikilink_with_lone_trailing_bracket_does_not_triple() {
        // Buffer has a single stray `]` after the target — must not
        // produce `]]]`.
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me]", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "see [[meeting]]");
    }

    #[tokio::test]
    async fn accepting_wikilink_preserves_existing_alias() {
        // `[[me|alias]]` — cursor in the target portion; alias must
        // survive and the cursor must land right before `|alias]]`.
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me|alias]]", 8); // cursor before `|`
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "see [[meeting|alias]]");
        // Cursor right after `meeting`, before `|alias]]`.
        assert_eq!(host.cursor, "see [[meeting".len());
    }

    #[tokio::test]
    async fn accepting_wikilink_preserves_existing_closing_brackets() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me]]", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "see [[meeting]]");
        assert_eq!(host.cursor, host.buffer.len());
    }

    #[tokio::test]
    async fn accepting_hashtag_inserts_label_no_trailing_space() {
        let (_tmp, vault) = new_vault_with(&[], &[("a", "x #projects")]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("about #pro", 10);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "about #projects");
        assert_eq!(host.cursor, host.buffer.len());
    }

    #[tokio::test]
    async fn esc_dismisses_without_changing_buffer() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        let host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome = c.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &host);
        assert_eq!(outcome, HandleKeyOutcome::Dismissed);
        assert_eq!(host.buffer, "see [[me");
        assert!(!c.is_open());
    }

    // ---- Generation / drop-stale ----

    #[tokio::test]
    async fn stale_results_are_dropped_on_query_change() {
        let (_tmp, vault) = new_vault_with(&["meeting", "memory"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);
        // First query for `me` — fires generation 1.
        let host1 = FakeHost::new("see [[me", 8);
        c.sync(&host1);
        // Immediately change query to `mem` — fires generation 2 before
        // generation 1 has had a chance to respond.
        let host2 = FakeHost::new("see [[mem", 9);
        c.sync(&host2);
        drain_results(&mut c).await;
        let st = c.state().unwrap();
        // Only the `mem` results should be present — `meeting` doesn't
        // start with `mem` so the only match is `memory`.
        assert_eq!(st.query, "mem");
        let names: Vec<&str> = st.items.iter().map(|s| s.display.as_str()).collect();
        assert_eq!(names, vec!["memory"]);
    }

    /// Regression for the opt-out cache-write skip (originally commit
    /// 5dc15309 against the `revision == 0` sentinel; now expressed
    /// via `content_revision() -> None`). Two invariants:
    ///   1. A sync from an opt-out host (revision == None) must NOT
    ///      populate `cached_text` — the search-box modal would
    ///      otherwise churn the heap allocating String + zones per
    ///      keystroke for a slot nothing ever reads.
    ///   2. An opt-out sync following a cached sync must NOT consult
    ///      the previously-cached entry, even though the cache slot
    ///      is Some(…). Otherwise stale zones from a prior editor
    ///      session could serve a fresh search-box host as a false hit.
    #[tokio::test]
    async fn opt_out_revision_does_not_populate_or_consult_cache() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);

        // (1) Opt-out from a fresh controller: cache stays empty.
        let mut sentinel = FakeHost::new("see [[me", 8);
        sentinel.revision = None;
        c.sync(&sentinel);
        assert!(
            c.cached_text.is_none(),
            "opt-out sync must not write to cached_text"
        );

        // Populate the cache with a normal (cached) reconcile.
        let host = FakeHost::new("see [[me", 8); // FakeHost::new auto-bumps revision
        c.sync(&host);
        let cached_rev = c
            .cached_text
            .as_ref()
            .map(|(rev, _, _)| *rev)
            .expect("cached sync should have populated the cache");

        // (2) Opt-out sync now: must not be served by the stale cache,
        // and must not overwrite the cache slot either.
        let mut sentinel2 = FakeHost::new("see [[nope", 10);
        sentinel2.revision = None;
        c.sync(&sentinel2);
        let preserved_rev = c
            .cached_text
            .as_ref()
            .map(|(rev, _, _)| *rev)
            .expect("opt-out sync should leave the previous cache entry alone");
        assert_eq!(
            preserved_rev, cached_rev,
            "opt-out sync must not overwrite cached_text"
        );
    }

    /// Regression: cursor moves on a host whose `content_revision`
    /// stays constant must HIT the cache slot — same key, same text
    /// pointer, same zones — not rebuild `ExclusionZones` or
    /// re-allocate the buffer text. This is the invariant the
    /// `text_revision` → `content_revision` rename was designed to
    /// preserve cleanly: cursor-only events never invalidate cached
    /// zones.
    #[tokio::test]
    async fn cursor_only_move_within_trigger_hits_cache() {
        let (_tmp, vault) = new_vault_with(&["memory"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);

        // Open the popup at the end of `[[me`.
        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        let (cached_rev, cached_text_ptr) = {
            let (rev, text, _) = c
                .cached_text
                .as_ref()
                .expect("initial sync populates cache");
            (*rev, text.as_ptr())
        };

        // Cursor moves one char back inside the same trigger token.
        // Revision UNCHANGED — controller must serve from cache.
        host.cursor = 7;
        c.sync(&host);
        let (preserved_rev, preserved_text_ptr) = {
            let (rev, text, _) = c
                .cached_text
                .as_ref()
                .expect("cursor-only sync must leave the cache populated");
            (*rev, text.as_ptr())
        };
        assert_eq!(
            preserved_rev, cached_rev,
            "cursor-only sync must not change the cache key"
        );
        assert_eq!(
            preserved_text_ptr, cached_text_ptr,
            "cursor-only sync must reuse the cached String, not rebuild it"
        );
    }

    /// Regression: `close()` must drop the `cached_text` slot. On a
    /// multi-MB note the slot holds a full clone of the buffer plus
    /// the parsed `ExclusionZones`; without the clear it survives
    /// popup dismissal until the next text edit overwrites it.
    #[tokio::test]
    async fn close_clears_cached_text() {
        let (_tmp, vault) = new_vault_with(&["memory"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);

        let host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        assert!(
            c.cached_text.is_some(),
            "sync should have populated cached_text"
        );

        c.close();
        assert!(
            c.cached_text.is_none(),
            "close() must drop cached_text so the buffer clone doesn't outlive the popup"
        );
    }

    /// A `[[` candidate reaches the wikilink veto, so zones are computed
    /// and memoized; a subsequent cursor-only move at the same revision
    /// reuses them rather than recomputing.
    #[tokio::test]
    async fn trigger_candidate_computes_zones_once() {
        let (_tmp, vault) = new_vault_with(&["memory"], &[]).await;
        let mut c = make_controller(vault, AutocompleteMode::Both);

        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        assert!(
            c.cached_text.as_ref().unwrap().2.is_some(),
            "a [[ candidate reaching the veto must compute and memoize zones"
        );

        host.cursor = 7;
        c.sync(&host);
        assert!(
            c.cached_text.as_ref().unwrap().2.is_some(),
            "memoized zones survive a cursor-only move at the same revision"
        );
    }
}
