//! What one **Editor frame** row paints, as data: glyph pieces placed on the
//! cell grid with their style, background fills, the strokes that stand in
//! for box-drawing glyphs, and the cells that are follow targets. Pure (no
//! GPUI text system), so every element of the markdown rendering is testable
//! without a window; the element shapes and paints it.
//!
//! Text is cut into [`Piece`]s so every glyph lands on its cell: a stretch of
//! one-cell clusters is shaped with GPUI's `force_width`, which pins each base
//! glyph to the cell advance; a wide (or zero-width-led) cluster is shaped on
//! its own and placed at its cell, since a forced width would squeeze a
//! two-cell glyph into one. Widths come from the logic layer's own
//! `cluster_display_width`, so painting and hit-testing agree with the layout.

use gpui_kit::Rgba;
use kimun_notes::logic::editor::markdown::cluster_display_width;
use kimun_notes::logic::editor::markdown::runs::{Role, Run};
use kimun_notes::logic::editor::overlay::OverlayKind;
use kimun_notes::settings::themes::Theme;
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::{self, CellStyle};

/// The blockquote gutter glyph the logic draws per quote depth.
const QUOTE_BAR: &str = "│";

/// A stretch of one frame row shaped and painted in one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// First cell, from the pane's left edge.
    pub col: u16,
    /// Cells covered.
    pub cells: u16,
    pub text: String,
    /// Index of the [`Run`] the text comes from (its style).
    pub run: usize,
    /// Every cluster is one cell wide: shape with the cell width forced.
    pub grid: bool,
}

/// One frame row cut into [`Piece`]s, plus each run's cell span `(col, cells)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowLayout {
    pub spans: Vec<(u16, u16)>,
    pub pieces: Vec<Piece>,
}

/// Lay out one row's runs on the cell grid. Zero-width clusters (combining
/// marks, ZWJ tails) ride on the piece before them.
pub fn layout_row(runs: &[Run]) -> RowLayout {
    let mut out = RowLayout::default();
    let mut col: u16 = 0;
    for (ix, run) in runs.iter().enumerate() {
        let start = col;
        // The piece a following zero-width cluster attaches to, and whether a
        // one-cell cluster may extend it.
        let mut open: Option<usize> = None;
        let mut extendable = false;
        for cluster in run.text.graphemes(true) {
            let width = cluster_cells(cluster);
            match (width, open) {
                (0, Some(p)) => out.pieces[p].text.push_str(cluster),
                (1, Some(p)) if extendable => {
                    out.pieces[p].text.push_str(cluster);
                    out.pieces[p].cells += 1;
                }
                _ => {
                    out.pieces.push(Piece {
                        col,
                        cells: width,
                        text: cluster.to_string(),
                        run: ix,
                        grid: width == 1,
                    });
                    open = Some(out.pieces.len() - 1);
                    extendable = width == 1;
                }
            }
            col = col.saturating_add(width);
        }
        out.spans.push((start, col - start));
    }
    out
}

/// The cluster drawn at cell `col` of a row: where it starts, how many cells
/// it covers, its text, and the run it comes from. `None` past the row's end
/// (the cursor after the last char) or between runs that leave a gap.
pub fn cluster_at(runs: &[Run], col: u16) -> Option<(u16, u16, String, usize)> {
    let mut at: u16 = 0;
    for (ix, run) in runs.iter().enumerate() {
        for cluster in run.text.graphemes(true) {
            let width = cluster_cells(cluster);
            if width > 0 && (at..at.saturating_add(width)).contains(&col) {
                return Some((at, width, cluster.to_string(), ix));
            }
            at = at.saturating_add(width);
        }
    }
    None
}

/// Cells `text` covers.
pub fn cells_of(text: &str) -> u16 {
    let n: usize = text.graphemes(true).map(cluster_display_width).sum();
    u16::try_from(n).unwrap_or(u16::MAX)
}

fn cluster_cells(cluster: &str) -> u16 {
    u16::try_from(cluster_display_width(cluster)).unwrap_or(u16::MAX)
}

/// A styled stretch of cells: `cells` cells from `col`, in `color`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellSpan {
    pub col: u16,
    pub cells: u16,
    pub color: Rgba,
}

/// A glyph piece and the style it paints in.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintPiece {
    pub col: u16,
    pub cells: u16,
    pub text: String,
    /// One-cell clusters only: shaped with the cell width forced.
    pub grid: bool,
    pub style: CellStyle,
}

/// Everything one frame row paints. Built once per distinct row content (the
/// element caches it with the shaped text).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RowPaint {
    /// Glyphs, in row order. Whitespace with nothing to draw (code-box pad,
    /// gutter spaces) is left out.
    pub pieces: Vec<PaintPiece>,
    /// Cell backgrounds (selection, code box, inline code), adjacent
    /// same-colour runs merged so a box has no seams.
    pub fills: Vec<CellSpan>,
    /// Horizontal rules: a hairline across the cells, at the row's centre,
    /// in place of the `─` glyphs (which never join cleanly).
    pub rules: Vec<CellSpan>,
    /// Blockquote bars: one per `│` cell, a stroke the full row height, so
    /// consecutive quote rows read as one bar.
    pub bars: Vec<CellSpan>,
    /// Cells that draw a follow target (link, wikilink, image, label): the
    /// pointer is a hand over them.
    pub targets: Vec<(u16, u16)>,
}

impl RowPaint {
    pub fn new(runs: &[Run], theme: &Theme) -> Self {
        let layout = layout_row(runs);
        let styles: Vec<CellStyle> = runs.iter().map(|r| theme::run_style(r, theme)).collect();
        let mut out = RowPaint::default();
        for ((run, style), &(col, cells)) in runs.iter().zip(&styles).zip(&layout.spans) {
            if cells == 0 {
                continue;
            }
            if let Some(bg) = style.bg {
                push_merged(
                    &mut out.fills,
                    CellSpan {
                        col,
                        cells,
                        color: bg,
                    },
                );
            }
            match run.role {
                Role::Rule => out.rules.push(CellSpan {
                    col,
                    cells,
                    color: style.fg,
                }),
                Role::BlockquoteBar => {
                    let mut at = col;
                    for cluster in run.text.graphemes(true) {
                        if cluster == QUOTE_BAR {
                            out.bars.push(CellSpan {
                                col: at,
                                cells: 1,
                                color: style.fg,
                            });
                        }
                        at = at.saturating_add(cluster_cells(cluster));
                    }
                }
                _ => {}
            }
            // The logic marks the whole drawn link, emphasis in its label
            // included (`OverlayKind::FollowTarget`).
            if run.overlays.contains(OverlayKind::FollowTarget) {
                match out.targets.last_mut() {
                    Some((c, n)) if *c + *n == col => *n += cells,
                    _ => out.targets.push((col, cells)),
                }
            }
        }
        for piece in layout.pieces {
            let run = &runs[piece.run];
            let style = styles[piece.run];
            // Strokes stand in for these glyphs.
            if matches!(run.role, Role::Rule | Role::BlockquoteBar) {
                continue;
            }
            // Blank and undecorated: nothing to draw over the fill.
            if piece.text.chars().all(|c| c == ' ') && !style.strike && !style.underline {
                continue;
            }
            out.pieces.push(PaintPiece {
                col: piece.col,
                cells: piece.cells,
                text: piece.text,
                grid: piece.grid,
                style,
            });
        }
        out
    }

    /// The background at cell `col`, if a fill covers it.
    pub fn fill_at(&self, col: u16) -> Option<Rgba> {
        self.fills
            .iter()
            .find(|f| (f.col..f.col.saturating_add(f.cells)).contains(&col))
            .map(|f| f.color)
    }

    /// The row's glyph text as painted, a space for each cell without one
    /// (tests: what a reader sees on this row).
    #[cfg(test)]
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut col = 0;
        for p in &self.pieces {
            while col < p.col {
                out.push(' ');
                col += 1;
            }
            out.push_str(&p.text);
            col = p.col + p.cells;
        }
        out
    }
}

fn push_merged(spans: &mut Vec<CellSpan>, span: CellSpan) {
    match spans.last_mut() {
        Some(last) if last.color == span.color && last.col + last.cells == span.col => {
            last.cells += span.cells;
        }
        _ => spans.push(span),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_notes::logic::editor::markdown::ElementKind;
    use kimun_notes::settings::themes::ThemeColor;

    fn plain(text: &str) -> Run {
        Run::new(text, Role::Plain, 0)
    }

    #[test]
    fn ascii_runs_are_one_grid_piece_each() {
        let l = layout_row(&[plain("ab"), Run::new("cd", Role::Sigil, 0)]);
        assert_eq!(l.spans, vec![(0, 2), (2, 2)]);
        assert_eq!(
            l.pieces,
            vec![
                Piece {
                    col: 0,
                    cells: 2,
                    text: "ab".into(),
                    run: 0,
                    grid: true
                },
                Piece {
                    col: 2,
                    cells: 2,
                    text: "cd".into(),
                    run: 1,
                    grid: true
                },
            ]
        );
    }

    #[test]
    fn wide_clusters_get_their_own_piece_at_their_cell() {
        let l = layout_row(&[plain("a理b")]);
        assert_eq!(l.spans, vec![(0, 4)]);
        let got: Vec<_> = l
            .pieces
            .iter()
            .map(|p| (p.col, p.cells, p.text.as_str(), p.grid))
            .collect();
        assert_eq!(
            got,
            vec![(0, 1, "a", true), (1, 2, "理", false), (3, 1, "b", true)]
        );
    }

    #[test]
    fn zero_width_clusters_ride_on_the_previous_piece() {
        let l = layout_row(&[plain("a\u{200B}b")]);
        assert_eq!(l.spans, vec![(0, 2)]);
        assert_eq!(l.pieces.len(), 1);
        assert_eq!(l.pieces[0].text, "a\u{200B}b");
        assert_eq!(l.pieces[0].cells, 2);
    }

    #[test]
    fn emoji_sequences_are_one_two_cell_piece() {
        let l = layout_row(&[plain("👨\u{200D}👩x")]);
        assert_eq!(l.spans, vec![(0, 3)]);
        assert_eq!((l.pieces[0].col, l.pieces[0].cells), (0, 2));
        assert_eq!((l.pieces[1].col, l.pieces[1].text.as_str()), (2, "x"));
    }

    #[test]
    fn cluster_at_finds_the_cell_under_the_caret() {
        let runs = [plain("a理"), Run::new("e\u{301}", Role::Sigil, 0)];
        assert_eq!(cluster_at(&runs, 0), Some((0, 1, "a".into(), 0)));
        assert_eq!(cluster_at(&runs, 1), Some((1, 2, "理".into(), 0)));
        assert_eq!(cluster_at(&runs, 2), Some((1, 2, "理".into(), 0)));
        assert_eq!(cluster_at(&runs, 3), Some((3, 1, "e\u{301}".into(), 1)));
        assert_eq!(cluster_at(&runs, 4), None);
        assert_eq!(cluster_at(&[], 0), None);
    }

    #[test]
    fn empty_runs_have_an_empty_span() {
        let l = layout_row(&[plain(""), plain("x")]);
        assert_eq!(l.spans, vec![(0, 0), (0, 1)]);
        assert_eq!(l.pieces.len(), 1);
    }

    // ---- RowPaint ----

    fn theme() -> Theme {
        Theme {
            bg: ThemeColor::Rgb(0, 0, 0),
            fg: ThemeColor::Rgb(200, 200, 200),
            gray: ThemeColor::Rgb(100, 100, 100),
            blockquote_bar: ThemeColor::Rgb(90, 90, 200),
            code_bg: ThemeColor::Rgb(30, 30, 30),
            bg_soft: ThemeColor::Rgb(40, 40, 40),
            selection_bg: ThemeColor::Rgb(60, 60, 90),
            ..Theme::default()
        }
    }

    fn boxed(text: &str, role: Role) -> Run {
        let mut r = Run::new(text, role, 0);
        r.code_box = true;
        r
    }

    #[test]
    fn a_code_box_row_is_one_seamless_fill_and_its_pad_draws_no_glyphs() {
        let t = theme();
        let row = RowPaint::new(
            &[
                boxed("fn x()", Role::CodeText),
                boxed("    ", Role::CodeBoxPad),
            ],
            &t,
        );
        let code = theme::to_gpui(&t.code_bg, theme::Token::Background, &t);
        assert_eq!(
            row.fills,
            vec![CellSpan {
                col: 0,
                cells: 10,
                color: code
            }]
        );
        assert_eq!(row.text(), "fn x()");
        assert_eq!(row.fill_at(9), Some(code));
        assert_eq!(row.fill_at(10), None);
    }

    #[test]
    fn a_selection_splits_the_box_fill_where_it_lies() {
        let t = theme();
        let mut sel = boxed("x()", Role::CodeText);
        sel.overlays.insert(OverlayKind::Selection);
        let row = RowPaint::new(
            &[
                boxed("fn ", Role::CodeText),
                sel,
                boxed("  ", Role::CodeBoxPad),
            ],
            &t,
        );
        let cols: Vec<_> = row.fills.iter().map(|f| (f.col, f.cells)).collect();
        assert_eq!(cols, vec![(0, 3), (3, 3), (6, 2)]);
        assert_eq!(row.fills[1].color, theme::selection_background(&t));
    }

    #[test]
    fn a_rule_is_a_stroke_across_its_cells_not_glyphs() {
        let t = theme();
        let row = RowPaint::new(&[Run::new("─".repeat(12), Role::Rule, 0)], &t);
        assert!(row.pieces.is_empty());
        assert_eq!(
            row.rules,
            vec![CellSpan {
                col: 0,
                cells: 12,
                color: theme::to_gpui(&t.gray, theme::Token::Foreground, &t)
            }]
        );
    }

    #[test]
    fn quote_bars_are_strokes_one_per_depth() {
        let t = theme();
        let row = RowPaint::new(
            &[
                Run::new("││ ", Role::BlockquoteBar, 0),
                Run::new("quote", Role::Element(ElementKind::Blockquote), 0),
            ],
            &t,
        );
        let bar = theme::to_gpui(&t.blockquote_bar, theme::Token::Foreground, &t);
        assert_eq!(
            row.bars,
            vec![
                CellSpan {
                    col: 0,
                    cells: 1,
                    color: bar
                },
                CellSpan {
                    col: 1,
                    cells: 1,
                    color: bar
                }
            ]
        );
        // The text starts after the gutter; no bar glyph is drawn.
        assert_eq!(row.text(), "   quote");
        assert_eq!(row.pieces.len(), 1);
    }

    #[test]
    fn follow_targets_come_from_the_overlay_and_links_are_underlined() {
        let t = theme();
        let target = |text: &str, role: Role| {
            let mut r = Run::new(text, role, 0);
            r.overlays.insert(OverlayKind::FollowTarget);
            r
        };
        let row = RowPaint::new(
            &[
                plain("A "),
                target("link", Role::Element(ElementKind::Link)),
                plain(" "),
                // Bold inside a link label: the role is Bold, the cell a target.
                target("Wi", Role::Element(ElementKind::Bold)),
                target("ki", Role::Element(ElementKind::WikiLink)),
                target("#tag", Role::Element(ElementKind::Label)),
                // A link role without the overlay (revealed elsewhere) is not.
                Run::new("x", Role::Element(ElementKind::Link), 0),
            ],
            &t,
        );
        // Adjacent targets merge into one hand-cursor span.
        assert_eq!(row.targets, vec![(2, 4), (7, 8)]);
        let link = row.pieces.iter().find(|p| p.text == "link").unwrap();
        assert!(link.style.underline && !link.style.strike);
    }

    #[test]
    fn struck_blanks_keep_their_piece_and_plain_blanks_do_not() {
        let t = theme();
        let mut done = plain("  ");
        done.overlays.insert(OverlayKind::TaskDone);
        let row = RowPaint::new(&[plain("a"), plain("   "), done], &t);
        let texts: Vec<_> = row.pieces.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "  "]);
        assert_eq!(row.pieces[1].col, 4);
    }
}
