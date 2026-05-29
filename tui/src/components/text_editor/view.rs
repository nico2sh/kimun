use super::markdown::{MarkdownSpanner, ParsedBuffer, opener_shape};
use super::word_wrap::WordWrapLayout;
use crate::settings::themes::Theme;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;
use std::ops::Range;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthStr;

/// Describes how `view.update`'s Gate 1 modified the parse caches this
/// frame. Read by Gate 2 to decide what subset of `rendered_cache` and
/// `WordWrapLayout` needs to be rebuilt.
#[derive(Debug, Clone)]
enum TextChangeKind {
    /// No text change this frame (cursor-only update). Gate 2 may keep
    /// its caches and only refresh the cursor-row entry.
    None,
    /// Gate 1 took the incremental splice path; only rows in this
    /// range had their ParsedLine entries replaced. Gate 2 should
    /// rebuild rendered_cache only for these rows + the cursor rows.
    Incremental(std::ops::Range<usize>),
    /// Full rebuild (initial parse, line-count change, cap trip,
    /// structural-marker change, post-slice verification miss). Gate 2
    /// must rebuild rendered_cache for every row.
    Full,
}

enum RenderedCacheRebuild {
    Full,
    Rows(Vec<usize>),
    None,
}

#[derive(Clone)]
pub struct MarkdownEditorView {
    pub layout: WordWrapLayout,
    visual_scroll_offset: usize,
    pub lines_snapshot: Vec<String>,
    pub cursor_snapshot: (usize, usize),
    /// Line ranges of every fenced code block in the buffer. Text-keyed
    /// (rebuilt only when `text_revision` changes); `is_in_code_block`
    /// does a cheap point lookup against this list per row so all fenced
    /// blocks render `force_raw` regardless of where the cursor is.
    fence_ranges: Vec<Range<usize>>,
    /// Cursor's last on-screen position (col, row), or `None` when the
    /// cursor was scrolled off-screen or the view was unfocused at the
    /// time of the previous `render`. Used as the anchor for floating
    /// overlays like the autocomplete popup, which is drawn after the
    /// editor itself.
    pub last_cursor_screen: Option<(u16, u16)>,
    /// Per-line parse cache built in `update()`. Eliminates redundant pulldown-cmark
    /// invocations across `render()`, cursor placement, and click mapping.
    /// Either a Real or Placeholder parse — see [`ParseState`].
    parse_state: ParseState,
    /// Last `text_revision` seen — gates the lines clone and parse-cache rebuild.
    /// Cursor-only moves do not bump `text_revision`, so navigating with the
    /// arrow keys reuses the parse cache instead of re-running pulldown-cmark
    /// over the whole buffer.
    last_seen_generation: u64,
    /// `text_revision`/width/cursor at which the layout was last computed.
    /// Used to skip `WordWrapLayout::compute()` when nothing affecting wrap has changed:
    /// horizontal cursor movement within the same element (or plain text) is free.
    last_layout_generation: u64,
    last_layout_width: u16,
    last_layout_cursor: (usize, usize),
    /// Visual row of the cursor, cached after layout so `render()` doesn't call
    /// `logical_to_visual` a second time.
    cursor_vrow: usize,
    /// Per-line rendered-position bitmask, cached between layout recomputes.
    /// Only the two cursor rows (old and new) are rebuilt when just the cursor row changes;
    /// all rows are rebuilt when content or width changes.
    rendered_cache: Vec<Vec<bool>>,
    /// Current selection range in logical (row, byte-col) coordinates.
    /// `None` when no selection is active.
    selection: Option<((usize, usize), (usize, usize))>,
    /// Diagnostic: true when the most recent Gate 1 invocation used the
    /// incremental splice path, false when it took the full-parse fallback.
    /// Read by tests; not part of the production observable surface.
    last_parse_was_incremental: bool,
    /// Diagnostic: which widener tier (`Strict` / `Heuristic`)
    /// produced the most recent successful incremental
    /// splice. `None` when no incremental splice has happened yet
    /// (first parse or full-rebuild fallbacks). Read by unit tests
    /// asserting the chosen widener path.
    last_splice_path: Option<SplicePath>,
    /// Tracks how Gate 1 changed (or did not change) the parse caches.
    /// Gate 2 reads this to decide the scope of rendered_cache rebuild.
    last_text_change: TextChangeKind,
}

/// True when `KIMUN_VIEW_VERIFY_INCREMENTAL=1` is set. Reads the
/// env var once per process and caches. Gates the debug-only
/// full-kinds assertion in Gate 1 that compares every incremental
/// splice against a fresh whole-buffer parse. (The per-splice
/// undamaged-row verify on the heuristic path runs in release
/// unconditionally — see `try_incremental_parse`.)
fn verify_incremental_enabled() -> bool {
    static VERIFY: OnceLock<bool> = OnceLock::new();
    *VERIFY.get_or_init(|| {
        std::env::var("KIMUN_VIEW_VERIFY_INCREMENTAL")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

/// Which widener produced the splice for the most recent successful
/// incremental parse. Test telemetry — read by `last_splice_path`
/// in unit tests to assert the chosen path. Mirror of
/// [`SuccessPath`] but kept private since callers shouldn't depend
/// on widener internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplicePath {
    /// Strict reset-boundary widener (`reset_boundaries`) succeeded.
    Strict,
    /// `widen_to_safe` heuristic succeeded after the strict
    /// reset-boundary widener returned `FullRebuild`.
    Heuristic,
}

/// The editor's per-buffer parse cache: either a fully-styled **Real
/// parse** or an unstyled **Placeholder parse** awaiting a background
/// full parse (see `CONTEXT.md`). Modelling the distinction as a type
/// makes the wrong-splice hazard unrepresentable: splicing is only
/// reachable through [`ParseState::splice_real`], whose `Placeholder`
/// arm is unreachable because Gate 1 declines the incremental path for
/// placeholders. The placeholder's all-`Plain` line kinds would
/// otherwise defeat the structural guards and accept a wrong splice.
#[derive(Clone)]
enum ParseState {
    Real(ParsedBuffer),
    /// `generation` is the `content_revision` the placeholder was
    /// installed for — handed to the owning component so it knows which
    /// buffer to parse on the background task. `spawned` flips true once
    /// that task has been requested, so `take_pending_full_parse` hands
    /// the generation out exactly once.
    Placeholder {
        buf: ParsedBuffer,
        generation: u64,
        spawned: bool,
    },
}

impl ParseState {
    /// State-agnostic buffer access. Render and Gate 2 read the buffer
    /// in both states — the placeholder has valid row counts, so the
    /// downstream path stays in-bounds; only the markdown styling is
    /// missing while it is a placeholder.
    fn buf(&self) -> &ParsedBuffer {
        match self {
            Self::Real(b) | Self::Placeholder { buf: b, .. } => b,
        }
    }

    fn is_placeholder(&self) -> bool {
        matches!(self, Self::Placeholder { .. })
    }

    /// Splice an incremental slice into a Real parse. Called only after
    /// the `is_placeholder()` gate in Gate 1 has declined the
    /// incremental path for placeholders, so the `Placeholder` arm is
    /// unreachable.
    fn splice_real(&mut self, range: std::ops::Range<usize>, slice: ParsedBuffer) {
        match self {
            Self::Real(b) => b.splice(range, slice),
            Self::Placeholder { .. } => {
                debug_assert!(false, "splice on placeholder parse");
            }
        }
    }
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        Self {
            layout: WordWrapLayout::default(),
            visual_scroll_offset: 0,
            lines_snapshot: Vec::new(),
            cursor_snapshot: (0, 0),
            fence_ranges: Vec::new(),
            last_cursor_screen: None,
            // Empty buffer, spliceable — preserves the previous
            // `placeholder_active: false` initial state.
            parse_state: ParseState::Real(ParsedBuffer::placeholder(&[])),
            last_seen_generation: u64::MAX, // force rebuild on first update
            last_layout_generation: u64::MAX,
            last_layout_width: 0,
            last_layout_cursor: (usize::MAX, usize::MAX),
            cursor_vrow: 0,
            rendered_cache: Vec::new(),
            selection: None,
            last_parse_was_incremental: false,
            last_splice_path: None,
            last_text_change: TextChangeKind::Full, // first update is a full rebuild
        }
    }

    /// Threshold above which a fallback to full parse runs
    /// asynchronously instead of blocking the typing thread. On
    /// buffers below this size the full parse is fast enough
    /// (<2ms for a paragraph-only 1000-line buffer per bench) that
    /// blocking is preferable to the one-frame-of-unstyled-text
    /// the async path imposes.
    const LARGE_BUFFER_THRESHOLD: usize = 1000;

    /// Returns `Some(generation)` if Gate 1 just installed a
    /// placeholder `ParsedBuffer` and the owning component should
    /// spawn a background full parse for this generation. Consumes
    /// the flag so the owner does not spawn twice; the owner is
    /// responsible for calling `install_full_parse` when the task
    /// completes.
    /// Whether the most recent Gate 1 invocation took the incremental
    /// splice path. Read-only diagnostic for the incremental-parse
    /// property tests (`tui/tests/incremental_property.rs`); not part
    /// of the production render path.
    pub fn last_parse_was_incremental(&self) -> bool {
        self.last_parse_was_incremental
    }

    pub fn take_pending_full_parse(&mut self) -> Option<u64> {
        if let ParseState::Placeholder {
            generation,
            spawned,
            ..
        } = &mut self.parse_state
        {
            if !*spawned {
                *spawned = true;
                return Some(*generation);
            }
        }
        None
    }

    /// Install the result of a background full parse. No-op when
    /// the editor has advanced past `generation` — that result is
    /// stale and a fresh spawn is already in flight. Invalidates the
    /// layout + rendered_cache so the next `update()` rebuilds Gate
    /// 2 against the fresh `ParsedBuffer`.
    pub fn install_full_parse(&mut self, generation: u64, buf: ParsedBuffer) {
        if generation != self.last_seen_generation {
            return; // stale
        }
        self.parse_state = ParseState::Real(buf);
        self.fence_ranges =
            super::parse_incremental::fence_ranges_from_kinds(&self.parse_state.buf().kinds);
        // Force Gate 2 full rebuild on the next update: the
        // placeholder's all-Plain kinds produced different fence
        // ranges and rendered masks than the real parse will.
        self.last_text_change = TextChangeKind::Full;
        self.last_layout_generation = u64::MAX;
    }

    pub fn update(
        &mut self,
        snap: &super::snapshot::EditorSnapshot<'_>,
        rect: Rect,
        selection: Option<((usize, usize), (usize, usize))>,
    ) {
        // Snapshot owns the (cursor, lines, content_revision) atomicity
        // — readers below can index `parsed_buffer.lines[cursor.0]`
        // without `.get()` guards once Gate 1 has rebuilt the parse
        // cache from these same `lines`.
        let lines: &[String] = &snap.lines;
        let cursor = snap.cursor;
        let generation = snap.content_revision.get();
        self.selection = selection;
        if rect.height == 0 {
            return;
        }

        // Gate 1: content changed — rebuild parse cache and snapshots.
        if generation != self.last_seen_generation {
            let incremental = if self.parse_state.is_placeholder() {
                None
            } else {
                self.try_incremental_parse(lines, cursor)
            };
            self.last_text_change = match incremental {
                Some((range, slice, path)) => {
                    self.parse_state.splice_real(range.clone(), slice);
                    self.last_parse_was_incremental = true;
                    self.last_splice_path = Some(path);
                    TextChangeKind::Incremental(range)
                }
                None => {
                    if lines.len() >= Self::LARGE_BUFFER_THRESHOLD {
                        // Async fallback: install a structurally-
                        // correct but unstyled placeholder so this
                        // frame can paint immediately; defer the
                        // real pulldown parse to a background tokio
                        // task spawned by the owning component (see
                        // `take_pending_full_parse` / `install_full_parse`).
                        // The placeholder has the same row count as
                        // `lines`, so the downstream Gate 2 / render
                        // path stays in-bounds; only the markdown
                        // styling is missing for one frame.
                        self.parse_state = ParseState::Placeholder {
                            buf: ParsedBuffer::placeholder(lines),
                            generation,
                            spawned: false,
                        };
                    } else {
                        self.parse_state = ParseState::Real(ParsedBuffer::parse(lines));
                    }
                    self.last_parse_was_incremental = false;
                    self.last_splice_path = None;
                    TextChangeKind::Full
                }
            };
            #[cfg(debug_assertions)]
            if self.last_parse_was_incremental && verify_incremental_enabled() {
                let fresh = ParsedBuffer::parse(lines);
                assert_eq!(
                    self.parse_state.buf().kinds,
                    fresh.kinds,
                    "incremental kinds diverge from full parse at generation={generation}"
                );
                assert_eq!(
                    self.parse_state.buf().lazy_depth,
                    fresh.lazy_depth,
                    "incremental lazy_depth diverges from full parse at generation={generation}"
                );
                assert_eq!(
                    self.parse_state.buf().reset_boundaries,
                    fresh.reset_boundaries,
                    "incremental reset_boundaries diverge from full parse at generation={generation}"
                );
                assert_eq!(
                    self.parse_state.buf().lines.len(),
                    fresh.lines.len(),
                    "incremental lines.len() diverges from full parse at generation={generation}"
                );
                for (i, (got, exp)) in self
                    .parse_state
                    .buf()
                    .lines
                    .iter()
                    .zip(fresh.lines.iter())
                    .enumerate()
                {
                    got.debug_assert_eq_to(exp, i);
                }
            }
            self.fence_ranges =
                super::parse_incremental::fence_ranges_from_kinds(&self.parse_state.buf().kinds);
            // Incremental update of `lines_snapshot` mirrors the parse
            // path: on the splice path only the rows in `range` can
            // have changed (try_incremental_parse already bails when
            // line count differs); on the full-parse fallback we lose
            // damage info, so re-clone everything.
            //
            // `String::clone_from` reuses the destination's existing
            // allocation when capacity permits, so the typical
            // single-char insert costs one String reallocation
            // (often zero — capacity stays put) instead of N.
            match &self.last_text_change {
                TextChangeKind::Incremental(range) => {
                    debug_assert_eq!(
                        self.lines_snapshot.len(),
                        lines.len(),
                        "incremental path requires equal line counts"
                    );
                    for i in range.clone() {
                        self.lines_snapshot[i].clone_from(&lines[i]);
                    }
                }
                TextChangeKind::Full | TextChangeKind::None => {
                    if self.lines_snapshot.len() == lines.len() {
                        for (dst, src) in self.lines_snapshot.iter_mut().zip(lines.iter()) {
                            dst.clone_from(src);
                        }
                    } else {
                        self.lines_snapshot.clear();
                        self.lines_snapshot.extend(lines.iter().cloned());
                    }
                }
            }
            self.last_seen_generation = generation;
        } else {
            self.last_text_change = TextChangeKind::None;
        }

        self.cursor_snapshot = cursor;

        // Gate 2: layout rebuild.
        // Skip when content, width, and the *effective element expansion* are all unchanged.
        // Horizontal cursor movement within the same element (or plain text with no elements)
        // does not change any wrap boundary — no recompute needed.
        let new_expanded = self
            .parse_state
            .buf()
            .lines
            .get(cursor.0)
            .and_then(|p| p.elem_at(cursor.1));
        let old_expanded = self
            .parse_state
            .buf()
            .lines
            .get(self.last_layout_cursor.0)
            .and_then(|p| p.elem_at(self.last_layout_cursor.1));
        let need_layout = generation != self.last_layout_generation
            || rect.width != self.last_layout_width
            || cursor.0 != self.last_layout_cursor.0
            || new_expanded != old_expanded;

        if need_layout {
            let width_changed = rect.width != self.last_layout_width;
            let cursor_changed = cursor.0 != self.last_layout_cursor.0;
            let expanded_changed = new_expanded != old_expanded;
            // Rows whose rendered mask depends on cursor state and may
            // have flipped this frame: the old and new cursor rows
            // when the cursor moved between rows, OR the cursor row
            // when an inline element (link/bold/etc.) was just expanded
            // or collapsed by a within-row cursor move. Both shapes
            // change `visible_positions_with`'s `expanded` argument,
            // so both rendered_cache AND wrap need to re-derive that
            // row's mask + visual-line splits.
            let cursor_affected_rows: Vec<usize> = if cursor_changed {
                let mut rows = vec![self.last_layout_cursor.0, cursor.0];
                rows.sort();
                rows.dedup();
                rows
            } else if expanded_changed {
                vec![cursor.0]
            } else {
                vec![]
            };
            // Drop any row past the current buffer end — happens when a
            // stale snapshot's cursor row exceeds `lines.len()`. Both
            // rendered_cache and wrap splices require in-range rows.
            let cursor_affected_rows: Vec<usize> = cursor_affected_rows
                .into_iter()
                .filter(|&r| r < lines.len())
                .collect();
            // Determine the set of rows to rebuild in rendered_cache.
            let rebuild_strategy = if self.rendered_cache.len() != lines.len() {
                // Line count differs → full rebuild required.
                RenderedCacheRebuild::Full
            } else {
                match &self.last_text_change {
                    TextChangeKind::Full => RenderedCacheRebuild::Full,
                    TextChangeKind::Incremental(range) => {
                        let mut rows: Vec<usize> = range.clone().collect();
                        rows.extend(cursor_affected_rows.iter().copied());
                        rows.sort();
                        rows.dedup();
                        RenderedCacheRebuild::Rows(rows)
                    }
                    TextChangeKind::None => {
                        if cursor_affected_rows.is_empty() {
                            RenderedCacheRebuild::None
                        } else {
                            RenderedCacheRebuild::Rows(cursor_affected_rows.clone())
                        }
                    }
                }
            };

            // Width-only change: masks are width-independent; skip rendered_cache rebuild.
            let _ = width_changed; // acknowledged: width doesn't affect rendered_cache
            match rebuild_strategy {
                RenderedCacheRebuild::Full => {
                    self.rendered_cache = lines
                        .iter()
                        .enumerate()
                        .map(|(i, l)| {
                            let force_raw = self.is_in_code_block(i);
                            let cursor_col = if i == cursor.0 { Some(cursor.1) } else { None };
                            MarkdownSpanner::visible_positions_with(
                                l,
                                &self.parse_state.buf().lines[i],
                                cursor_col,
                                force_raw,
                            )
                        })
                        .collect();
                }
                RenderedCacheRebuild::Rows(rows) => {
                    for row in rows {
                        if row >= lines.len() {
                            continue; // defensive
                        }
                        let force_raw = self.is_in_code_block(row);
                        let cursor_col = if row == cursor.0 {
                            Some(cursor.1)
                        } else {
                            None
                        };
                        let new_entry = MarkdownSpanner::visible_positions_with(
                            &lines[row],
                            &self.parse_state.buf().lines[row],
                            cursor_col,
                            force_raw,
                        );
                        if let Some(entry) = self.rendered_cache.get_mut(row) {
                            *entry = new_entry;
                        }
                    }
                }
                RenderedCacheRebuild::None => {
                    // Width-only change or no change: masks are width-independent; nothing to rebuild.
                }
            }

            // Width-aware wrap path:
            // - Width change or line-count change: full recompute (wrap
            //   depends on width; visual_lines indexing depends on row count).
            // - TextChangeKind::Full: full recompute.
            // - TextChangeKind::Incremental(range): splice the edited
            //   rows plus any cursor-affected rows whose mask flipped.
            // - TextChangeKind::None: splice only the cursor-affected
            //   rows. Wrap depends on the rendered mask
            //   (`wrap_one_row` reads `rendered_row`), and the mask is
            //   cursor-position-sensitive whenever the cursor crosses
            //   an inline element boundary — same row or different
            //   row.
            let line_count_changed = self.layout.row_starts_len() != lines.len();
            if width_changed || line_count_changed {
                self.layout = WordWrapLayout::compute(lines, rect.width, &self.rendered_cache);
            } else {
                match &self.last_text_change {
                    TextChangeKind::Full => {
                        self.layout =
                            WordWrapLayout::compute(lines, rect.width, &self.rendered_cache);
                    }
                    TextChangeKind::Incremental(range) => {
                        let start = range
                            .start
                            .min(cursor_affected_rows.first().copied().unwrap_or(range.start));
                        let end = range.end.max(
                            cursor_affected_rows
                                .last()
                                .copied()
                                .map(|r| r + 1)
                                .unwrap_or(range.end),
                        );
                        self.layout.splice_range(
                            lines,
                            rect.width,
                            &self.rendered_cache,
                            start..end,
                        );
                    }
                    TextChangeKind::None => {
                        if let (Some(&first), Some(&last)) =
                            (cursor_affected_rows.first(), cursor_affected_rows.last())
                        {
                            self.layout.splice_range(
                                lines,
                                rect.width,
                                &self.rendered_cache,
                                first..last + 1,
                            );
                        }
                    }
                }
            }
            self.last_layout_generation = generation;
            self.last_layout_width = rect.width;
            self.last_layout_cursor = cursor;
        }

        // Cache cursor_vrow for render() — avoids a second logical_to_visual call.
        self.cursor_vrow = self.layout.logical_to_visual(cursor.0, cursor.1).0;
        let height = rect.height as usize;
        if self.cursor_vrow < self.visual_scroll_offset {
            self.visual_scroll_offset = self.cursor_vrow;
        } else if self.cursor_vrow >= self.visual_scroll_offset + height {
            self.visual_scroll_offset = self.cursor_vrow - height + 1;
        }
    }

    /// Attempt an incremental Gate-1 parse.
    ///
    /// Returns `Some((range, slice, path))` when the damage can be
    /// cheaply isolated and widened to safe boundaries; `None` when
    /// the caller should fall back to a fresh full-buffer
    /// `ParsedBuffer::parse`. The `path` indicates which widener
    /// tier produced the splice (see [`SplicePath`]).
    fn try_incremental_parse(
        &self,
        lines: &[String],
        cursor: (usize, usize),
    ) -> Option<(std::ops::Range<usize>, ParsedBuffer, SplicePath)> {
        use super::parse_incremental::{
            LineConstructKind, WidenResult, compute_damage_range, expand_to_reset_boundary,
            widen_to_safe,
        };
        use super::widener_metrics::{BailReason, METRICS, SuccessPath};

        if self.parse_state.buf().lines.is_empty() {
            return None; // First parse — no snapshot to diff against. Uncategorised.
        }
        // Line count changes (insertions/deletions) require a full rebuild:
        // the widened range covers the same number of lines in the new buffer
        // as in the old kinds array, so a splice cannot reconcile the length
        // mismatch.
        if lines.len() != self.parse_state.buf().lines.len() {
            return METRICS.bail(BailReason::LineCountChange);
        }
        let Some(damaged) = compute_damage_range(&self.lines_snapshot, lines, cursor.0) else {
            return METRICS.bail(BailReason::NoDamage);
        };

        // Structural-marker change guard: any edit that converts a fence
        // marker line into a non-marker (or vice versa) can shift the
        // fence's extent beyond the widening window. Same for setext
        // underlines. Conservative fallback to full parse for correctness.
        for row in damaged.clone() {
            let old_kind = self.parse_state.buf().kinds[row];
            let old_line = self.lines_snapshot[row].as_str();
            let new_line = lines[row].as_str();

            // Old kind was a structural marker whose role an in-place edit
            // can change (fence opener↔closer↔content, setext underline
            // re-heading the line above) or which lazy-extends past the
            // widening window (indented code / HTML block per CommonMark
            // §4.4 / §4.6). These read pulldown's real classification, so
            // any edit on such a row punts to a full parse.
            if matches!(
                old_kind,
                LineConstructKind::FenceMarker
                    | LineConstructKind::SetextUnderline
                    | LineConstructKind::IndentedCode
                    | LineConstructKind::HtmlBlock
            ) {
                return METRICS.bail(BailReason::KindGuard);
            }
            // Context-free block-opener shape flip: the edit gained or lost
            // a fence / setext / indented-code / HTML / list / blockquote
            // opener shape. Any such flip can open or close a (possibly
            // lazy-continuable) construct that reshapes the document beyond
            // the widening window — e.g. `"x"` → `"* x"` next to a
            // blank-separated list leaks a loose-list merge. Comparing the
            // whole `OpenerShape` catches a flip in any field at once.
            if opener_shape(new_line) != opener_shape(old_line) {
                return METRICS.bail(BailReason::KindGuard);
            }

            // V2 lazy-construct neighbourhood guard: edit at row R
            // can re-shape a lazy construct open at R-1, R, or R+1.
            // R-1: blockquote paragraph lazy-continuation across a
            // former blank (§5.1). R: edit inside the construct. R+1:
            // paragraph eating a would-be IndentedCode start.
            //
            // §3.0 conditional relaxation (intra-construct-reset-boundaries):
            // when the damaged row's old kind is ListMarker AND
            // lazy_depth[row] == 1 (a top-level list, not nested inside
            // an outer lazy construct), the bail is skipped. List-marker
            // content edits are safe by construction: per-row
            // ListMarker/ListContinuation classification stays identical
            // across slice-vs-parent, and rows past widened.end are
            // unaffected by the slice's list-vs-non-list determination.
            // The widener's heuristic tier (widen_to_safe over the
            // loose-list blanks; or, on small buffers, the strict tier
            // widening to the whole buffer) takes the splice. The
            // post-slice verify backs this. The opener-shape /
            // blank-transition flips run as
            // separate guards above and below this check, so the relax
            // only ever fires on pure content edits.
            //
            // Initial relaxation also accepted ListContinuation +
            // Blockquote + Plain and arbitrary lazy_depth; both unlocks
            // reverted after the 100k proptest soak exposed downstream-
            // row-classification flips past widened.end that the
            // post-slice verify (which only covers rows INSIDE widened)
            // doesn't catch. The deeper fix is a post-widening sanity
            // check on `widened.end + 1` — see the design doc's
            // "Blockquote/Plain/ListContinuation unlocks" follow-up.
            let lazy = &self.parse_state.buf().lazy_depth;
            if lazy.is_empty() {
                // Defensive: invariant violation (lazy_depth.len() should
                // match lines.len()). Count as KindGuard to keep the
                // attempted-vs-success accounting consistent.
                return METRICS.bail(BailReason::KindGuard);
            }
            let lo = row.saturating_sub(1);
            let hi = (row + 1).min(lazy.len() - 1);
            if lazy[lo..=hi].iter().any(|&d| d > 0) {
                // §3.0 conditional relaxation — TIGHT VERSION.
                // Qualifying conditions (narrowed across two soak
                // rounds — see openspec change for the rationale):
                //   - old_kind == ListMarker (NOT ListContinuation)
                //   - lazy_depth[row] == 1 (top-level list only)
                //
                // ListContinuation rows are excluded after the 100k
                // soak surfaced a case where an edit on a
                // ListContinuation row (specifically a `>     ` row
                // inside a list, lazy_depth=1) caused the row AT
                // `damaged.end` (a blank, lazy_depth=0 in pre-edit)
                // to flip to ListContinuation in post-edit fresh
                // parse. The strict reset boundary at that row was
                // valid pre-edit but became invalid post-edit, and
                // the splice chose a widened range based on
                // pre-edit boundaries that didn't capture the new
                // row past `widened.end`.
                //
                // ListMarker rows are immune: a content edit on
                // "- a" → "- aX" cannot change row+1's classification
                // because the row+1 was either (a) Plain → became
                // ListContinuation via the post-pass regardless of
                // the edit, or (b) Blank/something-else that's outside
                // the list and unaffected by item-content changes.
                //
                // The depth==1 clause blocks edits on lists nested
                // inside another lazy construct (a list inside a
                // blockquote) where the OUTER construct can shift.
                //
                // Blockquote / Plain / ListContinuation unlocks are
                // deferred to a follow-up that adds a post-widening
                // sanity check on `widened.end + 1` (cheap re-parse
                // of one extra row to detect downstream flips).
                let kind_qualifies = matches!(old_kind, LineConstructKind::ListMarker);
                let depth_qualifies = row < lazy.len() && lazy[row] == 1;
                if kind_qualifies && depth_qualifies {
                    // Don't bail — let blank-transition guard run
                    // and reach the widener stage.
                } else {
                    return METRICS.bail(BailReason::LazyDepth);
                }
            }

            // V2 blank-transition guard: a row flipping between blank
            // and non-blank invalidates the pre-edit reset boundary
            // at that row in the post-edit world (paragraph lazy-
            // continuation, empty list-item shapes like `*` that
            // parse as ListMarker in slice but as paragraph
            // continuation in full). Use the pre-edit `kinds` for
            // the "blank" classification instead of `line.trim()` so
            // the predicate matches the parser's view exactly.
            let old_blank = matches!(old_kind, LineConstructKind::Blank);
            let new_blank = new_line.trim().is_empty();
            if old_blank != new_blank {
                let above_non_blank = row > 0
                    && !matches!(
                        self.parse_state.buf().kinds[row - 1],
                        LineConstructKind::Blank
                    );
                let below_non_blank = row + 1 < self.parse_state.buf().kinds.len()
                    && !matches!(
                        self.parse_state.buf().kinds[row + 1],
                        LineConstructKind::Blank
                    );
                if above_non_blank || below_non_blank {
                    return METRICS.bail(BailReason::BlankTransition);
                }
            }
        }

        // Two-tier widener:
        //
        //   1. `expand_to_reset_boundary(reset_boundaries, ...)` —
        //      strict. Provably equivalent to a fresh parse; no
        //      post-slice verify needed.
        //   2. `widen_to_safe` — heuristic fallback. NOT provably
        //      equivalent; the post-slice verify (below, release-on)
        //      is the correctness mechanism and bails to a full
        //      rebuild on any divergence.
        //
        // After a §3.0 relax fires the strict widener usually
        // cap-trips (lazy_depth > 0 around the edit means no nearby
        // blank-with-depth-0 reset boundary), but we still try strict
        // first — it costs only a binary search and succeeds in
        // degenerate cases (e.g. small buffers where strict widens
        // safely to the whole buffer). On failure we fall to
        // widen_to_safe.
        //
        // A former middle tier (`intra_construct_boundaries`, the V3
        // "IntraConstruct" path) was removed: it fired only on loose-
        // list edits and `widen_to_safe` covers every such case with
        // zero extra full rebuilds (measured), differing only in
        // reparse span (~11 vs ~2 rows — both far under the 256 cap).
        let mut splice_path = SplicePath::Strict;
        let widened = match expand_to_reset_boundary(
            &self.parse_state.buf().reset_boundaries,
            self.parse_state.buf().lines.len(),
            damaged.clone(),
        ) {
            WidenResult::Widened(r) => r,
            WidenResult::FullRebuild => {
                match widen_to_safe(&self.parse_state.buf().kinds, damaged.clone()) {
                    WidenResult::Widened(r) => {
                        splice_path = SplicePath::Heuristic;
                        r
                    }
                    WidenResult::FullRebuild => return METRICS.bail(BailReason::CapTrip),
                }
            }
        };
        let slice = ParsedBuffer::parse_range(lines, widened.clone());

        // Post-slice undamaged-row verification.
        //
        // - Strict path: skipped. Provably equivalent to a fresh
        //   parse (see `reset_boundaries` docstring).
        // - Heuristic path: NOT provably equivalent, so this verify
        //   is the correctness mechanism and runs in release. It is
        //   cheap: `slice` was already parsed above
        //   (unconditionally), and the loop only compares
        //   kinds/elements.len()/content_vis over the `widened` rows —
        //   bounded by the widen cap (≤256), negligible against the
        //   parse_range that already ran. A divergence (e.g. a pulldown
        //   version bump changing tokenisation) bails to a full rebuild
        //   rather than shipping a corrupt splice. The 600k proptest
        //   cases (100k × 6 strategies, 0 verify_failed) stay in the
        //   regression harness; this guard is the release backstop.
        let verify_eligible_path = matches!(splice_path, SplicePath::Heuristic);
        if verify_eligible_path {
            for row in widened.clone() {
                if damaged.contains(&row) {
                    continue; // Damaged row: kind change is expected/irrelevant.
                }
                let idx = row - widened.start;
                if slice.kinds[idx] != self.parse_state.buf().kinds[row] {
                    return METRICS.bail(BailReason::VerifyFailed);
                }
                if slice.lines[idx].elements.len()
                    != self.parse_state.buf().lines[row].elements.len()
                {
                    return METRICS.bail(BailReason::VerifyFailed);
                }
                if slice.lines[idx].content_vis != self.parse_state.buf().lines[row].content_vis {
                    return METRICS.bail(BailReason::VerifyFailed);
                }
            }
        }

        METRICS.ok(match splice_path {
            SplicePath::Strict => SuccessPath::ResetBoundary,
            SplicePath::Heuristic => SuccessPath::WidenToSafe,
        });
        Some((widened, slice, splice_path))
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        if rect.height == 0 {
            return;
        }
        let lines = &self.lines_snapshot;
        let cursor = self.cursor_snapshot;
        let scroll = self.visual_scroll_offset;
        let height = rect.height as usize;
        let vlines = self.layout.visual_lines();

        let selection = self.selection;
        let parsed_lines = &self.parse_state.buf().lines;
        let fence_ranges = &self.fence_ranges;

        let visible: Vec<Line> = vlines
            .iter()
            .skip(scroll)
            .take(height)
            .map(|vl| {
                let cursor_col = if vl.logical_row == cursor.0 {
                    Some(cursor.1)
                } else {
                    None
                };
                let force_raw = fence_ranges.iter().any(|r| r.contains(&vl.logical_row));
                // Snapshot invariant: every `vl.logical_row` is < lines.len()
                // because `layout` and `lines_snapshot` were rebuilt from
                // the same `EditorSnapshot` in the last `update()`.
                let logical_line = lines[vl.logical_row].as_str();
                let parsed = &parsed_lines[vl.logical_row];
                let content = vl.content(logical_line);
                let spans = MarkdownSpanner::render_with(
                    content,
                    logical_line,
                    parsed,
                    vl.start_col,
                    cursor_col,
                    vl.is_first_visual_line,
                    force_raw,
                    rect.width,
                    theme,
                );

                // Apply selection highlight if this visual line is within the selection.
                let spans = if let Some(((sel_sr, sel_sc), (sel_er, sel_ec))) = selection {
                    let row = vl.logical_row;
                    if row >= sel_sr && row <= sel_er {
                        let start_rendered = if row == sel_sr {
                            MarkdownSpanner::rendered_cursor_col_with(
                                logical_line,
                                parsed,
                                vl.start_col,
                                sel_sc,
                                vl.is_first_visual_line,
                                force_raw,
                            )
                        } else {
                            0
                        };
                        let end_rendered = if row == sel_er {
                            MarkdownSpanner::rendered_cursor_col_with(
                                logical_line,
                                parsed,
                                vl.start_col,
                                sel_ec,
                                vl.is_first_visual_line,
                                force_raw,
                            )
                        } else {
                            // Entire line is selected; use a sentinel larger than any line width.
                            u16::MAX as usize
                        };
                        apply_selection_highlight(spans, start_rendered..end_rendered, theme)
                    } else {
                        spans
                    }
                } else {
                    spans
                };

                Line::from(spans)
            })
            .collect();

        f.render_widget(
            Paragraph::new(Text::from(visible)).style(theme.base_style()),
            rect,
        );

        // Draw terminal cursor when focused. The `EditorSnapshot` the
        // last `update()` consumed guarantees `cursor.0` is in-bounds
        // for `parsed_buffer.lines` and `layout.visual_lines()` —
        // both were rebuilt from the same snapshot. The single
        // remaining edge case is an empty buffer (no rows at all),
        // handled by the early `is_empty` short-circuit below; the
        // previous defensive `.get()` chain (commit c03dc728) was
        // there to absorb stale Nvim snapshots where cursor outran
        // lines, which the snapshot invariant now rules out.
        self.last_cursor_screen = None;
        if focused
            && !self.parse_state.buf().lines.is_empty()
            && !self.layout.visual_lines().is_empty()
        {
            let cursor_vrow = self.cursor_vrow;
            if cursor_vrow >= scroll && cursor_vrow < scroll + height {
                let vl = &self.layout.visual_lines()[cursor_vrow];
                let parsed = &self.parse_state.buf().lines[cursor.0];
                // Snapshot invariant + outer `!is_empty()` guard: cursor.0
                // is in-bounds for `lines_snapshot` here.
                let logical_line = lines[cursor.0].as_str();
                let force_raw = self.is_in_code_block(cursor.0);
                let rendered_col = MarkdownSpanner::rendered_cursor_col_with(
                    logical_line,
                    parsed,
                    vl.start_col,
                    cursor.1,
                    vl.is_first_visual_line,
                    force_raw,
                );
                let cx = rect.x + rendered_col as u16;
                let cy = rect.y + (cursor_vrow - scroll) as u16;
                f.set_cursor_position(Position { x: cx, y: cy });
                self.last_cursor_screen = Some((cx, cy));
            }
        }
    }

    /// Test accessor: the kinds vector of the current parsed buffer.
    /// Used by the proptest harness to assert incremental = full parse.
    pub fn parsed_buffer_kinds(&self) -> &[super::parse_incremental::LineConstructKind] {
        &self.parse_state.buf().kinds
    }

    /// Test accessor: the parsed lines of the current parsed buffer.
    pub fn parsed_buffer_lines(&self) -> &[super::markdown::ParsedLine] {
        &self.parse_state.buf().lines
    }

    /// Test accessor: the rendered-position bitmask cache.
    /// Used by tests to construct a fresh `WordWrapLayout` from the same
    /// masks the view is using, for equivalence checks.
    #[cfg(test)]
    pub(crate) fn rendered_cache_for_testing(&self) -> &[Vec<bool>] {
        &self.rendered_cache
    }

    fn is_in_code_block(&self, row: usize) -> bool {
        // Every line inside any fenced block renders force-raw (no markdown
        // re-styling, distinct fg color). Previously this checked only the
        // fence the cursor was sitting in, so fenced blocks elsewhere in
        // the buffer looked like plain text until the cursor moved into
        // them.
        self.fence_ranges.iter().any(|r| r.contains(&row))
    }

    /// Markdown-aware mouse click: maps a rendered screen column to
    /// the correct logical column, accounting for hidden markdown
    /// sigils (links, bold markers, etc.).
    ///
    /// Reads `self`'s view-internal caches (`layout`, `lines_snapshot`,
    /// `parsed_buffer`), all rebuilt from the same `EditorSnapshot`
    /// in the last `update()` call. The snapshot invariant guarantees
    /// `vl.logical_row` is a valid index into both `lines_snapshot`
    /// and `parsed_buffer.lines`, so direct indexing is safe — the
    /// previous defensive `(Some, Some) else fallback` block (Fix #2
    /// in the holistic review) is no longer needed.
    /// Map a screen-relative click (row/col offset from the editor's
    /// top-left corner) to logical (row, col). Owns the
    /// visual-scroll-offset arithmetic so callers do not reach into
    /// `visual_scroll_offset` — the view knows where it is scrolled.
    pub fn click_at_screen(&self, screen_row: usize, screen_col: usize) -> (u16, u16) {
        let vrow = screen_row + self.visual_scroll_offset;
        self.click_to_logical_u16(vrow, screen_col)
    }

    fn click_to_logical_u16(&self, vrow: usize, vcol: usize) -> (u16, u16) {
        let vlines = self.layout.visual_lines();
        if vlines.is_empty() {
            return (0, 0);
        }
        let vrow = vrow.min(vlines.len() - 1);
        let vl = &vlines[vrow];
        let row_u16 = vl.logical_row.min(u16::MAX as usize) as u16;
        let logical_line = self.lines_snapshot[vl.logical_row].as_str();
        let parsed = &self.parse_state.buf().lines[vl.logical_row];
        let force_raw = self.is_in_code_block(vl.logical_row);
        let logical_col = MarkdownSpanner::rendered_col_to_logical_with(
            logical_line,
            parsed,
            vl.start_col,
            vcol,
            vl.is_first_visual_line,
            force_raw,
        );
        let col = logical_col.min(u16::MAX as usize) as u16;
        (row_u16, col)
    }
}

impl Default for MarkdownEditorView {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the byte offset into `s` after consuming exactly `target_width` display columns.
/// If `target_width` exceeds the string's display width, returns `s.len()`.
fn byte_offset_for_display_width(s: &str, target_width: usize) -> usize {
    let mut consumed = 0usize;
    for (byte_pos, ch) in s.char_indices() {
        if consumed >= target_width {
            return byte_pos;
        }
        consumed += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    s.len()
}

/// Re-style spans to apply `bg_selected` over the given rendered-column range.
///
/// `sel_cols` is a range of rendered (screen) column offsets within the visual line.
/// Spans that overlap the range are split at the boundaries; the overlapping portion
/// receives `.bg(theme.bg_selected)`. Non-overlapping portions keep their original style.
fn apply_selection_highlight<'a>(
    spans: Vec<ratatui::text::Span<'a>>,
    sel_cols: std::ops::Range<usize>,
    theme: &Theme,
) -> Vec<ratatui::text::Span<'a>> {
    if sel_cols.is_empty() {
        return spans;
    }

    let highlight_bg = theme.bg_selected.to_ratatui();
    let mut result = Vec::new();
    let mut col = 0usize;

    for span in spans {
        let content: &str = &span.content;
        let span_width = content.width();
        let span_end = col + span_width;

        let overlap_start = sel_cols.start.max(col);
        let overlap_end = sel_cols.end.min(span_end);

        if overlap_start >= overlap_end {
            // No overlap — emit as-is.
            result.push(span);
        } else {
            // Walk grapheme clusters by display width to find byte boundaries.
            let prefix_width = overlap_start - col;
            let selected_width = overlap_end - overlap_start;

            let prefix_byte = byte_offset_for_display_width(content, prefix_width);
            let selected_byte_end =
                byte_offset_for_display_width(&content[prefix_byte..], selected_width)
                    + prefix_byte;

            // Prefix (before selection)
            if prefix_byte > 0 {
                result.push(ratatui::text::Span::styled(
                    content[..prefix_byte].to_string(),
                    span.style,
                ));
            }
            // Selected portion
            result.push(ratatui::text::Span::styled(
                content[prefix_byte..selected_byte_end].to_string(),
                span.style.bg(highlight_bg),
            ));
            // Suffix (after selection)
            if selected_byte_end < content.len() {
                result.push(ratatui::text::Span::styled(
                    content[selected_byte_end..].to_string(),
                    span.style,
                ));
            }
        }

        col = span_end;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use std::num::NonZeroU64;

    fn rect(h: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 40,
            height: h,
        }
    }

    /// Test-only wrapper that builds an `EditorSnapshot::borrowed`
    /// from the legacy `(lines, cursor, generation)` shape, so the
    /// hundreds of existing call sites don't each have to construct
    /// the snapshot inline.
    ///
    /// Mirrors `snapshot_from_backend`'s producer-side cursor clamp,
    /// so tests that pass an intentionally-stale `cursor` (e.g. the
    /// regression for the Nvim shrink panic) still exercise the
    /// real production path: producer clamps, render trusts.
    fn update_view(
        v: &mut MarkdownEditorView,
        lines: &[String],
        cursor: (usize, usize),
        rect: Rect,
        generation: u64,
        selection: Option<((usize, usize), (usize, usize))>,
    ) {
        let rev = NonZeroU64::new(generation.max(1)).unwrap();
        let clamped = if lines.is_empty() {
            (0, 0)
        } else {
            (cursor.0.min(lines.len() - 1), cursor.1)
        };
        let snap = super::super::snapshot::EditorSnapshot::borrowed(lines, clamped, rev);
        v.update(&snap, rect, selection);
    }

    #[test]
    fn new_has_zero_scroll() {
        assert_eq!(MarkdownEditorView::new().visual_scroll_offset, 0);
    }

    #[test]
    fn zero_height_rect_does_not_panic() {
        let mut v = MarkdownEditorView::new();
        update_view(&mut v, &["hello".to_string()], (0, 0), rect(0), 1, None);
    }

    #[test]
    fn scroll_follows_cursor_down() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        update_view(&mut v, &lines, (4, 0), rect(3), 1, None);
        assert!(v.visual_scroll_offset >= 2);
    }

    #[test]
    fn scroll_follows_cursor_up() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        update_view(&mut v, &lines, (4, 0), rect(3), 1, None);
        update_view(&mut v, &lines, (0, 0), rect(3), 1, None); // same generation — scroll still adjusts
        assert_eq!(v.visual_scroll_offset, 0);
    }

    #[test]
    fn visual_to_logical_u16_accounts_for_scroll() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..10).map(|i| format!("line{}", i)).collect();
        update_view(&mut v, &lines, (5, 0), rect(3), 1, None);
        let scroll = v.visual_scroll_offset;
        let (row, _col) = v.click_to_logical_u16(scroll, 0);
        assert_eq!(row as usize, scroll);
    }

    #[test]
    fn code_block_detection_cursor_inside() {
        let lines = vec![
            "text".to_string(),
            "```rust".to_string(),
            "let x = 1;".to_string(),
            "```".to_string(),
            "more".to_string(),
        ];
        let pb = ParsedBuffer::parse(&lines);
        let ranges = super::super::parse_incremental::fence_ranges_from_kinds(&pb.kinds);
        let block = ranges.iter().find(|r| r.contains(&2)).cloned();
        assert!(block.is_some());
        let r = block.unwrap();
        assert_eq!(r.start, 1);
        assert_eq!(r.end, 4);
    }

    #[test]
    fn code_block_detection_cursor_outside() {
        let lines = vec![
            "text".to_string(),
            "```".to_string(),
            "code".to_string(),
            "```".to_string(),
        ];
        let pb = ParsedBuffer::parse(&lines);
        let ranges = super::super::parse_incremental::fence_ranges_from_kinds(&pb.kinds);
        assert!(ranges.iter().find(|r| r.contains(&0)).is_none());
    }

    #[test]
    fn click_to_logical_does_not_panic_on_stale_layout() {
        // Regression: click_to_logical_u16 raw-indexed parsed_buffer.lines
        // by vl.logical_row. A stale layout whose visual_lines outlive a
        // shrink of parsed_buffer.lines would panic on mouse click. The
        // guard now falls back to a raw visual-col mapping.
        let mut v = MarkdownEditorView::new();
        let long: Vec<String> = (0..20).map(|i| format!("line{}", i)).collect();
        update_view(&mut v, &long, (0, 0), rect(10), 1, None);
        // Drive a shrink so layout.visual_lines outruns parsed_buffer.lines
        // briefly. update() rebuilds layout from the new lines, so the
        // pure shrink shouldn't desynchronize them — but we still want a
        // black-box test that simulates a click against the last vrow.
        let vrows = v.layout.visual_lines().len();
        if vrows > 0 {
            let _ = v.click_to_logical_u16(vrows.saturating_sub(1), 0);
            let _ = v.click_to_logical_u16(vrows + 5, 0);
        }
    }

    #[test]
    fn render_does_not_panic_on_stale_cursor_past_line_count() {
        // Regression: render() previously did self.parsed_cache[cursor.0]
        // and self.layout.visual_lines()[cursor_vrow] directly. A stale
        // Nvim snapshot whose cursor row landed past the new line count
        // would panic the render thread. Now the test exercises the
        // producer-side clamp (via `update_view`'s mirror of
        // `snapshot_from_backend`): the snapshot constructor clamps
        // the cursor, render trusts the invariant, and direct
        // indexing is safe.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::gruvbox_dark();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut v = MarkdownEditorView::new();
        // Populate with 2 lines and a valid cursor first so parsed_cache /
        // layout are non-empty.
        update_view(
            &mut v,
            &["alpha".to_string(), "beta".to_string()],
            (0, 0),
            rect(8),
            1,
            None,
        );
        // Now feed a cursor row that exceeds the line count for this update
        // (simulates a stale snapshot arriving after a shrink). update() at
        // line 277 already uses `lines.get(cursor.0)` so it won't panic; the
        // real risk was the [] indexes inside render(). cursor_snapshot ends
        // up at (5, 0) which exceeds the parsed_cache len of 2 below.
        update_view(
            &mut v,
            &["alpha".to_string(), "beta".to_string()],
            (5, 0),
            rect(8),
            1,
            None,
        );
        // Render with focus so the cursor branch runs.
        terminal
            .draw(|f| v.render(f, f.area(), &theme, true))
            .expect("render must not panic on stale cursor");
    }

    #[test]
    fn cursor_into_link_refreshes_layout_for_same_row() {
        // Regression: when the cursor moves within a row, crossing into
        // or out of an expandable inline element (link/bold/etc.), the
        // rendered mask flips (the element reveals or hides its hidden
        // sigils). Both rendered_cache and the wrap layout depend on
        // the mask. Previously Gate 2 took the `TextChangeKind::None`
        // wrap branch and skipped re-splicing, leaving stale visual
        // lines until the next text edit.
        //
        // Use a link whose hidden URL is long enough that revealing it
        // forces an extra wrap line at width 40 — that lets us
        // black-box detect the mask flip via visual_lines.len().
        let mut v = MarkdownEditorView::new();
        let lines =
            vec!["see [link](http://example.com/very/long/path/to/some/page) more".to_string()];
        // First update: cursor outside the link (col 0).
        update_view(&mut v, &lines, (0, 0), rect(5), 1, None);
        let n_outside = v.layout.visual_lines().len();

        // Second update: cursor inside the link element.
        update_view(&mut v, &lines, (0, 8), rect(5), 1, None);
        let layout_inside = v.layout.visual_lines().to_vec();

        // Fresh view with cursor already inside must produce the same layout.
        let mut fresh = MarkdownEditorView::new();
        update_view(&mut fresh, &lines, (0, 8), rect(5), 1, None);
        let layout_fresh = fresh.layout.visual_lines().to_vec();
        assert_eq!(
            layout_inside, layout_fresh,
            "post-move layout must match a fresh full-recompute"
        );
        assert!(
            layout_inside.len() > n_outside,
            "expanding the link's hidden URL must produce more visual lines"
        );
    }

    #[test]
    fn try_incremental_parse_falls_back_on_indented_code_flip() {
        // Regression: a Plain row flipping to IndentedCode (4 leading
        // spaces) can lazy-extend an indented-code block across the
        // following Plain rows in the full buffer. The widened slice
        // can't see that context. Guard must trip fallback.
        let mut v = MarkdownEditorView::new();
        let lines = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(20), 1, None);
        let new_lines = vec![
            "alpha".to_string(),
            "    beta".to_string(),
            "gamma".to_string(),
        ];
        // try_incremental_parse must return None (full-rebuild signal).
        assert!(
            v.try_incremental_parse(&new_lines, (1, 0)).is_none(),
            "indented-code flip must force a full rebuild"
        );
    }

    /// V2 structural guard regression. Buffer `["    code", "",
    /// "    more"]` has lazy_depth `[1, 1, 1]` (indented code
    /// multi-chunk per CommonMark §4.4). An edit at row 1 (the blank
    /// inside the block) must trigger fallback, even though the row
    /// is itself Blank and would otherwise be a safe-looking
    /// boundary candidate.
    #[test]
    fn try_incremental_parse_falls_back_when_damaged_row_is_inside_lazy_block() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "    code".to_string(),
            "".to_string(),
            "    more".to_string(),
        ];
        update_view(&mut v, &lines, (0, 0), rect(20), 1, None);
        assert_eq!(
            v.parse_state.buf().lazy_depth,
            vec![1, 1, 1],
            "precondition: parsed_buffer.lazy_depth must mark all three rows as inside the block"
        );
        let new_lines = vec![
            "    code".to_string(),
            "x".to_string(),
            "    more".to_string(),
        ];
        assert!(
            v.try_incremental_parse(&new_lines, (1, 1)).is_none(),
            "edit inside an open lazy-continuable block must force a full rebuild"
        );
    }

    #[test]
    fn try_incremental_parse_falls_back_on_html_block_flip() {
        // Regression: a Plain row flipping to an HTML-block opener
        // (`<div>`) starts a block that lazy-extends through subsequent
        // Plain rows in the full buffer.
        let mut v = MarkdownEditorView::new();
        let lines = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(20), 1, None);
        let new_lines = vec![
            "alpha".to_string(),
            "<div>".to_string(),
            "gamma".to_string(),
        ];
        assert!(
            v.try_incremental_parse(&new_lines, (1, 0)).is_none(),
            "HTML-block opener flip must force a full rebuild"
        );
    }

    #[test]
    fn is_in_code_block_returns_true_for_any_fence_regardless_of_cursor() {
        // Regression: after commit cceef444, every fenced block renders
        // force-raw — not just the one the cursor sits in. Verify by
        // probing `is_in_code_block` for a row in a fence while the
        // cursor is positioned elsewhere.
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "```".to_string(),
            "code".to_string(),
            "```".to_string(),
            "outro".to_string(),
        ];
        // Cursor on the prose line; fence interior must still report in-block.
        update_view(&mut v, &lines, (4, 0), rect(10), 1, None);
        assert!(v.is_in_code_block(2), "fence interior is in-block");
        assert!(!v.is_in_code_block(0), "prose line is not in-block");
        assert!(!v.is_in_code_block(4), "trailing prose is not in-block");
    }

    #[test]
    fn parsed_cache_populated_after_update() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello".to_string(), "**bold**".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(10), 1, None);
        assert_eq!(v.parse_state.buf().lines.len(), 2);
    }

    #[test]
    fn layout_skipped_on_horizontal_cursor_move_in_plain_text() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);
        let layout_gen_after_first = v.last_layout_generation;
        // Move cursor right — same row, no elements, same generation → layout must be skipped.
        update_view(&mut v, &lines, (0, 5), rect(40), 1, None);
        assert_eq!(
            v.last_layout_cursor,
            (0, 0),
            "layout cursor unchanged = layout was skipped"
        );
        assert_eq!(v.last_layout_generation, layout_gen_after_first);
    }

    #[test]
    fn layout_recomputed_on_row_change() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..3).map(|i| format!("line{}", i)).collect();
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);
        update_view(&mut v, &lines, (1, 0), rect(40), 1, None); // cursor moves to row 1
        assert_eq!(v.last_layout_cursor.0, 1, "layout recomputed on row change");
    }

    #[test]
    fn layout_recomputed_on_width_change() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world foo bar".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);
        update_view(
            &mut v,
            &lines,
            (0, 0),
            Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            1,
            None,
        );
        assert_eq!(v.last_layout_width, 10);
    }

    #[test]
    fn same_generation_skips_snapshot_rebuild() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["original".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(10), 1, None);
        // Update with different content but same generation — snapshot must NOT change.
        let lines2 = vec!["changed".to_string()];
        update_view(&mut v, &lines2, (0, 0), rect(10), 1, None);
        assert_eq!(v.lines_snapshot, vec!["original".to_string()]);
    }

    #[test]
    fn new_generation_triggers_snapshot_rebuild() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["original".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(10), 1, None);
        let lines2 = vec!["changed".to_string()];
        update_view(&mut v, &lines2, (0, 0), rect(10), 2, None);
        assert_eq!(v.lines_snapshot, vec!["changed".to_string()]);
    }

    #[test]
    fn update_stores_selection() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(40), 1, Some(((0, 0), (0, 5))));
        assert_eq!(v.selection, Some(((0, 0), (0, 5))));
    }

    #[test]
    fn update_clears_selection_when_none() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(40), 1, Some(((0, 0), (0, 5))));
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);
        assert_eq!(v.selection, None);
    }

    #[test]
    fn typing_single_char_in_long_buffer_uses_incremental_path() {
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = (0..1000).map(|i| format!("paragraph {i}")).collect();
        update_view(&mut v, &lines, (500, 0), rect(40), 1, None);
        // The 1000-line buffer takes the async-parse placeholder path on
        // first parse. Simulate the background task completing before the
        // edit so the next update splices against a real (non-placeholder)
        // buffer; Gate 1 deliberately refuses to incrementally splice the
        // all-`Plain` placeholder.
        v.install_full_parse(1, ParsedBuffer::parse(&lines));

        // Single-char insert at row 500.
        lines[500].push('x');
        let edited_len = lines[500].len();
        update_view(&mut v, &lines, (500, edited_len), rect(40), 2, None);

        // The spliced result must equal a fresh full parse.
        let fresh = ParsedBuffer::parse(&lines);
        assert_eq!(v.parse_state.buf().lines.len(), fresh.lines.len());
        assert_eq!(v.parse_state.buf().kinds, fresh.kinds);
        // Regression: the heuristic widener splices a slice whose
        // local sentinel boundaries (slice rows 0 and len) are NOT
        // genuine reset boundaries of the merged buffer. splice must
        // not promote them — a 1000-line single-paragraph buffer has
        // reset boundaries only at [0, 1000].
        assert_eq!(
            v.parse_state.buf().reset_boundaries,
            fresh.reset_boundaries,
            "heuristic splice must not introduce spurious reset boundaries"
        );
        // And the incremental path was actually taken.
        assert!(
            v.last_parse_was_incremental,
            "single-char paragraph edit should take incremental path"
        );
    }

    #[test]
    fn edit_while_placeholder_active_refuses_incremental_and_rearms() {
        // Regression: a large-buffer edit installs an unstyled placeholder
        // (all-`Plain` kinds) pending a background full parse. If the next
        // edit lands before the parse completes, Gate 1 must NOT splice the
        // placeholder — its all-`Plain` kinds defeat the structural guards
        // and would lock in a wrong parse that install_full_parse then drops
        // as stale. The edit must re-install a placeholder + re-arm pending.
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = (0..1000).map(|i| format!("paragraph {i}")).collect();
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);
        assert!(
            v.parse_state.is_placeholder(),
            "first parse installs placeholder"
        );
        assert_eq!(v.take_pending_full_parse(), Some(1));

        // Edit before the background parse resolves the placeholder.
        lines[0].push_str("```");
        update_view(&mut v, &lines, (0, lines[0].len()), rect(40), 2, None);
        assert!(
            !v.last_parse_was_incremental,
            "must not splice the placeholder"
        );
        assert!(
            v.parse_state.is_placeholder(),
            "still placeholder pending parse"
        );
        assert_eq!(
            v.take_pending_full_parse(),
            Some(2),
            "re-armed for new generation"
        );

        // Background parse for the latest generation completes.
        v.install_full_parse(2, ParsedBuffer::parse(&lines));
        assert!(
            !v.parse_state.is_placeholder(),
            "placeholder cleared on install"
        );
        assert_eq!(v.parse_state.buf().kinds, ParsedBuffer::parse(&lines).kinds);
    }

    #[test]
    #[should_panic(expected = "splice on placeholder parse")]
    fn splice_real_on_placeholder_is_rejected() {
        // The type makes the wrong-splice hazard unrepresentable on the
        // Gate 1 path; this guards the `ParseState::splice_real` contract
        // directly so a future caller can't route a splice into a
        // placeholder without tripping the assert.
        let mut state = ParseState::Placeholder {
            buf: ParsedBuffer::placeholder(&["x".to_string()]),
            generation: 1,
            spawned: false,
        };
        state.splice_real(0..1, ParsedBuffer::parse(&["y".to_string()]));
    }

    #[test]
    fn fence_toggle_triggers_full_rebuild_fallback() {
        let mut v = MarkdownEditorView::new();
        // Use 700 lines so that an unclosed fence at row 350 widens to
        // end-of-buffer (~351 rows), exceeding the absolute cap (256).
        // Below the perf #9 LARGE_BUFFER_THRESHOLD (1000), so the
        // fallback runs synchronously and `parsed_buffer.kinds`
        // matches a fresh full parse immediately.
        let mut lines: Vec<String> = (0..700).map(|i| format!("paragraph {i}")).collect();
        update_view(&mut v, &lines, (350, 0), rect(40), 1, None);

        // Open a fence mid-buffer — structurally invasive, line count changes.
        lines.insert(350, "```".to_string());
        update_view(&mut v, &lines, (350, 3), rect(40), 2, None);

        let fresh = ParsedBuffer::parse(&lines);
        assert_eq!(
            v.parse_state.buf().kinds,
            fresh.kinds,
            "spliced kinds must equal fresh full parse"
        );
        // The unclosed fence at row 350 widens to end-of-buffer (~351 lines,
        // > 256 cap_abs), so the cap trips and the fallback fires.
        assert!(
            !v.last_parse_was_incremental,
            "fence toggle (unclosed fence, 700-line buffer) should fall back to full rebuild"
        );
        // Buffer < LARGE_BUFFER_THRESHOLD → sync fallback, no
        // pending-async signal.
        assert!(
            v.take_pending_full_parse().is_none(),
            "small-buffer fallback must NOT defer to async"
        );
    }

    #[test]
    fn fence_toggle_on_large_buffer_defers_to_async_fallback() {
        // Regression for perf #9: above LARGE_BUFFER_THRESHOLD, the
        // fallback installs a placeholder ParsedBuffer + signals
        // pending instead of blocking the typing thread on
        // ParsedBuffer::parse. The owning component spawns the real
        // parse on tokio and calls install_full_parse when done.
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = (0..1500).map(|i| format!("paragraph {i}")).collect();
        update_view(&mut v, &lines, (750, 0), rect(40), 1, None);

        // Force a fallback path on a large buffer.
        lines.insert(750, "```".to_string());
        update_view(&mut v, &lines, (750, 3), rect(40), 2, None);

        assert!(
            !v.last_parse_was_incremental,
            "fence toggle on 1500-line buffer should fall back"
        );
        let pending = v.take_pending_full_parse();
        assert!(
            pending.is_some(),
            "large-buffer fallback must signal pending async parse"
        );
        // Placeholder kinds: every row is Plain — no fence detection yet.
        assert!(
            v.parse_state
                .buf()
                .kinds
                .iter()
                .all(|k| matches!(k, super::super::parse_incremental::LineConstructKind::Plain)),
            "placeholder must classify every row as Plain"
        );
        assert_eq!(
            v.parse_state.buf().lines.len(),
            lines.len(),
            "placeholder row count must match input"
        );

        // Caller (TextEditorComponent in production) spawns the real
        // parse and installs the result. Simulate that here.
        let real = ParsedBuffer::parse(&lines);
        let generation = pending.unwrap();
        v.install_full_parse(generation, real);
        let fresh = ParsedBuffer::parse(&lines);
        assert_eq!(
            v.parse_state.buf().kinds,
            fresh.kinds,
            "post-install kinds must match fresh full parse"
        );
    }

    fn full_rebuild_equals_view_state(v: &MarkdownEditorView, lines: &[String]) {
        let fresh = ParsedBuffer::parse(lines);
        assert_eq!(v.parse_state.buf().kinds, fresh.kinds, "kinds diverge");
        assert_eq!(
            v.parse_state.buf().lines.len(),
            fresh.lines.len(),
            "row count diverge"
        );
        for (i, (got, exp)) in v
            .parse_state
            .buf()
            .lines
            .iter()
            .zip(fresh.lines.iter())
            .enumerate()
        {
            got.debug_assert_eq_to(exp, i);
        }
    }

    #[test]
    fn incremental_falls_back_when_fence_marker_modified() {
        // Regression: editing a row that is currently a FenceMarker can
        // change the fence's extent across the rest of the buffer.
        // Incremental parsing's window-bounded widening cannot capture
        // this, so we must fall back to a full parse.
        let mut v = MarkdownEditorView::new();
        let mut lines = vec!["```".to_string(), "".to_string(), "```".to_string()];
        // Fill out the buffer with blank lines so the cap doesn't trip first.
        for _ in 0..31 {
            lines.push(String::new());
        }
        update_view(&mut v, &lines, (2, 0), rect(40), 1, None);

        // Edit the closing fence marker — append a char so it's no longer a closer.
        let mut new_lines = lines.clone();
        new_lines[2].push('0');
        update_view(&mut v, &new_lines, (2, 4), rect(40), 2, None);

        assert!(
            !v.last_parse_was_incremental,
            "fence-marker edit must trigger full-rebuild fallback"
        );
        // And the resulting state must equal a fresh parse (which the
        // fallback path does anyway, but assert defensively).
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_paste_large_block_falls_back() {
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = (0..50).map(|i| format!("line {i}")).collect();
        update_view(&mut v, &lines, (25, 0), rect(40), 1, None);

        // Insert 300 lines at row 25.
        let payload: Vec<String> = (0..300).map(|i| format!("pasted {i}")).collect();
        for (offset, p) in payload.into_iter().enumerate() {
            lines.insert(25 + offset, p);
        }
        update_view(&mut v, &lines, (25, 0), rect(40), 2, None);
        assert!(
            !v.last_parse_was_incremental,
            "300-line paste must fall back"
        );
        full_rebuild_equals_view_state(&v, &lines);
    }

    #[test]
    fn incremental_enter_at_line_end() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        update_view(&mut v, &lines, (0, 5), rect(40), 1, None);

        // Press Enter at end of "alpha".
        let new_lines = vec!["alpha".to_string(), "".to_string(), "beta".to_string()];
        update_view(&mut v, &new_lines, (1, 0), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_backspace_merging_lines() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        update_view(&mut v, &lines, (1, 0), rect(40), 1, None);

        // Backspace at start of "beta" merges into "alphabeta".
        let new_lines = vec!["alphabeta".to_string()];
        update_view(&mut v, &new_lines, (0, 5), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_inside_fence_widens_both_markers() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "```rust".to_string(),
            "let x = 1;".to_string(),
            "let y = 2;".to_string(),
            "```".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        update_view(&mut v, &lines, (3, 0), rect(40), 1, None);

        // Edit inside the fence (same-length, no line-count change).
        let mut new_lines = lines.clone();
        new_lines[3] = "let x = 999;".to_string();
        update_view(&mut v, &new_lines, (3, 8), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_list_continuation_widens_to_outer_marker() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "- top".to_string(),
            "  body of top".to_string(),
            "  - nested".to_string(),
            "    body of nested".to_string(),
            "    body two".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        update_view(&mut v, &lines, (4, 0), rect(40), 1, None);

        // Edit the nested continuation line.
        let mut new_lines = lines.clone();
        new_lines[4] = "    body two changed".to_string();
        update_view(&mut v, &new_lines, (4, 10), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_setext_underline_edit() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "heading text".to_string(),
            "====".to_string(),
            "".to_string(),
            "body".to_string(),
        ];
        update_view(&mut v, &lines, (1, 0), rect(40), 1, None);

        // Edit the underline (same line count).
        let mut new_lines = lines.clone();
        new_lines[1] = "======".to_string();
        update_view(&mut v, &new_lines, (1, 6), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_blockquote_paragraph_edit() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "> quoted line one".to_string(),
            "> quoted line two".to_string(),
            "> quoted line three".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        update_view(&mut v, &lines, (3, 0), rect(40), 1, None);

        let mut new_lines = lines.clone();
        new_lines[3] = "> quoted line TWO".to_string();
        update_view(&mut v, &new_lines, (3, 17), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_html_block_edit() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "before".to_string(),
            "".to_string(),
            "<div>".to_string(),
            "body".to_string(),
            "</div>".to_string(),
            "".to_string(),
            "after".to_string(),
        ];
        update_view(&mut v, &lines, (3, 0), rect(40), 1, None);

        let mut new_lines = lines.clone();
        new_lines[3] = "body changed".to_string();
        update_view(&mut v, &new_lines, (3, 12), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn g1_nested_list_three_indent_continuation() {
        // Deeply nested continuation: damaged range touches a 3-indent
        // continuation line. Widening must reach the outermost col-0
        // ListMarker — otherwise parse_range sees `      text` as
        // IndentedCode.
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "- level 0".to_string(),
            "  - level 1".to_string(),
            "    - level 2".to_string(),
            "      continuation at 6 indent".to_string(),
            "".to_string(),
            "after".to_string(),
        ];
        update_view(&mut v, &lines, (5, 0), rect(40), 1, None);

        let mut new_lines = lines.clone();
        new_lines[5] = "      continuation at 6 indent EDITED".to_string();
        update_view(&mut v, &new_lines, (5, 30), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn g3_hashtag_inside_fence_not_labeled_after_incremental_edit() {
        // `#tag` inside a fenced code block must NOT produce a Label element.
        // After an incremental edit fully inside the fence, the widened
        // slice includes both fence markers — the label-suppression scan
        // sees the fence and skips. This test verifies the round-trip.
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "```".to_string(),
            "let s = \"#tag\";".to_string(),
            "// another #tag".to_string(),
            "```".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        update_view(&mut v, &lines, (4, 0), rect(40), 1, None);

        use crate::components::text_editor::markdown::ElementKind;

        // Pre-condition: no Label elements in the fence interior.
        for row in 3..5 {
            let has_label = v.parse_state.buf().lines[row]
                .elements
                .iter()
                .any(|e| matches!(e.kind, ElementKind::Label));
            assert!(
                !has_label,
                "row {row} should have no Label inside the fence"
            );
        }

        // Edit one of the in-fence lines.
        let mut new_lines = lines.clone();
        new_lines[4] = "// edited #tag here".to_string();
        update_view(&mut v, &new_lines, (4, 19), rect(40), 2, None);

        // Post-condition: still no Label elements in the fence interior.
        for row in 3..5 {
            let has_label = v.parse_state.buf().lines[row]
                .elements
                .iter()
                .any(|e| matches!(e.kind, ElementKind::Label));
            assert!(
                !has_label,
                "row {row} should still have no Label after incremental edit"
            );
        }
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn g8a_typing_into_empty_buffer() {
        let mut v = MarkdownEditorView::new();
        let empty = vec!["".to_string()];
        update_view(&mut v, &empty, (0, 0), rect(40), 1, None);

        let one = vec!["h".to_string()];
        update_view(&mut v, &one, (0, 1), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &one);

        let two = vec!["he".to_string()];
        update_view(&mut v, &two, (0, 2), rect(40), 3, None);
        full_rebuild_equals_view_state(&v, &two);

        let many = vec!["hello world".to_string()];
        update_view(&mut v, &many, (0, 11), rect(40), 4, None);
        full_rebuild_equals_view_state(&v, &many);
    }

    #[test]
    fn g8b_delete_last_char_one_line_buffer() {
        let mut v = MarkdownEditorView::new();
        let one = vec!["h".to_string()];
        update_view(&mut v, &one, (0, 1), rect(40), 1, None);

        let empty = vec!["".to_string()];
        update_view(&mut v, &empty, (0, 0), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &empty);
    }

    #[test]
    fn incremental_text_change_produces_same_layout_as_full_recompute() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..200)
            .map(|i| format!("paragraph {i} with some text that may wrap depending on width"))
            .collect();
        update_view(&mut v, &lines, (100, 0), rect(40), 1, None);
        let baseline_visual_lines = v.layout.visual_lines().to_vec();

        // Edit a paragraph mid-buffer (no line count change).
        let mut edited = lines.clone();
        edited[100].push_str(" extra text");
        update_view(&mut v, &edited, (100, edited[100].len()), rect(40), 2, None);

        // After incremental wrap, layout must equal a fresh compute of the edited buffer.
        let fresh_layout = WordWrapLayout::compute(&edited, 40, v.rendered_cache_for_testing());

        let actual = v.layout.visual_lines();
        let fresh = fresh_layout.visual_lines();
        assert_eq!(actual.len(), fresh.len(), "visual_lines count diverges");
        for (i, (a, f)) in actual.iter().zip(fresh.iter()).enumerate() {
            assert_eq!(a, f, "visual line {i} diverges");
        }

        // Sanity: a row outside the edit should have unchanged visual lines.
        let row_50_before = baseline_visual_lines
            .iter()
            .filter(|vl| vl.logical_row == 50)
            .count();
        let row_50_after = v
            .layout
            .visual_lines()
            .iter()
            .filter(|vl| vl.logical_row == 50)
            .count();
        assert_eq!(
            row_50_before, row_50_after,
            "row 50 visual_lines count should be unchanged"
        );

        assert!(v.last_parse_was_incremental, "expected incremental path");
    }

    #[test]
    fn incremental_text_change_does_not_rebuild_all_of_rendered_cache() {
        // Verify that after an incremental text edit, rendered_cache rows
        // outside the widened range are NOT re-derived from scratch. We
        // can't directly observe the rebuild, but we CAN verify the cache
        // contents stay correct (matching a full rebuild's output).
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..200)
            .map(|i| format!("paragraph {i} with some text"))
            .collect();
        update_view(&mut v, &lines, (100, 0), rect(40), 1, None);

        // Snapshot rendered_cache before the edit.
        let before: Vec<Vec<bool>> = v
            .rendered_cache
            .iter()
            .enumerate()
            .filter(|(i, _)| *i < 50 || *i > 150)
            .map(|(_, v)| v.clone())
            .collect();

        // Edit a paragraph in the middle.
        let mut edited = lines.clone();
        edited[100].push('x');
        update_view(&mut v, &edited, (100, edited[100].len()), rect(40), 2, None);

        // Rows far outside the damaged range must be byte-identical.
        let after: Vec<Vec<bool>> = v
            .rendered_cache
            .iter()
            .enumerate()
            .filter(|(i, _)| *i < 50 || *i > 150)
            .map(|(_, v)| v.clone())
            .collect();
        assert_eq!(
            before, after,
            "rendered_cache rows outside damaged range must be unchanged"
        );

        // The incremental path must have been taken.
        assert!(v.last_parse_was_incremental);
    }

    // §3.4 — heuristic widener fires on an in-list content edit.
    //
    // Needs a buffer big enough that strict widener (which on a
    // loose list with no interior reset boundaries expands to
    // `[0, lines.len()]`) cap-trips, so the edit falls to
    // widen_to_safe over the loose-list blanks. With
    // MAX_INCREMENTAL_LINES=256 we use ~500 items.

    fn make_loose_list(n_items: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(n_items * 2);
        for i in 0..n_items {
            out.push(format!("- item {i}"));
            if i + 1 < n_items {
                out.push(String::new());
            }
        }
        out
    }

    #[test]
    fn try_incremental_parse_uses_heuristic_on_in_list_edit() {
        let mut v = MarkdownEditorView::new();
        let lines = make_loose_list(300);
        let mid_row = 200;
        update_view(&mut v, &lines, (mid_row, 0), rect(20), 1, None);

        let mut edited = lines.clone();
        edited[mid_row].push('x');
        update_view(
            &mut v,
            &edited,
            (mid_row, edited[mid_row].len()),
            rect(20),
            2,
            None,
        );

        assert!(
            v.last_parse_was_incremental,
            "edit inside large loose list must take incremental path \
             (lazy-guard relaxation + widen_to_safe over the loose-list blanks)"
        );
        assert_eq!(
            v.last_splice_path,
            Some(SplicePath::Heuristic),
            "expected Heuristic path on large loose list edit, got {:?}",
            v.last_splice_path
        );
    }

    // §3.5 — lazy-guard relaxation must NOT skip when the edit is a
    // list-marker flip. The marker-flip guard above the lazy guard
    // should bail first, and even if it didn't, the lazy guard's
    // kind_qualifies check should also bail since ListMarker is the
    // OLD kind but the new line is a different marker (still a list
    // marker, so the `looks_like_list_marker` flip check passes —
    // both old and new look like list markers; the lazy guard would
    // relax). However the kinds-comparison test ensures the edit
    // becomes a divergent classification only via the verify path.
    //
    // Actually re-reading: marker-style flip "- a" → "* a" does NOT
    // change `looks_like_list_marker` (both return true). The lazy
    // guard relaxation lets it through. The widener attempts splice.
    // If the slice's per-row kinds match the parent's, no divergence;
    // splice succeeds. If marker-style switches the classification,
    // verify catches it.
    //
    // The §3.5 spec scenario "- a" → "* a" produces ListMarker in
    // both. Slice parses "* a" alone as a list with `*` marker;
    // kinds[0] = ListMarker. Parent had ListMarker too. No
    // divergence. Splice succeeds via the heuristic widener.
    //
    // This test instead asserts the negative: a more-aggressive
    // structural change (e.g. removing the marker entirely, turning
    // a list row into a Plain row) must bail via the existing
    // looks_like_list_marker flip guard (KindGuard bail).
    #[test]
    fn try_incremental_parse_lazy_guard_still_bails_on_marker_removal() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = vec!["- a".into(), "".into(), "- b".into()];
        update_view(&mut v, &lines, (0, 3), rect(20), 1, None);

        let mut edited = lines.clone();
        edited[0] = "a".into(); // remove marker — `- a` → `a`
        update_view(&mut v, &edited, (0, 1), rect(20), 2, None);

        // The looks_like_list_marker flip guard above the lazy guard
        // must bail this case (KindGuard). The lazy-guard relaxation
        // never sees it.
        assert!(
            !v.last_parse_was_incremental,
            "list-marker removal must NOT take incremental path \
             — looks_like_list_marker flip guard bails first"
        );
    }
}
