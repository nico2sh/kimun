//! How the editor feeds the wikilink and hashtag autocomplete engine: it
//! reconciles the popup with the buffer after input and runs the popup's
//! commands (keys, clicks, hover, the wheel). This module does not draw
//! the popup: the editor element does.

use std::sync::Arc;

use gpui_kit::{Context, Window};
use kimun_notes::logic::autocomplete::{
    AutocompleteEngine, AutocompleteMode, AutocompleteState, HandleKeyOutcome, PopupCommand,
    SuggestionSource,
};
use kimun_notes::logic::message::redraw_callback;

use super::EditorView;
use crate::autocomplete::EditorHost;

impl EditorView {
    /// Whether the editor completes `[[` and `#` from a vault (tests).
    #[cfg(test)]
    pub(crate) fn completes(&self) -> bool {
        self.autocomplete.is_some()
    }

    /// Complete `[[` and `#` from `source` (`None`: no vault, so no popup).
    /// The popup's results wake the window through the app channel.
    pub(crate) fn set_suggestions(&mut self, source: Option<Arc<dyn SuggestionSource>>) {
        self.autocomplete = source.map(|source| {
            let mut engine = AutocompleteEngine::new(source, AutocompleteMode::Both);
            engine.set_redraw_callback(redraw_callback(self.tx.clone()));
            engine
        });
        self.ac_revision = self.editor.content_revision();
    }

    /// The popup's state while it has something to show.
    pub(crate) fn autocomplete_state(&self) -> Option<&AutocompleteState> {
        self.autocomplete
            .as_ref()
            .filter(|engine| engine.is_open())
            .and_then(|engine| engine.state())
    }

    /// Reconcile the popup with the buffer after input. Text typed (the
    /// revision moved) may open it; a cursor move only keeps it or closes it.
    /// Only where the editor types: never in vim's command modes, never while
    /// the find bar has the keyboard.
    pub(super) fn sync_autocomplete(&mut self) {
        let Some(engine) = self.autocomplete.as_mut() else {
            return;
        };
        let types = !self.input_blocked
            && !self.editor.find_open()
            && self.editor.modal_is_insert() != Some(false);
        if !types {
            engine.close();
            return;
        }
        let revision = self.editor.content_revision();
        if revision == self.ac_revision {
            engine.refresh_if_open(&EditorHost(&self.editor));
            return;
        }
        self.ac_revision = revision;
        // Prose with no `[` or `#` before the caret cannot open a popup: skip
        // the buffer scan the engine would otherwise do per keystroke.
        let (row, col) = self.editor.cursor();
        let line = self.editor.buffer().row(row).unwrap_or_default();
        if !engine.may_need_sync(&line, col) {
            return;
        }
        engine.sync(&EditorHost(&self.editor));
    }

    /// Drain finished queries, and drop a popup that no longer belongs: the
    /// buffer changed without our input (another note loaded) or the editor
    /// lost the keyboard.
    pub(super) fn tidy_autocomplete(&mut self, window: &Window) {
        let revision = self.editor.content_revision();
        let focused = self.focus_handle.is_focused(window);
        if let Some(engine) = self.autocomplete.as_mut() {
            engine.poll_results();
            if engine.is_open() && (revision != self.ac_revision || !focused) {
                engine.close();
            }
        }
    }

    /// Run `command` on the open popup. `true` when the popup took it.
    pub(super) fn run_popup_command(
        &mut self,
        command: PopupCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.input_blocked || self.editor.find_open() {
            return false;
        }
        let Some(engine) = self.autocomplete.as_mut().filter(|e| e.is_open()) else {
            return false;
        };
        match engine.handle_command(command, &EditorHost(&self.editor)) {
            HandleKeyOutcome::NotHandled => false,
            HandleKeyOutcome::Consumed | HandleKeyOutcome::Dismissed => {
                self.settle(window, cx);
                true
            }
            HandleKeyOutcome::Accepted(action) => {
                self.preedit = None;
                self.editor.apply_autocomplete(&action);
                // The edit is ours: reconciling with it would reopen the popup
                // on the word just completed.
                self.ac_revision = self.editor.content_revision();
                self.settle(window, cx);
                true
            }
        }
    }

    /// A press on popup row `offset` (from the top of the window): accept it.
    pub(crate) fn autocomplete_click(
        &mut self,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_popup_command(PopupCommand::ClickRow(offset), window, cx);
    }

    /// The pointer over popup row `offset`: highlight it.
    pub(crate) fn autocomplete_hover(&mut self, offset: usize, cx: &mut Context<Self>) {
        if let Some(state) = self
            .autocomplete
            .as_mut()
            .and_then(|engine| engine.state_mut())
        {
            let (start, end) = state.visible_window();
            let idx = start + offset;
            if idx < end && state.highlighted != idx {
                state.highlighted = idx;
                cx.notify();
            }
        }
    }

    /// A navigation `command` on the popup (the wheel over it).
    pub(crate) fn autocomplete_command(&mut self, command: PopupCommand, cx: &mut Context<Self>) {
        if let Some(engine) = self.autocomplete.as_mut()
            && let Some(state) = engine.state_mut()
        {
            kimun_notes::logic::autocomplete::apply_command(state, command);
            cx.notify();
        }
    }
}
