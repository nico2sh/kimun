//! How read-only text answers the pointer: a press places the caret, a
//! drag selects, a double press selects a word, a click on a follow
//! target follows at its release, and the copy chord copies. This module
//! does not own key input.

use std::ops::Range;
use std::time::Instant;

use gpui_kit::{ClipboardItem, Context, Pixels, Point, Window};
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::input::{Mods, PointerInput};

use super::pointer::DOUBLE_CLICK;
use super::{EditorEvent, EditorView};
use crate::keymap;

/// How far, in pixels, the pointer may move from a press on a follow target
/// in read-only text and still click it. A longer drag selects text and
/// follows nothing, even inside the target.
const CLICK_SLOP: f32 = 4.0;

/// A read-only press on a follow target, waiting for its release.
#[derive(Debug)]
pub(super) struct PressedTarget {
    /// The pressed cell, and when.
    pub(super) col: u16,
    pub(super) row: u16,
    pub(super) at: Instant,
    /// The buffer row the target is on, and the target. The pointer stays
    /// on the target while the caret it drags stays on this row inside a
    /// span of this target.
    pub(super) buffer_row: usize,
    pub(super) target: FollowTarget,
}

/// The click state of a read-only surface: the press that waits for its
/// release, and the last click that followed.
#[derive(Debug, Default)]
pub(super) struct ReadOnlyClicks {
    /// A plain press on a follow target, until the release. A move that
    /// leaves the target's span (a drag) drops it.
    pub(super) pressed: Option<PressedTarget>,
    /// The last click that followed: its cell and when the last press of
    /// that run came. A press on the same cell within [`DOUBLE_CLICK`] of it
    /// is a plain press: no word, no second follow.
    followed: Option<(u16, u16, Instant)>,
}

impl ReadOnlyClicks {
    /// Forget the press and the last follow. New text has other targets.
    pub(super) fn reset(&mut self) {
        self.pressed = None;
        self.followed = None;
    }

    /// Whether a press on (`col`, `row`) at `now` comes right after a click
    /// that followed, on the same cell, within [`DOUBLE_CLICK`]. For such a
    /// press the run goes on from `now`. For any other press it ends.
    pub(super) fn press_after_follow(&mut self, col: u16, row: u16, now: Instant) -> bool {
        let after_follow = self.followed.is_some_and(|(c, r, at)| {
            (c, r) == (col, row) && now.saturating_duration_since(at) < DOUBLE_CLICK
        });
        self.followed = after_follow.then_some((col, row, now));
        after_follow
    }

    /// Record a click that followed, on (`col`, `row`) pressed at `at`.
    pub(super) fn followed_at(&mut self, col: u16, row: u16, at: Instant) {
        self.followed = Some((col, row, at));
    }
}

impl EditorView {
    /// A press in read-only text. A left press places the caret, may start
    /// a drag selection and may arm a click on a follow target. A right
    /// press does nothing.
    pub(super) fn read_only_press(
        &mut self,
        pointer: PointerInput,
        left: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Read-only text: a left press places the caret and may start a
        // drag selection, and a double press on text selects a word. A
        // plain press on a follow target arms a click: its release on the
        // same target follows ([`Self::pointer_up`]), unless the pointer
        // moved more than [`CLICK_SLOP`] pixels. A primary press on
        // a target follows at once. Presses right after a click that
        // followed are plain presses. Never a follow on the channel, a
        // menu or an edit; a right press does nothing.
        self.clicks_ro.pressed = None;
        if left {
            let now = (self.clock)();
            let after_follow = self
                .clicks_ro
                .press_after_follow(pointer.col, pointer.row, now);
            if after_follow {
                self.clicks.end();
                self.feed_pointer(&pointer, true);
                self.dragging = true;
                self.settle(window, cx);
                return;
            }
            let primary = pointer.mods == keymap::primary(cx);
            let double = pointer.mods == Mods::NONE
                && self
                    .clicks
                    .completes_double(pointer.col, pointer.row, (self.clock)());
            if pointer.mods != Mods::NONE {
                self.clicks.end();
            }
            self.feed_pointer(&pointer, true);
            let target = self.editor.follow_target_at_cursor();
            if primary && let Some(target) = target {
                self.clicks_ro.followed_at(pointer.col, pointer.row, now);
                cx.emit(EditorEvent::ReadOnlyFollow { target });
            } else if pointer.mods == Mods::NONE && !double {
                let buffer_row = self.editor.buffer().cursor().0;
                self.clicks_ro.pressed = target.map(|target| PressedTarget {
                    col: pointer.col,
                    row: pointer.row,
                    at: now,
                    buffer_row,
                    target,
                });
            }
            if double {
                self.select_word_at_caret();
            }
            // A double press is done; any other press stays armed so the
            // release (or a drag) reaches the editor.
            self.dragging = !double;
            self.settle(window, cx);
        }
    }

    /// Whether `at` lies more than [`CLICK_SLOP`] pixels from the press.
    pub(super) fn moved_past_slop(&self, at: Point<Pixels>) -> bool {
        let dx = f32::from(at.x - self.pressed_at.x);
        let dy = f32::from(at.y - self.pressed_at.y);
        dx.hypot(dy) > CLICK_SLOP
    }

    /// Whether the pointer is still on the pressed follow target (true with
    /// none pressed).
    pub(super) fn on_pressed_target(&self) -> bool {
        self.clicks_ro
            .pressed
            .as_ref()
            .is_none_or(|pressed| self.caret_on(pressed))
    }

    /// Whether the caret, which the pointer drags, sits inside a span of
    /// `pressed`'s target on its row: a wobble across a cell edge inside
    /// the tag keeps it there.
    pub(super) fn caret_on(&self, pressed: &PressedTarget) -> bool {
        self.editor.buffer().cursor().0 == pressed.buffer_row
            && self.editor.follow_target_at_cursor().as_ref() == Some(&pressed.target)
    }

    /// Select the word under the caret (a run of letters, digits and `_`;
    /// on anything else, the run of that kind of char). Nothing on an empty
    /// row.
    fn select_word_at_caret(&mut self) {
        let (row, col) = self.editor.buffer().cursor();
        let line = self.editor.buffer().row(row).unwrap_or_default();
        if let Some(cols) = word_at(&line, col) {
            self.editor
                .buffer_mut()
                .set_selection((row, cols.start), (row, cols.end));
            self.editor.sync_highlight();
        }
    }

    /// A read-only surface without the keyboard drops its selection: the copy
    /// chord only reaches the surface that has the keyboard, so a selection
    /// left drawn elsewhere could not be copied. The caret stays where the
    /// selection ended and the text is untouched. Nothing while a drag is
    /// still selecting (its release checks again), nor in an editor that
    /// edits.
    pub(super) fn drop_unreachable_selection(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.read_only
            || self.dragging
            || self.focus_handle.contains_focused(window, cx)
            || self.editor.selection().is_none()
        {
            return;
        }
        self.editor.buffer_mut().cancel_selection();
        self.editor.sync_highlight();
        cx.notify();
    }

    /// Put the selected text on the clipboard (a read-only surface's copy
    /// chord). Nothing when nothing is selected.
    pub(super) fn copy_selection(&self, cx: &mut Context<Self>) {
        if let Some(text) = self.editor.buffer().selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

/// The char columns of the word at char column `col` of `line`: the run of
/// word chars (letters, digits, `_`) there, else the run of whitespace or of
/// other chars. A column past the end counts as the last char. `None` on an
/// empty line.
fn word_at(line: &str, col: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = line.chars().collect();
    let at = col.min(chars.len().checked_sub(1)?);
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let kind = class(chars[at]);
    let start = chars[..at]
        .iter()
        .rposition(|&c| class(c) != kind)
        .map_or(0, |i| i + 1);
    let end = chars[at..]
        .iter()
        .position(|&c| class(c) != kind)
        .map_or(chars.len(), |i| at + i);
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_a_run_of_one_kind_of_char() {
        assert_eq!(word_at("a #idea b", 4), Some(3..7));
        assert_eq!(word_at("a #idea b", 2), Some(2..3), "the `#` alone");
        assert_eq!(word_at("a  b", 1), Some(1..3), "the spaces");
        assert_eq!(word_at("snake_case.rs", 3), Some(0..10));
        assert_eq!(word_at("été ok", 1), Some(0..3));
        assert_eq!(word_at("end", 9), Some(0..3), "past the end: the last word");
        assert_eq!(word_at("", 0), None);
    }
}
