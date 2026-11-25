//! Where a key-down goes: the autocomplete popup, an app command of the
//! keymap, the find bar or the editor. It also holds the text gate that
//! the IME reads. This module does not own pointer input or IME text.

use gpui_kit::{App, Context, KeyDownEvent, Window};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::autocomplete::key_command;
use kimun_notes::logic::editor::Editor;
use kimun_notes::logic::editor::handled::Handled;
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::text_action::TextAction;

use super::EditorView;
use crate::input;
use crate::keymap::{self, KeyCtx};

/// What the keymap made of a key-down, from the editor's side.
enum AppCommand {
    /// Not bound to an app command here: the editor's key.
    None,
    /// An editor command (find, replace), done.
    Handled,
    /// Another owner's command (focus, the slot): not the editor's.
    NotOurs,
}

impl EditorView {
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
    pub(super) fn open_find(&mut self) {
        if self.editor.open_find() {
            self.preedit = None;
        }
    }

    /// Open the find bar with its replace field, or step to the next match
    /// when it is open. A composition in progress belonged to the buffer.
    fn open_replace(&mut self) {
        if self.editor.open_replace() {
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
            // A read-only surface still copies what is selected in it.
            if self.read_only && key == KeyInput::new(Key::Char('c'), keymap::primary(cx)) {
                self.copy_selection(cx);
            }
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
        let vim = self.editor.is_vim() && !self.editor.find_open();
        KeyCtx {
            panel: Panel::Editor,
            vim,
            query: false,
            browser: false,
            vim_pending: vim && self.editor.pending_input_hint().is_some(),
            vim_insert: vim && self.editor.modal_is_insert() == Some(true),
        }
    }

    /// Open the find bar (`replace`: with its replace field), or step to the
    /// next match when open — the find and replace chords, run from the
    /// command palette.
    pub(crate) fn find_command(
        &mut self,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked {
            return;
        }
        if replace {
            self.open_replace();
        } else {
            self.open_find();
        }
        self.settle(window, cx);
    }

    /// Bold, italic or strikethrough on the selection (the leader's `+text`,
    /// run from the command palette). Nothing while the editor takes no
    /// input or its find bar has the keyboard.
    pub(crate) fn apply_text_action(
        &mut self,
        action: TextAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_blocked || self.editor.find_open() {
            return;
        }
        self.preedit = None;
        self.editor.apply_text_action(action);
        self.settle(window, cx);
    }

    /// Act on `key` if the keymap binds it to an app command here, whether or
    /// not the find bar is open: find opens the bar (or steps), replace opens
    /// it with the replace field.
    fn app_command(&mut self, key: KeyInput, cx: &App) -> AppCommand {
        let keymap = keymap::active(cx);
        match keymap.resolve(key, self.key_ctx()) {
            None => AppCommand::None,
            // A read-only surface has no find bar: it would take every key
            // (Esc too) and could not be closed. Not ours: the chord bubbles
            // to the owner, as the other chords do, and never reaches the
            // editor.
            Some(ActionShortcuts::FindInBuffer | ActionShortcuts::ReplaceInBuffer)
                if self.read_only =>
            {
                AppCommand::NotOurs
            }
            Some(ActionShortcuts::FindInBuffer) => {
                self.open_find();
                AppCommand::Handled
            }
            Some(ActionShortcuts::ReplaceInBuffer) => {
                self.open_replace();
                AppCommand::Handled
            }
            Some(_) => AppCommand::NotOurs,
        }
    }

    /// A key while the find bar is open: the bar takes it, whatever it is.
    ///
    /// Ctrl-V (and the platform's Cmd-V) pastes the clipboard into the
    /// focused field: a terminal pastes through its own channel, a window has
    /// to ask. The logic bar maps Cmd-Z/Y/Shift-Z to its undo/redo and drops
    /// every other Super (Cmd) chord rather than typing it. Cmd-A is not
    /// Ctrl-A there: that chord is replace-all. The bar's field keys are the
    /// logic bar's literal chords, not keymap actions; only the app commands
    /// go through the GUI keymap (ADR-0050 §6).
    fn find_key(&mut self, key: KeyInput, cx: &mut Context<Self>) {
        if Editor::is_paste_chord(key) {
            match cx.read_from_clipboard().and_then(|item| item.text()) {
                Some(text) if !text.is_empty() => {
                    self.editor.paste_into_find(&text);
                }
                _ => {
                    self.editor.ignore_find_key();
                }
            }
        } else {
            self.editor.handle_find_key(key);
        }
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.keystroke.is_modifier_key() {
            self.clicks.end();
        }
        if let Some(key) = input::key_down_input(event) {
            // A dead key or compose sequence finishes as a plain key-down
            // (`´` preedit, then `é` as a keystroke): whatever was being
            // composed is over once a key acts.
            // The popup, when open, has Up/Down/Tab/Enter/Esc before anything
            // else: they are its own, not the editor's or an app command.
            // Except mid-composition: those keys then belong to the IME
            // (Enter commits, Esc cancels), and an accept would drop the
            // composed text.
            if self.preedit.is_none()
                && let Some(command) = key_command(key, false)
                && self.run_popup_command(command, window, cx)
            {
                cx.stop_propagation();
                return;
            }
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
                    // macOS Option+letter composes text (dead keys, `ß`):
                    // the platform's input handler must see it, and the
                    // editor's own key path would swallow it as an Alt chord.
                    if self.accepts_text()
                        && !self.input_blocked
                        && input::option_composes_text(&event.keystroke, cfg!(target_os = "macos"))
                    {
                        return;
                    }
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
}
