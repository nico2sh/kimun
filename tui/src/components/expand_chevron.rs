//! The shared expand control of the list screens: one glyph in the last
//! column of a row. `›` grows the row's reach, `‹` shrinks it.

use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::{Frame, layout::Rect, text::Span, widgets::Paragraph};

use crate::components::clickable;
use crate::components::preview_pane::{PreviewPane, Step};
use crate::keys::key_combo::{KeyCombo, KeyModifiers};
use crate::keys::key_strike::KeyStrike;
use crate::settings::themes::Theme;

/// Which way the control moves the reach of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Grow,
    Shrink,
}

impl Reach {
    /// `collapsed` is the shortest state, `full` the longest; neither is the
    /// middle (context) state. Both at once cannot happen and gives `None`.
    /// The caller decides what to show when no row is selected.
    pub fn for_state(collapsed: bool, full: bool) -> Option<Reach> {
        match (collapsed, full) {
            (true, true) => None,
            (_, true) => Some(Reach::Shrink),
            _ => Some(Reach::Grow),
        }
    }
}

pub fn glyph(reach: Reach) -> &'static str {
    match reach {
        Reach::Grow => "›",
        Reach::Shrink => "‹",
    }
}

/// Draw the glyph in the last column of `row`, register that cell as
/// clickable and return it. A zero-width row draws nothing.
pub fn paint(f: &mut Frame, row: Rect, reach: Reach, theme: &Theme) -> Rect {
    if row.width == 0 || row.height == 0 {
        return Rect::new(row.x, row.y, 0, 0);
    }
    let cell = Rect::new(row.right() - 1, row.y, 1, 1);
    f.render_widget(
        Paragraph::new(Span::styled(glyph(reach), theme.action())),
        cell,
    );
    crate::components::clickable::register(cell);
    cell
}

/// Key and action labels for the hint row.
pub fn hint(reach: Reach) -> (String, String) {
    match reach {
        Reach::Grow => ("Alt+→".to_string(), "Expand".to_string()),
        Reach::Shrink => ("Alt+←".to_string(), "Shrink".to_string()),
    }
}

/// What a left press on the Full preview header hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderHit {
    /// The shrink chevron: step back one state.
    Chevron,
    /// The rest of the header (the title): collapse, like Enter.
    Title,
}

impl HeaderHit {
    /// The step a press on this part of the header takes.
    pub fn step(self) -> Step {
        match self {
            HeaderHit::Chevron => Step::Shrink,
            HeaderHit::Title => Step::Toggle,
        }
    }
}

/// The expand control one list host owns: the row chevron it drew last,
/// and the Alt+Right / Alt+Left chords that step the preview. FIND, SEM
/// and Sources share it, so the chevron's keys, hit-tests and paint
/// cannot drift between them.
#[derive(Debug)]
pub struct ExpandControl {
    /// The chevron on the selected row from the last render; empty when no
    /// row shows it.
    chevron: Rect,
    grow: KeyCombo,
    shrink: KeyCombo,
}

impl Default for ExpandControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ExpandControl {
    pub fn new() -> Self {
        let alt = |key| KeyCombo::new(KeyModifiers::new().and_alt(), key);
        Self {
            chevron: Rect::default(),
            grow: alt(KeyStrike::ArrowRight),
            shrink: alt(KeyStrike::ArrowLeft),
        }
    }

    /// The Alt+Right and Alt+Left chords, for a list engine to intercept.
    pub fn combos(&self) -> [KeyCombo; 2] {
        [self.grow, self.shrink]
    }

    /// Which way an intercepted chord steps the preview, if it is one of
    /// the control's chords.
    pub fn dir_of(&self, combo: &KeyCombo) -> Option<Reach> {
        if *combo == self.grow {
            Some(Reach::Grow)
        } else if *combo == self.shrink {
            Some(Reach::Shrink)
        } else {
            None
        }
    }

    /// [`Self::dir_of`] for a raw key event.
    pub fn dir_of_key(&self, key: &KeyEvent) -> Option<Reach> {
        crate::keys::key_event_to_combo(key).and_then(|c| self.dir_of(&c))
    }

    /// A left press on the row chevron as last drawn.
    pub fn hit_row(&self, mouse: &MouseEvent) -> bool {
        clickable::is_left_press(mouse) && clickable::rect_hit(self.chevron, mouse)
    }

    /// A left press on `pane`'s Full header: its chevron first, then the
    /// title. `None` outside the header (or outside Full: the header rects
    /// are empty then).
    pub fn hit_header(&self, pane: &PreviewPane, mouse: &MouseEvent) -> Option<HeaderHit> {
        if !clickable::is_left_press(mouse) {
            return None;
        }
        if clickable::rect_hit(pane.full_chevron_rect(), mouse) {
            Some(HeaderHit::Chevron)
        } else if clickable::rect_hit(pane.full_header_rect(), mouse) {
            Some(HeaderHit::Title)
        } else {
            None
        }
    }

    /// Paint the chevron at the end of `row` (the selected row, where the
    /// list is drawn) in the direction `pane`'s state calls for, and record
    /// it. No row, or Full (the list is gone), paints nothing.
    pub fn paint(&mut self, f: &mut Frame, row: Option<Rect>, pane: &PreviewPane, theme: &Theme) {
        self.chevron = Rect::default();
        if let Some(row) = row
            && let Some(reach) = Reach::for_state(pane.is_collapsed(), pane.is_full())
        {
            self.chevron = paint(f, row, reach, theme);
        }
    }

    /// The hint-row entry for the chevron's key, while a row is selected.
    pub fn hint(&self, pane: &PreviewPane, selected: bool) -> Option<(String, String)> {
        if !selected {
            return None;
        }
        Reach::for_state(pane.is_collapsed(), pane.is_full()).map(hint)
    }

    /// Drop the recorded chevron: the layout changed, so until the next
    /// render no click may land on it.
    pub fn clear(&mut self) {
        self.chevron = Rect::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::themes::Theme;
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    #[test]
    fn for_state_grows_collapsed_and_context_and_shrinks_full() {
        assert_eq!(Reach::for_state(true, false), Some(Reach::Grow));
        assert_eq!(Reach::for_state(false, false), Some(Reach::Grow));
        assert_eq!(Reach::for_state(false, true), Some(Reach::Shrink));
    }

    #[test]
    fn glyphs_and_hints() {
        assert_eq!(glyph(Reach::Grow), "›");
        assert_eq!(glyph(Reach::Shrink), "‹");
        assert_eq!(hint(Reach::Grow), ("Alt+→".into(), "Expand".into()));
        assert_eq!(hint(Reach::Shrink), ("Alt+←".into(), "Shrink".into()));
    }

    #[test]
    fn paint_puts_the_glyph_in_the_last_column() {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(10, 3)).unwrap();
        let mut got = Rect::default();
        t.draw(|f| got = paint(f, Rect::new(2, 1, 6, 1), Reach::Grow, &theme))
            .unwrap();
        assert_eq!(got, Rect::new(7, 1, 1, 1));
        let buf = t.backend().buffer();
        assert_eq!(buf[(7, 1)].symbol(), "›");
        assert_eq!(buf[(6, 1)].symbol(), " ");
    }

    fn press(x: u16, y: u16) -> MouseEvent {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn the_control_maps_its_chords_and_nothing_else() {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let c = ExpandControl::new();
        let [grow, shrink] = c.combos();
        assert_eq!(c.dir_of(&grow), Some(Reach::Grow));
        assert_eq!(c.dir_of(&shrink), Some(Reach::Shrink));
        let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
        assert_eq!(c.dir_of_key(&alt(KeyCode::Right)), Some(Reach::Grow));
        assert_eq!(c.dir_of_key(&alt(KeyCode::Left)), Some(Reach::Shrink));
        let plain = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(c.dir_of_key(&plain), None);
    }

    #[test]
    fn the_row_chevron_is_hit_until_cleared() {
        let theme = Theme::gruvbox_dark();
        let pane = PreviewPane::new();
        let mut c = ExpandControl::new();
        let mut t = Terminal::new(TestBackend::new(10, 3)).unwrap();
        t.draw(|f| c.paint(f, Some(Rect::new(0, 1, 6, 1)), &pane, &theme))
            .unwrap();
        assert!(c.hit_row(&press(5, 1)));
        assert!(!c.hit_row(&press(4, 1)));
        assert_eq!(c.hint(&pane, true), Some(hint(Reach::Grow)));
        assert_eq!(c.hint(&pane, false), None);
        c.clear();
        assert!(!c.hit_row(&press(5, 1)), "a cleared chevron is not hit");
        assert_eq!(c.hit_header(&pane, &press(5, 1)), None);
    }

    #[test]
    fn paint_draws_shrink_glyph() {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(4, 1)).unwrap();
        t.draw(|f| {
            paint(f, Rect::new(0, 0, 4, 1), Reach::Shrink, &theme);
        })
        .unwrap();
        assert_eq!(t.backend().buffer()[(3, 0)].symbol(), "‹");
    }

    #[test]
    fn paint_draws_nothing_on_a_zero_width_row() {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(4, 1)).unwrap();
        let mut got = Rect::new(9, 9, 9, 9);
        t.draw(|f| got = paint(f, Rect::new(2, 0, 0, 1), Reach::Grow, &theme))
            .unwrap();
        assert_eq!(got.width, 0);
        let buf = t.backend().buffer();
        assert!((0..4).all(|x| buf[(x, 0)].symbol() == " "));
    }
}
