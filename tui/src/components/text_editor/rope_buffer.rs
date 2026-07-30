//! The **edit buffer** over `ropetext`, presenting the surface the rest of the
//! editor already calls (adr/0039).
//!
//! This exists so the engine swap does not have to happen everywhere at once.
//! It keeps every method name and `(row, col)` signature the `TextArea`-backed
//! buffer had, so `vim.rs`, `find_bar.rs` and the component compile against it
//! unchanged, and `tests/rope_buffer_differential.rs` holds it to the incumbent's
//! behaviour operation by operation.
//!
//! Nothing here mirrors the text into a second representation. Callers that want
//! rows ask for them ([`RopeBuffer::rows`]) and pay for them there; the buffer
//! keeps one copy of the note and no derived copy in step with it.
//!
use ropetext::motion::{self, Goal, Words};
use ropetext::{Change, Column, EditBuffer as Rope, Position, Span, Text};

/// Visual columns per tab stop when `hard_tab_indent` is off.
const DEFAULT_TAB_LENGTH: u8 = 4;

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
    /// parser's cursor damage hint would under-report it (adr/0035).
    pub bulk: bool,
    /// Which rows the edits changed, in the new text's numbering — the hull when
    /// several ran before this was drained.
    ///
    /// Told by the engine rather than found by comparing the buffer with a copy
    /// of its previous self, which is what ADR-0040 exists to make possible. The
    /// **nvim** backend reports lines and not changes, so it leaves this `None`
    /// and its consumer falls back to a diff.
    pub damage: Option<std::ops::Range<usize>>,
}

impl EditOutcome {
    pub(super) fn unchanged() -> Self {
        Self::default()
    }
}

/// Whether a delete fills the register it removed text from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Yank {
    Keep,
    Discard,
}

/// A cursor movement, in the vocabulary the editor already speaks.
///
/// Deliberately the incumbent's variant set, so the 145 call sites need no
/// rewriting — but `Jump` takes `usize` rather than `u16`, because clamping a
/// row to 65535 is the defect adr/0038 recorded and this is the type where it
/// stops being representable.
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
    /// The column a vertical movement is aiming at, which is why walking down
    /// through a short row and out the other side returns to where it started.
    goal: Option<Column>,
    yank: String,
    search: Option<regex::Regex>,
    tab_length: u8,
    hard_tab_indent: bool,
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
            goal: None,
            yank: String::new(),
            search: None,
            tab_length: DEFAULT_TAB_LENGTH,
            hard_tab_indent: false,
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

    pub fn tab_length(&self) -> u8 {
        self.tab_length
    }

    pub fn set_tab_length(&mut self, columns: u8) {
        self.tab_length = columns;
    }

    pub fn hard_tab_indent(&self) -> bool {
        self.hard_tab_indent
    }

    pub fn set_hard_tab_indent(&mut self, hard: bool) {
        self.hard_tab_indent = hard;
    }

    pub fn snapshot(&self) -> ropetext::Snapshot {
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

    /// Apply one primitive as its own group, or as part of an open one.
    fn mutate(&mut self, f: impl FnOnce(&mut ropetext::Txn<'_>)) -> bool {
        let extending = self.depth > 0 && self.group_started;
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
        self.pending.damage = Some(match self.pending.damage.take() {
            Some(seen) => seen.start.min(change.rows().start)..seen.end.max(change.rows().end),
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

    pub fn insert_tab(&mut self) -> bool {
        if self.hard_tab_indent {
            return self.insert_str("\t");
        }
        let width = self.tab_length.max(1) as usize;
        let column = self.inner.cursor().column().get();
        let fill = width - (column % width);
        self.insert_str(" ".repeat(fill))
    }

    /// Delete `chars` scalars forward, a line break counting as one.
    pub fn delete_str(&mut self, chars: usize) -> bool {
        if self.take_selection() {
            return true;
        }
        if chars == 0 {
            return false;
        }
        let from = self.inner.cursor();
        let to = self.forward_by(from, chars);
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
    /// that made it, which adr/0038 records costing two notes.
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

    /// Apply a key the explicit shortcut table did not claim.
    ///
    /// Only the text-entry keys: everything navigational or modified is mapped
    /// before this is reached. Replaces the incumbent's `input_without_shortcuts`,
    /// which also quietly bound Ctrl+U and Ctrl+R to undo and redo — the fourth
    /// surprise in adr/0038, and the reason a key path into the engine is something
    /// this states rather than inherits. Step 7 turns this into the **plain**
    /// backend's full table; it is here so the swap has somewhere to land.
    pub fn apply_plain_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> bool {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
            return false;
        }
        match key.code {
            KeyCode::Char(c) => {
                self.insert_char(c);
                true
            }
            KeyCode::Backspace => self.delete_char(),
            KeyCode::Delete => self.delete_next_char(),
            KeyCode::Tab => self.insert_tab(),
            KeyCode::Enter => {
                self.insert_newline();
                true
            }
            _ => false,
        }
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
        self.inner.select(span);
        true
    }

    // ── Search ───────────────────────────────────────────────────────────────
    //
    // Not the engine's business (adr/0041): it holds the pattern because vim's
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

        for step in 0..=rows {
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
                if !same_row || beyond {
                    return text.position(row, Column::new(column));
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
    pub fn span_between(&self, start: (usize, usize), end: (usize, usize)) -> Option<Span> {
        let text = self.inner.text();
        let from = text.position(start.0, Column::new(start.1))?;
        let to = text.position(end.0, Column::new(end.1))?;
        text.span(from, to)
    }
}

fn rc(position: Position) -> (usize, usize) {
    (position.row(), position.column().get())
}
