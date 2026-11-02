//! The **Edit surface**'s retained half: an entity that owns the logic
//! [`Editor`], its focus, and the host glue (the app channel, the redraw hook,
//! the scroll remainder, the IME preedit). All editing state and rules live in
//! the `Editor`; this file only routes GPUI input to it and hands the
//! [`EditorElement`] what it paints.

use std::ops::Range;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    App, Bounds, Context, EntityInputHandler, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, NoAction, ParentElement, Pixels, Point, Render, ScrollEvent, Styled, Task,
    UTF16Selection, Window, div,
};
use kimun_notes::logic::editor::Editor;
use kimun_notes::logic::editor::handled::Handled;
use kimun_notes::logic::input::{Button, PointerKind};
use kimun_notes::logic::message::{AppTx, redraw_callback};
use kimun_notes::settings::themes::Theme;

use crate::editor_element::EditorElement;
use crate::grid::CellMetrics;
use crate::input::{self, ScrollAccumulator};
use crate::theme;

/// The key context of the Edit surface.
pub const KEY_CONTEXT: &str = "KimunEditor";

/// Register the Edit surface's key bindings. Call once, after `gpui_kit::init`.
///
/// The kit's `Root` binds `tab` / `shift-tab` to focus traversal and Ctrl-C /
/// Cmd-C to copying its own text selection. Bindings run before key-down
/// listeners, so inside the editor those keys would never reach it (and the
/// kit's debug inspector claims Ctrl-Shift-I / Cmd-Alt-I everywhere). A
/// `NoAction` in the deeper editor context disables them there, and the keys
/// fall through to the editor (indent, dedent, copy) like any other key.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-c", NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-c", NoAction, Some(KEY_CONTEXT)),
        // gpui-component's inspector toggle (debug builds), global: in the
        // editor every Ctrl chord is the editor's (literal Ctrl, ADR-0049 §7).
        KeyBinding::new("ctrl-shift-i", NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-alt-i", NoAction, Some(KEY_CONTEXT)),
    ]);
}

/// Where the last frame put the grid, for input that arrives outside paint
/// (the IME asking where the caret is) and for tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Painted {
    /// Window position of cell (0, 0).
    pub origin: Point<Pixels>,
    pub cells: CellMetrics,
    /// The caret cell in window coordinates, when one is drawn.
    pub caret: Option<Bounds<Pixels>>,
}

/// One editor pane: the logic [`Editor`] plus what only a GPUI host needs.
pub struct EditorView {
    pub(crate) editor: Editor,
    pub(crate) theme: Theme,
    pub(crate) focus_handle: FocusHandle,
    /// The app channel the editor reports on (clipboard flashes) and the
    /// background parse/layout wake-up rides on.
    tx: AppTx,
    scroll: ScrollAccumulator,
    /// A left press in this editor is being dragged. Drags that began
    /// elsewhere are not ours to turn into a selection.
    dragging: bool,
    /// IME composition text, shown at the caret but kept out of the buffer
    /// until committed: a preedit is not an edit, so it must not dirty the
    /// note or enter undo history.
    pub(crate) preedit: Option<String>,
    pub(crate) painted: Option<Painted>,
    _messages: Task<()>,
}

impl EditorView {
    /// A plain (non-vim) editor showing `text`, or an empty buffer.
    pub fn new(text: Option<String>, theme: Theme, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut editor = Editor::new(false);
        // Background parse/layout jobs of a large buffer call this on
        // completion; it lands as a message below and redraws, so the rich
        // parse appears without waiting for a keystroke.
        editor.session_mut().notifier = Some(redraw_callback(tx.clone()));
        if let Some(text) = text {
            editor.set_text(text);
        }
        // tokio's channels need no tokio executor to be awaited, so GPUI's
        // foreground executor can drain this one.
        let messages = cx.spawn(async move |this, cx| {
            while let Some(message) = rx.recv().await {
                // M1 has no footer to flash "copied" / "pasted" or a clipboard
                // error in, so a message's content is dropped. Each one still
                // redraws: a finished background job sends `Redraw`.
                drop(message);
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            editor,
            theme,
            focus_handle: cx.focus_handle(),
            tx,
            scroll: ScrollAccumulator::default(),
            dragging: false,
            preedit: None,
            painted: None,
            _messages: messages,
        }
    }

    /// After any input the editor handled: drop the requests M1 does not serve
    /// and redraw. M1 has no context menu, find bar or palette, and nothing
    /// (no popup) is anchored to the cursor for `ViewportScrolled` to move.
    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        drop(self.editor.take_requests());
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = input::key_down_input(event) {
            // A dead key or compose sequence finishes as a plain key-down
            // (`´` preedit, then `é` as a keystroke): whatever was being
            // composed is over once a key acts.
            self.preedit = None;
            let (handled, _) = self.editor.handle_key(key, &self.tx);
            self.settle(window, cx);
            // A consumed key must not also reach the platform's input handler,
            // or typed text would be inserted twice.
            if handled == Handled::Consumed {
                cx.stop_propagation();
            }
            return;
        }
        // Multi-char text (a grapheme cluster, an emoji sequence) propagates so
        // the platform hands it to the input handler; a bare modifier is not
        // an action at all.
        if input::carries_text(event) || event.keystroke.is_modifier_key() {
            return;
        }
        // A key with no editing meaning still ends the typing run (TUI parity).
        self.editor.ignore_unmapped_key();
        self.settle(window, cx);
    }

    pub(crate) fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(pointer) = input::pointer_down(event, origin, cells) else {
            return;
        };
        self.editor.handle_pointer(&pointer, &self.tx);
        let left = pointer.kind == PointerKind::Down(Button::Left);
        self.editor.settle_pointer(left);
        self.dragging = left;
        self.settle(window, cx);
    }

    pub(crate) fn pointer_move(
        &mut self,
        event: &MouseMoveEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging {
            return;
        }
        // The release happened where we could not see it (outside the window).
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = false;
            return;
        }
        let Some(pointer) = input::pointer_move(event, origin, cells) else {
            return;
        };
        self.editor.handle_pointer(&pointer, &self.tx);
        self.editor.settle_pointer(false);
        self.settle(window, cx);
    }

    pub(crate) fn pointer_up(
        &mut self,
        event: &MouseUpEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging || event.button != MouseButton::Left {
            return;
        }
        self.dragging = false;
        let Some(pointer) = input::pointer_up(event, origin, cells) else {
            return;
        };
        self.editor.handle_pointer(&pointer, &self.tx);
        self.editor.settle_pointer(false);
        self.settle(window, cx);
    }

    pub(crate) fn scroll_wheel(
        &mut self,
        event: &ScrollEvent,
        origin: Point<Pixels>,
        cells: &CellMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let steps = input::pointer_scroll(event, origin, cells, &mut self.scroll);
        if steps.is_empty() {
            return;
        }
        for step in &steps {
            self.editor.handle_pointer(step, &self.tx);
        }
        self.editor.settle_pointer(false);
        self.settle(window, cx);
    }

    /// The input handler's whole "document": the cursor row, with the preedit
    /// spliced in at the caret. The IME only ever needs the text around the
    /// caret, and a row-local document keeps every query O(row) instead of
    /// O(buffer). The preedit is in the document but not in the buffer.
    fn document(&self) -> Document {
        let (row, caret) = self.editor.cursor();
        let buffer_line = self
            .editor
            .buffer()
            .row(row)
            .map(|r| r.into_owned())
            .unwrap_or_default();
        Document::new(row, buffer_line, caret, self.preedit.as_deref())
    }

    /// Commit `text`: over `range` of the document when given, else at the
    /// cursor.
    ///
    /// Composition rule: a preedit never touches the buffer. A selection made
    /// before composing stays as it is while composing; *cancelling* (empty
    /// text over the preedit, or an empty preedit update) only drops the
    /// preedit, so nothing the user selected is ever lost to an abandoned
    /// composition; *committing* text acts exactly like typing it, replacing
    /// that selection.
    fn commit(&mut self, range: Option<Range<usize>>, text: &str) {
        let doc = self.document();
        let marked = doc.marked();
        let had_preedit = self.preedit.take().is_some();
        let over_preedit = had_preedit && (range.is_none() || range == marked);
        if over_preedit && text.is_empty() {
            return;
        }
        let target = range.filter(|r| Some(r) != marked.as_ref()).map(|r| {
            (
                doc.row,
                doc.to_buffer(r.start)..doc.to_buffer(r.end.max(r.start)),
            )
        });
        self.editor.commit_text(target, text);
    }
}

/// The cursor row as the input handler sees it (see [`EditorView::document`]).
struct Document {
    row: usize,
    /// The row with the preedit spliced in at the caret.
    line: String,
    /// UTF-16 offset of the caret (= the preedit's start) in `line`.
    caret: usize,
    /// The caret's char column in the buffer row.
    caret_col: usize,
    /// UTF-16 length of the preedit, 0 without one.
    preedit_len: usize,
}

impl Document {
    fn new(row: usize, buffer_line: String, caret_col: usize, preedit: Option<&str>) -> Self {
        let byte = buffer_line
            .char_indices()
            .nth(caret_col)
            .map_or(buffer_line.len(), |(i, _)| i);
        let preedit = preedit.unwrap_or("");
        let mut line = buffer_line;
        line.insert_str(byte, preedit);
        let caret = char_to_utf16(&line, caret_col);
        Self {
            row,
            line,
            caret,
            caret_col,
            preedit_len: preedit.encode_utf16().count(),
        }
    }

    /// Where the preedit sits, when there is one.
    fn marked(&self) -> Option<Range<usize>> {
        (self.preedit_len > 0).then(|| self.caret..self.caret + self.preedit_len)
    }

    /// A document UTF-16 offset as a buffer char column. Offsets inside the
    /// preedit land on the caret: none of it is in the buffer.
    fn to_buffer(&self, utf16: usize) -> usize {
        if utf16 <= self.caret {
            utf16_to_char(&self.line, utf16)
        } else if utf16 >= self.caret + self.preedit_len {
            utf16_to_char(&self.line, utf16) - self.preedit_chars()
        } else {
            self.caret_col
        }
    }

    fn preedit_chars(&self) -> usize {
        let end = utf16_to_char(&self.line, self.caret + self.preedit_len);
        end - self.caret_col
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("editor")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            // A press in the inset around the grid still focuses the editor.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                }),
            )
            .size_full()
            .px_4()
            .py_2()
            .bg(theme::background(&self.theme))
            .child(EditorElement::new(cx.entity()))
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let line = self.document().line;
        let (a, b) = (
            utf16_to_char(&line, range.start),
            utf16_to_char(&line, range.end.max(range.start)),
        );
        *adjusted_range = Some(char_to_utf16(&line, a)..char_to_utf16(&line, b));
        Some(line.chars().skip(a).take(b - a).collect())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let doc = self.document();
        // While composing the caret is drawn after the preedit.
        if let Some(marked) = doc.marked() {
            return Some(UTF16Selection {
                range: marked.end..marked.end,
                reversed: false,
            });
        }
        let (row, line) = (doc.row, &doc.line);
        let col = doc.caret_col;
        let caret = doc.caret;
        let selection = self
            .editor
            .selection()
            .filter(|((sr, _), (er, _))| *sr == row && *er == row);
        Some(match selection {
            Some(((_, sc), (_, ec))) => UTF16Selection {
                range: char_to_utf16(line, sc)..char_to_utf16(line, ec),
                reversed: col == sc && sc != ec,
            },
            None => UTF16Selection {
                range: caret..caret,
                reversed: false,
            },
        })
    }

    /// The preedit's place in the document. It is not in the buffer, but the
    /// platform must see it: Linux cancels a composition by deleting the
    /// marked range, and macOS reads "composing" from its presence (so Enter
    /// commits rather than inserting a newline).
    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.document().marked()
    }

    /// Ending composition keeps what was composed (GPUI's contract): with the
    /// preedit outside the buffer, that means committing it now.
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.preedit.take() {
            self.commit(None, &text);
            self.settle(window, cx);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit(range, text);
        self.settle(window, cx);
    }

    /// A new or updated preedit. An active selection is left alone (see the
    /// composition rule on `commit`). A `range` other than the current preedit
    /// is buffer text the platform explicitly hands to the composition (macOS
    /// reconversion): it is removed and the preedit starts where it was.
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let doc = self.document();
        if let Some(r) = range.filter(|r| Some(r) != doc.marked().as_ref()) {
            let cols = doc.to_buffer(r.start)..doc.to_buffer(r.end.max(r.start));
            self.editor.commit_text(Some((doc.row, cols)), "");
        }
        self.preedit = (!new_text.is_empty()).then(|| new_text.to_string());
        self.settle(window, cx);
    }

    /// Where the IME should put its candidate window: the caret cell.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.painted.and_then(|p| p.caret)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// The char index `utf16` code units into `s`, clamped to its end. An offset
/// inside a surrogate pair rounds down to the char that starts it.
fn utf16_to_char(s: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in s.chars().enumerate() {
        let next = units + c.len_utf16();
        if next > utf16 {
            return i;
        }
        units = next;
    }
    s.chars().count()
}

/// UTF-16 code units in the first `chars` chars of `s`, clamped to its end.
fn char_to_utf16(s: &str, chars: usize) -> usize {
    s.chars().take(chars).map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_round_trip_through_chars() {
        // 'a' (1 unit), 'é' (1), '😀' (2), 'b' (1).
        let s = "aé😀b";
        assert_eq!(char_to_utf16(s, 0), 0);
        assert_eq!(char_to_utf16(s, 3), 4);
        assert_eq!(char_to_utf16(s, 99), 5);
        assert_eq!(utf16_to_char(s, 4), 3);
        assert_eq!(utf16_to_char(s, 2), 2);
        // Inside the surrogate pair: the emoji's own index.
        assert_eq!(utf16_to_char(s, 3), 2);
        assert_eq!(utf16_to_char(s, 99), 4);
    }
}
