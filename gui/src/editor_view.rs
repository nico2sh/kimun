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
//! (ADR-0043: two plain left presses on one cell within [`pointer::DOUBLE_CLICK`]) or
//! a click with the primary modifier (Cmd on macOS, Ctrl elsewhere) on a link
//! or a label asks the window's owner to follow it, on the app channel; the
//! `FollowLink` chord bubbles to the owner like any app command. With the find
//! bar open a press never follows, as in the TUI.
//!
//! **Right-click.** With a selection a right press copies it and asks for
//! nothing. Without one, a right press on a drawn **Inline image** opens its
//! attachment's **Entry menu** (built by the owner, [`EditorView::set_entry_menu`]),
//! on a link or a label it asks the owner
//! for the Link menu ([`EditorEvent::LinkMenu`]), and anywhere else for the
//! note menu ([`EditorEvent::ContextMenu`]). Neither moves the caret: the
//! target is read from the cell under the pointer
//! ([`EditorView::target_under`]). A read-only editor asks for the Link menu
//! the same way, and does nothing on a right press off a target.
//!
//! **Read-only.** A read-only editor (a facet view's piece) is text: a press
//! places the caret, a drag selects, a double press on text selects a word,
//! and the copy chord copies. A click on a link or label (a press and its
//! release with the pointer kept on that target) also asks the owner to
//! follow it: it emits [`EditorEvent::ReadOnlyFollow`] at the release. A
//! primary press on a target emits it at the press. The presses right after
//! a click that followed, on the same cell, are plain presses. A drag that
//! leaves the target selects and follows nothing. It never follows on the
//! channel (ADR-0061). Its targets show the pointing hand at all times, as
//! in an editor that edits. A right press on a target asks for the Link
//! menu; elsewhere it does nothing.
//!
//! **The keymap.** Every key-down is first resolved against the GUI keymap
//! ([`crate::keymap`]) with the editor focused. Find and replace are the
//! editor's own; any other app command is left to bubble to whoever owns it,
//! and never reaches the editor. Under vim the keymap hands bare Ctrl+letter
//! chords to the editor (the vim rule, ADR-0050 §6).

use std::num::NonZeroU64;
use std::time::Instant;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyBinding, MouseButton, MouseDownEvent, NoAction, ParentElement, Pixels, Point, Render,
    Styled, Subscription, Task, Window, div, rems,
};
use kimun_notes::logic::autocomplete::AutocompleteEngine;
use kimun_notes::logic::editor::Editor;
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::editor::request::EditorRequest;
use kimun_notes::logic::message::{AppTx, redraw_callback};

use crate::editor_element::{EditorElement, ShapedRows};
use crate::editor_font::FaceInputs;
use crate::fonts::EditorFonts;
use crate::input::ScrollAccumulator;
use crate::theme;

mod faces;
mod images;
mod ime;
mod keys;
mod measure;
mod painted;
mod pointer;
mod popup;
mod read_only;

pub(crate) use self::images::{EntryMenu, ImageDraw, ImageLook, ImagePane};
#[cfg(test)]
pub(crate) use self::measure::probe as measure_probe;
pub(crate) use self::measure::{EditStamp, SLACK, line_height, measure_width};
#[cfg(test)]
pub use self::painted::ModeLine;
#[cfg(test)]
pub use self::painted::PaintedImage;
pub use self::painted::{CaretShape, Painted};
use self::pointer::ClickRun;
#[cfg(test)]
pub(crate) use self::pointer::frozen_clock;
use self::read_only::ReadOnlyClicks;

/// The pane's padding at each side of the grid, in rems (`px_4`). A facet
/// piece's height table reads it too (`workspace::facet::rows`).
pub(crate) const PANE_PAD_X_REMS: f32 = 1.0;
/// The pane's padding above and below the grid, in rems (`py_2`).
pub(crate) const PANE_PAD_Y_REMS: f32 = 0.5;

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

/// What the editor asks of the window that owns it, beyond the app channel:
/// a surface that needs a `Window` to open.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorEvent {
    /// Vim `:` — open the command palette.
    OpenPalette,
    /// A right-press with no selection — open the note menu at `at`, the
    /// pointer in window coordinates.
    ContextMenu { at: Point<Pixels> },
    /// A click on a **follow target** in a read-only editor: the owner
    /// decides how to follow it (the editor never follows on the app
    /// channel). A plain click emits at its release, while the pointer
    /// stayed on the target. A primary press (Cmd on macOS, Ctrl elsewhere)
    /// emits at the press. A drag off the target, a press off any target and
    /// the presses right after a follow emit nothing: they are text presses
    /// (caret, selection).
    ReadOnlyFollow { target: FollowTarget },
    /// A plain press on a task checkbox in a read-only editor: the owner
    /// decides whether to toggle it. `row` is the buffer row of the task
    /// line and `line` its text, which the owner checks against the note
    /// before it writes. The press places no caret and starts no selection.
    TaskToggle { row: usize, line: String },
    /// A right-press with no selection on a **follow target** (a link, a
    /// label), in an editor that edits or a read-only one: open the Link
    /// menu for `target` at `at`, the pointer in window coordinates. It
    /// replaces [`Self::ContextMenu`] for that press. The caret stays where
    /// it was.
    LinkMenu {
        at: Point<Pixels>,
        target: FollowTarget,
    },
}

impl EventEmitter<EditorEvent> for EditorView {}

/// One editor pane: the logic [`Editor`] plus what only a GPUI host needs.
pub struct EditorView {
    pub(crate) editor: Editor,
    pub(crate) focus_handle: FocusHandle,
    /// The app channel the editor reports on (clipboard flashes) and the
    /// background parse/layout wake-up rides on.
    tx: AppTx,
    scroll: ScrollAccumulator,
    /// A left press in this editor is being dragged. Drags that began
    /// elsewhere are not ours to turn into a selection.
    dragging: bool,
    /// Where the last press landed, in window coordinates: the note menu a
    /// right-press asks for opens there.
    pressed_at: Point<Pixels>,
    /// The presses that may form a double-click (follow the link there).
    clicks: ClickRun,
    /// What time it is, for the double-click rule: the real clock. Tests freeze it:
    /// two simulated presses are microseconds apart on a calm machine and seconds
    /// apart on a loaded one, and the rule is about the gap between them.
    clock: fn() -> Instant,
    /// IME composition text, shown at the caret but kept out of the buffer
    /// until committed: a preedit is not an edit, so it must not dirty the
    /// note or enter undo history.
    pub(crate) preedit: Option<String>,
    /// The editor takes no input (keys, text, composition): its owner is
    /// loading another note into it, which would drop whatever was typed.
    /// App chords still resolve.
    pub(crate) input_blocked: bool,
    /// A read-only surface: no caret drawn, presses select text (a double
    /// press a word), the copy chord copies, and a click on a follow target
    /// reports [`EditorEvent::ReadOnlyFollow`] instead of following.
    /// Losing the keyboard drops the selection. Other keys are stopped by
    /// `input_blocked`, which this sets.
    pub(crate) read_only: bool,
    /// The editor is as tall as its wrapped text, not as tall as its pane.
    /// It measures its own height (module `measure`). Set by
    /// `new_read_only`, for a facet piece.
    pub(crate) sized_to_text: bool,
    /// The columns the last measure laid the text out at. Prepaint of a
    /// text-sized editor draws at these columns.
    pub(crate) measured_cols: Option<u16>,
    /// The rows the last measure found.
    pub(crate) measured_rows: Option<usize>,
    /// The width the measure laid out at in this frame. Layout asks the
    /// measure more than once per frame. A call at this width again reuses
    /// the rows. The element clears it when it asks for its layout.
    pub(crate) measured_width: Option<Pixels>,
    /// The text revision and the caret that the last measure laid out.
    /// Prepaint checks them before it skips its own frame preparation.
    pub(crate) measured_stamp: Option<measure::EditStamp>,
    /// A read-only surface's click state: the press on a follow target
    /// that waits for its release, and the last click that followed.
    clicks_ro: ReadOnlyClicks,
    /// A read-only surface's watch on its keyboard: once the keyboard leaves
    /// it, its selection goes ([`Self::drop_unreachable_selection`]).
    /// Subscribed at the first render, which has the window.
    _focus_out: Option<Subscription>,
    pub(crate) painted: Option<Painted>,
    /// The face, and the inputs it was chosen for (re-chosen when they
    /// change).
    fonts: Option<(FaceInputs, EditorFonts)>,
    /// Rows painted in recent frames, reused while their content is unchanged.
    pub(crate) row_cache: ShapedRows,
    /// Wikilink and hashtag autocomplete, once a vault supplies suggestions.
    autocomplete: Option<AutocompleteEngine>,
    /// The content revision the autocomplete last reconciled with. A buffer
    /// that moved on without us (a note loaded, a reload) closes the popup.
    ac_revision: NonZeroU64,
    /// The vault the editor draws **Inline image**s from, once its owner
    /// hands it one ([`Self::set_image_vault`]). `None` in a read-only
    /// editor, which never draws them.
    images: Option<images::Images>,
    /// Builds the **Entry menu** a right press on an inline image opens,
    /// once its owner hands one ([`Self::set_entry_menu`]).
    entry_menu: Option<EntryMenu>,
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
    pub fn new(text: Option<String>, vim: bool, cx: &mut Context<Self>) -> Self {
        let (tx, messages) = Self::drain_own_channel(cx);
        Self::build(text, vim, tx, Some(messages), cx)
    }

    /// An editor over `text` that takes no input: a press places the caret
    /// and may select, as in read-only text; a click on a follow target
    /// emits [`EditorEvent::ReadOnlyFollow`] instead of following. It
    /// drains its own app channel, as `Self::new` does.
    pub fn new_read_only(text: &str, cx: &mut Context<Self>) -> Self {
        let (tx, messages) = Self::drain_own_channel(cx);
        let mut view = Self::read_only_with(text, tx, Some(messages), cx);
        view.sized_to_text = true;
        view
    }

    /// A channel of the editor's own, and the task that drains it. The task
    /// drops each message and redraws, and stops once the editor is gone.
    fn drain_own_channel(cx: &mut Context<Self>) -> (AppTx, Task<()>) {
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
        (tx, messages)
    }

    /// A read-only editor reporting on `tx` (tests watch the channel).
    #[cfg(test)]
    pub(crate) fn new_read_only_on(text: &str, tx: AppTx, cx: &mut Context<Self>) -> Self {
        Self::read_only_with(text, tx, None, cx)
    }

    fn read_only_with(
        text: &str,
        tx: AppTx,
        messages: Option<Task<()>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::build(Some(text.to_string()), false, tx, messages, cx);
        view.input_blocked = true;
        view.read_only = true;
        // Nothing is being edited, so no row ever shows as raw source
        // (ADR-0062).
        view.editor
            .session_mut()
            .display
            .set_reveal_caret_row(false);
        view
    }

    /// Show `text` in a read-only editor, the caret back at the start.
    pub fn set_text_read_only(&mut self, text: &str, cx: &mut Context<Self>) {
        // A press on the old text follows nothing in the new one.
        self.clicks_ro.reset();
        self.editor.set_text(text.to_string());
        self.forget_frame();
        cx.notify();
    }

    /// An editor reporting on the window's app channel `tx`, which its owner
    /// (the [`Workspace`](crate::workspace::Workspace)) drains and
    /// dispatches: redraws and clipboard flashes. The note session loads
    /// notes into its [`Editor`] (`pub(crate) editor`) through `NoteBuffer`.
    pub fn on_channel(vim: bool, tx: AppTx, cx: &mut Context<Self>) -> Self {
        Self::build(None, vim, tx, None, cx)
    }

    /// Report on `tx` from now on: the window's app channel was replaced
    /// (another vault entered). The background jobs' wake-up moves with it,
    /// so a job finishing for the old channel's buffer wakes no one.
    pub(crate) fn set_channel(&mut self, tx: AppTx) {
        self.editor.session_mut().notifier = Some(redraw_callback(tx.clone()));
        if let Some(engine) = self.autocomplete.as_mut() {
            engine.set_redraw_callback(redraw_callback(tx.clone()));
        }
        self.tx = tx;
    }

    /// Whether a left press in this editor is being dragged.
    pub(crate) fn dragging(&self) -> bool {
        self.dragging
    }

    /// Whether the editor reports on `tx`'s channel (tests).
    #[cfg(test)]
    pub(crate) fn reports_on(&self, tx: &AppTx) -> bool {
        self.tx.same_channel(tx)
    }

    /// Run the vim engine (`vim`) or the plain editor from now on: a fresh,
    /// empty [`Editor`] of that kind, on the same channel. For a vault
    /// change, whose note was already let go; vim's registers go with the
    /// old engine. Nothing changes when the editor already is that kind.
    pub(crate) fn set_vim(&mut self, vim: bool, cx: &mut Context<Self>) {
        if self.editor.is_vim() == vim {
            return;
        }
        let mut editor = Editor::new(vim);
        editor.session_mut().notifier = Some(redraw_callback(self.tx.clone()));
        self.editor = editor;
        if let Some(engine) = self.autocomplete.as_mut() {
            engine.close();
        }
        self.forget_frame();
        cx.notify();
    }

    /// Forget what the last frames knew of the old text: the popup's
    /// revision, the preedit, the painted grid and the cached rows.
    fn forget_frame(&mut self) {
        self.ac_revision = self.editor.content_revision();
        self.preedit = None;
        self.painted = None;
        self.row_cache = ShapedRows::default();
        if let Some(images) = self.images.as_mut() {
            images.forget_lines();
        }
    }

    fn build(
        text: Option<String>,
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
        let ac_revision = editor.content_revision();
        Self {
            editor,
            focus_handle: cx.focus_handle(),
            tx,
            scroll: ScrollAccumulator::default(),
            dragging: false,
            pressed_at: Point::default(),
            clicks: ClickRun::default(),
            clock: Instant::now,
            preedit: None,
            input_blocked: false,
            read_only: false,
            sized_to_text: false,
            measured_cols: None,
            measured_rows: None,
            measured_width: None,
            measured_stamp: None,
            clicks_ro: ReadOnlyClicks::default(),
            _focus_out: None,
            painted: None,
            fonts: None,
            row_cache: ShapedRows::default(),
            autocomplete: None,
            ac_revision,
            images: None,
            entry_menu: None,
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
                // Vim `:` opens the command palette, which the window owns.
                EditorRequest::OpenPalette => cx.emit(EditorEvent::OpenPalette),
                // Right-click with no selection opens the note menu, which
                // the window owns. With a selection the editor copies it and
                // asks for nothing.
                EditorRequest::ContextMenu => cx.emit(EditorEvent::ContextMenu {
                    at: self.pressed_at,
                }),
                // Nothing (no popup) is anchored to the cursor to move yet.
                EditorRequest::ViewportScrolled => {}
            }
        }
        self.sync_autocomplete();
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tidy_autocomplete(window);
        if self.read_only && self._focus_out.is_none() {
            let handle = self.focus_handle.clone();
            self._focus_out = Some(cx.on_focus_out(&handle, window, |this, _, window, cx| {
                this.drop_unreachable_selection(window, cx)
            }));
        }
        let pane = div()
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
            .px(rems(PANE_PAD_X_REMS))
            .py(rems(PANE_PAD_Y_REMS))
            .bg(theme::background(&theme::current(cx)))
            .child(EditorElement::new(cx.entity()));
        // A text-sized editor takes its height from its text.
        if self.sized_to_text {
            pane.w_full()
        } else {
            pane.size_full()
        }
    }
}
