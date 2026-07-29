//! **Undo groups**: making one user action cost one undo.
//!
//! `TextArea::insert_str` is `delete_selection` + `insert`, so a single
//! **replace** lands in history as two entries. Undoing once would leave the
//! match deleted and the replacement not yet inserted — a note with a hole in
//! it, which is indistinguishable from data loss for the user who reached for
//! undo because they thought something went wrong.
//!
//! The library exposes no transaction API (`push_history` is private), so
//! grouping happens here: a grouped action records the buffer content on both
//! sides of itself, and an undo that *starts* from the recorded after-state
//! pops the whole group instead of half of it.
//!
//! Identifying groups by content rather than by position is what makes this
//! survive intervening edits. Type after a replace, undo those keystrokes one
//! by one, and when the buffer arrives back at the replace's after-state the
//! group is recognised again and the next undo takes all of it.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Cap on tracked groups. Undo history is bounded by the textarea anyway; this
/// just stops a long session from accumulating stale entries for states the
/// history can no longer reach.
const MAX_GROUPS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Group {
    /// Buffer content hash before the grouped action ran.
    before: u64,
    /// Buffer content hash after it ran.
    after: u64,
    /// History entries beyond the first that belong to this action.
    extra: usize,
}

fn hash_lines(lines: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    lines.hash(&mut h);
    h.finish()
}

/// Tracks which buffer states are the seams of a multi-entry action.
#[derive(Debug, Default)]
pub struct UndoGrouper {
    undo: Vec<Group>,
    redo: Vec<Group>,
}

impl UndoGrouper {
    /// Record that an action taking `1 + extra` history entries moved the
    /// buffer from `before` to `after`.
    ///
    /// Recording a new action invalidates the redo side, exactly as the
    /// textarea's own history does.
    pub fn record(&mut self, before: &[String], after: &[String], extra: usize) {
        if extra == 0 {
            return;
        }
        self.redo.clear();
        if self.undo.len() == MAX_GROUPS {
            self.undo.remove(0);
        }
        self.undo.push(Group {
            before: hash_lines(before),
            after: hash_lines(after),
            extra,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    /// How many *extra* pops an undo from `current` needs, without consuming
    /// the group. For callers that must ask before the first pop happens
    /// elsewhere (the vim engine undoes inside its own command apply).
    pub fn peek_undo_extra(&self, current: &[String]) -> usize {
        let h = hash_lines(current);
        match self.undo.last() {
            Some(g) if g.after == h => g.extra,
            _ => 0,
        }
    }

    /// As [`Self::peek_undo_extra`], and move the group to the redo side.
    pub fn take_undo_extra(&mut self, current: &[String]) -> usize {
        let h = hash_lines(current);
        match self.undo.last() {
            Some(g) if g.after == h => {
                let g = self.undo.pop().expect("checked above");
                self.redo.push(g);
                g.extra
            }
            _ => 0,
        }
    }

    pub fn peek_redo_extra(&self, current: &[String]) -> usize {
        let h = hash_lines(current);
        match self.redo.last() {
            Some(g) if g.before == h => g.extra,
            _ => 0,
        }
    }

    pub fn take_redo_extra(&mut self, current: &[String]) -> usize {
        let h = hash_lines(current);
        match self.redo.last() {
            Some(g) if g.before == h => {
                let g = self.redo.pop().expect("checked above");
                self.undo.push(g);
                g.extra
            }
            _ => 0,
        }
    }

    /// Drop everything — the buffer was replaced wholesale (note switched,
    /// backend swapped), so recorded states no longer describe this history.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn undo_from_the_after_state_takes_the_whole_group() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["todo"]), &l(&["done"]), 1);
        assert_eq!(g.take_undo_extra(&l(&["done"])), 1);
    }

    #[test]
    fn undo_from_an_unrelated_state_takes_one_entry() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["todo"]), &l(&["done"]), 1);
        // The user typed after the replace, so this undo is undoing that
        // typing, not the replace.
        assert_eq!(g.take_undo_extra(&l(&["done x"])), 0);
    }

    #[test]
    fn the_group_is_recognised_again_once_the_buffer_returns_to_it() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["todo"]), &l(&["done"]), 1);
        assert_eq!(g.take_undo_extra(&l(&["done x"])), 0); // undoing the typing
        assert_eq!(g.take_undo_extra(&l(&["done"])), 1); // now the replace
    }

    #[test]
    fn peek_does_not_consume() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["a"]), &l(&["b"]), 1);
        assert_eq!(g.peek_undo_extra(&l(&["b"])), 1);
        assert_eq!(g.peek_undo_extra(&l(&["b"])), 1);
        assert_eq!(g.take_undo_extra(&l(&["b"])), 1);
        assert_eq!(g.peek_undo_extra(&l(&["b"])), 0);
    }

    #[test]
    fn redo_regroups_from_the_before_state() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["todo"]), &l(&["done"]), 1);
        assert_eq!(g.take_undo_extra(&l(&["done"])), 1);
        assert_eq!(g.take_redo_extra(&l(&["todo"])), 1);
        // And it is back on the undo side.
        assert_eq!(g.take_undo_extra(&l(&["done"])), 1);
    }

    #[test]
    fn a_new_group_invalidates_redo() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["a"]), &l(&["b"]), 1);
        g.take_undo_extra(&l(&["b"]));
        g.record(&l(&["a"]), &l(&["c"]), 1);
        assert_eq!(g.peek_redo_extra(&l(&["a"])), 0);
    }

    #[test]
    fn single_entry_actions_are_not_tracked() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["a"]), &l(&["b"]), 0);
        assert!(g.is_empty());
    }

    #[test]
    fn the_stack_is_bounded() {
        let mut g = UndoGrouper::default();
        for i in 0..(MAX_GROUPS + 5) {
            g.record(&l(&[&format!("{i}")]), &l(&[&format!("{}", i + 1)]), 1);
        }
        assert_eq!(g.undo.len(), MAX_GROUPS);
    }

    #[test]
    fn clear_drops_both_sides() {
        let mut g = UndoGrouper::default();
        g.record(&l(&["a"]), &l(&["b"]), 1);
        g.take_undo_extra(&l(&["b"]));
        g.clear();
        assert!(g.is_empty());
    }
}
