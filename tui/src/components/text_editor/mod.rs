pub mod markdown;
pub mod view;
pub mod word_wrap;

use ratatui::Frame;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

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
}

impl TextEditorComponent {
    pub fn new(key_bindings: KeyBindings) -> Self {
        Self {
            text_area: TextArea::default(),
            rect: Rect::default(),
            key_bindings,
            last_saved_text: String::new(),
            view: MarkdownEditorView::new(),
        }
    }

    pub fn lines(&self) -> &[String] {
        self.text_area.lines()
    }

    pub fn set_text(&mut self, text: String) {
        let lines = text.lines();
        self.text_area = TextArea::from(lines);
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
}

impl Component for TextEditorComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => {
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
                    MouseEventKind::Down(_) => {
                        tx.send(AppEvent::FocusEditor).ok();
                        let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
                        let vcol = (mouse.column - r.x) as usize;
                        let (lrow, lcol) = self.view.visual_to_logical_u16(vrow, vcol);
                        self.text_area.move_cursor(CursorMove::Jump(lrow, lcol));
                    }
                    _ => {
                        self.text_area.input(*mouse);
                    }
                }
                EventState::Consumed
            }
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.rect = rect;
        let lines: Vec<String> = self.text_area.lines().to_vec();
        let cursor = self.text_area.cursor();
        self.view.update(&lines, cursor, rect);
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
}
