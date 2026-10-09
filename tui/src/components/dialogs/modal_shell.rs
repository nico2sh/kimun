use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::Rect;

use crate::components::clickable::is_press_outside;
use crate::components::hint_row::HintRow;

/// Pointer plumbing shared by the dismissable popups: it remembers the popup's
/// rect from the last render, and turns the two universal clicks into the key
/// the dialog already handles — a press outside is `Esc`, a press on a hint
/// chip is that chip's key. Dialogs that must not be dismissed by a stray click
/// (delete, rename, anything holding typed input) simply don't use it.
#[derive(Debug, Default)]
pub struct ModalShell {
    popup_rect: Rect,
}

impl ModalShell {
    /// Record the popup's outer rect; call once per render.
    pub fn set(&mut self, rect: Rect) {
        self.popup_rect = rect;
    }

    /// The key a mouse event stands for: `Esc` for a press outside the popup,
    /// else the key of the hint chip under a left press (first row in `hints`
    /// wins). `None` leaves the event for the dialog's own targets.
    pub fn pointer_key(&self, m: &MouseEvent, hints: &[&HintRow]) -> Option<KeyEvent> {
        if is_press_outside(m, self.popup_rect) {
            return Some(KeyEvent::from(KeyCode::Esc));
        }
        hints.iter().find_map(|h| h.hit(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

    fn press(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn outside_press_is_esc_inside_is_none() {
        let mut shell = ModalShell::default();
        assert_eq!(
            shell.pointer_key(&press(0, 0), &[]),
            None,
            "not rendered yet"
        );
        shell.set(Rect::new(10, 5, 20, 8));
        assert_eq!(
            shell.pointer_key(&press(0, 0), &[]).map(|k| k.code),
            Some(KeyCode::Esc)
        );
        assert_eq!(shell.pointer_key(&press(12, 6), &[]), None);
    }

    #[test]
    fn hint_chip_press_yields_its_key_and_outside_wins() {
        use ratatui::style::Style;
        use ratatui::{Terminal, backend::TestBackend};
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        let mut hints = HintRow::new(&[(KeyCode::Esc, "Esc", "Close")]);
        let mut t = Terminal::new(TestBackend::new(40, 5)).unwrap();
        t.draw(|f| hints.render(f, Rect::new(10, 2, 20, 1), Style::default(), &theme))
            .unwrap();
        let (col, row) =
            crate::test_support::find_text(t.backend().buffer(), "Close").expect("chip drawn");
        let mut shell = ModalShell::default();
        shell.set(Rect::new(8, 1, 30, 3));
        let key = |m| shell.pointer_key(&m, &[&hints]).map(|k| k.code);
        assert_eq!(key(press(col, row)), Some(KeyCode::Esc), "chip key");
        assert_eq!(key(press(0, 0)), Some(KeyCode::Esc), "outside is Esc");
        assert_eq!(
            key(press(col, row.saturating_sub(1))),
            None,
            "inside, off chip"
        );
    }
}
