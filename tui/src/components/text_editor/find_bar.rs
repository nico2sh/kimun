//! The **find bar**'s terminal drawing.
//!
//! The bar itself — its fields, pattern, status, current match, key map — is
//! the toolkit-free [`crate::logic::editor::find_bar::FindBar`], owned by the
//! logic [`crate::logic::editor::Editor`]. This module only paints it: one row
//! while finding, two once a **replace field** is revealed. The fields'
//! horizontal scroll and caret are terminal drawing state, kept here in
//! [`FindBarPaint`] rather than on the logic bar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::components::single_line_input::FieldView;
use crate::logic::editor::find_bar::{
    BarFocus, FIND_HINTS, FIND_PROMPT, FindBar, REPLACE_PROMPT, SearchStatus, StatusTone,
};
use crate::logic::editor::line_field::LineField;
use crate::settings::themes::Theme;

/// The terminal drawing state of the bar's two fields.
#[derive(Debug, Default)]
pub struct FindBarPaint {
    find: FieldView,
    replace: FieldView,
}

/// Render one prompt-plus-input row, returning the columns the prompt and
/// value consumed so the caller can place a tail after them.
fn render_bar_row(
    f: &mut Frame,
    rect: Rect,
    prompt: &str,
    field: &LineField,
    view: &mut FieldView,
    theme: &Theme,
    focused: bool,
) -> u16 {
    let base = theme.base_style();
    let prompt_cols = unicode_width::UnicodeWidthStr::width(prompt) as u16;
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            prompt,
            base.add_modifier(Modifier::BOLD),
        )))
        .style(base),
        Rect {
            width: prompt_cols.min(rect.width),
            ..rect
        },
    );
    view.render(f, field, rect, base, prompt_cols, focused);
    // Tail sits after the full value (in display columns, accounting for
    // wide/CJK chars), not after the caret — otherwise it would overlap the
    // trailing characters when the user moves the cursor mid-string.
    prompt_cols.saturating_add(field.display_width() as u16)
}

fn render_tail(f: &mut Frame, rect: Rect, consumed: u16, text: &str, style: Style) {
    let tail_rect = Rect {
        x: rect.x.saturating_add(consumed),
        width: rect.width.saturating_sub(consumed),
        ..rect
    };
    f.render_widget(Paragraph::new(text.to_string()).style(style), tail_rect);
}

/// Rows `bar` occupies: one while finding, two once a **replace field** is
/// revealed.
pub fn rows(bar: &FindBar) -> u16 {
    if bar.is_replacing() { 2 } else { 1 }
}

impl FindBarPaint {
    /// Draw `bar`. `rect` is one row while finding, two once a **replace
    /// field** is revealed: row one is the pattern and what it matches, row
    /// two is the replacement and what will happen to it.
    pub fn render(
        &mut self,
        bar: &FindBar,
        f: &mut Frame,
        rect: Rect,
        theme: &Theme,
        focused: bool,
    ) {
        let muted = Style::default()
            .fg(theme.gray.to_ratatui())
            .bg(theme.bg.to_ratatui());
        let err = Style::default()
            .fg(theme.red.to_ratatui())
            .bg(theme.bg.to_ratatui());

        let replacing = bar.is_replacing();
        let find_focused = focused && bar.focus() == BarFocus::Find;
        let find_row = Rect { height: 1, ..rect };

        let tone = |t: StatusTone| match t {
            StatusTone::Muted => muted,
            StatusTone::Error => err,
        };
        // Row 1 tail: what the pattern matches (the logic's wording), plus the
        // key hints after a match count while only finding.
        let tail: Option<(String, Style)> = bar.status_text().map(|(text, t)| {
            let hints = match bar.status() {
                SearchStatus::Match if !replacing => FIND_HINTS,
                _ => "",
            };
            (format!("  {text}{hints}"), tone(t))
        });

        let consumed = render_bar_row(
            f,
            find_row,
            FIND_PROMPT,
            bar.find_field(),
            &mut self.find,
            theme,
            find_focused,
        );
        if let Some((text, style)) = tail {
            render_tail(f, find_row, consumed, &text, style);
        }

        if !replacing || rect.height < 2 {
            return;
        }
        let replace_row = Rect {
            y: rect.y + 1,
            height: 1,
            ..rect
        };
        let replace_focused = focused && bar.focus() == BarFocus::Replace;
        let consumed = render_bar_row(
            f,
            replace_row,
            REPLACE_PROMPT,
            bar.replace_field().expect("replacing"),
            &mut self.replace,
            theme,
            replace_focused,
        );
        let (hints, t) = bar.replace_hint();
        render_tail(f, replace_row, consumed, hints, tone(t));
    }
}
