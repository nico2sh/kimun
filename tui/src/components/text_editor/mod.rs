pub mod markdown;
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

use self::view::MarkdownEditorView;

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::events::AppTx;
use crate::components::events::InputEvent;
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::themes::Theme;

pub struct TextEditorComponent {
    text_area: TextArea<'static>,
    /// Tracks the rendered rect to map mouse click coordinates.
    rect: Rect,
    key_bindings: KeyBindings,
    last_saved_text: String,
    view: MarkdownEditorView,
    /// Incremented on every mutating input event. Passed to `view.update()` so the view
    /// can skip the expensive lines clone and parse-cache rebuild on idle frames.
    edit_generation: u64,
    /// Current selection range in logical (row, byte-col) coordinates, kept in sync with
    /// `text_area.selection_range()` after every input event.
    selection: Option<((usize, usize), (usize, usize))>,
    /// System clipboard handle. `None` if the clipboard is unavailable (e.g. headless CI).
    clipboard: Option<Clipboard>,
}

impl TextEditorComponent {
    pub fn new(key_bindings: KeyBindings) -> Self {
        Self {
            text_area: TextArea::default(),
            rect: Rect::default(),
            key_bindings,
            last_saved_text: String::new(),
            view: MarkdownEditorView::new(),
            edit_generation: 0,
            selection: None,
            clipboard: Clipboard::new().ok(),
        }
    }

    pub fn lines(&self) -> &[String] {
        self.text_area.lines()
    }

    pub fn set_text(&mut self, text: String) {
        let lines = text.lines();
        self.text_area = TextArea::from(lines);
        self.edit_generation = self.edit_generation.wrapping_add(1);
        let reconstructed = self.get_text();
        self.mark_saved(reconstructed);
    }

    pub fn get_text(&self) -> String {
        self.text_area.lines().join("\n")
    }

    pub fn mark_saved(&mut self, text: String) {
        self.last_saved_text = text;
    }

    pub fn is_dirty(&self) -> bool {
        self.get_text() != self.last_saved_text
    }

    /// Returns the raw link target under the cursor, or `None` if the cursor
    /// is not inside a wikilink or markdown link span.
    pub fn link_at_cursor(&self) -> Option<String> {
        let (row, col) = self.text_area.cursor();
        let line = self.text_area.lines().get(row)?;
        kimun_core::note::link_char_spans(line)
            .into_iter()
            .find(|s| s.start <= col && col < s.end)
            .map(|s| s.target)
    }

    /// Copy selected text to the system clipboard.
    fn copy_selection_to_clipboard(&mut self) {
        let Some(((sr, sc), (er, ec))) = self.text_area.selection_range() else {
            return;
        };
        let lines = self.text_area.lines();
        let text = if sr == er {
            lines[sr].get(sc..ec).unwrap_or("").to_string()
        } else {
            let mut parts = vec![lines[sr].get(sc..).unwrap_or("").to_string()];
            for row in (sr + 1)..er {
                parts.push(lines[row].to_string());
            }
            parts.push(lines[er].get(..ec).unwrap_or("").to_string());
            parts.join("\n")
        };
        if let Some(cb) = &mut self.clipboard {
            let _ = cb.set_text(text);
        }
    }

    /// Paste text from the system clipboard at the cursor, replacing any active selection.
    fn paste_from_clipboard(&mut self) {
        let text = match &mut self.clipboard {
            Some(cb) => match cb.get_text() {
                Ok(t) => t,
                Err(_) => return,
            },
            None => return,
        };
        if text.is_empty() {
            return;
        }
        // If a selection is active, delete it first.
        if self.text_area.selection_range().is_some() {
            self.text_area.cut();
        }
        self.text_area.insert_str(&text);
        self.selection = self.text_area.selection_range();
        self.edit_generation = self.edit_generation.wrapping_add(1);
        // Note: selection is synced here because paste is an early-exit path in handle_input.
    }
}

impl Component for TextEditorComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => {
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
                        _ => {}
                    }
                }

                // macOS-style navigation shortcuts not handled by ratatui-textarea.
                //
                // Alt+Left/Right (Option key on macOS) — word jump.
                // ratatui-textarea handles Alt+b/f but not Alt+Arrow, so we map them here.
                //
                // Super+Arrow (Cmd key on macOS) — line/document navigation.
                // Most terminal emulators on macOS do NOT forward the Cmd modifier; if
                // they do (e.g. via kitty/iTerm configured key bindings) this catches it.
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                let handled = match (key.modifiers & !KeyModifiers::SHIFT, key.code) {
                    (KeyModifiers::ALT, KeyCode::Left) => {
                        cursor_move!(self.text_area, CursorMove::WordBack, shift);
                        true
                    }
                    (KeyModifiers::ALT, KeyCode::Right) => {
                        cursor_move!(self.text_area, CursorMove::WordForward, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Left) => {
                        cursor_move!(self.text_area, CursorMove::Head, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Right) => {
                        cursor_move!(self.text_area, CursorMove::End, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Up) => {
                        cursor_move!(self.text_area, CursorMove::Top, shift);
                        true
                    }
                    (KeyModifiers::SUPER, KeyCode::Down) => {
                        cursor_move!(self.text_area, CursorMove::Bottom, shift);
                        true
                    }
                    _ => false,
                };
                if handled {
                    self.selection = self.text_area.selection_range();
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
                self.text_area.input(*key);
                self.selection = self.text_area.selection_range();
                self.edit_generation = self.edit_generation.wrapping_add(1);
                EventState::Consumed
            }
            InputEvent::Mouse(mouse) => {
                let r = &self.rect;
                let in_bounds = mouse.column >= r.x
                    && mouse.column < r.x + r.width
                    && mouse.row >= r.y
                    && mouse.row < r.y + r.height;
                if !in_bounds {
                    return EventState::NotConsumed;
                }
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Right) => {
                        tx.send(AppEvent::FocusEditor).ok();
                        self.copy_selection_to_clipboard();
                    }
                    MouseEventKind::Down(_) => {
                        tx.send(AppEvent::FocusEditor).ok();
                        self.text_area.cancel_selection();
                        let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
                        let vcol = (mouse.column - r.x) as usize;
                        let (lrow, lcol) = self.view.click_to_logical_u16(vrow, vcol);
                        self.text_area.move_cursor(CursorMove::Jump(lrow, lcol));
                        self.text_area.start_selection();
                    }
                    MouseEventKind::Drag(_) => {
                        let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
                        let vcol = (mouse.column - r.x) as usize;
                        let (lrow, lcol) = self.view.click_to_logical_u16(vrow, vcol);
                        self.text_area.move_cursor(CursorMove::Jump(lrow, lcol));
                    }
                    _ => {
                        self.text_area.input(*mouse);
                    }
                }
                self.selection = self.text_area.selection_range();
                self.edit_generation = self.edit_generation.wrapping_add(1);
                EventState::Consumed
            }
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.rect = rect;
        let cursor = self.text_area.cursor();
        self.view
            .update(self.text_area.lines(), cursor, rect, self.edit_generation, self.selection);
        self.view.render(f, rect, theme, focused);
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        use crate::keys::action_shortcuts::ActionShortcuts;
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
        TextEditorComponent::new(KeyBindings::empty())
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
        editor.text_area.start_selection();
        editor.text_area.move_cursor(ratatui_textarea::CursorMove::WordForward);
        assert!(editor.text_area.selection_range().is_some());
        editor.text_area.cancel_selection();
        editor.selection = editor.text_area.selection_range();
        assert!(editor.selection.is_none());
    }

    #[test]
    fn ctrl_c_copies_selected_text() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        editor.text_area.move_cursor(ratatui_textarea::CursorMove::Head);
        editor.text_area.start_selection();
        editor.text_area.move_cursor(ratatui_textarea::CursorMove::WordForward);
        let range = editor.text_area.selection_range().unwrap();
        let ((sr, sc), (er, ec)) = range;
        let lines = editor.text_area.lines();
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
        editor.text_area.move_cursor(ratatui_textarea::CursorMove::End);
        editor.text_area.insert_str(" world");
        assert_eq!(editor.get_text(), "hello world");
    }
}
