//! A row of clickable key-hint chips: `[Enter] Rename  [Esc] Cancel`.
//!
//! Each chip carries the key it advertises, and a click on it *is* that key:
//! [`HintRow::hit`] returns the `KeyEvent` and the host feeds it to its own
//! `handle_key`. One action path for both inputs, so a click can never do
//! something the key would not. Rects are recorded at render time and
//! hit-tested against later mouse events (same pattern as `ButtonRow`): no
//! render, no hits.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::settings::themes::Theme;

struct Chip {
    key: KeyEvent,
    /// `[key]` — drawn in the action colour when the chip is clickable.
    key_text: String,
    /// ` label` — drawn in the row (or override) style.
    label_text: String,
    enabled: bool,
    /// Replaces the row style for this chip when set.
    style: Option<Style>,
}

pub struct HintRow {
    chips: Vec<Chip>,
    /// Columns before the first chip.
    indent: u16,
    /// Columns between chips.
    gap: u16,
    /// Plain text drawn after the indent, before the first chip.
    prefix: String,
    rects: Vec<Rect>,
}

impl HintRow {
    /// Chips as `(key, key label, action label)`, drawn `[label] action`.
    /// A `KeyCode::Null` chip is informational (`[↑↓] Move`): drawn like the
    /// others, never clickable.
    /// Starts with the dialogs' usual two-column indent and gap.
    pub fn new(chips: &[(KeyCode, &str, &str)]) -> Self {
        Self {
            chips: chips
                .iter()
                .map(|(code, key, label)| Chip {
                    key: KeyEvent::new(*code, KeyModifiers::NONE),
                    key_text: format!("[{key}]"),
                    label_text: format!(" {label}"),
                    enabled: true,
                    style: None,
                })
                .collect(),
            indent: 2,
            gap: 2,
            prefix: String::new(),
            rects: Vec::new(),
        }
    }

    /// Override the gap between chips (some rows space them wider).
    pub fn with_gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Override the columns before the first chip.
    pub fn with_indent(mut self, indent: u16) -> Self {
        self.indent = indent;
        self
    }

    /// Plain, unclickable text before the first chip (`type a key · `).
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = prefix.to_string();
        self
    }

    /// Replace the prefix text (it can change per frame, e.g. a name).
    pub fn set_prefix(&mut self, prefix: &str) {
        self.prefix = prefix.to_string();
    }

    /// Give chip `idx` a modified key (e.g. `Shift+Enter`).
    pub fn with_modifiers(mut self, idx: usize, modifiers: KeyModifiers) -> Self {
        if let Some(c) = self.chips.get_mut(idx) {
            c.key.modifiers = modifiers;
        }
        self
    }

    /// A disabled chip draws dimmed and ignores clicks.
    pub fn set_enabled(&mut self, idx: usize, enabled: bool) {
        if let Some(c) = self.chips.get_mut(idx) {
            c.enabled = enabled;
        }
    }

    /// Draw chip `idx` with `style` instead of the row style (`None` resets).
    pub fn set_style(&mut self, idx: usize, style: Option<Style>) {
        if let Some(c) = self.chips.get_mut(idx) {
            c.style = style;
        }
    }

    /// Change chip `idx`'s action label, keeping its key.
    pub fn set_label(&mut self, idx: usize, key: &str, label: &str) {
        if let Some(c) = self.chips.get_mut(idx) {
            c.key_text = format!("[{key}]");
            c.label_text = format!(" {label}");
        }
    }

    /// Draw the chips left-aligned in `rect` with `style`; disabled chips
    /// get `style` dimmed. A clickable chip's `[key]` takes the theme's
    /// action colour and the chip registers for hover (see
    /// `components::clickable`); informational chips stay plain.
    pub fn render(&mut self, f: &mut Frame, rect: Rect, style: Style, theme: &Theme) {
        self.rects.clear();
        let mut spans = vec![Span::styled(" ".repeat(self.indent as usize), style)];
        let mut x = rect.x.saturating_add(self.indent);
        if !self.prefix.is_empty() {
            spans.push(Span::styled(self.prefix.clone(), style));
            x = x.saturating_add(self.prefix.width() as u16);
        }
        for (i, chip) in self.chips.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ".repeat(self.gap as usize), style));
                x = x.saturating_add(self.gap);
            }
            let w = (chip.key_text.width() + chip.label_text.width()) as u16;
            // Clip to the row: a chip past the right edge is not clickable.
            let r = Rect {
                x,
                y: rect.y,
                width: rect.right().saturating_sub(x).min(w),
                height: 1,
            };
            self.rects.push(r);
            let clickable = chip.enabled && chip.key.code != KeyCode::Null;
            if clickable {
                crate::components::clickable::register(r);
            }
            let base = chip.style.unwrap_or(style);
            let (key_style, label_style) = if !chip.enabled {
                let dim = base.add_modifier(Modifier::DIM);
                (dim, dim)
            } else if clickable {
                (base.patch(theme.action()), base)
            } else {
                (base, base)
            };
            spans.push(Span::styled(chip.key_text.clone(), key_style));
            spans.push(Span::styled(chip.label_text.clone(), label_style));
            x = x.saturating_add(w);
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(style), rect);
    }

    /// The key of the enabled chip under a left press, from the last render.
    pub fn hit(&self, m: &MouseEvent) -> Option<KeyEvent> {
        if !is_left_press(m) {
            return None;
        }
        let pos = Position::new(m.column, m.row);
        self.rects
            .iter()
            .zip(&self.chips)
            .find(|(r, c)| c.enabled && c.key.code != KeyCode::Null && r.contains(pos))
            .map(|(_, c)| c.key)
    }
}

/// A left-button press — the one mouse kind dialogs act on.
pub fn is_left_press(m: &MouseEvent) -> bool {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
}

/// The key of the `(rect, key)` target under a left press — for click
/// targets that are not chips (menu actions, launcher rows).
pub fn key_at(targets: &[(Rect, KeyCode)], m: &MouseEvent) -> Option<KeyEvent> {
    if !is_left_press(m) {
        return None;
    }
    crate::components::clickable::target_at(targets, m.column, m.row).map(KeyEvent::from)
}

/// The index of the ratatui `List` row under a left press: `rect` is where
/// the rows are drawn (inside any border), `offset` the list state's first
/// visible index, `len` the item count.
pub fn list_index_at(m: &MouseEvent, rect: Rect, offset: usize, len: usize) -> Option<usize> {
    if !is_left_press(m) || !rect.contains(Position::new(m.column, m.row)) {
        return None;
    }
    let idx = offset + (m.row - rect.y) as usize;
    (idx < len).then_some(idx)
}

/// A press of any button outside `rect` — how a read-only popup is dismissed.
/// An empty `rect` (not rendered yet) is never "outside".
pub fn is_press_outside(m: &MouseEvent, rect: Rect) -> bool {
    use ratatui::crossterm::event::MouseEventKind;
    matches!(m.kind, MouseEventKind::Down(_))
        && !rect.is_empty()
        && !rect.contains(Position::new(m.column, m.row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    fn press(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn drawn(row: &mut HintRow) -> String {
        let mut t = Terminal::new(TestBackend::new(40, 1)).unwrap();
        t.draw(|f| row.render(f, f.area(), Style::default(), &Theme::gruvbox_dark()))
            .unwrap();
        let buf = t.backend().buffer().clone();
        (0..40).map(|x| buf[(x, 0)].symbol().to_string()).collect()
    }

    #[test]
    fn draws_like_the_old_hint_text() {
        let mut row = HintRow::new(&[
            (KeyCode::Enter, "Enter", "Rename"),
            (KeyCode::Esc, "Esc", "Cancel"),
        ]);
        assert!(drawn(&mut row).starts_with("  [Enter] Rename  [Esc] Cancel"));
    }

    #[test]
    fn a_click_on_a_chip_is_its_key() {
        let mut row = HintRow::new(&[
            (KeyCode::Enter, "Enter", "Rename"),
            (KeyCode::Esc, "Esc", "Cancel"),
        ]);
        drawn(&mut row);
        assert_eq!(row.hit(&press(2, 0)).map(|k| k.code), Some(KeyCode::Enter));
        assert_eq!(row.hit(&press(15, 0)).map(|k| k.code), Some(KeyCode::Enter));
        assert_eq!(row.hit(&press(16, 0)), None, "the gap is not a chip");
        assert_eq!(row.hit(&press(18, 0)).map(|k| k.code), Some(KeyCode::Esc));
        assert_eq!(row.hit(&press(0, 0)), None, "the indent is not a chip");
    }

    #[test]
    fn a_disabled_chip_ignores_clicks() {
        let mut row = HintRow::new(&[(KeyCode::Enter, "Enter", "Rename")]);
        row.set_enabled(0, false);
        drawn(&mut row);
        assert_eq!(row.hit(&press(3, 0)), None);
    }

    #[test]
    fn modifiers_ride_along() {
        let mut row = HintRow::new(&[(KeyCode::Enter, "Shift+Enter", "Save & Open")])
            .with_modifiers(0, KeyModifiers::SHIFT);
        drawn(&mut row);
        assert_eq!(
            row.hit(&press(3, 0)).map(|k| k.modifiers),
            Some(KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn an_informational_chip_ignores_clicks() {
        let mut row = HintRow::new(&[(KeyCode::Null, "↑↓", "Move")]);
        drawn(&mut row);
        assert_eq!(row.hit(&press(3, 0)), None);
    }

    #[test]
    fn a_prefix_shifts_the_chips() {
        let mut row = HintRow::new(&[(KeyCode::Esc, "Esc", "Close")]).with_prefix("type · ");
        assert!(drawn(&mut row).starts_with("  type · [Esc] Close"));
        assert_eq!(row.hit(&press(3, 0)), None, "the prefix is inert");
        assert_eq!(row.hit(&press(9, 0)).map(|k| k.code), Some(KeyCode::Esc));
    }

    #[test]
    fn nothing_hits_before_render() {
        let row = HintRow::new(&[(KeyCode::Enter, "Enter", "Go")]);
        assert_eq!(row.hit(&press(3, 0)), None);
    }

    #[test]
    fn outside_press_needs_a_rendered_rect() {
        let rect = Rect::new(10, 10, 5, 5);
        assert!(is_press_outside(&press(0, 0), rect));
        assert!(!is_press_outside(&press(11, 11), rect));
        assert!(!is_press_outside(&press(0, 0), Rect::default()));
    }
}
