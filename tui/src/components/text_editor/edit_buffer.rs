//! The **edit buffer**: the open note's text and its edit history as one
//! module (adr/0037).
//!
//! Every mutation on the textarea and vim backends passes through [`EditBuffer::edit`].
//! Because the buffer sees both sides of each edit, the facts that follow from
//! one — did the content change, was the damage local, which history entries
//! belong together — are *derived* here rather than predicted by each caller.
//!
//! Predicting them is what the previous design did, and each prediction was
//! wrong in a different way: a zero-width match pushes one history entry rather
//! than two, `insert_str` returns `false` after deleting when the replacement is
//! empty, and a hand-placed revision bump can simply be forgotten.
//!
//! The nvim backend has no edit buffer — neovim owns its own buffer and history.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use ratatui_textarea::{CursorMove, DataCursor, TextArea};

/// Cap on how many history entries one grouped undo may replay.
///
/// Must exceed the largest group any caller can build, or that group becomes
/// un-undoable: `indent_lines` pushes one entry per selected row, so a 15-line
/// indent is 15 entries. Sized above `TextArea`'s own 50-entry history, beyond
/// which the group's start is evicted and genuinely unreachable anyway.
const MAX_GROUP_REPLAY: usize = 64;

/// Cap on tracked groups. `TextArea` keeps 50 history entries; tracking more
/// group boundaries than that cannot help.
const MAX_GROUPS: usize = 64;

/// The buffer states one grouped edit ran between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Group {
    before: u64,
    after: u64,
}

fn hash_lines(lines: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    lines.hash(&mut h);
    h.finish()
}

/// What one call to [`EditBuffer::edit`] did, measured rather than predicted.
///
/// `#[must_use]` on purpose: the caller still applies these (the revision clock
/// serves both backends and so stays on the component), and forgetting to is
/// exactly the failure this type exists to prevent. A warning is a check; a
/// convention is not.
#[must_use = "an edit's outcome drives the revision bump and the parse-damage signal"]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditOutcome {
    /// The buffer's text differs from before the edit. A content comparison —
    /// never a library return value, which can report `false` after mutating.
    pub changed: bool,
    /// The change is not confined to the cursor's row, so the incremental
    /// parser's cursor damage hint would under-report it (adr/0035).
    pub bulk: bool,
}

impl EditOutcome {
    fn unchanged() -> Self {
        Self::default()
    }
}

/// The open note's text plus its edit history.
///
/// Reads reach the inner `TextArea` through [`Deref`]. The handful of `&mut`
/// operations that push no history are delegated. `&mut TextArea` escapes only
/// through [`Self::edit`] — see adr/0037 for why that enforcement is worth the
/// delegation.
#[derive(Debug)]
pub struct EditBuffer {
    ta: TextArea<'static>,
    /// Groups on the undo side, oldest first.
    undo: Vec<Group>,
    /// Groups moved across by an undo, awaiting a redo.
    redo: Vec<Group>,
    /// Outcomes accumulated since the last [`EditBuffer::take_outcome`]. The
    /// vim engine edits without caring what follows; the host drains this once
    /// per dispatched key and applies it. One drain site replaces the 22
    /// hand-placed revision bumps.
    pending: EditOutcome,
    /// Nesting depth of [`EditBuffer::edit`]. Only the outermost call records a
    /// group, so a compound action built from the single-mutation helpers below
    /// is still one undo.
    depth: u32,
}

impl Default for EditBuffer {
    fn default() -> Self {
        Self::new(TextArea::default())
    }
}

impl Deref for EditBuffer {
    type Target = TextArea<'static>;
    fn deref(&self) -> &Self::Target {
        &self.ta
    }
}

impl EditBuffer {
    pub fn new(ta: TextArea<'static>) -> Self {
        Self {
            ta,
            undo: Vec::new(),
            redo: Vec::new(),
            pending: EditOutcome::unchanged(),
            depth: 0,
        }
    }

    /// Replace the whole buffer. Groups recorded against the old history
    /// describe states the new one cannot reach, so they are dropped.
    pub fn replace(&mut self, ta: TextArea<'static>) {
        self.ta = ta;
        self.undo.clear();
        self.redo.clear();
        self.pending = EditOutcome::unchanged();
    }

    /// Take the outcome of every edit since the last drain.
    ///
    /// The host calls this once per dispatched key. Because the buffer measures
    /// rather than the caller predicting, "did this change the note?" cannot be
    /// answered wrongly by a library return value.
    pub fn take_outcome(&mut self) -> EditOutcome {
        std::mem::replace(&mut self.pending, EditOutcome::unchanged())
    }

    fn merge(&mut self, out: EditOutcome) {
        self.pending.changed |= out.changed;
        self.pending.bulk |= out.bulk;
    }

    /// The one door to `&mut TextArea`.
    ///
    /// Records the buffer states either side of `f` so a later undo can replay
    /// back to the start of it, however many history entries `f` happened to
    /// push. Nested calls are not expected; each call is one group.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut TextArea<'static>) -> R) -> R {
        // Nested calls belong to the outermost group: a compound action built
        // from the single-mutation helpers must still be one undo.
        if self.depth > 0 {
            return f(&mut self.ta);
        }
        let before: Vec<String> = self.ta.lines().to_vec();
        let cursor_row_before = self.ta.cursor().0;
        let out = {
            // Restore the nesting depth even if `f` unwinds: a leaked depth
            // would send every later edit down the nested path, which records
            // no group and merges no outcome — the note would then be edited
            // without ever being marked dirty.
            struct DepthGuard<'a>(&'a mut u32);
            impl Drop for DepthGuard<'_> {
                fn drop(&mut self) {
                    *self.0 -= 1;
                }
            }
            let Self { ta, depth, .. } = self;
            *depth += 1;
            let _guard = DepthGuard(depth);
            f(ta)
        };
        let after = self.ta.lines();

        if before.as_slice() == after {
            // Content is unchanged — but the closure may still have pushed
            // history (a cut plus an identical re-insert, e.g. pasting a
            // selection over itself). Orphaned entries have no group, so the
            // next undo pops half of one and lands on a state the user never
            // saw. Pop them back off; if the buffer does not return to where it
            // started, we were popping someone else's entries, so put them back.
            let mut popped = 0;
            while popped < MAX_GROUP_REPLAY && self.ta.undo() {
                popped += 1;
                if self.ta.lines() == before.as_slice() {
                    break;
                }
            }
            if self.ta.lines() != before.as_slice() {
                for _ in 0..popped {
                    self.ta.redo();
                }
            }
            return out;
        }

        let bulk = is_bulk(&before, after, cursor_row_before);
        let group = Group {
            before: hash_lines(&before),
            after: hash_lines(after),
        };
        // A new edit invalidates the redo side, exactly as the textarea's own
        // history does.
        self.redo.clear();
        if self.undo.len() == MAX_GROUPS {
            self.undo.remove(0);
        }
        self.undo.push(group);
        self.merge(EditOutcome {
            changed: true,
            bulk,
        });
        out
    }

    // ── Single-mutation helpers ──────────────────────────────────────────────
    //
    // Each is one **undo group**, which is the correct default: one mutation is
    // one user action unless a caller says otherwise with `edit()`. Having them
    // here rather than at 74 call sites is also what keeps `edit()` the only
    // route to `&mut TextArea`.

    pub fn insert_str(&mut self, s: impl AsRef<str>) -> bool {
        self.edit(|ta| ta.insert_str(s))
    }

    pub fn insert_char(&mut self, c: char) {
        self.edit(|ta| ta.insert_char(c));
    }

    pub fn insert_newline(&mut self) {
        self.edit(|ta| ta.insert_newline());
    }

    pub fn insert_tab(&mut self) -> bool {
        self.edit(|ta| ta.insert_tab())
    }

    pub fn delete_str(&mut self, chars: usize) -> bool {
        self.edit(|ta| ta.delete_str(chars))
    }

    pub fn delete_char(&mut self) -> bool {
        self.edit(|ta| ta.delete_char())
    }

    pub fn delete_next_char(&mut self) -> bool {
        self.edit(|ta| ta.delete_next_char())
    }

    pub fn delete_word(&mut self) -> bool {
        self.edit(|ta| ta.delete_word())
    }

    pub fn delete_next_word(&mut self) -> bool {
        self.edit(|ta| ta.delete_next_word())
    }

    pub fn cut(&mut self) -> bool {
        self.edit(|ta| ta.cut())
    }

    pub fn paste(&mut self) -> bool {
        self.edit(|ta| ta.paste())
    }

    pub fn input_without_shortcuts(&mut self, input: impl Into<ratatui_textarea::Input>) -> bool {
        self.edit(|ta| ta.input_without_shortcuts(input))
    }

    /// Undo one *user action*, replaying history until the buffer reaches the
    /// state that action started from.
    ///
    /// Nothing here counts entries. The extent is the group's own endpoints, so
    /// an operation that pushed one entry and one that pushed two are undone by
    /// the same code.
    pub fn undo(&mut self) -> bool {
        let before: Vec<String> = self.ta.lines().to_vec();
        // Peek only: moving the group across before the undo is known to
        // succeed would corrupt both stacks when history has been exhausted.
        let target = match self.undo.last() {
            Some(g) if g.after == hash_lines(&before) => Some(g.before),
            // Not at a group boundary: an ordinary single-entry undo.
            _ => None,
        };
        if !self.ta.undo() {
            return false;
        }
        if target.is_some() {
            let g = self.undo.pop().expect("peeked above");
            self.redo.push(g);
        }
        if let Some(target) = target {
            for _ in 0..MAX_GROUP_REPLAY {
                if hash_lines(self.ta.lines()) == target {
                    break;
                }
                if !self.ta.undo() {
                    break;
                }
            }
        }
        let out = self.outcome_against(&before);
        self.merge(out);
        out.changed
    }

    /// Redo one *user action*. Mirror of [`Self::undo`].
    pub fn redo(&mut self) -> bool {
        let before: Vec<String> = self.ta.lines().to_vec();
        let target = match self.redo.last() {
            Some(g) if g.before == hash_lines(&before) => Some(g.after),
            _ => None,
        };
        if !self.ta.redo() {
            return false;
        }
        if target.is_some() {
            let g = self.redo.pop().expect("peeked above");
            self.undo.push(g);
        }
        if let Some(target) = target {
            for _ in 0..MAX_GROUP_REPLAY {
                if hash_lines(self.ta.lines()) == target {
                    break;
                }
                if !self.ta.redo() {
                    break;
                }
            }
        }
        let out = self.outcome_against(&before);
        self.merge(out);
        out.changed
    }

    fn outcome_against(&self, before: &[String]) -> EditOutcome {
        let after = self.ta.lines();
        if before == after {
            return EditOutcome::unchanged();
        }
        EditOutcome {
            changed: true,
            // An undo can restore a whole-buffer rewrite in one step, which the
            // cursor damage hint cannot describe.
            bulk: is_bulk(before, after, self.ta.cursor().0),
        }
    }

    // ── The non-history `&mut` operations ────────────────────────────────────
    //
    // Delegated rather than reached through `edit()`: none of them pushes a
    // history entry, and wrapping the 145 `move_cursor` calls would record a
    // group for every motion. Delegating is also what keeps `edit()` the only
    // route to `&mut TextArea` (adr/0037).

    pub fn move_cursor(&mut self, m: CursorMove) {
        self.ta.move_cursor(m);
    }

    pub fn start_selection(&mut self) {
        self.ta.start_selection();
    }

    pub fn cancel_selection(&mut self) {
        self.ta.cancel_selection();
    }

    pub fn select_all(&mut self) {
        self.ta.select_all();
    }

    pub fn set_search_pattern(&mut self, pattern: &str) -> Result<(), regex::Error> {
        self.ta.set_search_pattern(pattern)
    }

    pub fn search_forward(&mut self, match_cursor: bool) -> bool {
        self.ta.search_forward(match_cursor)
    }

    pub fn search_back(&mut self, match_cursor: bool) -> bool {
        self.ta.search_back(match_cursor)
    }

    /// The span of the match starting exactly at the cursor, if any.
    ///
    /// Lives here rather than on the find bar because it reads only buffer
    /// state — the persisted pattern and the cursor — and is needed with the
    /// bar closed (vim `n`/`N`) as well as open.
    pub fn match_at_cursor(&self) -> Option<((usize, usize), (usize, usize))> {
        let re = self.ta.search_pattern()?;
        let DataCursor(row, col_chars) = self.ta.cursor();
        let line = self.ta.lines().get(row)?;
        let byte_off = super::char_col_to_byte(line, col_chars);
        let m = re.find_at(line, byte_off)?;
        // `find_at` finds the next match at OR AFTER the offset; only one
        // starting exactly here is the match under the cursor.
        if m.start() != byte_off {
            return None;
        }
        let match_chars = line[m.range()].chars().count();
        Some(((row, col_chars), (row, col_chars + match_chars)))
    }

    /// Repeat the buffer's persisted search pattern (vim `n` / `N`).
    ///
    /// A buffer operation, not a find-bar one: the pattern outlives the bar,
    /// which is the whole point of `n`/`N` working once the bar is closed.
    /// Pushes no history — it only moves the cursor.
    pub fn search_repeat(&mut self, backward: bool) -> bool {
        if backward {
            self.ta.search_back(false)
        } else {
            self.ta.search_forward(false)
        }
    }

    /// Feed a mouse event to the textarea: cursor placement and drag
    /// selection. Pushes no history, so it stays outside [`Self::edit`] — one
    /// scroll tick would otherwise cost a full buffer clone and two hashes.
    pub fn mouse_input(&mut self, input: impl Into<ratatui_textarea::Input>) -> bool {
        self.ta.input(input)
    }

    pub fn copy(&mut self) {
        self.ta.copy();
    }
}

/// Whether the change between `before` and `after` reaches beyond the cursor's
/// row.
///
/// This is the question `compute_damage_range`'s fast path assumes the answer
/// to: it trusts the cursor row to be the only edited row, and under-reports
/// otherwise. Deriving it here is what stops callers from hand-placing the
/// signal and occasionally forgetting.
fn is_bulk(before: &[String], after: &[String], cursor_row: usize) -> bool {
    if before.len() != after.len() {
        return true;
    }
    before
        .iter()
        .zip(after.iter())
        .enumerate()
        .any(|(row, (a, b))| a != b && row != cursor_row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(lines: &[&str]) -> EditBuffer {
        EditBuffer::new(TextArea::from(lines.iter().map(|s| s.to_string())))
    }

    fn lines(b: &EditBuffer) -> Vec<String> {
        b.lines().to_vec()
    }

    // ── grouping by extent ──────────────────────────────────────────────────

    /// The case the old count-based design got wrong: an empty replacement
    /// means `insert_str` deletes the selection and still returns `false`, so
    /// the operation is ONE history entry, not two.
    #[test]
    fn an_edit_that_only_deletes_is_undone_in_one_step() {
        let mut b = buf(&["todo and todo"]);
        b.edit(|ta| {
            ta.move_cursor(CursorMove::Jump(0, 0));
            ta.start_selection();
            ta.move_cursor(CursorMove::Jump(0, 5));
            ta.insert_str("")
        });
        assert!(
            b.take_outcome().changed,
            "the buffer was modified even though insert_str reports false for \
             an empty insert"
        );
        assert_eq!(lines(&b), vec!["and todo".to_string()]);
        b.undo();
        assert_eq!(lines(&b), vec!["todo and todo".to_string()]);
    }

    /// And the two-entry case, undone by the same code path.
    #[test]
    fn a_delete_plus_insert_is_undone_in_one_step() {
        let mut b = buf(&["todo"]);
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str("done")
        });
        assert!(b.take_outcome().changed);
        assert_eq!(lines(&b), vec!["done".to_string()]);
        b.undo();
        assert_eq!(
            lines(&b),
            vec!["todo".to_string()],
            "one undo must reach the state the edit started from, not half of it"
        );
    }

    /// A zero-width match pushes one entry. Nothing has to know that.
    #[test]
    fn a_zero_width_edit_needs_no_special_case() {
        let mut b = buf(&["ab"]);
        b.edit(|ta| {
            ta.move_cursor(CursorMove::Jump(0, 0));
            ta.insert_str("|")
        });
        assert_eq!(lines(&b), vec!["|ab".to_string()]);
        b.undo();
        assert_eq!(lines(&b), vec!["ab".to_string()]);
    }

    #[test]
    fn redo_restores_the_whole_group() {
        let mut b = buf(&["todo"]);
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str("done")
        });
        b.undo();
        let _ = b.take_outcome();
        assert_eq!(lines(&b), vec!["todo".to_string()]);
        assert!(b.redo(), "redo must move");
        assert!(b.take_outcome().changed);
        assert_eq!(lines(&b), vec!["done".to_string()]);
    }

    #[test]
    fn an_intervening_edit_does_not_disturb_the_group() {
        let mut b = buf(&["todo"]);
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str("done")
        });
        b.edit(|ta| {
            ta.move_cursor(CursorMove::End);
            ta.insert_char('!')
        });
        b.undo(); // the '!'
        assert_eq!(lines(&b), vec!["done".to_string()]);
        b.undo(); // the whole replace
        assert_eq!(lines(&b), vec!["todo".to_string()]);
    }

    #[test]
    fn undo_on_an_empty_history_reports_no_change() {
        let mut b = buf(&["todo"]);
        assert!(!b.undo(), "nothing to undo");
        assert_eq!(b.take_outcome(), EditOutcome::unchanged());
        assert_eq!(lines(&b), vec!["todo".to_string()]);
    }

    // ── derived outcomes ────────────────────────────────────────────────────

    #[test]
    fn a_cursor_only_edit_reports_no_change() {
        let mut b = buf(&["todo"]);
        b.edit(|ta| ta.move_cursor(CursorMove::End));
        assert_eq!(b.take_outcome(), EditOutcome::unchanged());
    }

    #[test]
    fn an_edit_on_the_cursor_row_is_not_bulk() {
        let mut b = buf(&["aaa", "bbb", "ccc"]);
        b.move_cursor(CursorMove::Jump(1, 0));
        b.insert_char('x');
        let out = b.take_outcome();
        assert!(out.changed);
        assert!(
            !out.bulk,
            "a keystroke is exactly what the damage hint assumes"
        );
    }

    #[test]
    fn an_edit_reaching_past_the_cursor_row_is_bulk() {
        let mut b = buf(&["todo", "x", "todo"]);
        b.move_cursor(CursorMove::Jump(2, 0));
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str("done\nx\ndone")
        });
        assert!(
            b.take_outcome().bulk,
            "rows the cursor does not point at also changed"
        );
    }

    #[test]
    fn a_line_count_change_is_bulk() {
        let mut b = buf(&["a"]);
        b.insert_newline();
        assert!(b.take_outcome().bulk);
    }

    #[test]
    fn undoing_a_whole_buffer_rewrite_reports_bulk() {
        let mut b = buf(&["todo", "x", "todo"]);
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str("done\nx\ndone")
        });
        let _ = b.take_outcome();
        assert!(b.undo());
        let out = b.take_outcome();
        assert!(out.changed);
        assert!(
            out.bulk,
            "an undo can restore a rewrite the cursor hint cannot describe"
        );
    }

    // ── lifecycle ───────────────────────────────────────────────────────────

    /// A closure that nets zero can still have pushed history — cutting a
    /// selection and re-inserting the same text. Those entries have no group,
    /// so before this was handled one undo popped half of one and destroyed
    /// text the user never asked to lose.
    #[test]
    fn a_net_zero_edit_leaves_no_orphaned_history() {
        let mut b = buf(&["hello foo"]);
        b.edit(|ta| {
            ta.move_cursor(CursorMove::Jump(0, 6));
            ta.start_selection();
            ta.move_cursor(CursorMove::End);
            ta.cut();
            ta.insert_str("foo")
        });
        assert_eq!(lines(&b), vec!["hello foo".to_string()]);
        assert_eq!(b.take_outcome(), EditOutcome::unchanged());
        assert!(!b.undo(), "there is nothing left to undo");
        assert_eq!(lines(&b), vec!["hello foo".to_string()]);
    }

    /// A group larger than a handful of entries must still undo in one step —
    /// indenting N lines pushes one entry per row.
    #[test]
    fn a_large_group_undoes_in_one_step() {
        let mut b = buf(&["l0"]);
        let rows: Vec<String> = (0..15).map(|i| format!("line {i}")).collect();
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str(rows.join("\n"))
        });
        let _ = b.take_outcome();
        b.edit(|ta| {
            for row in 0..15u16 {
                ta.move_cursor(CursorMove::Jump(row, 0));
                ta.insert_str("    ");
            }
        });
        assert!(b.lines()[14].starts_with("    "));
        assert!(b.undo());
        assert_eq!(
            b.lines(),
            rows.as_slice(),
            "one undo must revert all 15 rows, not stall part-way"
        );
    }

    /// A failed undo must not move a group across: history exhaustion would
    /// otherwise leave the stacks describing a state that never happens.
    #[test]
    fn an_exhausted_undo_leaves_the_group_stacks_alone() {
        let mut b = buf(&["a"]);
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str("b")
        });
        let _ = b.take_outcome();
        assert!(b.undo());
        let _ = b.take_outcome();
        assert!(!b.undo(), "history is exhausted");
        // The group went across on the first undo and must not come back.
        assert!(b.redo());
        assert_eq!(lines(&b), vec!["b".to_string()]);
    }

    #[test]
    fn replacing_the_buffer_drops_its_groups() {
        let mut b = buf(&["todo"]);
        b.edit(|ta| {
            ta.select_all();
            ta.insert_str("done")
        });
        b.replace(TextArea::from(["fresh".to_string()]));
        assert!(!b.undo(), "the new history has nothing to undo");
        assert_eq!(lines(&b), vec!["fresh".to_string()]);
    }

    #[test]
    fn the_group_stack_is_bounded() {
        let mut b = buf(&["x"]);
        for i in 0..(MAX_GROUPS + 5) {
            b.edit(|ta| {
                ta.select_all();
                ta.insert_str(format!("v{i}"))
            });
        }
        assert_eq!(b.undo.len(), MAX_GROUPS);
    }
}
