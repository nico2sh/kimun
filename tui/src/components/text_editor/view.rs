use super::markdown::{MarkdownSpanner, ParsedBuffer, opener_shape};
use crate::settings::themes::Theme;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;
use ropetext::{Column, Layout, Metrics, RowHints, motion};

use super::rope_buffer::RopeBuffer;
use std::ops::Range;
use std::sync::OnceLock;

/// A styled range of logical columns on one row (see CONTEXT.md **Overlay**).
///
/// Every highlight the editor paints over a rendered line has this shape. They
/// arrive in logical coordinates so producers never reason about rendered
/// columns — markdown conceals sigils, so the two differ — and the mapping
/// happens once, here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlay {
    pub row: usize,
    /// Logical char column where the overlay starts.
    pub start: usize,
    /// Logical char column just past its end.
    pub end: usize,
    pub kind: OverlayKind,
}

impl Overlay {
    pub fn new(row: usize, start: usize, end: usize, kind: OverlayKind) -> Self {
        Self {
            row,
            start,
            end,
            kind,
        }
    }
}

/// What an [`Overlay`] means, and — by declaration order — how it stacks.
///
/// Later kinds paint over earlier ones. That order used to be implicit in
/// statement order across `view.rs`'s render loop and `mod.rs`'s cell post-pass,
/// which meant reasoning it out by hand for each new highlight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayKind {
    /// A task checkbox: `- [ ]` / `- [x]`.
    TaskBox,
    /// The text of a completed task, struck through.
    TaskDone,
    /// A vault-search **needle** carried in from the query that opened the note.
    Needle,
    /// A **find pattern** match.
    Match,
    /// The editor's selection.
    Selection,
    /// The **current match** — where the next find-bar action lands.
    CurrentMatch,
    /// Text the **replace preview** is showing in place of a match.
    Preview,
    /// The previewed **current match**.
    PreviewCurrent,
}

impl OverlayKind {
    /// How this kind restyles the spans it covers.
    ///
    /// The one place presentation for overlays lives: producers carry a kind,
    /// never a `Style`, so a find bar cannot hold an opinion about colour that
    /// has to be kept in sync with anything.
    fn restyle(self, theme: &Theme, style: ratatui::style::Style) -> ratatui::style::Style {
        use ratatui::style::Modifier;
        match self {
            OverlayKind::TaskBox => style.fg(theme.accent.to_ratatui()),
            OverlayKind::TaskDone => style.add_modifier(Modifier::DIM | Modifier::CROSSED_OUT),
            OverlayKind::Needle | OverlayKind::Match => style
                .fg(theme.color_search_match.to_ratatui())
                .add_modifier(Modifier::BOLD),
            OverlayKind::Selection | OverlayKind::CurrentMatch => {
                style.bg(theme.selection_bg.to_ratatui())
            }
            OverlayKind::Preview => style.bg(theme.color_replace_preview.to_ratatui()),
            // A foreground override, not a modifier: BOLD is a no-op on text
            // that is already bold, which once left the current match
            // indistinguishable from the rest (adr/0035).
            OverlayKind::PreviewCurrent => style
                .bg(theme.color_replace_preview.to_ratatui())
                .fg(cursor_fg(theme))
                .add_modifier(Modifier::BOLD),
        }
    }
}

/// The `cursor` role, substituting a chromatic colour when the theme defers to
/// the terminal — `Reset` foreground on `Reset` body text marks nothing.
fn cursor_fg(theme: &Theme) -> ratatui::style::Color {
    match theme.cursor {
        crate::settings::themes::ThemeColor::Reset => theme.fg_bright.to_ratatui(),
        _ => theme.cursor.to_ratatui(),
    }
}

/// Terminal cursor shape the editor requests while focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    Block,
}

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
    pub layout: Layout,
    visual_scroll_offset: usize,
    /// Viewport height from the last `update`, so overlay derivation can bound
    /// itself to the visible rows.
    last_height: usize,
    /// The text the caches below were built from.
    ///
    /// Held rather than borrowed because `render` runs without the frame's
    /// snapshot in scope. Keeping it costs nothing: a `Text` clone shares its
    /// structure, so this is the same text rather than a copy of it — which is why
    /// the twenty lines that used to copy changed rows into a `Vec<String>` are
    /// now one assignment.
    pub text_snapshot: ropetext::Text,
    pub cursor_snapshot: (usize, usize),
    /// Line ranges of every fenced code block in the buffer. Text-keyed
    /// (rebuilt only when `text_revision` changes); `is_in_code_block`
    /// does a cheap point lookup against this list per row so all fenced
    /// blocks render `force_raw` regardless of where the cursor is.
    fence_ranges: Vec<Range<usize>>,
    /// Per-logical-row code-box width (display cols), or `None` when the row
    /// is not in a code block. All rows of one block share the block's
    /// widest-rendered-line width, capped at the editor width. Rebuilt in
    /// `update()` whenever text or width changes.
    code_box_width: Vec<Option<u16>>,
    /// Per-logical-row left gutter width (display cols) for the blockquote
    /// bar: `depth + 1` on blockquote rows that are NOT the cursor row, else
    /// 0. Cursor-dependent (the cursor row reveals raw `> `), so rebuilt with
    /// the same cursor-affected-row logic as `rendered_cache`.
    gutter_insets: Vec<usize>,
    /// Cursor's last on-screen position (col, row), or `None` when the
    /// cursor was scrolled off-screen or the view was unfocused at the
    /// time of the previous `render`. Used as the anchor for floating
    /// overlays like the autocomplete popup, which is drawn after the
    /// editor itself.
    pub last_cursor_screen: Option<(u16, u16)>,
    /// Cursor style last written to the terminal, or `None` when the
    /// terminal is on the user's default shape. The terminal cursor style
    /// is global state, so on focus loss we must emit an explicit reset —
    /// otherwise the editor's bar/block shape leaks into every other text
    /// input (search sidebar, dialogs).
    applied_cursor_style: Option<CursorShape>,
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
    /// Every **overlay** to paint this frame, from outside. The view derives
    /// task and needle overlays itself (they come from the lines it already
    /// holds, and only visible rows are worth scanning).
    overlays: Vec<Overlay>,
    /// Vault-search **needles** to emphasise, lower-cased.
    needles: Vec<String>,
    /// Set when the next update follows an edit that touched rows the cursor
    /// does not identify — a **replace all**, not a keystroke. Consumed by the
    /// next `update`, which then skips `compute_damage_range`'s cursor fast
    /// path: that path assumes the cursor row is the only edited row, and a
    /// bulk edit violates it silently, leaving distant rows with a stale parse.
    bulk_edit_pending: bool,
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
    /// The cell a run of ↑/↓ is aiming at.
    ///
    /// Vim calls it `curswant`: without it, passing through a short drawn line
    /// clamps the column and the next press continues from there, so a column is
    /// lost permanently rather than borrowed. Cleared by any other cursor move —
    /// the component says when, because only it sees the other keys.
    visual_goal: Option<usize>,
    /// Rows the last edits changed, as the **edit buffer** reported them.
    ///
    /// Consumed by the next `update`, which then has no reason to compare the
    /// buffer against a copy of its previous self. `None` means nobody told us —
    /// the **nvim** backend hands over lines rather than changes, and whole-buffer
    /// replacements report nothing — and the diff is the fallback for exactly
    /// those.
    reported_damage: Option<std::ops::Range<usize>>,
    /// Set when Gate 2 installed a cheap `Layout::unwrapped` stub instead of
    /// blocking on `Layout::compute`, mirroring `ParseState::Placeholder`.
    /// While this is `Some`, every content-changing edit re-stubs and
    /// re-arms for the new generation rather than relaying just the edited
    /// rows — the same discipline Gate 1 applies via `is_placeholder()`, so
    /// a run of edits can never leave the untouched rows of a large buffer
    /// permanently unwrapped because one of them happened to parse
    /// incrementally. Cleared by `install_full_layout`.
    layout_pending: Option<PendingLayout>,
}

/// A `Layout::unwrapped` stub awaiting a background `Layout::compute`, the
/// layout-side twin of `ParseState::Placeholder`. `generation` is the
/// `content_revision` the stub was installed for; `spawned` flips true once
/// `take_pending_full_layout` has handed the job out, so it is claimed
/// exactly once per stub.
#[derive(Debug, Clone, Copy)]
struct PendingLayout {
    generation: u64,
    spawned: bool,
}

/// Everything a background task needs to compute the real `Layout` for a
/// stubbed generation, fully owned so it can move into `tokio::spawn`.
/// `RowHints` borrows, so it is rebuilt from `rendered_cache`/`gutter_insets`
/// *inside* the task rather than carried across the boundary itself.
pub struct PendingLayoutJob {
    pub generation: u64,
    pub text: ropetext::Text,
    pub width: usize,
    pub rendered_cache: Vec<Vec<bool>>,
    pub gutter_insets: Vec<usize>,
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
            layout: Layout::compute(&ropetext::Text::new(), 0, Metrics::default(), &[]),
            visual_scroll_offset: 0,
            last_height: 0,
            text_snapshot: ropetext::Text::new(),
            cursor_snapshot: (0, 0),
            fence_ranges: Vec::new(),
            code_box_width: Vec::new(),
            gutter_insets: Vec::new(),
            last_cursor_screen: None,
            applied_cursor_style: None,
            // Empty buffer, spliceable — preserves the previous
            // `placeholder_active: false` initial state.
            parse_state: ParseState::Real(ParsedBuffer::placeholder(&ropetext::Text::new())),
            last_seen_generation: u64::MAX, // force rebuild on first update
            last_layout_generation: u64::MAX,
            last_layout_width: 0,
            last_layout_cursor: (usize::MAX, usize::MAX),
            cursor_vrow: 0,
            rendered_cache: Vec::new(),
            overlays: Vec::new(),
            needles: Vec::new(),
            bulk_edit_pending: false,
            last_parse_was_incremental: false,
            last_splice_path: None,
            last_text_change: TextChangeKind::Full, // first update is a full rebuild
            visual_goal: None,
            reported_damage: None,
            layout_pending: None,
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
            && !*spawned
        {
            *spawned = true;
            return Some(*generation);
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

    /// Returns `Some(job)` if Gate 2 just installed a `Layout::unwrapped`
    /// stub and the owning component should spawn a background
    /// `Layout::compute` for it. Consumes the flag so the owner does not
    /// spawn twice; the owner is responsible for calling
    /// `install_full_layout` when the task completes.
    pub fn take_pending_full_layout(&mut self) -> Option<PendingLayoutJob> {
        let pending = self.layout_pending.as_mut()?;
        if pending.spawned {
            return None;
        }
        pending.spawned = true;
        Some(PendingLayoutJob {
            generation: pending.generation,
            text: self.text_snapshot.clone(),
            width: self.last_layout_width as usize,
            rendered_cache: self.rendered_cache.clone(),
            gutter_insets: self.gutter_insets.clone(),
        })
    }

    /// Install the result of a background `Layout::compute`. No-op when the
    /// editor has advanced past `generation` (a fresh spawn is already in
    /// flight — mirrors `install_full_parse`'s staleness gate) or when the
    /// pane was resized since the job was captured (`layout.width()` no
    /// longer matches `last_layout_width` — a generation match alone
    /// cannot catch this, since a resize with no content change never
    /// bumps `content_revision`).
    pub fn install_full_layout(&mut self, generation: u64, layout: Layout) {
        if generation != self.last_seen_generation
            || layout.width() != self.last_layout_width as usize
        {
            return; // stale
        }
        self.layout = layout;
        self.layout_pending = None;
        self.last_layout_generation = generation;
    }

    /// Full (non-incremental) layout rebuild: synchronous on a small
    /// buffer, deferred to a background task on a large one — the
    /// layout-side twin of Gate 1's placeholder-parse fallback. Called
    /// from every Gate 2 branch that would otherwise call
    /// `Layout::compute` unconditionally.
    fn full_layout_rebuild(
        &mut self,
        text: &ropetext::Text,
        width: u16,
        row_count: usize,
        generation: u64,
    ) {
        if row_count >= Self::LARGE_BUFFER_THRESHOLD {
            self.layout = Layout::unwrapped(text);
            self.layout_pending = Some(PendingLayout {
                generation,
                spawned: false,
            });
        } else {
            let hints = row_hints(&self.rendered_cache, &self.gutter_insets);
            self.layout = Layout::compute(text, width as usize, Metrics::default(), &hints);
            self.layout_pending = None;
        }
    }

    /// Hand the view this frame's **overlays**, in logical coordinates. Must be
    /// called *after* `update`, which clears them.
    ///
    /// The view appends the two kinds it derives itself — task decorations and
    /// **needle** emphasis. Those come from the lines it already holds, and it
    /// is the only thing that knows which rows are visible, so scanning them
    /// anywhere else would mean shipping the viewport outward.
    pub fn set_overlays(&mut self, overlays: Vec<Overlay>) {
        self.overlays = overlays;
        self.derive_content_overlays();
    }

    /// Derive task and needle overlays for the visible rows only.
    ///
    /// This replaces a post-pass over drawn terminal cells, which reconstructed
    /// row text with a byte→column map purely because it ran after render. That
    /// put it in a different coordinate space from everything else, and cost a
    /// defect: a find pattern targeting concealed markdown counted and stepped
    /// to matches it could never paint.
    fn derive_content_overlays(&mut self) {
        let scroll = self.visual_scroll_offset;
        let height = self.last_height;
        let rows: Vec<usize> = self
            .layout
            .visual_lines()
            .iter()
            .skip(scroll)
            .take(height)
            .map(|vl| vl.logical_row)
            .collect();
        let mut seen = usize::MAX;
        for row in rows {
            if row == seen {
                continue; // wrapped continuation of a row already handled
            }
            seen = row;
            let Some(line) = self.text_snapshot.line(row) else {
                continue;
            };
            // Task checkboxes: optional indent, then `- [ ] ` / `- [x] `.
            let indent = line.len() - line.trim_start().len();
            let after = &line[indent..];
            let done = after.starts_with("- [x] ") || after.starts_with("- [X] ");
            if done || after.starts_with("- [ ] ") {
                let box_start = line[..indent].chars().count() + 2;
                self.overlays.push(Overlay::new(
                    row,
                    box_start,
                    box_start + 3,
                    OverlayKind::TaskBox,
                ));
                if done {
                    self.overlays.push(Overlay::new(
                        row,
                        box_start + 3,
                        line.chars().count(),
                        OverlayKind::TaskDone,
                    ));
                }
            }
            // Needle emphasis, over the logical line rather than drawn cells.
            let line = line.as_ref();
            for (s, e) in crate::components::preview_highlight::match_ranges(line, &self.needles) {
                let start = line[..s].chars().count();
                let end = start + line[s..e].chars().count();
                self.overlays
                    .push(Overlay::new(row, start, end, OverlayKind::Needle));
            }
        }
    }

    /// Vault-search **needles** to emphasise. Sticky across frames, unlike
    /// overlays: they come from the query that opened the note.
    pub fn set_needles(&mut self, needles: Vec<String>) {
        self.needles = needles;
    }

    /// Declare that the edit just performed was a *bulk* one — it changed rows
    /// the cursor does not point at.
    ///
    /// `compute_damage_range`'s fast path trusts the cursor row to be the only
    /// edited row and will otherwise under-report the damage, leaving distant
    /// rows rendered from a stale parse. Every edit that rewrites more than the
    /// cursor's neighbourhood must call this.
    /// Forget where a run of ↑/↓ was aiming. Any other cursor movement ends it.
    pub fn clear_visual_goal(&mut self) {
        self.visual_goal = None;
    }

    /// Move the cursor one *drawn* line, which is what an arrow key means in a
    /// wrapped editor: one press moves one line the reader can see, not past the
    /// whole remainder of a soft-wrapped paragraph.
    ///
    /// Lives on the view because only the view has the layout — the buffer holds
    /// the text and the cursor, and neither alone can answer "which line is this
    /// drawn on". That split is exactly why the incumbent could not do this.
    ///
    /// Returns `false` when the layout does not describe the buffer's current
    /// text — an edit lands before the frame that re-lays it out — and the caller
    /// falls back to a logical move rather than reading a stale layout.
    pub fn move_cursor_visually(&mut self, buf: &mut RopeBuffer, down: bool, extend: bool) -> bool {
        let text = buf.text().clone();
        // Exactly, not approximately: comparing row counts missed every edit that
        // stayed inside one row, and the stale byte ranges then sliced out of
        // bounds — `end byte index 4 is out of bounds for string of length 1`.
        if !self.layout.describes(&text) {
            return false;
        }
        let Some(cursor) = text.position(buf.cursor().0, Column::new(buf.cursor().1)) else {
            return false;
        };
        let hints = row_hints(&self.rendered_cache, &self.gutter_insets);
        let goal = self
            .visual_goal
            .unwrap_or_else(|| self.layout.cell_of(&text, &hints, cursor).column);
        let landed = motion::visual_vertical(
            &text,
            &self.layout,
            &hints,
            cursor,
            if down { 1 } else { -1 },
            motion::VisualGoal::Cell(goal),
        );
        self.visual_goal = Some(goal);

        if extend {
            if buf.selection_range().is_none() {
                buf.start_selection();
            }
        } else {
            buf.cancel_selection();
        }
        buf.move_to(landed);
        true
    }

    /// Record which rows an edit changed, for the next `update` to act on.
    pub fn note_damage(&mut self, rows: std::ops::Range<usize>) {
        self.reported_damage = Some(match self.reported_damage.take() {
            Some(seen) => seen.start.min(rows.start)..seen.end.max(rows.end),
            None => rows,
        });
    }

    pub fn note_bulk_edit(&mut self) {
        self.bulk_edit_pending = true;
    }

    pub fn update(&mut self, snap: &super::snapshot::EditorSnapshot, rect: Rect) {
        self.last_height = rect.height as usize;
        // Snapshot owns the (cursor, lines, content_revision) atomicity
        // — readers below can index `parsed_buffer.lines[cursor.0]`
        // without `.get()` guards once Gate 1 has rebuilt the parse
        // cache from these same `lines`.
        let text = &snap.text;
        let row_count = text.line_count();
        let cursor = snap.cursor;
        let generation = snap.content_revision.get();
        // Overlays belong to the snapshot they were built from. Clearing here
        // means a caller that stops previewing (or closes the find bar) cannot
        // leave stale ones painted over real text — it simply stops setting
        // them.
        self.overlays.clear();
        if rect.height == 0 {
            return;
        }

        // Gate 1: content changed — rebuild parse cache and snapshots.
        if generation != self.last_seen_generation {
            let reported = self.reported_damage.take();
            let incremental = if self.parse_state.is_placeholder() {
                None
            } else {
                self.try_incremental_parse(text, cursor, reported)
            };
            // Consumed here, not inside `try_incremental_parse`: the flag must
            // clear even on the placeholder path above, or a bulk edit
            // followed by a keystroke would still be suppressing the hint.
            self.bulk_edit_pending = false;
            self.last_text_change = match incremental {
                Some((range, slice, path)) => {
                    self.parse_state.splice_real(range.clone(), slice);
                    self.last_parse_was_incremental = true;
                    self.last_splice_path = Some(path);
                    TextChangeKind::Incremental(range)
                }
                None => {
                    if row_count >= Self::LARGE_BUFFER_THRESHOLD {
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
                            buf: ParsedBuffer::placeholder(&snap.text),
                            generation,
                            spawned: false,
                        };
                    } else {
                        self.parse_state = ParseState::Real(ParsedBuffer::parse(&snap.text));
                    }
                    self.last_parse_was_incremental = false;
                    self.last_splice_path = None;
                    TextChangeKind::Full
                }
            };
            #[cfg(debug_assertions)]
            if self.last_parse_was_incremental && verify_incremental_enabled() {
                let fresh = ParsedBuffer::parse(&snap.text);
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
            // Skip on a successful incremental splice: `try_incremental_parse`
            // already refuses to splice any edit that could flip a row into
            // or out of a fence/indented-code/HTML-block role (the
            // structural-marker and opener-shape guards bail to a full
            // rebuild first) — so `fence_ranges` is provably identical to
            // before, and re-scanning the whole `kinds` array to confirm
            // that would defeat the point of having taken the fast path.
            if !self.last_parse_was_incremental {
                self.fence_ranges = super::parse_incremental::fence_ranges_from_kinds(
                    &self.parse_state.buf().kinds,
                );
            }
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
                TextChangeKind::Incremental(_) | TextChangeKind::Full | TextChangeKind::None => {
                    self.text_snapshot = snap.text.clone();
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
                .filter(|&r| r < row_count)
                .collect();
            // Determine the set of rows to rebuild in rendered_cache.
            let rebuild_strategy = if self.rendered_cache.len() != row_count {
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
                    self.rendered_cache = text
                        .lines()
                        .enumerate()
                        .map(|(i, l)| {
                            let force_raw = self.is_in_code_block(i);
                            let cursor_col = if i == cursor.0 { Some(cursor.1) } else { None };
                            MarkdownSpanner::visible_positions_with(
                                &l,
                                &self.parse_state.buf().lines[i],
                                cursor_col,
                                force_raw,
                            )
                        })
                        .collect();
                }
                RenderedCacheRebuild::Rows(rows) => {
                    for row in rows {
                        if row >= row_count {
                            continue; // defensive
                        }
                        let force_raw = self.is_in_code_block(row);
                        let cursor_col = if row == cursor.0 {
                            Some(cursor.1)
                        } else {
                            None
                        };
                        let new_entry = MarkdownSpanner::visible_positions_with(
                            &text.line(row).unwrap_or_default(),
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
            // Full rebuild only on a genuine full-rebuild frame (or a
            // length mismatch, defensively) — otherwise the structural
            // guards that gate the incremental splice already guarantee no
            // row's blockquote depth changed, so only the rows the cursor
            // just left or entered need a fresh inset.
            if matches!(self.last_text_change, TextChangeKind::Full)
                || self.gutter_insets.len() != row_count
            {
                self.rebuild_gutter_insets(row_count, cursor.0);
            } else if !cursor_affected_rows.is_empty() {
                self.patch_gutter_insets(&cursor_affected_rows, cursor.0);
            }
            let line_count_changed = self.layout.row_count() != row_count;
            // A stub is still outstanding from an earlier full rebuild and
            // content changed again this frame: re-stub for the new
            // generation rather than let an incremental relayout patch a
            // couple of rows while the rest of the buffer stays permanently
            // unwrapped waiting on a background result that will land too
            // late (stale-generation) to matter. Mirrors Gate 1's
            // `is_placeholder()` gate — a cursor-only frame (`None`) leaves
            // an in-flight job alone rather than aborting it for nothing.
            let stub_still_pending = self.layout_pending.is_some()
                && !matches!(self.last_text_change, TextChangeKind::None);
            if width_changed || line_count_changed || stub_still_pending {
                self.full_layout_rebuild(&snap.text, rect.width, row_count, generation);
            } else {
                match &self.last_text_change {
                    TextChangeKind::Full => {
                        self.full_layout_rebuild(&snap.text, rect.width, row_count, generation);
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
                        let hints = row_hints(&self.rendered_cache, &self.gutter_insets);
                        // Line count is unchanged on this path — the caller above
                        // takes the full-recompute branch when it is not — so the
                        // relayout shifts nothing.
                        self.layout.relayout_rows(&snap.text, &hints, start..end, 0);
                    }
                    TextChangeKind::None => {
                        if let (Some(&first), Some(&last)) =
                            (cursor_affected_rows.first(), cursor_affected_rows.last())
                        {
                            let hints = row_hints(&self.rendered_cache, &self.gutter_insets);
                            self.layout
                                .relayout_rows(&snap.text, &hints, first..last + 1, 0);
                        }
                    }
                }
            }
            // Code-box widths depend only on text content and the wrap width,
            // not the cursor — so skip the (grapheme-walking) rebuild on
            // cursor-only moves, where neither changed. A width change caps
            // every block afresh regardless of content, so it always forces
            // the full rebuild; a successful incremental splice narrows to
            // just the block(s) overlapping the edited range — the same
            // structural guards mean any OTHER block's boundaries (and thus
            // whether it needs re-measuring at all) can't have moved.
            if width_changed
                || matches!(self.last_text_change, TextChangeKind::Full)
                || self.code_box_width.len() != row_count
            {
                self.rebuild_code_box_width(text, rect.width);
            } else if let TextChangeKind::Incremental(range) = &self.last_text_change {
                self.patch_code_box_width(text, rect.width, range.clone());
            }
            self.last_layout_generation = generation;
            self.last_layout_width = rect.width;
            self.last_layout_cursor = cursor;
        }

        // Cache cursor_vrow for render() — avoids a second lookup there.
        self.cursor_vrow = snap
            .text
            .position(cursor.0, Column::new(cursor.1))
            .map(|at| self.layout.visual_row_of(at))
            .unwrap_or(0);
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
        text: &ropetext::Text,
        cursor: (usize, usize),
        reported: Option<std::ops::Range<usize>>,
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
        if text.line_count() != self.parse_state.buf().lines.len() {
            return METRICS.bail(BailReason::LineCountChange);
        }
        // The row-by-row guards below read the previous content, so it has to
        // describe the same buffer shape. It does not on the first update, and an
        // empty text still has one row — so "no previous state" cannot be inferred
        // from the parse cache being empty.
        if self.text_snapshot.line_count() != text.line_count() {
            return METRICS.bail(BailReason::LineCountChange);
        }
        // A bulk edit invalidates the cursor hint: pass `usize::MAX` so the
        // fast path's `cursor_row < old.len()` test fails and the LCP/LCS slow
        // path computes the real span. The flag is cleared by `update` whether
        // or not the incremental attempt gets this far.
        let hint = if self.bulk_edit_pending {
            usize::MAX
        } else {
            cursor.0
        };
        // Told, not found: the engine knows which rows its own edit touched, so the
        // only reason to compare the buffer with a copy of its previous self is
        // that nobody told us — a whole-buffer replacement, or the **nvim**
        // backend, which reports lines rather than changes.
        let damaged = match reported {
            Some(rows) if rows.end <= text.line_count() => rows,
            _ => {
                // Only this path needs the previous content as rows, and only
                // because it has to compare. Materialising it here keeps that cost
                // where the comparison is, rather than on every edit.
                let previous: Vec<String> =
                    self.text_snapshot.lines().map(|l| l.to_string()).collect();
                let current: Vec<String> = text.lines().map(|l| l.to_string()).collect();
                let Some(damaged) = compute_damage_range(&previous, &current, hint) else {
                    return METRICS.bail(BailReason::NoDamage);
                };
                damaged
            }
        };
        if damaged.is_empty() {
            return METRICS.bail(BailReason::NoDamage);
        }

        // Structural-marker change guard: any edit that converts a fence
        // marker line into a non-marker (or vice versa) can shift the
        // fence's extent beyond the widening window. Same for setext
        // underlines. Conservative fallback to full parse for correctness.
        for row in damaged.clone() {
            let old_kind = self.parse_state.buf().kinds[row];
            let previous_row = self.text_snapshot.line(row).unwrap_or_default();
            let old_line = previous_row.as_ref();
            let current_row = text.line(row).unwrap_or_default();
            let new_line = current_row.as_ref();

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
        let slice = ParsedBuffer::parse_range(text, widened.clone());

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

    pub fn render(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        theme: &Theme,
        focused: bool,
        cursor_shape: Option<CursorShape>,
    ) {
        if rect.height == 0 {
            return;
        }
        let text = &self.text_snapshot;
        let cursor = self.cursor_snapshot;
        let scroll = self.visual_scroll_offset;
        let height = rect.height as usize;
        let vlines = self.layout.visual_lines();

        let parsed_lines = &self.parse_state.buf().lines;
        let fence_ranges = &self.fence_ranges;

        // The rows the visible lines draw from, materialised for this frame.
        // Bounded by the pane's height rather than the note's length, and needed
        // because the spans below borrow their row and outlive the closure that
        // builds them.
        let window: Vec<String> = vlines
            .iter()
            .skip(scroll)
            .take(height)
            .map(|vl| text.line(vl.logical_row).unwrap_or_default().into_owned())
            .collect();

        let visible: Vec<Line> = vlines
            .iter()
            .skip(scroll)
            .take(height)
            .zip(window.iter())
            .map(|(vl, row_text)| {
                let cursor_col = if vl.logical_row == cursor.0 {
                    Some(cursor.1)
                } else {
                    None
                };
                let force_raw = fence_ranges.iter().any(|r| r.contains(&vl.logical_row));
                // Snapshot invariant: every `vl.logical_row` is < lines.len()
                // because `layout` and `lines_snapshot` were rebuilt from
                // the same `EditorSnapshot` in the last `update()`.
                let logical_line = row_text.as_str();
                let parsed = &parsed_lines[vl.logical_row];
                let content = &logical_line[vl.bytes.clone()];
                let spans = MarkdownSpanner::render_with(
                    content,
                    logical_line,
                    parsed,
                    vl.chars.start,
                    cursor_col,
                    vl.first,
                    force_raw,
                    rect.width,
                    theme,
                );

                // Apply code-block background before selection so selection bg wins on selected text.
                let spans =
                    if let Some(bw) = self.code_box_width.get(vl.logical_row).copied().flatten() {
                        apply_code_box(spans, bw, theme)
                    } else {
                        spans
                    };

                // Every highlight this row carries, painted in one pass.
                // `OverlayKind`'s declaration order is the stacking order, so
                // "preview wins over selection" is a property of the enum
                // rather than of where the code happens to sit.
                let spans = {
                    let gutter_off = if vl.first {
                        0
                    } else {
                        self.gutter_insets.get(vl.logical_row).copied().unwrap_or(0)
                    };
                    let to_rendered = |col: usize| {
                        MarkdownSpanner::rendered_col_with_reveal(
                            logical_line,
                            parsed,
                            vl.chars.start,
                            col,
                            cursor_col,
                            vl.first,
                            force_raw,
                        ) + gutter_off
                    };
                    let mut row_overlays: Vec<&Overlay> = self
                        .overlays
                        .iter()
                        .filter(|o| o.row == vl.logical_row)
                        .collect();
                    row_overlays.sort_by_key(|o| o.kind);

                    let mut spans = spans;
                    for o in row_overlays {
                        let start = to_rendered(o.start);
                        let mut end = to_rendered(o.end);
                        // A zero-width overlay would paint nothing — which is
                        // exactly the case where the user most needs to see
                        // where they are (an empty replacement previews a match
                        // as nothing at all). Give it one cell, like a caret.
                        if end == start && o.kind == OverlayKind::PreviewCurrent {
                            end = start + 1;
                        }
                        spans =
                            restyle_over_range(spans, start..end, &|st| o.kind.restyle(theme, st));
                    }
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
        let mut desired_style: Option<CursorShape> = None;
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
                let row_text = text.line(cursor.0).unwrap_or_default();
                let logical_line = row_text.as_ref();
                let force_raw = self.is_in_code_block(cursor.0);
                let rendered_col = MarkdownSpanner::rendered_cursor_col_with(
                    logical_line,
                    parsed,
                    vl.chars.start,
                    cursor.1,
                    vl.first,
                    force_raw,
                );
                let cx = rect.x + rendered_col as u16;
                let cy = rect.y + (cursor_vrow - scroll) as u16;
                f.set_cursor_position(Position { x: cx, y: cy });
                self.last_cursor_screen = Some((cx, cy));
                desired_style = cursor_shape;
            }
        }
        if desired_style != self.applied_cursor_style {
            use ratatui::crossterm::cursor::SetCursorStyle;
            let style = match desired_style {
                Some(CursorShape::Block) => SetCursorStyle::SteadyBlock,
                Some(CursorShape::Bar) => SetCursorStyle::SteadyBar,
                None => SetCursorStyle::DefaultUserShape,
            };
            let _ = ratatui::crossterm::execute!(std::io::stdout(), style);
            self.applied_cursor_style = desired_style;
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

    #[cfg(test)]
    pub(crate) fn code_box_width_for_testing(&self) -> &[Option<u16>] {
        &self.code_box_width
    }

    #[cfg(test)]
    pub(crate) fn gutter_insets_for_testing(&self) -> &[usize] {
        &self.gutter_insets
    }

    fn is_in_code_block(&self, row: usize) -> bool {
        // Every line inside any fenced block renders force-raw (no markdown
        // re-styling, distinct fg color). Previously this checked only the
        // fence the cursor was sitting in, so fenced blocks elsewhere in
        // the buffer looked like plain text until the cursor moved into
        // them.
        self.fence_ranges.iter().any(|r| r.contains(&row))
    }

    /// Rebuild `code_box_width` from the current parse kinds and snapshot
    /// lines. Box width per block = max rendered display width of its lines,
    /// capped at `width`.
    fn rebuild_code_box_width(&mut self, text: &ropetext::Text, width: u16) {
        let mut out = vec![None; text.line_count()];
        let ranges =
            super::parse_incremental::code_block_ranges_from_kinds(&self.parse_state.buf().kinds);
        for r in ranges {
            let mut max_w = 0usize;
            for row in r.clone() {
                if let Some(line) = text.line(row) {
                    max_w = max_w.max(super::markdown::raw_display_width(&line));
                }
            }
            let boxed = (max_w.min(width as usize)) as u16;
            for row in r {
                if row < out.len() {
                    out[row] = Some(boxed);
                }
            }
        }
        self.code_box_width = out;
    }

    /// Update `code_box_width` for just the code-block range(s) overlapping
    /// `damaged` — the incremental-path sibling of `rebuild_code_box_width`.
    /// Safe because the structural guards in `try_incremental_parse` already
    /// refuse to splice an edit that adds, removes, or moves a code-block
    /// boundary; a block that doesn't overlap the edit can only have kept
    /// the same lines it had before, so its width can't have changed. A
    /// block's own content growing or shrinking *can* change its width, and
    /// that only happens inside `damaged`.
    fn patch_code_box_width(
        &mut self,
        text: &ropetext::Text,
        width: u16,
        damaged: std::ops::Range<usize>,
    ) {
        let ranges =
            super::parse_incremental::code_block_ranges_from_kinds(&self.parse_state.buf().kinds);
        for r in ranges {
            if r.start >= damaged.end || r.end <= damaged.start {
                continue; // no overlap — this block's width can't have changed
            }
            let mut max_w = 0usize;
            for row in r.clone() {
                if let Some(line) = text.line(row) {
                    max_w = max_w.max(super::markdown::raw_display_width(&line));
                }
            }
            let boxed = (max_w.min(width as usize)) as u16;
            for row in r {
                if let Some(entry) = self.code_box_width.get_mut(row) {
                    *entry = Some(boxed);
                }
            }
        }
    }

    /// Rebuild `gutter_insets` from parse state + cursor. A blockquote row
    /// that is not the cursor row reserves `depth + 1` cols for the bar; the
    /// cursor row reserves 0 (its markers are revealed raw). Full
    /// `O(row_count)` rebuild — see `patch_gutter_insets` for the
    /// incremental-path sibling that only touches the rows that can
    /// plausibly have changed.
    fn rebuild_gutter_insets(&mut self, row_count: usize, cursor_row: usize) {
        let parsed = &self.parse_state.buf().lines;
        self.gutter_insets = (0..row_count)
            .map(|row| {
                if row == cursor_row {
                    return 0;
                }
                match parsed.get(row).and_then(|p| p.blockquote_depth()) {
                    Some(d) => super::markdown::blockquote_gutter_width(d),
                    None => 0,
                }
            })
            .collect();
    }

    /// Update `gutter_insets` for exactly `rows`, in place. Safe whenever
    /// the parse took the incremental splice path: the structural guards in
    /// `try_incremental_parse` (opener-shape / lazy-depth) already refuse
    /// to splice an edit that could change a row's blockquote depth, so the
    /// only thing that can legitimately change `gutter_insets` between two
    /// incrementally-linked frames is which row the cursor is on.
    fn patch_gutter_insets(&mut self, rows: &[usize], cursor_row: usize) {
        let parsed = &self.parse_state.buf().lines;
        for &row in rows {
            let inset = if row == cursor_row {
                0
            } else {
                match parsed.get(row).and_then(|p| p.blockquote_depth()) {
                    Some(d) => super::markdown::blockquote_gutter_width(d),
                    None => 0,
                }
            };
            if let Some(entry) = self.gutter_insets.get_mut(row) {
                *entry = inset;
            }
        }
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
        let row_text = self.text_snapshot.line(vl.logical_row).unwrap_or_default();
        let logical_line = row_text.as_ref();
        let parsed = &self.parse_state.buf().lines[vl.logical_row];
        let force_raw = self.is_in_code_block(vl.logical_row);
        let gutter = self.gutter_insets.get(vl.logical_row).copied().unwrap_or(0);
        let vcol = vcol.saturating_sub(gutter);
        // When a blockquote gutter is drawn (gutter > 0), the ">" and space
        // sigil chars are hidden and replaced by the "│ " bar. On the first
        // visual line, skip those hidden sigil chars so that rendered_col 0
        // maps to the first content char, not to the hidden ">".
        let effective_start_col = if gutter > 0 && vl.first {
            parsed.blockquote_sigil_end().unwrap_or(vl.chars.start)
        } else {
            vl.chars.start
        };
        let logical_col = MarkdownSpanner::rendered_col_to_logical_with(
            logical_line,
            parsed,
            effective_start_col,
            vcol,
            vl.first,
            force_raw,
        );
        let col = logical_col.min(u16::MAX as usize) as u16;
        (row_u16, col)
    }

    #[cfg(test)]
    pub(crate) fn click_to_logical_for_testing(&self, vrow: usize, vcol: usize) -> (u16, u16) {
        self.click_to_logical_u16(vrow, vcol)
    }
}

impl Default for MarkdownEditorView {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the byte offset into `s` after consuming exactly `target_width` display columns.
/// If `target_width` exceeds the string's display width, returns `s.len()`.
///
/// Walks whole grapheme clusters (not codepoints) and measures each with
/// [`cluster_display_width`], so the result never lands mid-cluster (which would
/// split an emoji across two styled spans) and stays consistent with the width
/// model used by wrap and cursor math — an emoji presentation sequence (flag,
/// VS16 heart, keycap) counts as its full rendered width, not its first codepoint.
fn byte_offset_for_display_width(s: &str, target_width: usize) -> usize {
    use super::markdown::cluster_display_width;
    use unicode_segmentation::UnicodeSegmentation;
    let mut consumed = 0usize;
    for (byte_pos, g) in s.grapheme_indices(true) {
        if consumed >= target_width {
            return byte_pos;
        }
        consumed += cluster_display_width(g);
    }
    s.len()
}

/// Split `spans` at the boundaries of a rendered-column range and apply
/// `restyle` to the overlapping portion. The one place column-to-byte
/// accounting for a partial restyle lives.
fn restyle_over_range<'a>(
    spans: Vec<ratatui::text::Span<'a>>,
    sel_cols: std::ops::Range<usize>,
    restyle: &dyn Fn(ratatui::style::Style) -> ratatui::style::Style,
) -> Vec<ratatui::text::Span<'a>> {
    if sel_cols.is_empty() {
        return spans;
    }
    let mut result = Vec::new();
    let mut col = 0usize;

    for span in spans {
        let content: &str = &span.content;
        // Same cluster-based width model as `byte_offset_for_display_width`
        // below, so column accounting and the byte boundaries it computes can
        // never disagree on emoji presentation sequences.
        let span_width = super::markdown::string_display_width(content);
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
                restyle(span.style),
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

/// Paint `code_bg` behind every span of a code-block visual line and pad the
/// line with bg-colored spaces up to `box_width` display columns, producing a
/// solid rectangle hugging the block's widest line. Content already wider than
/// the box (the box was capped at editor width; wider rows wrap) is left as-is.
fn apply_code_box<'a>(
    spans: Vec<ratatui::text::Span<'a>>,
    box_width: u16,
    theme: &Theme,
) -> Vec<ratatui::text::Span<'a>> {
    use ratatui::text::Span;
    use unicode_segmentation::UnicodeSegmentation;
    let bg = theme.code_bg.to_ratatui();
    // Measure with the same cluster + tab-aware model as `raw_display_width`
    // (which sizes `box_width` in `rebuild_code_box_width`), so the padding
    // can never disagree with the target on emoji presentation sequences or
    // tabs. `cluster_width_at` needs the running column for tab stops.
    let mut width = 0usize;
    let mut out: Vec<Span<'a>> = spans
        .into_iter()
        .map(|s| {
            for g in s.content.graphemes(true) {
                width += super::markdown::cluster_width_at(g, width);
            }
            let style = s.style.bg(bg);
            Span::styled(s.content, style)
        })
        .collect();
    let target = box_width as usize;
    if width < target {
        out.push(Span::styled(
            " ".repeat(target - width),
            Style::default().bg(bg),
        ));
    }
    out
}

/// Per-row hints for the layout: what the syntax layer draws, and how far each
/// row is inset by its gutter.
///
/// Built per rebuild rather than stored, because both halves already live on the
/// view and a third copy would be a third thing to keep in step.
///
/// `pub(super)`: the background wrap task spawned by the owning
/// `TextEditorComponent` (`mod.rs`) rebuilds the same hints from a
/// [`PendingLayoutJob`]'s owned `rendered_cache`/`gutter_insets` clones —
/// `RowHints` borrows, so it cannot cross the `tokio::spawn` boundary
/// itself and has to be reconstructed on the other side from owned data.
pub(super) fn row_hints<'a>(rendered: &'a [Vec<bool>], insets: &'a [usize]) -> Vec<RowHints<'a>> {
    let rows = rendered.len().max(insets.len());
    (0..rows)
        .map(|row| RowHints {
            visible: rendered.get(row).map(Vec::as_slice).unwrap_or(&[]),
            inset: insets.get(row).copied().unwrap_or(0),
        })
        .collect()
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
    /// Tests describe buffers as rows; the view takes the text they make up.
    fn text_of(lines: &[String]) -> ropetext::Text {
        ropetext::Text::from(lines.join("\n").as_str())
    }

    fn update_view(
        v: &mut MarkdownEditorView,
        lines: &[String],
        cursor: (usize, usize),
        rect: Rect,
        generation: u64,
        selection: Option<((usize, usize), (usize, usize))>,
    ) {
        // Selection reaches the view as an **overlay** now.
        let rev = NonZeroU64::new(generation.max(1)).unwrap();
        let clamped = if lines.is_empty() {
            (0, 0)
        } else {
            (cursor.0.min(lines.len() - 1), cursor.1)
        };
        let snap = super::super::snapshot::EditorSnapshot::borrowed(lines, clamped, rev);
        v.update(&snap, rect);
        let overlays = match selection {
            Some(((sr, sc), (er, ec))) => (sr..=er)
                .map(|row| {
                    Overlay::new(
                        row,
                        if row == sr { sc } else { 0 },
                        if row == er { ec } else { usize::MAX },
                        OverlayKind::Selection,
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        v.set_overlays(overlays);
    }

    /// Build a freshly-updated view from `lines` with the cursor at
    /// `cursor` and the given editor `width`, using the real snapshot +
    /// `update()` path. Height is fixed at 24.
    fn make_view_for_lines(
        lines: &[String],
        cursor: (usize, usize),
        width: u16,
    ) -> MarkdownEditorView {
        let mut v = MarkdownEditorView::new();
        let r = Rect {
            x: 0,
            y: 0,
            width,
            height: 24,
        };
        update_view(&mut v, lines, cursor, r, 1, None);
        v
    }

    #[test]
    fn selection_highlight_respects_emoji_cluster_width() {
        // Span "a❤️b" where ❤️ = U+2764 + VS16 renders as 2 display columns:
        // a=col0, ❤️=cols1..3, b=col3. Selecting cols 1..3 must highlight
        // exactly the heart cluster — not split it, and not bleed into 'b'.
        let theme = Theme::default();
        let sel_bg = theme.selection_bg.to_ratatui();
        let heart = "\u{2764}\u{FE0F}";
        let content = format!("a{heart}b");
        let spans = vec![ratatui::text::Span::raw(content)];
        let out = restyle_over_range(spans, 1..3, &|st| st.bg(sel_bg));

        let highlighted: String = out
            .iter()
            .filter(|s| s.style.bg == Some(sel_bg))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(highlighted, heart, "selection must cover exactly the heart");

        // No output span may split the cluster: every span's content must
        // recluster identically (the heart stays whole within one span).
        for s in &out {
            let c = s.content.as_ref();
            assert!(
                !c.contains('\u{2764}') || c.contains(heart),
                "emoji cluster split across spans: {c:?}"
            );
        }
    }

    #[test]
    fn code_box_background_reaches_rendered_cells() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let lines = vec![
            "```".to_string(),
            "let x = 1;".to_string(),
            "```".to_string(),
            "plain".to_string(),
        ];
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        let mut view = make_view_for_lines(&lines, (3, 0), 40);
        let mut terminal = Terminal::new(TestBackend::new(40, 5)).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &theme, true, Some(CursorShape::Bar)))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let code_bg = theme.code_bg.to_ratatui();
        let cell = |x: u16, y: u16| &buf.content[(y as usize) * 40 + (x as usize)];

        // A cell on the fenced code content row carries the code-box bg...
        assert_eq!(
            cell(0, 1).bg,
            code_bg,
            "code content cell must have code_bg"
        );
        // ...including the padding past the text (box is a solid rectangle).
        assert_eq!(cell(8, 1).bg, code_bg, "code-box padding must have code_bg");
        // A prose row outside the block does NOT get the code bg.
        assert_ne!(cell(0, 3).bg, code_bg, "prose row must not have code_bg");
    }

    #[test]
    fn blockquote_gutter_inset_off_cursor_row_only() {
        // Two blockquote lines; cursor on row 0.
        let lines = vec!["> first".to_string(), ">> second".to_string()];
        let view = make_view_for_lines(&lines, (0, 1), 80);
        let g = view.gutter_insets_for_testing();
        assert_eq!(g[0], 0); // cursor row → revealed, no gutter
        assert_eq!(g[1], 3); // depth 2 → 2 bars + 1 space
    }

    #[test]
    fn code_box_width_is_block_max_capped_to_width() {
        let lines = vec![
            "```".to_string(),
            "let x = 1;".to_string(),    // 10
            "let yy = 222;".to_string(), // 13 (widest)
            "```".to_string(),
            "plain".to_string(),
        ];
        let view = make_view_for_lines(&lines, (0, 0), 80); // width 80
        let w = view.code_box_width_for_testing();
        assert_eq!(w[0], Some(13));
        assert_eq!(w[1], Some(13));
        assert_eq!(w[2], Some(13));
        assert_eq!(w[3], Some(13));
        assert_eq!(w[4], None);
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
        let pb = ParsedBuffer::parse_lines(&lines);
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
        let pb = ParsedBuffer::parse_lines(&lines);
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
            .draw(|f| v.render(f, f.area(), &theme, true, Some(CursorShape::Bar)))
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
    fn reported_damage_agrees_with_the_diff_it_replaced() {
        // The engine tells the view which rows it changed, so the view no longer
        // compares the buffer against a copy of its previous self. The two must
        // reach the same answer, or "told" quietly means something else than
        // "found" and every parse after an edit is subtly wrong.
        // Blank lines between blocks, so widening stops at a paragraph boundary
        // rather than reaching the buffer's edges. Without them every damage range
        // widens to the whole buffer and this would pass whatever the report says,
        // proving nothing about it being read.
        let mut lines: Vec<String> = Vec::new();
        for block in 0..4 {
            lines.push(format!("block {block} first line"));
            lines.push(format!("block {block} second line"));
            lines.push(String::new());
        }
        let mut edited = lines.clone();
        edited[7].push_str(" more");

        let mut found = MarkdownEditorView::new();
        update_view(&mut found, &lines, (7, 0), rect(20), 1, None);
        let by_diff = found.try_incremental_parse(&text_of(&edited), (7, 0), None);

        let mut told = MarkdownEditorView::new();
        update_view(&mut told, &lines, (7, 0), rect(20), 1, None);
        let by_report = told.try_incremental_parse(&text_of(&edited), (7, 0), Some(7..8));

        assert!(by_diff.is_some(), "the diff finds this edit incrementally");
        let (diff_range, diff_slice, _) = by_diff.expect("checked");
        let (report_range, report_slice, _) = by_report.expect("the report must too");
        assert_eq!(diff_range, report_range, "widened ranges diverge");
        assert_eq!(diff_slice.kinds, report_slice.kinds, "parsed kinds diverge");
    }

    #[test]
    fn a_report_past_the_end_of_the_buffer_falls_back_to_the_diff() {
        // A stale report — rows that no longer exist — must not be trusted. The
        // guard is what keeps a report from indexing outside the buffer.
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        let mut edited = lines.clone();
        edited[1].push_str(" more");
        let mut v = MarkdownEditorView::new();
        update_view(&mut v, &lines, (1, 0), rect(20), 1, None);
        assert!(
            v.try_incremental_parse(&text_of(&edited), (1, 0), Some(0..99))
                .is_some(),
            "an out-of-range report falls back rather than panicking"
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
            v.try_incremental_parse(&text_of(&new_lines), (1, 0), None)
                .is_none(),
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
            v.try_incremental_parse(&text_of(&new_lines), (1, 1), None)
                .is_none(),
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
            v.try_incremental_parse(&text_of(&new_lines), (1, 0), None)
                .is_none(),
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
        assert_eq!(v.text_snapshot.to_string(), "original");
    }

    #[test]
    fn new_generation_triggers_snapshot_rebuild() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["original".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(10), 1, None);
        let lines2 = vec!["changed".to_string()];
        update_view(&mut v, &lines2, (0, 0), rect(10), 2, None);
        assert_eq!(v.text_snapshot.to_string(), "changed");
    }

    /// Task and needle decoration moved out of a cell-space post-pass and into
    /// overlay derivation. Same behaviour, logical coordinates, visible rows
    /// only — and now expressible without a terminal buffer.
    #[test]
    fn content_overlays_cover_needles_and_tasks() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "find the needle here".to_string(),
            "- [x] done task".to_string(),
            "- [ ] open task".to_string(),
        ];
        v.set_needles(vec!["needle".to_string()]);
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);

        let kinds: Vec<_> = v.overlays.iter().map(|o| (o.row, o.kind)).collect();
        assert!(
            kinds.contains(&(0, OverlayKind::Needle)),
            "the needle must be emphasised, got {kinds:?}"
        );
        assert!(kinds.contains(&(1, OverlayKind::TaskBox)));
        assert!(
            kinds.contains(&(1, OverlayKind::TaskDone)),
            "a done task strikes its text"
        );
        assert!(kinds.contains(&(2, OverlayKind::TaskBox)));
        assert!(
            !kinds.contains(&(2, OverlayKind::TaskDone)),
            "an open task does not"
        );

        // "needle" starts at logical char 9 — a logical column, not a cell.
        let needle = v
            .overlays
            .iter()
            .find(|o| o.kind == OverlayKind::Needle)
            .unwrap();
        assert_eq!((needle.start, needle.end), (9, 15));
    }

    #[test]
    fn update_takes_a_selection_overlay() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(40), 1, Some(((0, 0), (0, 5))));
        assert_eq!(
            v.overlays,
            vec![Overlay::new(0, 0, 5, OverlayKind::Selection)]
        );
    }

    /// Overlays belong to the frame they were built for: `update` clears them,
    /// so a caller that stops producing one cannot leave it painted.
    #[test]
    fn update_clears_the_previous_frame_s_overlays() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        update_view(&mut v, &lines, (0, 0), rect(40), 1, Some(((0, 0), (0, 5))));
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);
        assert!(v.overlays.is_empty());
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
        v.install_full_parse(1, ParsedBuffer::parse_lines(&lines));

        // Single-char insert at row 500.
        lines[500].push('x');
        let edited_len = lines[500].len();
        update_view(&mut v, &lines, (500, edited_len), rect(40), 2, None);

        // The spliced result must equal a fresh full parse.
        let fresh = ParsedBuffer::parse_lines(&lines);
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
        v.install_full_parse(2, ParsedBuffer::parse_lines(&lines));
        assert!(
            !v.parse_state.is_placeholder(),
            "placeholder cleared on install"
        );
        assert_eq!(
            v.parse_state.buf().kinds,
            ParsedBuffer::parse_lines(&lines).kinds
        );
    }

    #[test]
    #[should_panic(expected = "splice on placeholder parse")]
    fn splice_real_on_placeholder_is_rejected() {
        // The type makes the wrong-splice hazard unrepresentable on the
        // Gate 1 path; this guards the `ParseState::splice_real` contract
        // directly so a future caller can't route a splice into a
        // placeholder without tripping the assert.
        let mut state = ParseState::Placeholder {
            buf: ParsedBuffer::placeholder_lines(&["x".to_string()]),
            generation: 1,
            spawned: false,
        };
        state.splice_real(0..1, ParsedBuffer::parse_lines(&["y".to_string()]));
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

        let fresh = ParsedBuffer::parse_lines(&lines);
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
        let real = ParsedBuffer::parse_lines(&lines);
        let generation = pending.unwrap();
        v.install_full_parse(generation, real);
        let fresh = ParsedBuffer::parse_lines(&lines);
        assert_eq!(
            v.parse_state.buf().kinds,
            fresh.kinds,
            "post-install kinds must match fresh full parse"
        );
    }

    /// Rows long enough that a real 40-wide wrap would split them into
    /// more than one visual line each — needed to tell a `Layout::unwrapped`
    /// stub (always exactly one visual line per row) apart from a real
    /// compute that happens not to have wrapped anything.
    fn make_long_lines(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                format!(
                    "paragraph number {i} with quite a bit of extra padding text \
                     so this row is longer than forty columns wide for sure"
                )
            })
            .collect()
    }

    #[test]
    fn layout_defers_to_async_fallback_on_large_buffer() {
        // Layout-side twin of `fence_toggle_on_large_buffer_defers_to_async_fallback`:
        // above LARGE_BUFFER_THRESHOLD, a full-rebuild trigger installs a
        // `Layout::unwrapped` stub + signals pending instead of blocking
        // the typing thread on `Layout::compute`. The owning component
        // spawns the real wrap on tokio and calls install_full_layout
        // when done.
        let mut v = MarkdownEditorView::new();
        let mut lines = make_long_lines(1500);
        update_view(&mut v, &lines, (750, 0), rect(40), 1, None);

        // Line-count change forces a full layout rebuild regardless of
        // what the parse decided.
        lines.insert(750, "```".to_string());
        update_view(&mut v, &lines, (750, 3), rect(40), 2, None);

        let pending = v.take_pending_full_layout();
        assert!(
            pending.is_some(),
            "large-buffer full layout rebuild must signal pending async wrap"
        );
        assert_eq!(
            v.layout.row_count(),
            lines.len(),
            "stub row count must match input"
        );
        let real_visual_lines = {
            let hints = row_hints(&v.rendered_cache, &v.gutter_insets);
            Layout::compute(&v.text_snapshot, 40, Metrics::default(), &hints).visual_line_count()
        };
        assert!(
            v.layout.visual_line_count() < real_visual_lines,
            "the installed stub must not have wrapped these long rows yet \
             (stub: {}, real: {})",
            v.layout.visual_line_count(),
            real_visual_lines
        );

        // Caller (TextEditorComponent in production) spawns the real wrap
        // and installs the result. Simulate that here.
        let job = pending.unwrap();
        let hints = row_hints(&job.rendered_cache, &job.gutter_insets);
        let real = Layout::compute(&job.text, job.width, Metrics::default(), &hints);
        let real_count = real.visual_line_count();
        v.install_full_layout(job.generation, real);
        assert!(v.layout_pending.is_none(), "pending cleared on install");
        assert_eq!(
            v.layout.visual_line_count(),
            real_count,
            "post-install layout must equal a fresh compute"
        );
    }

    #[test]
    fn small_buffer_layout_stays_synchronous() {
        // Mirrors `fence_toggle_triggers_full_rebuild_fallback`: below
        // LARGE_BUFFER_THRESHOLD, layout rebuilds stay synchronous.
        let mut v = MarkdownEditorView::new();
        let mut lines = make_long_lines(700);
        update_view(&mut v, &lines, (350, 0), rect(40), 1, None);
        assert!(
            v.take_pending_full_layout().is_none(),
            "small buffer must not defer layout on first parse"
        );

        lines.insert(350, "```".to_string());
        update_view(&mut v, &lines, (350, 3), rect(40), 2, None);
        assert!(
            v.take_pending_full_layout().is_none(),
            "small-buffer full rebuild must NOT defer layout to async"
        );
    }

    #[test]
    fn edit_while_layout_pending_rearms() {
        // Layout-side twin of
        // `edit_while_placeholder_active_refuses_incremental_and_rearms`:
        // an edit landing before the async wrap resolves must re-stub and
        // re-arm for the new generation, and a stale (superseded)
        // install must be a no-op.
        let mut v = MarkdownEditorView::new();
        let mut lines = make_long_lines(1500);
        update_view(&mut v, &lines, (750, 0), rect(40), 1, None);
        assert!(
            v.layout_pending.is_some(),
            "first layout defers on a large buffer"
        );
        assert_eq!(v.take_pending_full_layout().map(|j| j.generation), Some(1));

        // Edit before the background wrap resolves.
        lines[0].push('x');
        update_view(&mut v, &lines, (0, lines[0].len()), rect(40), 2, None);
        assert!(
            v.layout_pending.is_some(),
            "still pending — a content-changing edit must not silently keep the stale stub"
        );
        assert_eq!(
            v.take_pending_full_layout().map(|j| j.generation),
            Some(2),
            "re-armed for the new generation"
        );

        // A stale install (superseded generation) must be dropped.
        let stale = Layout::compute(&text_of(&lines), 40, Metrics::default(), &[]);
        v.install_full_layout(1, stale);
        assert!(
            v.layout_pending.is_some(),
            "stale-generation install must be a no-op"
        );

        // The current generation's install lands.
        let hints = row_hints(&v.rendered_cache, &v.gutter_insets);
        let real = Layout::compute(&v.text_snapshot, 40, Metrics::default(), &hints);
        v.install_full_layout(2, real);
        assert!(
            v.layout_pending.is_none(),
            "pending cleared on matching-generation install"
        );
    }

    #[test]
    fn install_full_layout_rejects_width_mismatch_from_a_resize() {
        // A resize with no content change never bumps content_revision, so
        // the generation check alone cannot catch a wrap job computed for
        // a width the pane no longer has — `install_full_layout` must also
        // compare `layout.width()` against the current `last_layout_width`.
        let mut v = MarkdownEditorView::new();
        let lines = make_long_lines(1200);
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);
        let job = v
            .take_pending_full_layout()
            .expect("large buffer defers layout on first parse");
        assert_eq!(job.width, 40);

        // Pane resized before the background wrap for width 40 lands.
        update_view(
            &mut v,
            &lines,
            (0, 0),
            Rect {
                width: 80,
                ..rect(40)
            },
            1,
            None,
        );

        let hints = row_hints(&job.rendered_cache, &job.gutter_insets);
        let stale_width_layout = Layout::compute(&job.text, job.width, Metrics::default(), &hints);
        v.install_full_layout(job.generation, stale_width_layout);
        assert!(
            v.layout_pending.is_some(),
            "width-mismatched install must be rejected even though the generation matches"
        );
    }

    /// Assert the view's cached parse matches a fresh one.
    ///
    /// The per-line comparison uses `debug_assert_eq_to`, which is
    /// `#[cfg(debug_assertions)]` like its one production caller — so the
    /// *body* is gated, not the function. Gating the whole helper would remove
    /// a symbol six tests call; leaving it ungated stops the lib-test target
    /// compiling under any profile with assertions off, which is why
    /// `cargo bench --no-run` did not build.
    fn full_rebuild_equals_view_state(v: &MarkdownEditorView, lines: &[String]) {
        #[cfg(not(debug_assertions))]
        let _ = (v, lines);
        #[cfg(debug_assertions)]
        {
            let fresh = ParsedBuffer::parse_lines(lines);
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
        let fresh_text = ropetext::Text::from(edited.join("\n").as_str());
        let fresh_hints = row_hints(v.rendered_cache_for_testing(), &[]);
        let fresh_layout = Layout::compute(&fresh_text, 40, Metrics::default(), &fresh_hints);

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
    fn incremental_edit_reuses_fence_ranges_without_rescanning() {
        // A fence block plus plain paragraphs elsewhere. An edit inside a
        // plain paragraph (not touching the fence) must take the
        // incremental path — at which point `fence_ranges` is skipped
        // rather than rescanned, per the structural guards that already
        // gate the splice. Verify it stays correct anyway.
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = vec![
            "```".to_string(),
            "code line".to_string(),
            "```".to_string(),
        ];
        lines.extend((0..200).map(|i| format!("paragraph {i} with some text")));
        update_view(&mut v, &lines, (100, 0), rect(40), 1, None);

        lines[100].push('x');
        update_view(&mut v, &lines, (100, lines[100].len()), rect(40), 2, None);
        assert!(v.last_parse_was_incremental, "expected incremental path");

        let fresh = ParsedBuffer::parse_lines(&lines);
        assert_eq!(
            v.fence_ranges,
            super::super::parse_incremental::fence_ranges_from_kinds(&fresh.kinds),
            "fence_ranges must stay correct after a skipped recompute"
        );
    }

    #[test]
    fn incremental_edit_patches_only_cursor_rows_of_gutter_insets() {
        // Blockquote rows at the top; a content edit far away (row 150)
        // combined with the cursor moving between two blockquote rows in
        // the same frame. The edit alone keeps the parse incremental; the
        // cursor move is what gutter_insets must still react to correctly
        // without re-walking every row.
        let mut v = MarkdownEditorView::new();
        // Blank line after the blockquote so the paragraph run below gets
        // its own reset boundary instead of lazily continuing the quote —
        // otherwise every row folds into one giant construct and even a
        // distant edit falls back to a full rebuild.
        let mut lines: Vec<String> = vec![
            "> quoted line 0".to_string(),
            "> quoted line 1".to_string(),
            "> quoted line 2".to_string(),
            String::new(),
        ];
        lines.extend((0..200).map(|i| format!("paragraph {i} with some text")));
        update_view(&mut v, &lines, (0, 0), rect(40), 1, None);

        lines[151].push('x');
        update_view(&mut v, &lines, (1, 0), rect(40), 2, None);
        assert!(v.last_parse_was_incremental, "expected incremental path");

        let mut fresh_view = MarkdownEditorView::new();
        update_view(&mut fresh_view, &lines, (1, 0), rect(40), 1, None);
        assert_eq!(
            v.gutter_insets_for_testing(),
            fresh_view.gutter_insets_for_testing(),
            "patched gutter_insets must match a full rebuild"
        );
    }

    #[test]
    fn incremental_edit_patches_only_the_touched_code_block_of_code_box_width() {
        // Two fenced blocks. Growing a line inside the SECOND block must
        // not touch the first block's cached width, and the result must
        // match a full rebuild.
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = vec![
            "```".to_string(),
            "short".to_string(),
            "```".to_string(),
            "paragraph between blocks".to_string(),
            "```".to_string(),
            "also short".to_string(),
            "```".to_string(),
        ];
        update_view(&mut v, &lines, (5, 0), rect(40), 1, None);
        let before_first_block = v.code_box_width_for_testing()[0..3].to_vec();

        lines[5].push_str(" grown considerably wider now");
        update_view(&mut v, &lines, (5, lines[5].len()), rect(40), 2, None);
        assert!(v.last_parse_was_incremental, "expected incremental path");

        assert_eq!(
            v.code_box_width_for_testing()[0..3],
            before_first_block[..],
            "the untouched first block's width must be unchanged"
        );

        let mut fresh_view = MarkdownEditorView::new();
        update_view(
            &mut fresh_view,
            &lines,
            (5, lines[5].len()),
            rect(40),
            1,
            None,
        );
        assert_eq!(
            v.code_box_width_for_testing(),
            fresh_view.code_box_width_for_testing(),
            "patched code_box_width must match a full rebuild"
        );
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

    #[test]
    fn apply_code_box_sets_bg_and_pads_to_width() {
        use ratatui::text::Span;
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        let spans = vec![Span::raw("ab")]; // 2 cols
        let out = super::apply_code_box(spans, 5, &theme);
        let total: usize = out.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(total, 5); // padded to box width
        let bg = theme.code_bg.to_ratatui();
        assert!(out.iter().all(|s| s.style.bg == Some(bg)));
    }

    #[test]
    fn apply_code_box_measures_emoji_cluster_at_full_width() {
        // Regression: padding must use the same cluster model as
        // `raw_display_width` (which sizes the box). "a❤️" = 'a' (1) + VS16 heart
        // (2) = 3 rendered cols. Per-codepoint width undercounts the heart as 1,
        // over-padding the box and overshooting box_width. With cluster width the
        // content already fills 3 cols, so a box_width of 3 needs zero padding.
        use ratatui::text::Span;
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        let content = "a\u{2764}\u{FE0F}";
        assert_eq!(super::super::markdown::raw_display_width(content), 3);
        let out = super::apply_code_box(vec![Span::raw(content)], 3, &theme);
        // No padding span appended — content already 3 cols.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content.as_ref(), content);
    }

    #[test]
    fn click_on_barred_blockquote_maps_past_gutter() {
        // Blockquote on row 0 is NOT the cursor row (cursor parked on row 1),
        // so row 0 renders "│ hello". vrow 0 is that row's single visual line.
        let lines = vec!["> hello".to_string(), "tail".to_string()];
        let view = make_view_for_lines(&lines, (1, 0), 80);
        // Click screen col 2 ('h' after the 2-col "│ " gutter) → logical col 2.
        let (row, col) = view.click_to_logical_for_testing(0, 2);
        assert_eq!((row, col), (0, 2));
    }
}
