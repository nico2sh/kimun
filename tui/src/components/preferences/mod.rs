pub mod appearance_section;
pub mod display_section;
pub mod editor_section;
pub mod indexing_section;
pub mod server_section;
pub mod sorting_section;
pub mod theme_picker;
pub mod vault_section;
pub mod workspaces_section;

use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::components::hint_row::is_left_press;

/// What a click in a preferences section asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionClick {
    /// A row: select it. Clicking the row that is already selected runs
    /// the row's own key instead (the "click it again" list convention).
    Row(usize),
    /// A control on a row (`[x]`, `◀`, `▶`, `[Name]`, a button): select the
    /// row and run `key` at once.
    Control { row: usize, key: KeyCode },
}

/// The mouse model shared by every preferences section: rows select,
/// controls act, the wheel moves the selection. A click is always turned
/// into the key it stands for, so the screen runs it through the same path
/// a key takes — settings write-back included.
///
/// Rows are list rows and get no hover; controls are click targets and
/// register for it (`components::clickable`).
#[derive(Default)]
pub struct ClickMap {
    targets: Vec<(Rect, SectionClick)>,
}

/// What the section should do with a mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionMouse {
    /// Select this row (no key).
    Select(usize),
    /// Select this row, then run this key.
    Key(usize, KeyEvent),
    /// The wheel: run this key (Up/Down) against the current selection.
    Wheel(KeyEvent),
    None,
}

impl ClickMap {
    /// Forget the last render's targets — call at the top of `render`.
    pub fn clear(&mut self) {
        self.targets.clear();
    }

    /// Record a selectable row.
    pub fn row(&mut self, rect: Rect, row: usize) {
        if !rect.is_empty() {
            self.targets.push((rect, SectionClick::Row(row)));
        }
    }

    /// Record a control that runs `key` for `row`; it gets hover.
    pub fn control(&mut self, rect: Rect, row: usize, key: KeyCode) {
        if !rect.is_empty() {
            crate::components::clickable::register(rect);
            self.targets
                .push((rect, SectionClick::Control { row, key }));
        }
    }

    /// Resolve a mouse event. `selected` is the section's current row and
    /// `activate` the key a click on the already-selected row runs (`None`
    /// when re-clicking does nothing, e.g. a theme list).
    pub fn resolve(
        &self,
        m: &MouseEvent,
        selected: Option<usize>,
        activate: Option<KeyCode>,
    ) -> SectionMouse {
        match m.kind {
            MouseEventKind::ScrollUp => return SectionMouse::Wheel(KeyEvent::from(KeyCode::Up)),
            MouseEventKind::ScrollDown => {
                return SectionMouse::Wheel(KeyEvent::from(KeyCode::Down));
            }
            _ => {}
        }
        if !is_left_press(m) {
            return SectionMouse::None;
        }
        let pos = Position::new(m.column, m.row);
        // Controls sit on top of their row: the last match wins.
        match self.targets.iter().rev().find(|(r, _)| r.contains(pos)) {
            Some((_, SectionClick::Control { row, key })) => {
                SectionMouse::Key(*row, KeyEvent::from(*key))
            }
            Some((_, SectionClick::Row(row))) => match activate {
                Some(key) if selected == Some(*row) => SectionMouse::Key(*row, KeyEvent::from(key)),
                _ => SectionMouse::Select(*row),
            },
            None => SectionMouse::None,
        }
    }
}

/// A one-line rect for the text at `col` cells into `row`, `width` wide,
/// clipped to `row`.
pub fn text_rect(row: Rect, col: u16, width: u16) -> Rect {
    let x = row.x.saturating_add(col);
    if x >= row.right() {
        return Rect::default();
    }
    Rect::new(x, row.y, width.min(row.right() - x), 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyModifiers, MouseButton};

    fn press(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn rows_select_controls_act_and_reclick_activates() {
        let mut map = ClickMap::default();
        map.row(Rect::new(0, 0, 20, 1), 0);
        map.row(Rect::new(0, 1, 20, 1), 1);
        map.control(Rect::new(10, 1, 3, 1), 1, KeyCode::Right);
        let enter = Some(KeyCode::Enter);
        assert_eq!(
            map.resolve(&press(2, 1), Some(0), enter),
            SectionMouse::Select(1)
        );
        assert_eq!(
            map.resolve(&press(2, 1), Some(1), enter),
            SectionMouse::Key(1, KeyEvent::from(KeyCode::Enter)),
            "re-clicking the selected row runs its key"
        );
        assert_eq!(
            map.resolve(&press(11, 1), Some(0), enter),
            SectionMouse::Key(1, KeyEvent::from(KeyCode::Right)),
            "a control wins over its row"
        );
        assert_eq!(
            map.resolve(&press(2, 1), Some(1), None),
            SectionMouse::Select(1)
        );
        assert_eq!(
            map.resolve(&press(2, 5), Some(1), enter),
            SectionMouse::None
        );
    }

    #[test]
    fn text_rect_clips_to_the_row() {
        let row = Rect::new(5, 2, 10, 1);
        assert_eq!(text_rect(row, 2, 4), Rect::new(7, 2, 4, 1));
        assert_eq!(text_rect(row, 8, 4), Rect::new(13, 2, 2, 1));
        assert_eq!(text_rect(row, 12, 4), Rect::default());
    }
}
