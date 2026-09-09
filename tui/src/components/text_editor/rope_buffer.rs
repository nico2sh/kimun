//! The **rope buffer**: the editor's buffer over the `ropetext` **edit buffer**.
//!
//! The engine owns the text, the cursor, the selection and the history, and
//! deliberately nothing else (ADR-0041). This is the rest of an editor's buffer —
//! what kimün adds, not what it adapts:
//!
//! - **undo groups**: [`RopeBuffer::edit`] scopes and [`RopeBuffer::continue_group`],
//!   so a compound action or a typing run is one history entry;
//! - the edit outcome: what a mutation did, measured — changed, bulk, the damage
//!   hull in one numbering, the line delta — drained by the component through
//!   [`RopeBuffer::take_outcome`];
//! - the goal column a vertical motion aims at;
//! - the **indent step** ([`RopeBuffer::indent_rows`]): the one place Tab, `>>` and
//!   a list continuation's dedent move a row by;
//! - the **find pattern** and its row-wise, wrapping search, which outlive the
//!   find bar so vim's `n`/`N` can repeat them;
//! - the yank transport the vim engine's **unnamed register** fills from;
//! - the `(row, col)` vocabulary every caller speaks: the vim engine, the plain
//!   key table, the find bar and the component all take `&mut RopeBuffer`.
//!
//! It knows text, not markdown. List continuation, **auto-surround** and the
//! emphasis markers are operations *over* it, in `markdown_edits`, and are
//! tested against a bare one.
//!
//! Nothing here mirrors the text into a second representation. Callers that want
//! rows ask for them ([`RopeBuffer::rows`]) and pay for them there; the buffer
//! keeps one copy of the note and no derived copy in step with it.
//!
use crate::ropetext::motion::{self, Goal, Words};
use crate::ropetext::{Change, Column, EditBuffer as Rope, Position, Span, Text};

/// How far one **indent step** moves a row, in spaces — what Tab, `>>` and the
/// visual `>` add, and what their inverses remove.
///
/// Not a tab stop, and deliberately not derived from one. A tab stop is elastic
/// (a `\t` advances to the next multiple of it, so its width depends on where it
/// starts) and describes how an existing character *draws*; an indent step is a
/// fixed amount of text an edit *inserts*, and here it is always spaces. Vim
/// keeps the two apart as `tabstop` and `shiftwidth`, EditorConfig as
/// `tab_width` and `indent_size`. That both are 4 today
/// ([`crate::ropetext::Metrics::DEFAULT_TAB_WIDTH`]) is a coincidence of defaults.
const DEFAULT_INDENT_WIDTH: std::num::NonZeroU8 = std::num::NonZeroU8::new(4).unwrap();

/// What one call to [`RopeBuffer::edit`] did, measured rather than predicted.
///
/// `#[must_use]` on purpose: the caller still applies these (the revision clock
/// serves both backends and so stays on the component), and forgetting to is
/// exactly the failure this type exists to prevent. A warning is a check; a
/// convention is not.
#[must_use = "an edit's outcome drives the revision bump and the parse-damage signal"]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditOutcome {
    /// The buffer's text differs from before the edit. A content comparison —
    /// never a library return value, which can report `false` after mutating.
    pub changed: bool,
    /// The change is not confined to the cursor's row, so the incremental
    /// parser's cursor damage hint would under-report it.
    pub bulk: bool,
    /// Which rows the edits changed, in the new text's numbering — the hull when
    /// several ran before this was drained.
    ///
    /// Told by the engine rather than found by comparing the buffer with a copy
    /// of its previous self, which is what the revision-tagged rope makes
    /// possible. The **nvim** backend reports lines and not changes, so it
    /// leaves this `None` and its consumer falls back to a diff.
    pub damage: Option<std::ops::Range<usize>>,
    /// Net rows added (or removed, when negative) by the edits behind `damage`.
    ///
    /// Travels with the range because the range is only meaningful in a
    /// numbering, and a consumer that accumulates reports across several drains
    /// has to bring the older one forward before it can union them.
    pub line_delta: isize,
}

/// `range`, renumbered for a change of `delta` lines starting at `at`.
///
/// Rows above the change keep their index; the rest move with it. A `delta` of
/// zero — every edit that stays within its rows, which is most of them — leaves
/// the range alone.
///
/// Shared because damage is accumulated in two places: here, across the
/// mutations of one group, and in the view, across the drains between two
/// frames. Both union ranges recorded against different texts, and both are
/// wrong in the same way without this.
pub(super) fn shift_rows(
    range: std::ops::Range<usize>,
    at: usize,
    delta: isize,
) -> std::ops::Range<usize> {
    let shift = |row: usize| {
        if row < at {
            row
        } else {
            row.saturating_add_signed(delta)
        }
    };
    shift(range.start)..shift(range.end)
}

/// Whether a delete fills the register it removed text from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Yank {
    Keep,
    Discard,
}

/// One row's share of an indent, addressed against the text before the block
/// began; the transaction carries it across the rows edited before it.
#[derive(Debug, Clone, Copy)]
enum RowEdit {
    Insert(Position),
    Delete(Span),
}

/// A cursor movement, in the vocabulary the editor already speaks.
///
/// Deliberately the incumbent's variant set, so the 145 call sites need no
/// rewriting — but `Jump` takes `usize` rather than `u16`, because clamping a
/// row to 65535 is a defect the old widget's contract allowed, and this is the
/// type where it stops being representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMove {
    Forward,
    Back,
    Up,
    Down,
    Head,
    End,
    Top,
    Bottom,
    WordForward,
    WordBack,
    WordEnd,
    /// `W` — a WORD is any run of non-blanks.
    WordForwardBig,
    /// `B`.
    WordBackBig,
    /// `E`.
    WordEndBig,
    /// `ge` / `gE`.
    WordEndBack {
        big: bool,
    },
    /// `%`. Stays put when there is no bracket ahead on the row, or it is
    /// unbalanced.
    MatchingPair,
    ParagraphForward,
    ParagraphBack,
    Jump(usize, usize),
}

impl CursorMove {
    /// Whether the movement is vertical, and so keeps the goal column.
    ///
    /// `Top` and `Bottom` are row movements but deliberately *not* goal-preserving:
    /// vim's `gg`/`G` go to a row's first non-blank rather than to a remembered
    /// column, and making them sticky would be a third behaviour that neither vim
    /// nor the incumbent has. Change #8 is about `Up`/`Down`.
    fn is_vertical(self) -> bool {
        matches!(self, CursorMove::Up | CursorMove::Down)
    }
}

/// The open note's text, cursor, selection and history.
#[derive(Debug)]
pub struct RopeBuffer {
    inner: Rope,
    /// Accumulated since the last drain. The component owns the revision clock
    /// for as long as the nvim backend has no edit buffer.
    pending: EditOutcome,
    /// Nesting depth of [`Self::edit`]. Above zero, a mutation extends the open
    /// group instead of starting its own.
    depth: u32,
    /// Whether the open group has recorded anything yet, so the first mutation
    /// inside `edit` starts the group and the rest extend it.
    group_started: bool,
    /// Set by a backend that has decided the next mutation continues what the
    /// last one started — a typing run, an insert session. Cleared by using it,
    /// so continuing is asked for per edit rather than left switched on.
    continue_group: bool,
    /// The column a vertical movement is aiming at, which is why walking down
    /// through a short row and out the other side returns to where it started.
    goal: Option<Column>,
    yank: String,
    search: Option<regex::Regex>,
    indent_width: std::num::NonZeroU8,
}

impl Default for RopeBuffer {
    fn default() -> Self {
        Self::new(Text::new())
    }
}

impl RopeBuffer {
    pub fn new(text: Text) -> Self {
        Self {
            inner: Rope::new(text),
            pending: EditOutcome::default(),
            depth: 0,
            group_started: false,
            continue_group: false,
            goal: None,
            yank: String::new(),
            search: None,
            indent_width: DEFAULT_INDENT_WIDTH,
        }
    }

    /// Replace the whole buffer, dropping the history with it.
    pub fn replace(&mut self, text: Text) {
        self.inner.set_text(text);
        self.pending = EditOutcome::default();
        self.goal = None;
    }

    pub fn text(&self) -> &Text {
        self.inner.text()
    }

    /// Pin the **indent step** to a non-default width, so a test can catch a
    /// reintroduced literal. Nothing configures the step in production yet.
    #[cfg(test)]
    pub fn set_indent_width(&mut self, spaces: u8) {
        self.indent_width =
            std::num::NonZeroU8::new(spaces).expect("an indent step is at least one space");
    }

    pub fn snapshot(&self) -> crate::ropetext::Snapshot {
        self.inner.snapshot()
    }

    // ── Reads ────────────────────────────────────────────────────────────────

    /// One row's text, or `None` past the end.
    pub fn row(&self, row: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.inner.text().line(row)
    }

    /// How many rows the buffer has. Never zero.
    pub fn row_count(&self) -> usize {
        self.inner.text().line_count()
    }

    /// Every row, materialised.
    ///
    /// Not a cache: nothing is maintained between calls, so the cost lands on the
    /// caller that wants a vector rather than on every edit. That is the whole
    /// difference from the shim this replaced.
    pub fn rows(&self) -> Vec<String> {
        self.inner.text().lines().map(|l| l.to_string()).collect()
    }

    /// Rows `first..=last`, joined with newlines.
    pub fn joined_rows(&self, first: usize, last: usize) -> String {
        (first..=last)
            .filter_map(|row| self.row(row))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Characters in one row.
    pub fn row_len(&self, row: usize) -> usize {
        self.inner.text().line_len_chars(row).unwrap_or(0)
    }

    pub fn cursor(&self) -> (usize, usize) {
        let cursor = self.inner.cursor();
        (cursor.row(), cursor.column().get())
    }

    pub fn is_empty(&self) -> bool {
        self.inner.text().len_bytes() == 0
    }

    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let span = self.inner.selection()?;
        Some((rc(span.start()), rc(span.end())))
    }

    pub fn yank_text(&self) -> String {
        self.yank.clone()
    }

    pub fn set_yank_text(&mut self, text: impl Into<String>) {
        self.yank = text.into();
    }

    pub fn search_pattern(&self) -> Option<&regex::Regex> {
        self.search.as_ref()
    }

    pub fn take_outcome(&mut self) -> EditOutcome {
        std::mem::take(&mut self.pending)
    }

    // ── Groups ───────────────────────────────────────────────────────────────

    /// Run `f` as one **undo group**.
    ///
    /// Every mutation inside lands in a single history entry, however many
    /// primitives it takes. Nested calls belong to the outermost group, so a
    /// compound action built from the single-mutation helpers is still one undo.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        if self.depth > 0 {
            return f(self);
        }
        self.depth = 1;
        self.group_started = false;
        let out = f(self);
        self.depth = 0;
        self.group_started = false;
        out
    }

    /// The next mutation joins the previous group instead of starting one.
    ///
    /// The policy is the backend's, because only it knows what the user was doing
    /// — mid-word against after a pause, inside an Insert session against having
    /// left it. This is the mechanism; `typing_run` and the vim engine are the two
    /// callers that hold an opinion.
    pub fn continue_group(&mut self) {
        self.continue_group = true;
    }

    /// Apply one primitive as its own group, or as part of an open one.
    fn mutate(&mut self, f: impl FnOnce(&mut crate::ropetext::Txn<'_>)) -> bool {
        let extending =
            (self.depth > 0 && self.group_started) || std::mem::take(&mut self.continue_group);
        let mut txn = if extending {
            self.inner.begin_extending()
        } else {
            self.inner.begin()
        };
        f(&mut txn);
        let change = txn.commit();
        if self.depth > 0 {
            self.group_started = true;
        }
        self.record(change)
    }

    fn record(&mut self, change: Option<Change>) -> bool {
        let Some(change) = change else {
            return false;
        };
        self.pending.changed = true;
        self.pending.bulk |= change.is_bulk();
        self.pending.line_delta += change.line_delta();
        self.pending.damage = Some(match self.pending.damage.take() {
            Some(seen) => {
                // `seen` was recorded against the text as it stood before *this*
                // change, which may have moved those rows. Hulling the two
                // directly unions ranges from two different numberings, and the
                // result is not a superset of either: an edit high in the buffer
                // followed by one above it that adds a line leaves the first
                // edit's row below the hull's end, so it is never re-parsed and
                // renders stale. Bring it into the current numbering first.
                let seen = shift_rows(seen, change.rows().start, change.line_delta());
                seen.start.min(change.rows().start)..seen.end.max(change.rows().end)
            }
            None => change.rows(),
        });
        true
    }

    // ── Mutations ────────────────────────────────────────────────────────────

    pub fn insert_str(&mut self, s: impl AsRef<str>) -> bool {
        let text = s.as_ref().to_string();
        // The anchor goes whether or not it spanned anything, as it does for a
        // delete: typing is not a selection gesture either.
        let span = self.inner.selection().filter(|span| !span.is_empty());
        self.inner.clear_selection();
        let cursor = self.inner.cursor();
        self.goal = None;
        self.mutate(|txn| match span {
            Some(span) => {
                txn.replace(span, &text);
            }
            None => {
                txn.insert(cursor, &text);
            }
        })
    }

    pub fn insert_char(&mut self, c: char) {
        self.insert_str(c.to_string());
    }

    pub fn insert_newline(&mut self) {
        self.insert_str("\n");
    }

    /// Delete `clusters` grapheme clusters forward, a line break counting as one.
    ///
    /// Clusters and not scalars, because a delete may not leave half a character
    /// behind: `forward_by` steps whole clusters, so a caller counting scalars
    /// over a flag or a ZWJ emoji spends the difference on the text after it.
    pub fn delete_str(&mut self, clusters: usize) -> bool {
        if self.take_selection() {
            return true;
        }
        if clusters == 0 {
            return false;
        }
        let from = self.inner.cursor();
        let to = self.forward_by(from, clusters);
        self.delete_between(from, to, Yank::Keep)
    }

    /// Backspace.
    pub fn delete_char(&mut self) -> bool {
        if self.take_selection() {
            return true;
        }
        let to = self.inner.cursor();
        let from = motion::prev_cluster(self.inner.text(), to);
        // A backspace does not fill the register; only `delete_str`, the word
        // deletes and `cut` do. Matching the incumbent, which is also vim: `x`
        // yanks, but a plain backspace in Insert does not.
        self.delete_between(from, to, Yank::Discard)
    }

    /// Forward delete.
    pub fn delete_next_char(&mut self) -> bool {
        if self.take_selection() {
            return true;
        }
        let from = self.inner.cursor();
        let to = motion::next_cluster(self.inner.text(), from);
        self.delete_between(from, to, Yank::Discard)
    }

    pub fn delete_word(&mut self) -> bool {
        if self.take_selection() {
            return true;
        }
        let to = self.inner.cursor();
        let text = self.inner.text();
        // The incumbent's cascade, which *is* the contract: a word start on this
        // row, else the row's start, else the line break before it. The last case
        // is why this cannot simply be a row-local motion.
        let candidate = motion::word_start_back(text, to, Words::Small);
        let (from, yank) = if candidate.row() == to.row() && candidate.byte() < to.byte() {
            (candidate, Yank::Keep)
        } else if to.column().get() > 0 {
            (motion::row_start(text, to), Yank::Keep)
        } else {
            // Joining rows goes through the incumbent's `delete_newline`, which
            // does not fill the register. A pasted newline in place of the last
            // yanked word is a surprising thing to hand back.
            (motion::prev_cluster(text, to), Yank::Discard)
        };
        self.delete_between(from, to, yank)
    }

    pub fn delete_next_word(&mut self) -> bool {
        if self.take_selection() {
            return true;
        }
        let from = self.inner.cursor();
        let text = self.inner.text();
        // Mirror of `delete_word`: the end of the word at or after the cursor on
        // this row, else the row's end, else the line break after it. `word_end_
        // at_or_after` rather than `word_end_forward`, because deleting to the end
        // of a word must name the word the cursor is *in* — vim's `e` deliberately
        // looks past it.
        let candidate = motion::word_end_at_or_after(text, from, Words::Small);
        let row_end = motion::row_end(text, from);
        let (to, yank) = match candidate {
            Some(end) if end.row() == from.row() && end.byte() > from.byte() => (end, Yank::Keep),
            _ if from.byte() < row_end.byte() => (row_end, Yank::Keep),
            _ => (motion::next_cluster(text, from), Yank::Discard),
        };
        self.delete_between(from, to, yank)
    }

    pub fn cut(&mut self) -> bool {
        // Takes the anchor whether or not it spanned anything, like every other
        // operation that consumes a selection.
        let span = self.inner.selection().filter(|span| !span.is_empty());
        self.inner.clear_selection();
        let Some(span) = span else {
            return false;
        };
        self.yank = self
            .inner
            .text()
            .slice(span)
            .map(|text| text.to_string())
            .unwrap_or_default();
        self.goal = None;
        self.mutate(|txn| {
            txn.delete(span);
        })
    }

    /// Copying reads: it leaves the selection where it is, and an empty one leaves
    /// the register alone rather than emptying it.
    pub fn copy(&mut self) {
        if let Some(span) = self.inner.selection().filter(|span| !span.is_empty())
            && let Some(text) = self.inner.text().slice(span)
        {
            self.yank = text.to_string();
        }
    }

    pub fn paste(&mut self) -> bool {
        if self.yank.is_empty() {
            return false;
        }
        let text = std::mem::take(&mut self.yank);
        let changed = self.insert_str(&text);
        self.yank = text;
        changed
    }

    /// Deleting a selection as a *side effect* of typing or of a forward delete
    /// does not fill the register; an explicit delete does. That asymmetry is the
    /// incumbent's and vim's both: `d` fills the unnamed register, typing over a
    /// selection does not.
    /// Take the selection and delete it, reporting whether anything went.
    ///
    /// Taking it is unconditional: an empty selection is not a range, so the
    /// caller proceeds as though there were none — but the anchor is gone either
    /// way. Leaving it alive is how an invisible selection outlives the gesture
    /// that made it — a defect that cost two notes before it was found.
    fn take_selection(&mut self) -> bool {
        let span = self.inner.selection().filter(|span| !span.is_empty());
        self.inner.clear_selection();
        let Some(span) = span else {
            return false;
        };
        self.delete_between(span.start(), span.end(), Yank::Discard)
    }

    fn delete_between(&mut self, from: Position, to: Position, yank: Yank) -> bool {
        let Some(span) = self.inner.text().span(from, to) else {
            return false;
        };
        if span.is_empty() {
            return false;
        }
        if yank == Yank::Keep
            && let Some(text) = self.inner.text().slice(span)
        {
            self.yank = text.to_string();
        }
        self.goal = None;
        self.mutate(|txn| {
            txn.delete(span);
        })
    }

    // ── History ──────────────────────────────────────────────────────────────

    pub fn undo(&mut self) -> bool {
        let change = self.inner.undo();
        self.after_history(change)
    }

    pub fn redo(&mut self) -> bool {
        let change = self.inner.redo();
        self.after_history(change)
    }

    /// The engine restores the selection an entry began with, which the incumbent
    /// does not. Dropping it keeps undo behaving as it does today: a selection is
    /// painted, so putting one back is a visible change, and an engine swap is the
    /// wrong place to make one. The capability stays in the engine for when it is
    /// asked for on purpose.
    fn after_history(&mut self, change: Option<Change>) -> bool {
        if change.is_none() {
            // Nothing to undo: an operation that did not happen changes nothing,
            // the selection included.
            return false;
        }
        self.goal = None;
        self.inner.clear_selection();
        self.record(change)
    }

    // ── Cursor and selection ─────────────────────────────────────────────────

    /// Move the cursor, extending a live selection.
    ///
    /// Directional movement keeps the anchor deliberately: that is how vim's
    /// Visual mode extends.
    pub fn move_cursor(&mut self, movement: CursorMove) {
        let text = self.inner.text();
        let from = self.inner.cursor();
        let goal = self
            .goal
            .filter(|_| movement.is_vertical())
            .unwrap_or_else(|| from.column());

        let to = match movement {
            CursorMove::Forward => motion::next_cluster(text, from),
            CursorMove::Back => motion::prev_cluster(text, from),
            CursorMove::Up => motion::vertical(text, from, -1, Goal::Column(goal)),
            CursorMove::Down => motion::vertical(text, from, 1, Goal::Column(goal)),
            CursorMove::Head => motion::row_start(text, from),
            CursorMove::End => motion::row_end(text, from),
            // The first and last *row*, keeping the column — not the start and
            // end of the text, which is a different place on a non-empty row.
            CursorMove::Top => {
                let up = -(from.row() as isize);
                motion::vertical(text, from, up, Goal::Column(goal))
            }
            CursorMove::Bottom => {
                let down = (text.line_count().saturating_sub(1) as isize) - from.row() as isize;
                motion::vertical(text, from, down, Goal::Column(goal))
            }
            CursorMove::WordForward => motion::word_start_forward(text, from, Words::Small),
            CursorMove::WordBack => motion::word_start_back(text, from, Words::Small),
            CursorMove::WordForwardBig => motion::word_start_forward(text, from, Words::Big),
            CursorMove::WordBackBig => motion::word_start_back(text, from, Words::Big),
            // Inclusive, like `WordEnd`: a cursor landing on a word's end wants the
            // last cluster, not the place after it.
            CursorMove::WordEndBig => match motion::word_end_forward(text, from, Words::Big) {
                Some(end) => motion::prev_cluster(text, end),
                None => from,
            },
            CursorMove::WordEndBack { big } => {
                let words = if big { Words::Big } else { Words::Small };
                match motion::word_end_back(text, from, words) {
                    Some(end) => motion::prev_cluster(text, end),
                    None => from,
                }
            }
            CursorMove::MatchingPair => motion::matching_bracket(text, from).unwrap_or(from),
            // The crate's word end is exclusive — just past the last cluster —
            // because that is what an operator range wants. A *cursor* landing on
            // a word end wants the last cluster itself, as vim's `e` does. This is
            // the inclusive-to-half-open conversion CONTEXT names under **span
            // kind**, and the adapter is where it belongs: the engine holds no view
            // on which convention a caller uses.
            CursorMove::WordEnd => match motion::word_end_forward(text, from, Words::Small) {
                Some(end) => motion::prev_cluster(text, end),
                // Nothing ahead: the incumbent walks to the end of the text rather
                // than staying put, and vim's `e` on a trailing blank line does the
                // same.
                None => motion::text_end(text),
            },
            CursorMove::ParagraphForward => motion::paragraph_forward(text, from),
            CursorMove::ParagraphBack => motion::paragraph_back(text, from),
            CursorMove::Jump(row, column) => {
                match text.position(row, Column::new(column)) {
                    Some(position) => position,
                    // Refused, not clamped: a keypress that did nothing is
                    // recoverable in a way one that edited elsewhere is not.
                    None => return,
                }
            }
        };

        self.goal = if movement.is_vertical() {
            Some(goal)
        } else {
            None
        };
        self.place(to);
    }

    /// Move the cursor to `(row, col)`, refusing a position the buffer cannot
    /// address rather than landing somewhere else.
    pub fn jump_to(&mut self, row: usize, col: usize) -> bool {
        let Some(to) = self.inner.text().position(row, Column::new(col)) else {
            return false;
        };
        self.goal = None;
        self.place(to);
        true
    }

    /// Move to a position the caller worked out itself — a visual-line motion,
    /// which needs a layout the buffer does not have.
    pub fn move_to(&mut self, to: Position) {
        if self.inner.text().is_stale(to) {
            return;
        }
        self.goal = None;
        self.place(to);
    }

    fn place(&mut self, to: Position) {
        if self.inner.selection().is_some() {
            self.inner.extend_to(to);
        } else {
            self.inner.set_cursor(to);
        }
    }

    pub fn start_selection(&mut self) {
        // Anchors *here*, even when a selection is already live: starting one is
        // a fresh gesture, not an extension of the last.
        let cursor = self.inner.cursor();
        self.inner.clear_selection();
        self.inner.extend_to(cursor);
    }

    pub fn cancel_selection(&mut self) {
        self.inner.clear_selection();
    }

    pub fn select_all(&mut self) {
        let span = self.inner.text().full_span();
        self.inner.select(span);
    }

    pub fn set_selection(&mut self, start: (usize, usize), end: (usize, usize)) -> bool {
        let text = self.inner.text();
        let Some(from) = text.position(start.0, Column::new(start.1)) else {
            return false;
        };
        let Some(to) = text.position(end.0, Column::new(end.1)) else {
            return false;
        };
        let Some(span) = text.span(from, to) else {
            return false;
        };
        self.inner.select(span)
    }

    /// The text between two `(row, col)` pairs, in either order; `None` when the
    /// range is empty or names a position the buffer cannot address.
    pub fn text_between(&self, start: (usize, usize), end: (usize, usize)) -> Option<String> {
        let span = self
            .span_between(start, end)
            .filter(|span| !span.is_empty())?;
        self.inner.text().slice(span).map(|text| text.into_owned())
    }

    /// The selected text, or `None` when nothing — or nothing of width — is
    /// selected.
    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        self.text_between(start, end)
    }

    // ── Indent ───────────────────────────────────────────────────────────────

    /// Indent or dedent `rows` by one **indent step**, as one **undo group** —
    /// and one transaction.
    ///
    /// Indenting puts `indent_width` spaces at the start of every row — always
    /// spaces, never a tab. Dedenting takes up to one step of leading spaces,
    /// or the run up to and including a first tab, which counts as a whole
    /// step: `"  \tfoo"` loses both spaces and the tab and stops there.
    ///
    /// The cursor keeps the character it sat on — its column moves by its own
    /// row's change, so `>>` on the `n` of `one` leaves it on `n`, neovim's
    /// rule — and a selection is put back the same way, endpoint by endpoint.
    /// Both fall out of the engine's own mapping across the transaction, so the
    /// history entry holds the cursor's real before and after, and an undo or
    /// redo lands where the user was. Rows past the end are ignored. Reports
    /// whether any text changed; a dedent that finds nothing to remove changes
    /// nothing and says so.
    pub fn indent_rows(&mut self, rows: std::ops::RangeInclusive<usize>, dedent: bool) -> bool {
        let step = self.indent_width.get() as usize;
        let first = *rows.start();
        let last = (*rows.end()).min(self.row_count() - 1);
        if first > last {
            return false;
        }
        // Address every row's edit against the text as it stands; the
        // transaction carries each later one across the earlier ones.
        let text = self.inner.text();
        let mut edits: Vec<RowEdit> = Vec::with_capacity(last - first + 1);
        let mut deltas: Vec<isize> = Vec::with_capacity(last - first + 1);
        for row in first..=last {
            let at = text
                .position(row, Column::new(0))
                .expect("row is within the buffer");
            if dedent {
                let count = self.leading_step(row);
                if count > 0
                    && let Some(span) = text.span(at, self.forward_by(at, count))
                {
                    edits.push(RowEdit::Delete(span));
                }
                deltas.push(-(count as isize));
            } else {
                edits.push(RowEdit::Insert(at));
                deltas.push(step as isize);
            }
        }
        if edits.is_empty() {
            return false;
        }
        let selection = self.selection_range();
        let shifted = |(row, col): (usize, usize)| -> (usize, usize) {
            if row < first || row > last {
                return (row, col);
            }
            (row, col.saturating_add_signed(deltas[row - first]))
        };
        let spaces = " ".repeat(step);
        self.goal = None;
        self.mutate(|txn| {
            for edit in &edits {
                match *edit {
                    RowEdit::Insert(at) => {
                        txn.insert(at, &spaces);
                    }
                    RowEdit::Delete(span) => {
                        txn.delete(span);
                    }
                }
            }
            // An edit drops the anchor; a selection that was live goes back,
            // each endpoint shifted by its own row's change.
            if let Some((start, end)) = selection {
                let (start, end) = (shifted(start), shifted(end));
                let text = txn.text();
                if let Some(from) = text.position(start.0, Column::new(start.1))
                    && let Some(to) = text.position(end.0, Column::new(end.1))
                    && let Some(span) = text.span(from, to)
                {
                    txn.select(span);
                }
            }
        })
    }

    /// What one dedent removes from the front of `row`: up to a step of spaces,
    /// or the run up to and including a first tab. Counted in grapheme
    /// clusters — the unit the delete steps — so a space that carries a
    /// combining mark ends the run rather than being half-removed.
    fn leading_step(&self, row: usize) -> usize {
        use unicode_segmentation::UnicodeSegmentation;
        let step = self.indent_width.get() as usize;
        let Some(line) = self.row(row) else {
            return 0;
        };
        let mut count = 0;
        for cluster in line.graphemes(true).take(step) {
            match cluster {
                " " => count += 1,
                "\t" => return count + 1,
                _ => break,
            }
        }
        count
    }

    // ── Search ───────────────────────────────────────────────────────────────
    //
    // Not the engine's business: it holds the pattern because vim's
    // `n`/`N` outlive the find bar, and matches a row at a time because a **find
    // pattern** can never span a newline.

    pub fn set_search_pattern(&mut self, pattern: &str) -> Result<(), regex::Error> {
        if pattern.is_empty() {
            self.search = None;
            return Ok(());
        }
        self.search = Some(regex::Regex::new(pattern)?);
        Ok(())
    }

    /// Move the cursor to the next match. Never extends a selection.
    pub fn search_forward(&mut self, match_cursor: bool) -> bool {
        self.step_search(false, match_cursor)
    }

    /// Move the cursor to the previous match. Never extends a selection.
    pub fn search_back(&mut self, match_cursor: bool) -> bool {
        self.step_search(true, match_cursor)
    }

    /// Repeat the persisted pattern (vim `n` / `N`).
    pub fn search_repeat(&mut self, backward: bool) -> bool {
        self.step_search(backward, false)
    }

    fn step_search(&mut self, backward: bool, match_cursor: bool) -> bool {
        // A search is not a selection gesture, so the anchor goes first — and
        // here that is one call rather than an invariant to remember, because
        // `set_cursor` drops it and `extend_to` keeps it.
        self.cancel_selection();
        let Some(found) = self.find_match(backward, match_cursor) else {
            return false;
        };
        self.goal = None;
        self.inner.set_cursor(found);
        true
    }

    fn find_match(&self, backward: bool, match_cursor: bool) -> Option<Position> {
        let pattern = self.search.as_ref()?;
        let text = self.inner.text();
        let cursor = self.inner.cursor();
        let rows = text.line_count();

        // `0..=rows` visits the cursor's row twice: once at the start, and once
        // more at the end. That last visit IS the wrap, so the hits the first
        // visit stepped over — the ones behind the cursor — are exactly what it
        // is for. Filtering them again there is what made search unable to come
        // back around to them.
        for step in 0..=rows {
            let wrapped = step == rows;
            let row = if backward {
                (cursor.row() + rows - (step % rows.max(1))) % rows
            } else {
                (cursor.row() + step) % rows
            };
            let line = text.line(row)?;
            let mut hits: Vec<usize> = pattern
                .find_iter(&line)
                .map(|found| line[..found.start()].chars().count())
                .collect();
            if backward {
                hits.reverse();
            }
            for column in hits {
                let same_row = row == cursor.row();
                let beyond = if backward {
                    column < cursor.column().get()
                } else if match_cursor {
                    column >= cursor.column().get()
                } else {
                    column > cursor.column().get()
                };
                if wrapped || !same_row || beyond {
                    // A match can start inside a grapheme cluster — a regex like
                    // `.` or a search for a scalar that also appears inside a ZWJ
                    // sequence. That start is not addressable, so skip the
                    // candidate; abandoning the whole scan there would report "no
                    // match" while the bar's own count says otherwise.
                    if let Some(at) = text.position(row, Column::new(column)) {
                        return Some(at);
                    }
                }
            }
        }
        None
    }

    /// The span of the match starting exactly at the cursor, if any.
    pub fn match_at_cursor(&self) -> Option<((usize, usize), (usize, usize))> {
        let pattern = self.search.as_ref()?;
        let text = self.inner.text();
        let cursor = self.inner.cursor();
        let line = text.line(cursor.row())?;
        let byte = line
            .char_indices()
            .nth(cursor.column().get())
            .map(|(at, _)| at)
            .unwrap_or(line.len());
        let found = pattern.find_at(&line, byte)?;
        if found.start() != byte {
            return None;
        }
        let chars = line[found.range()].chars().count();
        Some((rc(cursor), (cursor.row(), cursor.column().get() + chars)))
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// `chars` scalars forward of `from`, clamped to the end of the text.
    fn forward_by(&self, from: Position, chars: usize) -> Position {
        let text = self.inner.text();
        let mut at = from;
        for _ in 0..chars {
            let next = motion::next_cluster(text, at);
            if next.byte() == at.byte() {
                break;
            }
            at = next;
        }
        at
    }

    /// The span between two `(row, col)` pairs, for callers that still speak in
    /// them.
    fn span_between(&self, start: (usize, usize), end: (usize, usize)) -> Option<Span> {
        let text = self.inner.text();
        let from = text.position(start.0, Column::new(start.1))?;
        let to = text.position(end.0, Column::new(end.1))?;
        text.span(from, to)
    }
}

fn rc(position: Position) -> (usize, usize) {
    (position.row(), position.column().get())
}

#[cfg(test)]
mod search_tests {
    use super::*;
    use crate::ropetext::Text;

    fn buffer(text: &str, pattern: &str, cursor: (usize, usize)) -> RopeBuffer {
        let mut buf = RopeBuffer::new(Text::from(text));
        buf.set_search_pattern(pattern).expect("valid pattern");
        buf.move_cursor(CursorMove::Jump(cursor.0, cursor.1));
        buf
    }

    #[test]
    fn a_forward_search_wraps_to_a_match_behind_the_cursor() {
        // One row, one match, cursor past it. Before the wrap visit stopped
        // re-filtering, this reported no match while the find bar counted one.
        let mut buf = buffer("xx foo", "foo", (0, 5));
        assert!(buf.search_forward(false), "the match is behind the cursor");
        assert_eq!(buf.cursor(), (0, 3));
    }

    #[test]
    fn a_backward_search_wraps_to_a_match_ahead_of_the_cursor() {
        let mut buf = buffer("xx foo", "foo", (0, 1));
        assert!(buf.search_back(false));
        assert_eq!(buf.cursor(), (0, 3));
    }

    #[test]
    fn wrapping_crosses_rows_back_to_the_cursors_own_row() {
        let mut buf = buffer("aaa\nxx foo", "foo", (1, 5));
        assert!(buf.search_forward(false));
        assert_eq!(buf.cursor(), (1, 3));
    }

    #[test]
    fn the_only_match_is_re_offered_rather_than_reported_missing() {
        // vim's answer: "search hit BOTTOM, continuing at TOP" lands back on the
        // same match. Reporting false would paint "no match" over a match that is
        // highlighted on screen.
        let mut buf = buffer("xx foo", "foo", (0, 3));
        assert!(buf.search_forward(false), "the one match is still a match");
        assert_eq!(
            buf.cursor(),
            (0, 3),
            "and the cursor has nowhere else to go"
        );
    }

    #[test]
    fn a_match_starting_inside_a_cluster_is_skipped_not_fatal() {
        // "\u{1F469}\u{200D}\u{1F4BB}" is one cluster; the laptop scalar sits at
        // char column 2, inside it. That start is unaddressable — but the real
        // match on row 1 is, and abandoning the scan at the first unaddressable
        // candidate is what made the bar say "no match" beside a count of two.
        let mut buf = buffer(
            "\u{1F469}\u{200D}\u{1F4BB}\nx\u{1F4BB}",
            "\u{1F4BB}",
            (0, 0),
        );
        assert!(buf.search_forward(false), "the row 1 match is reachable");
        assert_eq!(buf.cursor(), (1, 1));
    }
}

#[cfg(test)]
mod cluster_tests {
    use super::*;
    use crate::ropetext::Text;

    #[test]
    fn delete_str_spends_its_count_on_clusters() {
        // "[[" plus a regional-indicator flag: 4 scalars, 3 clusters. Three is
        // what removes exactly `[[` and the flag — a caller counting the four
        // scalars would take the space after them too.
        let mut buf = RopeBuffer::new(Text::from("[[\u{1F1EA}\u{1F1F8} rest"));
        buf.move_cursor(CursorMove::Jump(0, 0));
        buf.delete_str(3);
        assert_eq!(buf.rows(), &[" rest"]);
    }

    #[test]
    fn inserting_before_a_combining_mark_keeps_the_cursor_addressable() {
        // A row starting with a lone combining acute — NFD text pasted from
        // macOS. Typing 'a' in front of it makes "a\u{301}", one cluster, and
        // the post-edit cursor byte lands inside it.
        let mut buf = RopeBuffer::new(Text::from("\u{301}f"));
        buf.move_cursor(CursorMove::Jump(0, 0));
        buf.insert_char('a');
        assert_eq!(buf.rows(), &["a\u{301}f"]);
    }
}

#[cfg(test)]
mod damage_tests {
    use super::*;
    use crate::ropetext::Text;

    #[test]
    fn damage_from_several_edits_is_in_one_numbering() {
        // Two mutations in one group, the second ABOVE the first and changing
        // the line count — so the first edit's row moves before the group ends.
        let mut buf = RopeBuffer::new(Text::from("r0\nr1\nr2\nr3\nr4"));
        buf.edit(|b| {
            b.move_cursor(CursorMove::Jump(4, 0));
            b.insert_str("X");
            b.move_cursor(CursorMove::Jump(0, 0));
            b.insert_newline();
        });
        assert_eq!(buf.rows(), ["", "r0", "r1", "r2", "r3", "Xr4"]);

        let damage = buf.take_outcome().damage.expect("the edits were reported");
        assert!(
            damage.contains(&5),
            "the row edited first is row 5 once the group ends, but the damage \
             reported was {damage:?} — a range in the older numbering"
        );
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use crate::ropetext::Text;

    #[test]
    fn text_between_slices_by_char_columns_across_rows_in_either_order() {
        let buf = RopeBuffer::new(Text::from("héllo🦀\nworld"));
        assert_eq!(
            buf.text_between((0, 1), (1, 2)).as_deref(),
            Some("éllo🦀\nwo")
        );
        assert_eq!(
            buf.text_between((1, 2), (0, 1)).as_deref(),
            Some("éllo🦀\nwo")
        );
    }

    #[test]
    fn an_empty_or_unaddressable_range_is_none() {
        let buf = RopeBuffer::new(Text::from("abc"));
        assert_eq!(buf.text_between((0, 1), (0, 1)), None);
        assert_eq!(buf.text_between((0, 0), (7, 0)), None);
    }

    #[test]
    fn selection_text_is_the_live_selection_or_none() {
        let mut buf = RopeBuffer::new(Text::from("hello world"));
        assert_eq!(buf.selection_text(), None);
        assert!(buf.set_selection((0, 0), (0, 5)));
        assert_eq!(buf.selection_text().as_deref(), Some("hello"));
        buf.start_selection();
        assert_eq!(
            buf.selection_text(),
            None,
            "a zero-width selection is not text"
        );
    }
}

#[cfg(test)]
mod indent_tests {
    use super::*;
    use crate::ropetext::Text;

    fn buffer(text: &str) -> RopeBuffer {
        RopeBuffer::new(Text::from(text))
    }

    #[test]
    fn indent_inserts_one_step_of_spaces_never_a_tab() {
        let mut buf = buffer("foo\nbar");
        assert!(buf.indent_rows(0..=1, false));
        assert_eq!(buf.rows(), &["    foo", "    bar"]);
    }

    #[test]
    fn indent_follows_indent_width() {
        let mut buf = buffer("x");
        buf.set_indent_width(2);
        assert!(buf.indent_rows(0..=0, false));
        assert_eq!(buf.rows(), &["  x"]);
    }

    #[test]
    fn dedent_removes_up_to_one_step_of_spaces() {
        let mut buf = buffer("        x\n  y\nz");
        assert!(buf.indent_rows(0..=2, true));
        assert_eq!(buf.rows(), &["    x", "y", "z"]);
    }

    #[test]
    fn dedent_counts_a_leading_tab_as_a_whole_step() {
        let mut buf = buffer("\t\tx\n  \ty");
        assert!(buf.indent_rows(0..=1, true));
        // One tab is one step; the spaces before a tab go with it, and the
        // step ends there.
        assert_eq!(buf.rows(), &["\tx", "y"]);
    }

    #[test]
    fn dedent_stops_at_a_space_that_carries_a_combining_mark() {
        // "  \u{301}foo": the second space and the acute accent are one cluster.
        let mut buf = buffer("  \u{301}foo");
        assert!(buf.set_selection((0, 3), (0, 6)));
        assert!(buf.indent_rows(0..=0, true));
        assert_eq!(buf.rows(), &[" \u{301}foo"]);
        assert_eq!(buf.selection_range(), Some(((0, 2), (0, 5))));
    }

    #[test]
    fn dedent_with_nothing_to_remove_reports_no_change() {
        let mut buf = buffer("foo");
        assert!(!buf.indent_rows(0..=0, true));
        assert_eq!(buf.rows(), &["foo"]);
        assert!(!buf.take_outcome().changed);
    }

    #[test]
    fn rows_past_the_end_are_ignored() {
        let mut buf = buffer("a\nb");
        assert!(buf.indent_rows(1..=9, false));
        assert_eq!(buf.rows(), &["a", "    b"]);
        assert!(!buf.indent_rows(5..=9, false));
        assert_eq!(buf.rows(), &["a", "    b"]);
    }

    #[test]
    fn the_cursor_keeps_its_character() {
        let mut buf = buffer("one\ntwo");
        assert!(buf.jump_to(0, 1)); // on 'n'
        buf.indent_rows(0..=1, false);
        assert_eq!(buf.cursor(), (0, 1 + 4));
        buf.indent_rows(0..=1, true);
        assert_eq!(buf.cursor(), (0, 1));
    }

    #[test]
    fn a_cursor_outside_the_rows_does_not_move() {
        let mut buf = buffer("a\nb\nc");
        assert!(buf.jump_to(2, 1));
        buf.indent_rows(0..=1, false);
        assert_eq!(buf.cursor(), (2, 1));
    }

    #[test]
    fn a_dedent_never_pushes_the_cursor_below_column_zero() {
        let mut buf = buffer("    x");
        assert!(buf.jump_to(0, 2));
        buf.indent_rows(0..=0, true);
        assert_eq!(buf.cursor(), (0, 0));
    }

    #[test]
    fn the_selection_is_put_back_shifted_with_its_rows() {
        let mut buf = buffer("hello world\nnext");
        assert!(buf.set_selection((0, 6), (1, 2)));
        buf.indent_rows(0..=1, false);
        assert_eq!(buf.selection_range(), Some(((0, 10), (1, 6))));
        assert_eq!(buf.rows()[0].trim_start(), "hello world");
    }

    #[test]
    fn a_dedent_under_a_selection_shifts_each_endpoint_by_its_own_row() {
        let mut buf = buffer("    foo\n  bar\nbaz");
        assert!(buf.set_selection((0, 4), (2, 3)));
        assert!(buf.indent_rows(0..=2, true));
        assert_eq!(buf.rows(), &["foo", "bar", "baz"]);
        assert_eq!(buf.selection_range(), Some(((0, 0), (2, 3))));
    }

    #[test]
    fn a_block_indent_is_one_undo_group() {
        let mut buf = buffer("a\nb\nc");
        buf.indent_rows(0..=2, false);
        assert!(buf.undo(), "the block is one entry");
        assert_eq!(buf.rows(), &["a", "b", "c"]);
        assert!(!buf.undo(), "and has nothing left to take back");
    }

    #[test]
    fn undo_and_redo_of_an_indent_land_the_cursor_on_its_own_character() {
        let mut buf = buffer("one\ntwo");
        assert!(buf.jump_to(0, 1)); // on 'n'
        buf.indent_rows(0..=1, false);
        assert_eq!(buf.cursor(), (0, 5));
        assert!(buf.undo());
        assert_eq!(buf.cursor(), (0, 1), "undo returns to where the user was");
        assert!(buf.redo());
        assert_eq!(
            buf.cursor(),
            (0, 5),
            "redo lands where the indent left them"
        );
    }

    #[test]
    fn a_block_indent_is_one_transaction() {
        let mut buf = buffer("a\nb\nc");
        buf.indent_rows(0..=2, false);
        let outcome = buf.take_outcome();
        assert!(outcome.changed);
        assert!(outcome.bulk, "three rows in one change");
        assert!(buf.undo());
        assert_eq!(buf.rows(), &["a", "b", "c"]);
        assert!(!buf.undo());
    }

    #[test]
    fn a_dedent_does_not_fill_the_yank_transport() {
        let mut buf = buffer("    x");
        buf.set_yank_text("kept");
        buf.indent_rows(0..=0, true);
        assert_eq!(buf.yank_text(), "kept");
    }

    #[test]
    fn damage_covers_every_touched_row() {
        let mut buf = buffer("a\nb\nc\nd");
        buf.indent_rows(1..=2, false);
        let outcome = buf.take_outcome();
        assert!(outcome.changed);
        let damage = outcome.damage.expect("the edits were reported");
        assert!(damage.contains(&1) && damage.contains(&2), "{damage:?}");
        assert_eq!(outcome.line_delta, 0);
    }
}
