//! Built-in vim emulation: a modal input interpreter over a `TextArea`.
//! Pure over `&mut TextArea` — no component state, no async (adr/0012).

use super::snapshot::EditorMode;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::{CursorMove, TextArea};

/// What a key did, so the host can bump the right revision counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimKeyOutcome {
    /// Buffer text changed — host calls `bump_content()`.
    TextMutated,
    /// Only the cursor/selection moved — host refreshes view, not content.
    CursorOnly,
    /// Nothing happened (unmapped key in Normal mode).
    NoOp,
    /// Insert mode: defer to the existing `handle_textarea_key` path.
    PassThrough,
}

/// Modal vim state layered over the textarea buffer.
#[derive(Debug)]
pub struct VimEngine {
    mode: EditorMode,
}

impl Default for VimEngine {
    fn default() -> Self {
        // Notes open in Normal mode (vim convention).
        Self { mode: EditorMode::Normal }
    }
}

impl VimEngine {
    pub fn mode(&self) -> &EditorMode {
        &self.mode
    }

    /// Footer label for the current mode (e.g. "NORMAL").
    pub fn mode_label(&self) -> String {
        self.mode.label().to_string()
    }

    pub fn reset_to_normal(&mut self) {
        self.mode = EditorMode::Normal;
    }

    /// Interpret one key. In Insert mode everything except `Esc` is
    /// `PassThrough` (the host runs the existing direct textarea path).
    /// In Normal mode, motions move the cursor and the insert-entry keys
    /// switch to Insert mode.
    pub fn handle_key(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        match self.mode {
            EditorMode::Insert => self.handle_insert(key, ta),
            _ => self.handle_normal(key, ta),
        }
    }

    fn handle_insert(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if key.code == KeyCode::Esc {
            self.mode = EditorMode::Normal;
            if super::cursor_tuple(ta).1 > 0 {
                ta.move_cursor(CursorMove::Back);
            }
            return VimKeyOutcome::CursorOnly;
        }
        VimKeyOutcome::PassThrough
    }

    fn handle_normal(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        let plain = key.modifiers == KeyModifiers::NONE
            || key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Char(c) if plain => self.normal_char(c, ta),
            KeyCode::Left => self.motion(CursorMove::Back, ta),
            KeyCode::Right => self.motion(CursorMove::Forward, ta),
            KeyCode::Up => self.motion(CursorMove::Up, ta),
            KeyCode::Down => self.motion(CursorMove::Down, ta),
            _ => VimKeyOutcome::NoOp,
        }
    }

    fn motion(&self, m: CursorMove, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        ta.move_cursor(m);
        VimKeyOutcome::CursorOnly
    }

    fn normal_char(&mut self, c: char, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        match c {
            'h' => self.motion(CursorMove::Back, ta),
            'l' => self.motion(CursorMove::Forward, ta),
            'k' => self.motion(CursorMove::Up, ta),
            'j' => self.motion(CursorMove::Down, ta),
            'w' => self.motion(CursorMove::WordForward, ta),
            'b' => self.motion(CursorMove::WordBack, ta),
            'e' => self.motion(CursorMove::WordEnd, ta),
            '0' => self.motion(CursorMove::Head, ta),
            '$' => self.motion(CursorMove::End, ta),
            '^' => self.motion(CursorMove::Head, ta), // refined to first-non-blank in Plan 2
            'G' => self.motion(CursorMove::Bottom, ta),
            'i' => self.enter_insert(ta, None),
            'a' => self.enter_insert(ta, Some(CursorMove::Forward)),
            'I' => self.enter_insert(ta, Some(CursorMove::Head)),
            'A' => self.enter_insert(ta, Some(CursorMove::End)),
            'o' => self.open_line(ta, false),
            'O' => self.open_line(ta, true),
            _ => VimKeyOutcome::NoOp,
        }
    }

    fn enter_insert(
        &mut self,
        ta: &mut TextArea<'static>,
        pre_move: Option<CursorMove>,
    ) -> VimKeyOutcome {
        if let Some(m) = pre_move {
            ta.move_cursor(m);
        }
        self.mode = EditorMode::Insert;
        VimKeyOutcome::CursorOnly
    }

    fn open_line(&mut self, ta: &mut TextArea<'static>, above: bool) -> VimKeyOutcome {
        if above {
            ta.move_cursor(CursorMove::Head);
            ta.insert_newline();
            ta.move_cursor(CursorMove::Up);
        } else {
            ta.move_cursor(CursorMove::End);
            ta.insert_newline();
        }
        self.mode = EditorMode::Insert;
        VimKeyOutcome::TextMutated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui_textarea::TextArea;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn ta() -> TextArea<'static> {
        TextArea::from(["hello world", "second line"])
    }

    #[test]
    fn i_enters_insert_mode() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('i'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(out, VimKeyOutcome::CursorOnly);
    }

    #[test]
    fn esc_returns_to_normal_and_steps_back() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('i'), &mut t);
        t.move_cursor(ratatui_textarea::CursorMove::Forward);
        t.move_cursor(ratatui_textarea::CursorMove::Forward);
        let col_before = super::super::cursor_tuple(&t).1;
        let out = e.handle_key(&esc(), &mut t);
        assert_eq!(*e.mode(), EditorMode::Normal);
        assert_eq!(out, VimKeyOutcome::CursorOnly);
        assert_eq!(super::super::cursor_tuple(&t).1, col_before - 1);
    }

    #[test]
    fn insert_mode_passes_through() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('i'), &mut t);
        let out = e.handle_key(&key('x'), &mut t);
        assert_eq!(out, VimKeyOutcome::PassThrough);
    }

    #[test]
    fn l_moves_right_cursor_only() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('l'), &mut t);
        assert_eq!(out, VimKeyOutcome::CursorOnly);
        assert_eq!(super::super::cursor_tuple(&t), (0, 1));
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn a_enters_insert_after_cursor() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('a'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(super::super::cursor_tuple(&t), (0, 1));
    }

    #[test]
    fn o_opens_line_below_in_insert() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('o'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(out, VimKeyOutcome::TextMutated);
        assert_eq!(t.lines().len(), 3);
        assert_eq!(super::super::cursor_tuple(&t).0, 1);
    }

    #[test]
    fn reset_returns_to_normal_from_insert() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('i'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        e.reset_to_normal();
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn unknown_normal_key_is_noop() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('z'), &mut t);
        assert_eq!(out, VimKeyOutcome::NoOp);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }
}
