//! The shared expand control of the list screens: one glyph in the last
//! column of a row. `›` grows the row's reach, `‹` shrinks it.

use ratatui::{Frame, layout::Rect, text::Span, widgets::Paragraph};

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
