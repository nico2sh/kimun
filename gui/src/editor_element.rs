//! The **Edit surface**'s painter: a custom GPUI [`Element`] that fills its
//! pane, sizes the editor's cell grid to it, and paints the **Editor frame**
//! cell-exactly in a monospace face (ADR-0049 §5).
//!
//! Text is shaped in [`Piece`]s so every glyph lands on its cell: a stretch of
//! one-cell clusters is shaped with GPUI's `force_width`, which pins each base
//! glyph to the cell advance; a wide (or zero-width-led) cluster is shaped on
//! its own and placed at its cell, since a forced width would squeeze a
//! two-cell glyph into one. Widths come from the logic layer's own
//! `cluster_display_width`, so painting and hit-testing agree with the layout.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler, Entity, Font,
    FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Rgba, ScrollEvent,
    ShapedLine, SharedString, StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle,
    Window, fill, font, point, px, relative, size,
};
use kimun_notes::logic::editor::markdown::cluster_display_width;
use kimun_notes::logic::editor::markdown::runs::Run;
use unicode_segmentation::UnicodeSegmentation;

use crate::editor_view::{EditorView, Painted};
use crate::grid::CellMetrics;
use crate::theme::{self, CellStyle};

/// Row height as a multiple of the font size; the result is rounded to whole
/// pixels so rows tile without seams.
const LINE_HEIGHT: f32 = 1.5;

/// Caret bar thickness. A physical stroke, not layout, so a fixed pixel size
/// (the design guides' hairline exception).
const CARET_WIDTH: f32 = 2.0;

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

/// One frame row cut into [`Piece`]s, plus each run's cell span `(col, cells)`
/// for its background.
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
            let width = u16::try_from(cluster_display_width(cluster)).unwrap_or(u16::MAX);
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

/// The face a style paints in.
fn styled_font(base: &Font, style: &CellStyle) -> Font {
    let mut f = base.clone();
    if style.bold {
        f.weight = FontWeight::BOLD;
    }
    if style.italic {
        f.style = FontStyle::Italic;
    }
    f
}

/// Paints one [`EditorView`]. Built fresh each frame by the view's `render`.
pub struct EditorElement {
    view: Entity<EditorView>,
}

impl EditorElement {
    pub fn new(view: Entity<EditorView>) -> Self {
        Self { view }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Everything `paint` draws, resolved in `prepaint`.
pub struct EditorPrepaint {
    hitbox: Hitbox,
    cells: CellMetrics,
    line_height: Pixels,
    background: Rgba,
    /// Cell-aligned backgrounds (selection, code box, inline code).
    fills: Vec<(Bounds<Pixels>, Rgba)>,
    lines: Vec<(gpui_kit::Point<Pixels>, ShapedLine)>,
    /// The IME preedit: the cells it covers, their background (the caret
    /// cell's, so it reads as part of the line it is typed into), and its
    /// underline-styled text.
    preedit: Option<(Bounds<Pixels>, Rgba, ShapedLine)>,
    caret: Option<(Bounds<Pixels>, Rgba)>,
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Fill the pane; the grid is whatever fits.
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // The kit theme supplies the installed monospace face and its size;
        // every colour comes from the kimün theme (ADR-0049 §6).
        let (family, font_size) = {
            let t = cx.theme();
            (t.mono_font_family.clone(), t.mono_font_size)
        };
        let line_height = px((f32::from(font_size) * LINE_HEIGHT).round());
        let base_font = font(family);
        let cells =
            CellMetrics::from_text_system(window.text_system(), &base_font, font_size, line_height);
        let (cols, rows) = cells.fit(bounds.size.width.into(), bounds.size.height.into());
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let focused = self.view.read(cx).focus_handle.is_focused(window);
        let origin = bounds.origin;
        let cell_bounds = |col: u16, row: u16, n: u16| {
            Bounds::new(
                point(
                    origin.x + px(cells.x_of(col)),
                    origin.y + px(cells.y_of(row)),
                ),
                size(px(cells.width() * f32::from(n)), line_height),
            )
        };

        self.view.update(cx, |view, _| {
            view.editor.prepare_frame(cols, rows);
            let frame = view.editor.session().display.frame(cols, focused);
            let theme = &view.theme;
            let text_system = window.text_system().clone();

            let mut fills = Vec::new();
            let mut lines = Vec::new();
            // Background under the caret cell, for the preedit's backing.
            let mut caret_bg = None;
            for (r, row) in frame.rows.iter().enumerate() {
                let Ok(r) = u16::try_from(r) else { break };
                let layout = layout_row(&row.runs);
                let styles: Vec<CellStyle> = row
                    .runs
                    .iter()
                    .map(|run| theme::run_style(run, theme))
                    .collect();
                for (style, &(col, n)) in styles.iter().zip(&layout.spans) {
                    if let Some((cc, cr)) = frame.cursor
                        && cr == r
                        && (col..col.saturating_add(n)).contains(&cc)
                    {
                        caret_bg = style.bg;
                    }
                    if let (Some(bg), true) = (style.bg, n > 0) {
                        fills.push((cell_bounds(col, r, n), bg));
                    }
                }
                for piece in layout.pieces {
                    let style = &styles[piece.run];
                    let color = style.fg.into();
                    let text_run = TextRun {
                        len: piece.text.len(),
                        font: styled_font(&base_font, style),
                        color,
                        background_color: None,
                        underline: style.underline.then_some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(color),
                            wavy: false,
                        }),
                        strikethrough: style.strike.then_some(StrikethroughStyle {
                            thickness: px(1.),
                            color: Some(color),
                        }),
                    };
                    let force = piece.grid.then(|| px(cells.width()));
                    let shaped = text_system.shape_line(
                        SharedString::from(piece.text),
                        font_size,
                        &[text_run],
                        force,
                    );
                    lines.push((cell_bounds(piece.col, r, 0).origin, shaped));
                }
            }

            let caret_cell = frame
                .cursor
                .filter(|&(_, r)| r < rows)
                .map(|(c, r)| cell_bounds(c, r, 1));
            let preedit = view.preedit.as_deref().zip(caret_cell).map(|(text, at)| {
                let n = text
                    .graphemes(true)
                    .map(cluster_display_width)
                    .sum::<usize>();
                let n = u16::try_from(n).unwrap_or(u16::MAX);
                let fg = theme::foreground(theme).into();
                let run = TextRun {
                    len: text.len(),
                    font: base_font.clone(),
                    color: fg,
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(fg),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let shaped = text_system.shape_line(
                    SharedString::from(text.to_string()),
                    font_size,
                    &[run],
                    None,
                );
                let mut covered = at;
                covered.size.width = px(cells.width() * f32::from(n));
                let backing = caret_bg.unwrap_or_else(|| theme::background(theme));
                (covered, backing, shaped)
            });
            // The caret sits after the preedit while composing.
            let caret = caret_cell.map(|mut at| {
                if let Some((covered, _, _)) = &preedit {
                    at.origin.x += covered.size.width;
                }
                at.size.width = px(CARET_WIDTH);
                (at, theme::caret(theme))
            });
            view.painted = Some(Painted {
                origin,
                cells,
                caret: caret_cell,
            });
            EditorPrepaint {
                hitbox,
                cells,
                line_height,
                background: theme::background(theme),
                fills,
                lines,
                preedit,
                caret,
            }
        })
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.view.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, prepaint.background));
            for (at, color) in &prepaint.fills {
                window.paint_quad(fill(*at, *color));
            }
            for (origin, line) in &prepaint.lines {
                // A glyph the face cannot raster is skipped, not fatal.
                line.paint(
                    *origin,
                    prepaint.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
            }
            if let Some((covered, backing, line)) = &prepaint.preedit {
                window.paint_quad(fill(*covered, *backing));
                line.paint(
                    covered.origin,
                    prepaint.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
            }
            if let Some((at, color)) = prepaint.caret {
                window.paint_quad(fill(at, color));
            }
        });

        let origin = bounds.origin;
        let cells = prepaint.cells;
        window.on_mouse_event({
            let view = self.view.clone();
            let hitbox = prepaint.hitbox.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase.bubble() && hitbox.is_hovered(window) {
                    view.update(cx, |v, cx| {
                        v.pointer_down(event, origin, &cells, window, cx)
                    });
                    cx.stop_propagation();
                }
            }
        });
        // Move and release are taken anywhere: a drag that leaves the pane
        // keeps selecting, and its release must end it.
        window.on_mouse_event({
            let view = self.view.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    view.update(cx, |v, cx| {
                        v.pointer_move(event, origin, &cells, window, cx)
                    });
                }
            }
        });
        window.on_mouse_event({
            let view = self.view.clone();
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() {
                    view.update(cx, |v, cx| v.pointer_up(event, origin, &cells, window, cx));
                }
            }
        });
        window.on_mouse_event({
            let view = self.view.clone();
            let hitbox = prepaint.hitbox.clone();
            move |event: &ScrollEvent, phase, window, cx| {
                if phase.bubble() && hitbox.should_handle_scroll(window) {
                    view.update(cx, |v, cx| {
                        v.scroll_wheel(event, origin, &cells, window, cx)
                    });
                    cx.stop_propagation();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_notes::logic::editor::markdown::runs::Role;

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
        // ZWSP after "a" stays in the "a" piece; the next char starts a new
        // one-cell stretch only if the piece is still extendable.
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
    fn empty_runs_have_an_empty_span() {
        let l = layout_row(&[plain(""), plain("x")]);
        assert_eq!(l.spans, vec![(0, 0), (0, 1)]);
        assert_eq!(l.pieces.len(), 1);
    }
}
