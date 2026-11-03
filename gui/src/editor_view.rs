//! The **Edit surface**'s retained half: an entity that owns the logic
//! [`Editor`], its focus, and the host glue (the app channel, the redraw hook,
//! the scroll remainder, the IME preedit). All editing state and rules live in
//! the `Editor`, vim's modes included; this file only routes GPUI input to it
//! and hands the [`EditorElement`] what it paints.
//!
//! **Vim and text input.** Under vim a keystroke outside Insert is a command,
//! so text must never reach the buffer around the engine: every key-down goes
//! through `Editor::handle_key`, and the platform input handler (IME
//! composition, multi-char text, press-and-hold accents) accepts text only
//! where the editor types, i.e. plain mode and vim Insert
//! ([`EditorView::accepts_text`]).
//!
//! **The find bar.** While the editor's find bar is open it owns the
//! keyboard, as in the TUI: every key goes to `Editor::handle_find_key`
//! (the keymap's find and replace chords still open / advance it first), and
//! text input (IME, multi-char text) lands in the bar's focused field, never in
//! the buffer.
//!
//! **Following links.** A single press only places the caret. A double-click
//! (ADR-0043: two plain left presses on one cell within [`DOUBLE_CLICK`])
//! on a link or a label asks the window's owner to follow it, on the app
//! channel; the `FollowLink` chord bubbles to the owner like any app command.
//!
//! **The keymap.** Every key-down is first resolved against the GUI keymap
//! ([`crate::keymap`]) with the editor focused. Find and replace are the
//! editor's own; any other app command is left to bubble to whoever owns it,
//! and never reaches the editor. Under vim the keymap hands bare Ctrl+letter
//! chords to the editor (the vim rule, ADR-0050 §6).

use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    App, Bounds, Context, EntityInputHandler, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, NoAction, ParentElement, Pixels, Point, Render, ScrollEvent, SharedString,
    Styled, Task, TextSystem, UTF16Selection, Window, div,
};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::editor::Editor;
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::editor::handled::Handled;
use kimun_notes::logic::editor::request::EditorRequest;
use kimun_notes::logic::input::{Button, Key, KeyInput, Mods, PointerKind};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::message::{AppMessage, AppTx, redraw_callback};
use kimun_notes::settings::themes::Theme;

use crate::editor_element::{EditorElement, ShapedRows};
use crate::find_bar::{self, BarPaint};
use crate::fonts::EditorFonts;
use crate::grid::CellMetrics;
use crate::input::{self, ScrollAccumulator};
use crate::keymap::{self, KeyCtx};
use crate::row_paint::RowPaint;
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
        // editor a Ctrl chord is the editor's or the GUI keymap's (ADR-0050 §6).
        KeyBinding::new("ctrl-shift-i", NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-alt-i", NoAction, Some(KEY_CONTEXT)),
    ]);
}

/// What the keymap made of a key-down, from the editor's side.
enum AppCommand {
    /// Not bound to an app command here: the editor's key.
    None,
    /// An editor command (find, replace), done.
    Handled,
    /// Another owner's command (focus, the drawer): not the editor's.
    NotOurs,
}

/// How the caret is drawn. Mirrors the TUI's terminal cursor shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretShape {
    /// A thin bar before the cell: where typing inserts (plain mode, vim
    /// Insert).
    Bar,
    /// The whole cell inverted, its glyph still readable: vim Normal, Visual
    /// and every other non-Insert mode, where the cursor sits *on* a char.
    Block,
}

/// What the mode line shows: vim's mode on the leading edge, the command
/// typed so far (a count, an operator: `2d`) on the trailing edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeLine {
    pub label: String,
    pub hint: Option<String>,
}

/// Where the last frame put the grid, for input that arrives outside paint
/// (the IME asking where the caret is) and for tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Painted {
    /// Window position of cell (0, 0).
    pub origin: Point<Pixels>,
    pub cells: CellMetrics,
    /// Text rows of the grid; the mode line, when shown, is the row below.
    pub rows: u16,
    /// The caret cell in window coordinates, when one is drawn.
    pub caret: Option<Bounds<Pixels>>,
    /// The caret's shape, when one is drawn.
    pub caret_shape: Option<CaretShape>,
    /// The mode line, when one is drawn (vim only).
    pub mode_line: Option<ModeLine>,
    /// What each text row painted, top to bottom.
    pub row_paint: Vec<Rc<RowPaint>>,
    /// The face it was painted in.
    pub fonts: EditorFonts,
    /// Where a block caret's repainted glyph starts, when one is drawn.
    pub caret_glyph: Option<Point<Pixels>>,
    /// The find bar, when open: docked in the rows below the text (above the
    /// mode line). While it is open `caret` is the bar's.
    pub find_bar: Option<BarPaint>,
}

/// How close together two presses on a cell must be to follow (ADR-0043;
/// the TUI's `DOUBLE_CLICK`).
pub(crate) const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The presses forming one double-click (ADR-0043): two plain left presses on
/// the same cell within [`DOUBLE_CLICK`]. Not a neighbourhood, so two clicks
/// on adjacent chars never follow; a fired double starts over, so a third
/// press does not pair with the second. Holds no clock: the caller passes
/// `now`. GPUI's own `click_count` is not used: its timing and slop are the
/// platform's, not this rule.
#[derive(Debug, Default)]
pub(crate) struct ClickRun {
    last: Option<(u16, u16, Instant)>,
}

impl ClickRun {
    /// Whether a press on (`col`, `row`) at `now` completes a double-click,
    /// recording it either way.
    pub(crate) fn completes_double(&mut self, col: u16, row: u16, now: Instant) -> bool {
        let doubled = self.last.is_some_and(|(c, r, at)| {
            c == col && r == row && now.saturating_duration_since(at) < DOUBLE_CLICK
        });
        self.last = (!doubled).then_some((col, row, now));
        doubled
    }

    /// The next press starts a new gesture: a key, a scroll, another
    /// button, a modifier held, a press off the text.
    pub(crate) fn end(&mut self) {
        self.last = None;
    }
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
    /// The presses that may form a double-click (follow the link there).
    clicks: ClickRun,
    /// IME composition text, shown at the caret but kept out of the buffer
    /// until committed: a preedit is not an edit, so it must not dirty the
    /// note or enter undo history.
    pub(crate) preedit: Option<String>,
    /// The editor takes no input (keys, text, composition): its owner is
    /// loading another note into it, which would drop whatever was typed.
    /// App chords still resolve.
    pub(crate) input_blocked: bool,
    pub(crate) painted: Option<Painted>,
    /// The face, chosen for the kit theme's monospace family (re-chosen if
    /// that family changes).
    fonts: Option<(SharedString, EditorFonts)>,
    /// Rows painted in recent frames, reused while their content is unchanged.
    pub(crate) row_cache: ShapedRows,
    /// Drains the editor's own channel, when it has one (the tests'
    /// stand-alone `EditorView::new`).
    _messages: Option<Task<()>>,
}

impl EditorView {
    /// A stand-alone editor showing `text`, or an empty buffer: vim's when
    /// `vim`, plain direct typing otherwise. It drains its own app channel
    /// and only redraws on a message (no window owner to dispatch to). The
    /// rendering tests' editor; the app's is [`Self::on_channel`].
    #[cfg(test)]
    pub fn new(text: Option<String>, theme: Theme, vim: bool, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        // tokio's channels need no tokio executor to be awaited, so GPUI's
        // foreground executor can drain this one.
        let messages = cx.spawn(async move |this, cx| {
            while let Some(message) = rx.recv().await {
                // Nothing here shows a flash, so a message's content is
                // dropped. Each one still redraws: a finished background job
                // sends `Redraw`.
                drop(message);
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self::build(text, theme, vim, tx, Some(messages), cx)
    }

    /// An editor reporting on the window's app channel `tx`, which its owner
    /// (the [`Workspace`](crate::workspace::Workspace)) drains and
    /// dispatches: redraws and clipboard flashes. The note session loads
    /// notes into its [`Editor`] (`pub(crate) editor`) through `NoteBuffer`.
    pub fn on_channel(theme: Theme, vim: bool, tx: AppTx, cx: &mut Context<Self>) -> Self {
        Self::build(None, theme, vim, tx, None, cx)
    }

    fn build(
        text: Option<String>,
        theme: Theme,
        vim: bool,
        tx: AppTx,
        messages: Option<Task<()>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = Editor::new(vim);
        // Background parse/layout jobs of a large buffer call this on
        // completion; it lands as a `Redraw` on the channel and redraws, so
        // the rich parse appears without waiting for a keystroke.
        editor.session_mut().notifier = Some(redraw_callback(tx.clone()));
        if let Some(text) = text {
            editor.set_text(text);
        }
        Self {
            editor,
            theme,
            focus_handle: cx.focus_handle(),
            tx,
            scroll: ScrollAccumulator::default(),
            dragging: false,
            clicks: ClickRun::default(),
            preedit: None,
            input_blocked: false,
            painted: None,
            fonts: None,
            row_cache: ShapedRows::default(),
            _messages: messages,
        }
    }

    /// After any input the editor handled: serve its requests (the find bar)
    /// or drop the ones nothing serves yet, and redraw.
    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for request in self.editor.take_requests() {
            match request {
                // Vim `/` and `?`: open the find bar (or step, when open).
                EditorRequest::OpenSearch => self.open_find(),
                // Vim `n` / `N`: repeat the last search, bar open or not.
                EditorRequest::SearchNext => self.editor.search_repeat(false),
                EditorRequest::SearchPrev => self.editor.search_repeat(true),
                // Vim `:` opens the command palette: M6.
                EditorRequest::OpenPalette => {}
                // Right-click with no selection opens the note menu: M6.
                EditorRequest::ContextMenu => {}
                // Nothing (no popup) is anchored to the cursor to move yet.
                EditorRequest::ViewportScrolled => {}
            }
        }
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    /// Whether text input (IME, multi-char text) may land in the buffer: only
    /// where the editor types, plain mode or vim Insert. Elsewhere vim reads a
    /// keystroke as a command, which must reach it through `handle_key`.
    ///
    /// An open find bar takes text in any mode: it lands in its focused field.
    pub(crate) fn accepts_text(&self) -> bool {
        self.editor.find_open() || self.editor.modal_is_insert() != Some(false)
    }

    /// Open the find bar, or step to the next match when it is open
    /// (Ctrl-F, vim `/`). A composition in progress belonged to the buffer.
    fn open_find(&mut self) {
        if self.editor.open_find() {
            self.preedit = None;
        }
    }

    /// `key` as the editor's own (the find bar's while it is open), past the
    /// keymap. True when it was consumed. The workspace uses it to hand back
    /// a chord it does not act on (Alt+Up with nothing stacked, ADR-0050 §4).
    pub(crate) fn editor_key(
        &mut self,
        key: KeyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.preedit = None;
        if self.input_blocked {
            // Taken, so it reaches nothing else either.
            return true;
        }
        if self.editor.find_open() {
            self.find_key(key, cx);
            self.settle(window, cx);
            return true;
        }
        let (handled, _) = self.editor.handle_key(key, &self.tx);
        self.settle(window, cx);
        handled == Handled::Consumed
    }

    /// Where a key arrives, for the keymap: the editor, running vim unless
    /// the find bar (a plain field whatever the backend) has the keyboard.
    /// The workspace resolves the chords the editor lets bubble in this same
    /// context, so the two never disagree about whose a key is.
    pub(crate) fn key_ctx(&self) -> KeyCtx {
        KeyCtx {
            panel: Panel::Editor,
            vim: self.editor.is_vim() && !self.editor.find_open(),
        }
    }

    /// Act on `key` if the keymap binds it to an app command here, whether or
    /// not the find bar is open: find opens the bar (or steps), replace opens
    /// it with the replace field.
    fn app_command(&mut self, key: KeyInput, cx: &App) -> AppCommand {
        let keymap = keymap::active(cx);
        match keymap.resolve(key, self.key_ctx()) {
            None => AppCommand::None,
            Some(ActionShortcuts::FindInBuffer) => {
                self.open_find();
                AppCommand::Handled
            }
            Some(ActionShortcuts::ReplaceInBuffer) => {
                if self.editor.open_replace() {
                    self.preedit = None;
                }
                AppCommand::Handled
            }
            Some(_) => AppCommand::NotOurs,
        }
    }

    /// A key while the find bar is open: the bar takes it, whatever it is.
    ///
    /// Ctrl-V (and the platform's Cmd-V) pastes the clipboard into the
    /// focused field: a terminal pastes through its own channel, a window has
    /// to ask. Any other Super (Cmd) chord is ignored rather than typed: the
    /// logic field types a char with Super held, and Cmd-Z must not type `z`.
    /// The bar's own field keys (Ctrl-Z undo, Ctrl-A replace-all) are the
    /// logic bar's literal Ctrl chords, not keymap actions, so Cmd-Z/Cmd-A do
    /// not stand in for them; only the app commands go through the GUI keymap
    /// (ADR-0050 §6).
    fn find_key(&mut self, key: KeyInput, cx: &mut Context<Self>) {
        let paste = matches!(key.key, Key::Char('v'))
            && (key.mods == Mods::CTRL || key.mods == Mods::SUPER);
        if paste {
            match cx.read_from_clipboard().and_then(|item| item.text()) {
                Some(text) if !text.is_empty() => {
                    self.editor.paste_into_find(&text);
                }
                _ => {
                    self.editor.ignore_find_key();
                }
            }
        } else if key.mods.sup {
            self.editor.ignore_find_key();
        } else {
            self.editor.handle_find_key(key);
        }
    }

    /// The caret's shape for the editor's mode: the TUI's rule (bar in vim
    /// Insert, block in every other vim mode). The plain editor always types,
    /// so it gets the bar.
    pub(crate) fn caret_shape(&self) -> CaretShape {
        match self.editor.modal_is_insert() {
            Some(false) => CaretShape::Block,
            Some(true) | None => CaretShape::Bar,
        }
    }

    /// The editor's face for the kit's monospace `kit_family` (see
    /// [`EditorFonts`]); probing the machine's fonts happens once.
    pub(crate) fn editor_fonts(
        &mut self,
        kit_family: &SharedString,
        text_system: &TextSystem,
    ) -> EditorFonts {
        match &self.fonts {
            Some((family, fonts)) if family == kit_family => fonts.clone(),
            _ => {
                let fonts = EditorFonts::resolve(text_system, kit_family);
                self.fonts = Some((kit_family.clone(), fonts.clone()));
                fonts
            }
        }
    }

    /// The mode line's content; `None` for the plain editor, which has no mode.
    pub(crate) fn mode_line(&self) -> Option<ModeLine> {
        self.editor.mode_label().map(|label| ModeLine {
            label,
            hint: self.editor.pending_input_hint(),
        })
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.keystroke.is_modifier_key() {
            self.clicks.end();
        }
        if let Some(key) = input::key_down_input(event) {
            // A dead key or compose sequence finishes as a plain key-down
            // (`´` preedit, then `é` as a keystroke): whatever was being
            // composed is over once a key acts.
            self.preedit = None;
            match self.app_command(key, cx) {
                AppCommand::Handled => {
                    self.settle(window, cx);
                    cx.stop_propagation();
                }
                // Bubbles to the command's owner (the workspace); the editor
                // never sees it unless the owner hands it back.
                AppCommand::NotOurs => {}
                AppCommand::None => {
                    // A consumed key must not also reach the platform's input
                    // handler, or typed text would be inserted twice.
                    if self.editor_key(key, window, cx) {
                        cx.stop_propagation();
                    }
                }
            }
            return;
        }
        // A bare modifier is not an action at all.
        if event.keystroke.is_modifier_key() {
            return;
        }
        // Multi-char text (a grapheme cluster, an emoji sequence) propagates so
        // the platform hands it to the input handler, where the editor types.
        // In a vim command mode it is no command: it ends up as an unmapped
        // key below, and must not reach the handler either.
        let carries_text = input::carries_text(event);
        if carries_text && self.accepts_text() {
            return;
        }
        if carries_text {
            cx.stop_propagation();
        }
        // A key with no editing meaning still ends the typing run (TUI parity);
        // the open find bar owns it all the same.
        if !self.editor.ignore_find_key() {
            self.editor.ignore_unmapped_key();
        }
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
        // The find bar and the mode line are not text: a press on them only
        // focuses (and, on the bar's other field, moves the keyboard there).
        // Without either (plain mode) a press below the last full row still
        // reaches the editor, as in M1.
        if let Some(p) = &self.painted
            && (p.mode_line.is_some() || p.find_bar.is_some())
            && pointer.row >= p.rows
        {
            self.clicks.end();
            if pointer.kind == PointerKind::Down(Button::Left) {
                self.focus_bar_row(pointer.row - p.rows);
                self.settle(window, cx);
            }
            return;
        }
        let left = pointer.kind == PointerKind::Down(Button::Left);
        // A press on a link (the hand cursor) only places the caret, as the
        // TUI's first press does. A second plain left press on the same cell
        // soon after follows the **follow target** the first one put the
        // caret on (ADR-0043): the workspace does it, as for the chord. With
        // nothing to follow there, it is an ordinary press.
        if left && pointer.mods == Mods::NONE {
            if self
                .clicks
                .completes_double(pointer.col, pointer.row, Instant::now())
                && self.follow_at_caret()
            {
                self.dragging = false;
                self.settle(window, cx);
                return;
            }
        } else {
            self.clicks.end();
        }
        self.editor.handle_pointer(&pointer, &self.tx);
        self.editor.settle_pointer(left);
        self.dragging = left;
        self.settle(window, cx);
    }

    /// Ask the window's owner to follow the follow target under the caret
    /// (a link, a label). False when there is none.
    fn follow_at_caret(&self) -> bool {
        let message = match self.editor.follow_target_at_cursor() {
            Some(FollowTarget::Link(target)) => AppMessage::FollowLink(target),
            Some(FollowTarget::Label(name)) => AppMessage::FollowLabel(name),
            None => return false,
        };
        self.tx.send(message).ok();
        true
    }

    /// A press on find bar row `row` (0 the query, 1 the replacement) gives
    /// that field the keyboard. The bar's own Tab is the one way it moves
    /// focus, so the press uses it.
    fn focus_bar_row(&mut self, row: u16) {
        let Some(bar) = self.editor.find_bar() else {
            return;
        };
        let want = match row {
            0 => kimun_notes::logic::editor::find_bar::BarFocus::Find,
            1 if bar.is_replacing() => kimun_notes::logic::editor::find_bar::BarFocus::Replace,
            _ => return,
        };
        if bar.focus() != want {
            self.preedit = None;
            self.editor.handle_find_key(KeyInput::plain(Key::Tab));
        }
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
        // The same cell now shows other text.
        self.clicks.end();
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
    ///
    /// While the find bar is open the document is its focused field instead.
    fn document(&self) -> Document {
        if let Some(bar) = self.editor.find_bar() {
            let field = find_bar::focused_field(bar);
            return Document::new(
                0,
                field.value().to_string(),
                field.cursor_char_offset(),
                self.preedit.as_deref(),
            );
        }
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
        if self.editor.find_open() {
            self.commit_to_bar(target.map(|(_, cols)| cols), text);
        } else {
            self.editor.commit_text(target, text);
        }
    }

    /// Committed text into the find bar's focused field: over `cols` (chars
    /// of the field) when given, else at its caret. The field is the logic
    /// bar's, reached only through its key and paste API, so a range is
    /// walked to with Left / Right, cleared with Backspace, and the text goes
    /// in as the one key it is, or as a paste (first line only; a field is
    /// one line).
    fn commit_to_bar(&mut self, cols: Option<Range<usize>>, text: &str) {
        let press = |editor: &mut Editor, key: Key, n: usize| {
            for _ in 0..n {
                editor.handle_find_key(KeyInput::plain(key));
            }
        };
        if let Some(cols) = cols
            && let Some(bar) = self.editor.find_bar()
        {
            let caret = find_bar::focused_field(bar).cursor_char_offset();
            let len = find_bar::focused_field(bar).value().chars().count();
            let end = cols.end.min(len);
            let start = cols.start.min(end);
            if caret > end {
                press(&mut self.editor, Key::Left, caret - end);
            } else {
                press(&mut self.editor, Key::Right, end - caret);
            }
            press(&mut self.editor, Key::Backspace, end - start);
        }
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (None, _) => {}
            (Some(c), None) if c.is_control() => {}
            (Some(c), None) => {
                self.editor.handle_find_key(KeyInput::plain(Key::Char(c)));
            }
            _ => {
                self.editor.paste_into_find(text);
            }
        }
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
        // A find bar field has a caret and no selection.
        let selection = self
            .editor
            .selection()
            .filter(|_| !self.editor.find_open())
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

    /// Turns the platform's text services (IME) off outside plain mode and vim
    /// Insert. Linux and Windows honour it each frame; the methods below gate
    /// on the same rule for a platform that does not.
    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.accepts_text()
    }

    /// Ending composition keeps what was composed (GPUI's contract): with the
    /// preedit outside the buffer, that means committing it now.
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.preedit.take() {
            if self.accepts_text() && !self.input_blocked {
                self.commit(None, &text);
            }
            self.settle(window, cx);
        }
    }

    /// Committed text. Outside plain mode and vim Insert it is not typing: a
    /// single char the platform composed (a dead-key `é`, say) goes to vim as
    /// the keystroke it is, so `r` then `é` still replaces; anything longer
    /// is dropped.
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked {
            self.preedit = None;
        } else if self.accepts_text() {
            self.commit(range, text);
        } else {
            self.preedit = None;
            let mut chars = text.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                self.editor
                    .handle_key(KeyInput::plain(Key::Char(c)), &self.tx);
            }
        }
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
        // No composing where the editor does not type.
        if self.input_blocked || !self.accepts_text() {
            self.preedit = None;
            self.settle(window, cx);
            return;
        }
        let doc = self.document();
        if let Some(r) = range.filter(|r| Some(r) != doc.marked().as_ref()) {
            let cols = doc.to_buffer(r.start)..doc.to_buffer(r.end.max(r.start));
            // The preedit is not in the field/buffer: drop it before editing.
            self.preedit = None;
            if self.editor.find_open() {
                self.commit_to_bar(Some(cols), "");
            } else {
                self.editor.commit_text(Some((doc.row, cols)), "");
            }
        }
        self.preedit = (!new_text.is_empty()).then(|| new_text.to_string());
        self.settle(window, cx);
    }

    /// Where the IME should put its candidate window: the caret cell (the
    /// find bar's while it is open).
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.painted.as_ref().and_then(|p| p.caret)
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

    fn at(millis: u64) -> Instant {
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(millis)
    }

    #[test]
    fn two_quick_presses_on_one_cell_are_a_double_and_end_the_run() {
        let mut run = ClickRun::default();
        assert!(!run.completes_double(4, 2, at(0)), "one press is not");
        assert!(run.completes_double(4, 2, at(399)));
        // A third press starts over rather than pairing with the second.
        assert!(!run.completes_double(4, 2, at(450)));
    }

    #[test]
    fn a_slow_pair_or_another_cell_is_no_double() {
        let mut run = ClickRun::default();
        assert!(!run.completes_double(4, 2, at(0)));
        assert!(
            !run.completes_double(4, 2, at(400)),
            "the window is exclusive"
        );
        assert!(!run.completes_double(5, 2, at(500)), "a neighbour cell");
        assert!(!run.completes_double(5, 3, at(600)), "another row");
        assert!(run.completes_double(5, 3, at(700)));
    }

    #[test]
    fn ending_the_run_forgets_the_first_press() {
        let mut run = ClickRun::default();
        assert!(!run.completes_double(1, 1, at(0)));
        run.end();
        assert!(!run.completes_double(1, 1, at(100)));
    }

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
