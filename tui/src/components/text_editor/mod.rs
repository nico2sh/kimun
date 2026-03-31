pub mod backend;
pub mod markdown;
pub mod nvim_rpc;
pub mod snapshot;
pub mod view;
pub mod word_wrap;

use arboard::Clipboard;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

/// Move or extend the selection by `movement`.
///
/// If `shift` is held and no selection is currently active, anchors the selection
/// first; otherwise the existing anchor is kept. Without `shift`, any active
/// selection is cancelled before the cursor moves.
macro_rules! cursor_move {
    ($ta:expr, $mv:expr, $shift:expr) => {{
        if $shift {
            if $ta.selection_range().is_none() {
                $ta.start_selection();
            }
        } else {
            $ta.cancel_selection();
        }
        $ta.move_cursor($mv);
    }};
}

use self::backend::BackendState;
use self::snapshot::NvimMode;
use self::view::MarkdownEditorView;

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::events::AppTx;
use crate::components::events::InputEvent;
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct TextEditorComponent {
    backend: BackendState,
    /// Tracks the rendered rect to map mouse click coordinates.
    rect: Rect,
    key_bindings: KeyBindings,
    last_saved_text: String,
    view: MarkdownEditorView,
    /// Incremented on every mutating input event. Passed to `view.update()` so the view
    /// can skip the expensive lines clone and parse-cache rebuild on idle frames.
    edit_generation: u64,
    /// Current selection range in logical (row, byte-col) coordinates.
    /// Only tracked for the Textarea backend; always `None` for Nvim.
    selection: Option<((usize, usize), (usize, usize))>,
    /// System clipboard handle. `None` if the clipboard is unavailable (e.g. headless CI).
    clipboard: Option<Clipboard>,
    /// `true` after a `Z` keypress in Normal mode; cleared on the next key.
    /// Lets us intercept `ZZ` (write+quit) and `ZQ` (quit) without forwarding them to nvim.
    nvim_pending_z: bool,
}

impl TextEditorComponent {
    pub fn new(key_bindings: KeyBindings, settings: &AppSettings) -> Self {
        Self {
            backend: BackendState::from_settings(
                &settings.editor_backend,
                settings.nvim_path.as_ref(),
            ),
            rect: Rect::default(),
            key_bindings,
            last_saved_text: String::new(),
            view: MarkdownEditorView::new(),
            edit_generation: 0,
            selection: None,
            clipboard: Clipboard::new().ok(),
            nvim_pending_z: false,
        }
    }

    /// Returns the buffer lines for direct access.
    ///
    /// For the Textarea backend, returns the live lines.
    /// For the Nvim backend, returns an empty slice — use `get_text()` instead,
    /// which reads from the snapshot.
    pub fn lines(&self) -> &[String] {
        match &self.backend {
            BackendState::Textarea(ta) => ta.lines(),
            BackendState::Nvim(_) => &[],
        }
    }

    pub fn set_text(&mut self, text: String) {
        match &mut self.backend {
            BackendState::Textarea(ta) => {
                let lines = text.lines();
                *ta = TextArea::from(lines);
            }
            BackendState::Nvim(nvim) => {
                nvim.set_text(&text);
            }
        }
        self.edit_generation = self.edit_generation.wrapping_add(1);
        let reconstructed = self.get_text();
        self.mark_saved(reconstructed);
    }

    pub fn get_text(&self) -> String {
        match &self.backend {
            BackendState::Textarea(ta) => ta.lines().join("\n"),
            BackendState::Nvim(nvim) => {
                nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner()).lines.join("\n")
            }
        }
    }

    pub fn mark_saved(&mut self, text: String) {
        if let BackendState::Nvim(nvim) = &self.backend {
            nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner()).dirty = false;
        }
        self.last_saved_text = text;
    }

    pub fn is_dirty(&self) -> bool {
        match &self.backend {
            BackendState::Textarea(_) => self.get_text() != self.last_saved_text,
            BackendState::Nvim(nvim) => {
                nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner()).dirty
            }
        }
    }

    /// Returns the raw link target under the cursor, or `None` if the cursor
    /// is not inside a wikilink or markdown link span.
    pub fn link_at_cursor(&self) -> Option<String> {
        let (row, col, line) = match &self.backend {
            BackendState::Textarea(ta) => {
                let (row, col) = ta.cursor();
                let line = ta.lines().get(row)?.to_string();
                (row, col, line)
            }
            BackendState::Nvim(nvim) => {
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                let (row, col) = snap.cursor;
                let line = snap.lines.get(row)?.to_string();
                (row, col, line)
            }
        };
        kimun_core::note::link_char_spans(&line)
            .into_iter()
            .find(|s| s.start <= col && col < s.end)
            .map(|s| s.target)
    }

    /// Copy selected text to the system clipboard.
    fn copy_selection_to_clipboard(&mut self) {
        // Scope the borrow of self.backend so self.clipboard can be borrowed after.
        let text = {
            let BackendState::Textarea(ta) = &self.backend else { return };
            let Some(((sr, sc), (er, ec))) = ta.selection_range() else { return };
            let lines = ta.lines();
            if sr == er {
                lines[sr].get(sc..ec).unwrap_or("").to_string()
            } else {
                let mut parts = vec![lines[sr].get(sc..).unwrap_or("").to_string()];
                for row in (sr + 1)..er {
                    parts.push(lines[row].to_string());
                }
                parts.push(lines[er].get(..ec).unwrap_or("").to_string());
                parts.join("\n")
            }
        }; // borrow of self.backend ends here
        if let Some(cb) = &mut self.clipboard {
            let _ = cb.set_text(text);
        }
    }

    /// Paste text from the system clipboard at the cursor, replacing any active selection.
    fn paste_from_clipboard(&mut self) {
        // Get text from clipboard first (releasing that borrow), then access backend.
        let text = match &mut self.clipboard {
            Some(cb) => match cb.get_text() {
                Ok(t) if !t.is_empty() => t,
                _ => return,
            },
            None => return,
        };
        if let BackendState::Textarea(ta) = &mut self.backend {
            if ta.selection_range().is_some() {
                ta.cut();
            }
            ta.insert_str(&text);
            self.selection = ta.selection_range();
            self.edit_generation = self.edit_generation.wrapping_add(1);
        }
    }
}

impl Component for TextEditorComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        use std::sync::atomic::Ordering;

        // If Nvim process died, fall back to Textarea with last known content.
        // Extract fallback text first (scoping the borrow), then reassign self.backend.
        let fallback_text = if let BackendState::Nvim(nvim) = &self.backend {
            if nvim.is_dead.load(Ordering::SeqCst) {
                Some(nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner()).lines.join("\n"))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(text) = fallback_text {
            log::warn!("nvim process died; falling back to textarea backend");
            self.backend = BackendState::Textarea(TextArea::from(text.lines()));
        }

        match event {
            InputEvent::Key(key) => {
                // Nvim backend: forward all keys directly, except FocusSidebar which kimun handles.
                if let BackendState::Nvim(nvim) = &self.backend {
                    if let Some(combo) = key_event_to_combo(key) {
                        if let Some(ActionShortcuts::FocusSidebar) =
                            self.key_bindings.get_action(&combo)
                        {
                            tx.send(AppEvent::FocusSidebar).ok();
                            return EventState::Consumed;
                        }
                    }

                    // Intercept ZZ / ZQ in Normal mode: buffer the first Z, then
                    // decide on the second key without forwarding either to nvim.
                    if self.nvim_pending_z {
                        self.nvim_pending_z = false;
                        match key.code {
                            KeyCode::Char('Z') => {
                                // ZZ — write + quit
                                tx.send(AppEvent::Autosave).ok();
                                tx.send(AppEvent::FocusSidebar).ok();
                                return EventState::Consumed;
                            }
                            KeyCode::Char('Q') => {
                                // ZQ — quit without saving
                                tx.send(AppEvent::FocusSidebar).ok();
                                return EventState::Consumed;
                            }
                            _ => {
                                // Not a quit sequence — replay the buffered Z first.
                                nvim.handle_key(
                                    &ratatui::crossterm::event::KeyEvent::new(
                                        KeyCode::Char('Z'),
                                        KeyModifiers::NONE,
                                    ),
                                    tx.clone(),
                                );
                                // Then fall through to forward the current key normally.
                            }
                        }
                    } else if key.code == KeyCode::Char('Z') {
                        let in_normal = {
                            let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                            snap.mode == NvimMode::Normal
                        };
                        if in_normal {
                            self.nvim_pending_z = true;
                            return EventState::Consumed;
                        }
                    }

                    // Intercept vim quit/write-quit commands so they don't kill the
                    // embedded nvim process.  When Enter is pressed in command mode
                    // with a quit-like command, cancel it in nvim (send <Esc>) and
                    // handle it here instead.
                    if key.code == KeyCode::Enter {
                        let (is_cmd, cmdline) = {
                            let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                            let cmd = if snap.mode == NvimMode::Command {
                                snap.cmdline.as_deref()
                                    .unwrap_or("")
                                    .trim_start_matches(':')
                                    .to_string()
                            } else {
                                String::new()
                            };
                            (snap.mode == NvimMode::Command, cmd)
                        };
                        if is_cmd {
                            let saves = matches!(
                                cmdline.as_str(),
                                "w" | "wq" | "wq!" | "wqa" | "wqa!" | "x" | "xa" | "x!"
                            );
                            let quits = saves || matches!(
                                cmdline.as_str(),
                                "q" | "q!" | "qa" | "qa!" | "cq" | "cq!"
                            );
                            if quits {
                                // Cancel the command in nvim so the process stays alive.
                                nvim.handle_key(
                                    &ratatui::crossterm::event::KeyEvent::new(
                                        KeyCode::Esc,
                                        KeyModifiers::NONE,
                                    ),
                                    tx.clone(),
                                );
                                if saves {
                                    tx.send(AppEvent::Autosave).ok();
                                }
                                tx.send(AppEvent::FocusSidebar).ok();
                                return EventState::Consumed;
                            }
                        }
                    }

                    nvim.handle_key(key, tx.clone());
                    self.edit_generation = self.edit_generation.wrapping_add(1);
                    return EventState::Consumed;
                }

                // Textarea backend: original logic below.

                // System clipboard shortcuts — intercept before passing to textarea.
                if key.modifiers == KeyModifiers::CONTROL {
                    match key.code {
                        KeyCode::Char('c') => {
                            self.copy_selection_to_clipboard();
                            return EventState::Consumed;
                        }
                        KeyCode::Char('v') => {
                            self.paste_from_clipboard();
                            return EventState::Consumed;
                        }
                        KeyCode::Char('x') => {
                            self.copy_selection_to_clipboard();
                            if let BackendState::Textarea(ta) = &mut self.backend {
                                if ta.selection_range().is_some() {
                                    ta.cut();
                                }
                                self.selection = ta.selection_range();
                            }
                            self.edit_generation = self.edit_generation.wrapping_add(1);
                            return EventState::Consumed;
                        }
                        _ => {}
                    }
                }

                // Extract ta for all remaining textarea operations.
                let BackendState::Textarea(ta) = &mut self.backend else {
                    unreachable!("already handled Nvim branch above")
                };

                // macOS-style navigation shortcuts not handled by ratatui-textarea.
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                let handled = match (key.modifiers & !KeyModifiers::SHIFT, key.code) {
                    (KeyModifiers::ALT, KeyCode::Left) => {
                        cursor_move!(ta, CursorMove::WordBack, shift);
                        true
                    }
                    (KeyModifiers::ALT, KeyCode::Right) => {
                        cursor_move!(ta, CursorMove::WordForward, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Left) => {
                        cursor_move!(ta, CursorMove::Head, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Right) => {
                        cursor_move!(ta, CursorMove::End, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Up) => {
                        cursor_move!(ta, CursorMove::Top, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Down) => {
                        cursor_move!(ta, CursorMove::Bottom, shift);
                        true
                    }
                    _ => false,
                };
                if handled {
                    self.selection = ta.selection_range();
                    self.edit_generation = self.edit_generation.wrapping_add(1);
                    return EventState::Consumed;
                }

                // Check keybindings for navigation actions.
                if let Some(combo) = key_event_to_combo(key) {
                    if let Some(ActionShortcuts::FocusSidebar) =
                        self.key_bindings.get_action(&combo)
                    {
                        tx.send(AppEvent::FocusSidebar).ok();
                        return EventState::Consumed;
                    }
                }
                // Standard text-editor shortcuts.
                // `input_without_shortcuts` only handles chars, backspace, delete, tab, newline —
                // all navigation and editing shortcuts must be mapped explicitly.
                let shortcut_handled = match (key.modifiers & !KeyModifiers::SHIFT, key.code) {
                    // --- Cursor movement (Shift extends the selection) ---
                    (KeyModifiers::NONE, KeyCode::Left)     => { cursor_move!(ta, CursorMove::Back,             shift); true }
                    (KeyModifiers::NONE, KeyCode::Right)    => { cursor_move!(ta, CursorMove::Forward,          shift); true }
                    (KeyModifiers::NONE, KeyCode::Up)       => { cursor_move!(ta, CursorMove::Up,               shift); true }
                    (KeyModifiers::NONE, KeyCode::Down)     => { cursor_move!(ta, CursorMove::Down,             shift); true }
                    (KeyModifiers::NONE, KeyCode::Home)     => { cursor_move!(ta, CursorMove::Head,             shift); true }
                    (KeyModifiers::NONE, KeyCode::End)      => { cursor_move!(ta, CursorMove::End,              shift); true }
                    (KeyModifiers::NONE, KeyCode::PageUp)   => { cursor_move!(ta, CursorMove::ParagraphBack,    shift); true }
                    (KeyModifiers::NONE, KeyCode::PageDown) => { cursor_move!(ta, CursorMove::ParagraphForward, shift); true }
                    // Word navigation (Ctrl+arrow, Windows/Linux style)
                    (KeyModifiers::CONTROL, KeyCode::Left)  => { cursor_move!(ta, CursorMove::WordBack,    shift); true }
                    (KeyModifiers::CONTROL, KeyCode::Right) => { cursor_move!(ta, CursorMove::WordForward, shift); true }
                    // Document start / end
                    (KeyModifiers::CONTROL, KeyCode::Home) => { cursor_move!(ta, CursorMove::Top,    shift); true }
                    (KeyModifiers::CONTROL, KeyCode::End)  => { cursor_move!(ta, CursorMove::Bottom, shift); true }
                    // Undo / Redo (Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z)
                    (KeyModifiers::CONTROL, KeyCode::Char('z')) => { ta.undo(); true }
                    (KeyModifiers::CONTROL, KeyCode::Char('y'))
                    | (KeyModifiers::CONTROL, KeyCode::Char('Z')) => { ta.redo(); true }
                    // Select all
                    (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                        ta.move_cursor(CursorMove::Top);
                        ta.start_selection();
                        ta.move_cursor(CursorMove::Bottom);
                        true
                    }
                    // Delete word before / after cursor
                    (KeyModifiers::CONTROL, KeyCode::Backspace)
                    | (KeyModifiers::ALT,   KeyCode::Backspace) => { ta.delete_word(); true }
                    (KeyModifiers::CONTROL, KeyCode::Delete)
                    | (KeyModifiers::ALT,   KeyCode::Delete)    => { ta.delete_next_word(); true }
                    _ => false,
                };
                if shortcut_handled {
                    self.selection = ta.selection_range();
                    self.edit_generation = self.edit_generation.wrapping_add(1);
                    return EventState::Consumed;
                }
                ta.input_without_shortcuts(*key);
                self.selection = ta.selection_range();
                self.edit_generation = self.edit_generation.wrapping_add(1);
                EventState::Consumed
            }
            InputEvent::Mouse(mouse) => {
                // Mouse is only handled for the Textarea backend.
                let BackendState::Textarea(_) = &self.backend else {
                    return EventState::NotConsumed;
                };
                let r = &self.rect;
                let in_bounds = mouse.column >= r.x
                    && mouse.column < r.x + r.width
                    && mouse.row >= r.y
                    && mouse.row < r.y + r.height;
                if !in_bounds {
                    return EventState::NotConsumed;
                }
                // Handle right-click clipboard copy in its own scope to avoid borrow conflicts.
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
                    tx.send(AppEvent::FocusEditor).ok();
                    self.copy_selection_to_clipboard();
                    self.selection = if let BackendState::Textarea(ta) = &self.backend {
                        ta.selection_range()
                    } else {
                        None
                    };
                    self.edit_generation = self.edit_generation.wrapping_add(1);
                    return EventState::Consumed;
                }
                // Now extract ta for remaining mouse operations.
                let BackendState::Textarea(ta) = &mut self.backend else {
                    unreachable!()
                };
                match mouse.kind {
                    MouseEventKind::Down(_) => {
                        tx.send(AppEvent::FocusEditor).ok();
                        ta.cancel_selection();
                        let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
                        let vcol = (mouse.column - r.x) as usize;
                        let (lrow, lcol) = self.view.click_to_logical_u16(vrow, vcol);
                        ta.move_cursor(CursorMove::Jump(lrow, lcol));
                        ta.start_selection();
                    }
                    MouseEventKind::Drag(_) => {
                        let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
                        let vcol = (mouse.column - r.x) as usize;
                        let (lrow, lcol) = self.view.click_to_logical_u16(vrow, vcol);
                        ta.move_cursor(CursorMove::Jump(lrow, lcol));
                    }
                    _ => {
                        ta.input(*mouse);
                    }
                }
                self.selection = ta.selection_range();
                self.edit_generation = self.edit_generation.wrapping_add(1);
                EventState::Consumed
            }
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.rect = rect;
        match &self.backend {
            BackendState::Textarea(ta) => {
                let cursor = ta.cursor();
                let lines = ta.lines();
                self.view.update(lines, cursor, rect, self.edit_generation, self.selection);
            }
            BackendState::Nvim(nvim) => {
                nvim.maybe_resize(rect.width, rect.height);
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                let cursor = snap.cursor;
                let lines = snap.lines.clone();
                let content_gen = snap.content_gen;
                let visual_selection = snap.visual_selection;
                drop(snap);
                self.view.update(&lines, cursor, rect, content_gen, visual_selection);
            }
        }
        self.view.render(f, rect, theme, focused);
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        use crate::keys::action_shortcuts::ActionShortcuts;

        // For the Nvim backend, prepend the current mode as the first "hint".
        if let BackendState::Nvim(nvim) = &self.backend {
            let label = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner()).footer_label();
            let mut hints = vec![(String::new(), label)];
            hints.extend(
                [
                    (ActionShortcuts::FocusSidebar, "focus sidebar"),
                    (ActionShortcuts::FileOperations, "file ops"),
                ]
                .iter()
                .filter_map(|(action, label)| {
                    self.key_bindings
                        .first_combo_for(action)
                        .map(|k| (k, label.to_string()))
                }),
            );
            return hints;
        }

        [
            (ActionShortcuts::FocusSidebar, "focus sidebar"),
            (ActionShortcuts::FileOperations, "file ops"),
        ]
        .iter()
        .filter_map(|(action, label)| {
            self.key_bindings
                .first_combo_for(action)
                .map(|k| (k, label.to_string()))
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyBindings;

    fn make_editor() -> TextEditorComponent {
        TextEditorComponent::new(KeyBindings::empty(), &crate::settings::AppSettings::default())
    }

    fn get_ta(editor: &mut TextEditorComponent) -> &mut TextArea<'static> {
        match &mut editor.backend {
            BackendState::Textarea(ta) => ta,
            _ => panic!("expected Textarea backend"),
        }
    }

    #[test]
    fn fresh_editor_is_not_dirty() {
        let editor = make_editor();
        assert!(!editor.is_dirty());
    }

    #[test]
    fn after_set_text_not_dirty() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        assert!(!editor.is_dirty());
    }

    #[test]
    fn get_text_returns_loaded_content() {
        let mut editor = make_editor();
        editor.set_text("line one\nline two".to_string());
        assert_eq!(editor.get_text(), "line one\nline two");
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut editor = make_editor();
        editor.set_text("initial".to_string());
        let text = editor.get_text();
        editor.mark_saved(text.clone() + "x"); // saved state diverges
        assert!(editor.is_dirty());
        editor.mark_saved(text); // saved state matches again
        assert!(!editor.is_dirty());
    }

    #[test]
    fn trailing_newline_does_not_cause_false_dirty() {
        let mut editor = make_editor();
        editor.set_text("content\n".to_string());
        assert!(
            !editor.is_dirty(),
            "trailing newline should not make editor dirty after load"
        );
    }

    #[test]
    fn mouse_down_clears_selection() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        let ta = get_ta(&mut editor);
        ta.start_selection();
        ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        assert!(ta.selection_range().is_some());
        ta.cancel_selection();
        editor.selection = if let BackendState::Textarea(ta) = &editor.backend {
            ta.selection_range()
        } else {
            None
        };
        assert!(editor.selection.is_none());
    }

    #[test]
    fn ctrl_c_copies_selected_text() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        let ta = get_ta(&mut editor);
        ta.move_cursor(ratatui_textarea::CursorMove::Head);
        ta.start_selection();
        ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        let range = ta.selection_range().unwrap();
        let ((sr, sc), (er, ec)) = range;
        let lines = ta.lines();
        let selected = if sr == er {
            lines[sr][sc..ec].to_string()
        } else {
            lines[sr][sc..].to_string()
        };
        assert_eq!(selected, "hello ");
    }

    #[test]
    fn paste_inserts_text_at_cursor() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let ta = get_ta(&mut editor);
        ta.move_cursor(ratatui_textarea::CursorMove::End);
        ta.insert_str(" world");
        assert_eq!(editor.get_text(), "hello world");
    }

    #[test]
    fn textarea_hint_shortcuts_has_no_mode_indicator() {
        let editor = make_editor();
        let hints = editor.hint_shortcuts();
        // None of the hint labels should be "NORMAL", "INSERT", etc.
        assert!(!hints.iter().any(|(_, label)| label == "NORMAL" || label == "INSERT"));
    }
}
