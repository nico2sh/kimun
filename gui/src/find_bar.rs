//! What the **find bar** paints, as data. The bar itself (fields, pattern,
//! status, key map, wording) is the logic [`FindBar`] the editor owns;
//! this module lays it out on the cell grid the way the TUI's
//! `components/text_editor/find_bar.rs` does: one row while finding (prompt,
//! query, what it matches), a second once the **replace field** is revealed
//! (prompt, replacement, its hints). Pure, so it is tested without a window;
//! the element shapes the rows like any text row and docks them above the
//! mode line.
//!
//! A field wider than its cells scrolls so its caret stays in view. The
//! IME preedit, which is not in the field yet, is drawn at the caret,
//! underlined, as the editor draws it at its own caret.

use kimun_notes::logic::editor::find_bar::{
    BarFocus, FIND_HINTS, FIND_PROMPT, FindBar, REPLACE_PROMPT, SearchStatus, StatusTone,
};
use kimun_notes::logic::editor::line_field::LineField;
use kimun_notes::logic::editor::markdown::runs::{Role, Run};
use kimun_notes::settings::themes::Theme;
use unicode_segmentation::UnicodeSegmentation;

use crate::row_paint::{PaintPiece, RowPaint, cells_of, layout_row};
use crate::theme::{self, CellStyle};

/// Cells left of the prompt: the mode line's inset, so the two docked rows
/// share one leading edge.
const PAD: u16 = 1;

/// The bar as painted: its rows top to bottom, and the caret.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarPaint {
    /// One row while finding, two while replacing.
    pub rows: Vec<RowPaint>,
    /// The caret as (column, bar row), when the window has focus.
    pub caret: Option<(u16, u16)>,
}

/// Rows `bar` takes: one while finding, two once a **replace field** is
/// revealed.
pub fn rows(bar: &FindBar) -> u16 {
    if bar.is_replacing() { 2 } else { 1 }
}

/// The field the keyboard drives: the one text input lands in.
pub fn focused_field(bar: &FindBar) -> &LineField {
    match (bar.focus(), bar.replace_field()) {
        (BarFocus::Replace, Some(field)) => field,
        _ => bar.find_field(),
    }
}

/// Lay `bar` out in `cols` cells. `preedit` is an IME composition in the
/// focused field; `focused` is whether the editor has the keyboard (else no
/// caret is drawn).
pub fn paint(
    bar: &FindBar,
    preedit: Option<&str>,
    theme: &Theme,
    cols: u16,
    focused: bool,
) -> BarPaint {
    let muted = text_style(theme::muted(theme));
    let tone = |t: StatusTone| match t {
        StatusTone::Muted => muted,
        StatusTone::Error => text_style(theme::error(theme)),
    };
    let replacing = bar.is_replacing();
    let find_focused = bar.focus() == BarFocus::Find || !replacing;

    // Row 1: the query and what it matches; the key hints follow a match
    // count while only finding (the TUI's tail).
    let find_tail = bar.status_text().map(|(text, t)| {
        let hints = match bar.status() {
            SearchStatus::Match if !replacing => FIND_HINTS,
            _ => "",
        };
        (format!("  {text}{hints}"), tone(t))
    });
    let mut out = BarPaint::default();
    let (row, caret) = field_row(
        FIND_PROMPT,
        bar.find_field(),
        preedit.filter(|_| find_focused),
        find_tail,
        theme,
        cols,
    );
    out.rows.push(row);
    if find_focused {
        out.caret = caret.map(|c| (c, 0));
    }

    if let Some(field) = bar.replace_field() {
        let (hint, t) = bar.replace_hint();
        let (row, caret) = field_row(
            REPLACE_PROMPT,
            field,
            preedit.filter(|_| !find_focused),
            Some((hint.to_string(), tone(t))),
            theme,
            cols,
        );
        out.rows.push(row);
        if !find_focused {
            out.caret = caret.map(|c| (c, 1));
        }
    }
    if !focused {
        out.caret = None;
    }
    out
}

fn text_style(fg: gpui_kit::Rgba) -> CellStyle {
    CellStyle {
        fg,
        bg: None,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
    }
}

/// One prompt + field + tail row, and the caret's column (`None` when the
/// field has no cell to show it in).
fn field_row(
    prompt: &str,
    field: &LineField,
    preedit: Option<&str>,
    tail: Option<(String, CellStyle)>,
    theme: &Theme,
    cols: u16,
) -> (RowPaint, Option<u16>) {
    let plain = text_style(theme::foreground(theme));
    let composing = CellStyle {
        underline: true,
        ..plain
    };
    let value = field.value();
    let (before, after) = value.split_at(field.cursor_byte());
    let preedit = preedit.unwrap_or("");

    let mut segments: Vec<(String, CellStyle)> = vec![
        (" ".repeat(usize::from(PAD)), plain),
        (
            prompt.to_string(),
            CellStyle {
                bold: true,
                ..plain
            },
        ),
    ];
    let field_col = PAD.saturating_add(cells_of(prompt));
    let avail = cols.saturating_sub(field_col);
    let caret_cells = cells_of(before).saturating_add(cells_of(preedit));
    let total = caret_cells.saturating_add(cells_of(after));
    // Scrolled just far enough that the caret's cell is the last one shown.
    let scroll = caret_cells.saturating_sub(avail.saturating_sub(1));
    let window = scroll..scroll.saturating_add(avail);
    let mut at: u16 = 0;
    for (part, style) in [(before, plain), (preedit, composing), (after, plain)] {
        let shown = visible(part, &mut at, &window);
        if !shown.is_empty() {
            segments.push((shown, style));
        }
    }
    let used = field_col.saturating_add(total.saturating_sub(scroll).min(avail));
    if let Some((text, style)) = tail {
        // A regex error spans several lines; a one-row tail shows the first,
        // as the TUI's one-row paragraph does.
        let first = text.lines().next().unwrap_or_default();
        let fit = fit_cells(first, cols.saturating_sub(used));
        if !fit.is_empty() {
            segments.push((fit, style));
        }
    }
    let caret = (avail > 0).then(|| field_col + (caret_cells - scroll));
    (styled_row(&segments), caret)
}

/// The part of `text` (starting `at` cells into the field) inside `window`,
/// advancing `at`. A wide cluster cut by the window's edge leaves blank cells,
/// so everything after it stays on its column.
fn visible(text: &str, at: &mut u16, window: &std::ops::Range<u16>) -> String {
    let mut out = String::new();
    for cluster in text.graphemes(true) {
        let w = cells_of(cluster);
        let (start, end) = (*at, at.saturating_add(w));
        *at = end;
        if start >= window.start && end <= window.end {
            out.push_str(cluster);
        } else {
            let lo = start.max(window.start);
            let hi = end.min(window.end);
            for _ in lo..hi.max(lo) {
                out.push(' ');
            }
        }
    }
    out
}

/// The leading clusters of `text` that fit in `cells`.
fn fit_cells(text: &str, cells: u16) -> String {
    let mut used = 0u16;
    let mut out = String::new();
    for cluster in text.graphemes(true) {
        let w = cells_of(cluster);
        if used.saturating_add(w) > cells {
            break;
        }
        used += w;
        out.push_str(cluster);
    }
    out
}

/// A row of styled text segments on the cell grid, cut into pieces exactly as
/// a text row is (wide clusters on their own), blank stretches left undrawn.
fn styled_row(segments: &[(String, CellStyle)]) -> RowPaint {
    let runs: Vec<Run> = segments
        .iter()
        .map(|(text, _)| Run::new(text.clone(), Role::Plain, 0))
        .collect();
    let layout = layout_row(&runs);
    let pieces = layout
        .pieces
        .into_iter()
        .filter_map(|p| {
            let style = segments[p.run].1;
            let blank = p.text.chars().all(|c| c == ' ');
            (!blank || style.underline).then_some(PaintPiece {
                col: p.col,
                cells: p.cells,
                text: p.text,
                grid: p.grid,
                style,
            })
        })
        .collect();
    RowPaint {
        pieces,
        ..RowPaint::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_notes::logic::editor::find_bar::REPLACE_HINTS;
    use kimun_notes::logic::editor::rope_buffer::RopeBuffer;
    use kimun_notes::logic::input::{Key, KeyInput, Mods};

    fn bar_on(text: &str, query: &str) -> (FindBar, RopeBuffer) {
        let mut buf = RopeBuffer::new(kimun_notes::ropetext::Text::from(text));
        let mut bar = FindBar::new();
        for c in query.chars() {
            bar.handle_key(KeyInput::plain(Key::Char(c)), &mut buf);
        }
        (bar, buf)
    }

    #[test]
    fn the_find_row_reads_like_the_tui_with_the_caret_after_the_query() {
        let t = Theme::default();
        let (bar, _) = bar_on("foo bar foo", "foo");
        let p = paint(&bar, None, &t, 120, true);
        assert_eq!(p.rows.len(), 1);
        assert_eq!(
            p.rows[0].text(),
            format!(" Find: foo  2 matches  any case{FIND_HINTS}")
        );
        // Prompt bold, count muted.
        let prompt = &p.rows[0].pieces[0];
        assert_eq!((prompt.text.as_str(), prompt.style.bold), ("Find: ", true));
        let tail = p.rows[0].pieces.last().unwrap();
        assert_eq!(tail.style.fg, theme::muted(&t));
        // " Find: " is 7 cells, then three typed.
        assert_eq!(p.caret, Some((10, 0)));
        assert_eq!(paint(&bar, None, &t, 120, false).caret, None);
    }

    #[test]
    fn no_match_and_invalid_regex_read_as_errors() {
        let t = Theme::default();
        let (bar, _) = bar_on("abc", "zz");
        let p = paint(&bar, None, &t, 80, true);
        assert_eq!(p.rows[0].text(), " Find: zz  no match  any case");
        assert_eq!(p.rows[0].pieces.last().unwrap().style.fg, theme::error(&t));
        let (bar, _) = bar_on("abc", "(");
        let row = paint(&bar, None, &t, 80, true).rows[0].text();
        assert!(row.starts_with(" Find: (  invalid regex: "), "{row}");
        // One row: the multi-line regex error is cut to its first line.
        assert!(!row.contains('\n'), "{row}");
    }

    #[test]
    fn the_replace_row_carries_the_caret_and_its_hints() {
        let t = Theme::default();
        let (mut bar, mut buf) = bar_on("foo", "foo");
        bar.handle_key(KeyInput::plain(Key::Tab), &mut buf);
        bar.handle_key(KeyInput::plain(Key::Char('x')), &mut buf);
        let p = paint(&bar, None, &t, 120, true);
        assert_eq!(p.rows.len(), 2);
        // No key hints on the find row while replacing.
        assert_eq!(p.rows[0].text(), " Find: foo  1 match  any case");
        assert_eq!(p.rows[1].text(), format!(" Replace: x{REPLACE_HINTS}"));
        assert_eq!(p.caret, Some((11, 1)));
        // Armed: the confirm, as an error.
        bar.handle_key(KeyInput::plain(Key::Backspace), &mut buf);
        bar.handle_key(KeyInput::new(Key::Char('a'), Mods::CTRL), &mut buf);
        let p = paint(&bar, None, &t, 120, true);
        let tail = p.rows[1].pieces.last().unwrap();
        assert!(tail.text.contains("delete every match?"), "{}", tail.text);
        assert_eq!(tail.style.fg, theme::error(&t));
    }

    #[test]
    fn a_long_query_scrolls_to_keep_the_caret_in_view_and_drops_the_tail() {
        let t = Theme::default();
        let (bar, _) = bar_on("x", "abcdefghij");
        // 7 cells of " Find: ", 5 for the field: the caret is the last cell.
        let p = paint(&bar, None, &t, 12, true);
        assert_eq!(p.rows[0].text(), " Find: ghij");
        assert_eq!(p.caret, Some((11, 0)));
    }

    #[test]
    fn a_preedit_is_drawn_underlined_at_the_caret_of_the_focused_field() {
        let t = Theme::default();
        let (mut bar, mut buf) = bar_on("x", "ab");
        bar.handle_key(KeyInput::plain(Key::Left), &mut buf);
        let p = paint(&bar, Some("に"), &t, 80, true);
        assert!(p.rows[0].text().starts_with(" Find: aにb"));
        let pre = p.rows[0].pieces.iter().find(|p| p.text == "に").unwrap();
        assert!(pre.style.underline);
        assert_eq!(pre.col, 8);
        // The caret sits after the two-cell preedit.
        assert_eq!(p.caret, Some((10, 0)));
    }
}
